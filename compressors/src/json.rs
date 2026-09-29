use regex::Regex;
use serde_json::Value;
use std::sync::LazyLock;
use tracing::debug;

use crate::anomaly::detect_anomalies;

/// Key in the elision marker: number of elements dropped.
pub const ELIDED_KEY: &str = "__retoken_elided__";

/// Key for human-readable note on the first elision marker of a payload.
pub const ELIDED_NOTE_KEY: &str = "__retoken_note__";

/// Matches object keys whose subtrees carry error/warning signal and must be
/// preserved verbatim. Ported from legacy `errorKeyRe`.
static ERROR_KEY_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)^(error|errors|message|msg|stack|stacktrace|stack_trace|trace|traceback|exception|reason|detail|details|warning|warnings)$")
        .expect("ERROR_KEY_RE must compile")
});

/// Matches critical keywords anywhere in an element's canonical JSON, so items
/// in an error/failure state are force-kept regardless of position. Ported from
/// legacy `errorValueRe` and extended.
static ERROR_VALUE_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)\b(error|errors|exception|failed|failure|critical|fatal|crash|panic|abort|timeout|denied|rejected|refused|broken|segfault)\b")
        .expect("ERROR_VALUE_RE must compile")
});

/// Configuration for the JSON array compressor.
pub struct JsonCompressorConfig {
    /// Arrays longer than this are eligible for collapse.
    pub array_threshold: usize,
    /// Elements always kept from the front.
    pub keep_first: usize,
    /// Elements always kept from the back.
    pub keep_last: usize,
    /// Robust |z| (MAD-based) threshold for anomaly detection.
    pub anomaly_z: f64,
}

impl Default for JsonCompressorConfig {
    fn default() -> Self {
        Self {
            array_threshold: 8,
            keep_first: 3,
            keep_last: 2,
            anomaly_z: 2.0,
        }
    }
}

/// Compresses a JSON value in-place, collapsing repetitive arrays while
/// preserving structure, error subtrees, and statistical anomalies.
///
/// This is a Rust reimplementation of Caveman's `jsonCompressor`, improved with:
/// - Proper MAD-based anomaly detection on all numeric leaf values
/// - Recursive error-key preservation
/// - Elision markers that are valid JSON objects
/// - Idempotency (already-elided arrays pass through)
pub fn compress_json_heuristic(value: &Value) -> Value {
    let config = JsonCompressorConfig::default();
    let mut collapsed = false;
    transform(value, false, &config, &mut collapsed)
}

/// Returns (compressed_value, did_compress).
pub fn compress_json_heuristic_tracked(value: &Value) -> (Value, bool) {
    let config = JsonCompressorConfig::default();
    let mut collapsed = false;
    let result = transform(value, false, &config, &mut collapsed);
    (result, collapsed)
}

/// Recursively transforms the value. `preserve` is set inside error/message subtrees.
fn transform(v: &Value, preserve: bool, config: &JsonCompressorConfig, collapsed: &mut bool) -> Value {
    match v {
        Value::Object(map) => {
            let mut out = serde_json::Map::with_capacity(map.len());
            for (k, val) in map {
                let child_preserve = preserve || ERROR_KEY_RE.is_match(k);
                out.insert(k.clone(), transform(val, child_preserve, config, collapsed));
            }
            Value::Object(out)
        }
        Value::Array(arr) => {
            // Check for already-elided markers (idempotency)
            if contains_elided_marker(arr) {
                return v.clone();
            }
            if preserve {
                return v.clone(); // keep error/message arrays in full
            }
            if arr.len() <= config.array_threshold {
                // Recurse into children but don't collapse
                Value::Array(
                    arr.iter()
                        .map(|e| transform(e, preserve, config, collapsed))
                        .collect(),
                )
            } else {
                select_array(arr, config, collapsed)
            }
        }
        _ => v.clone(),
    }
}

/// Returns true if the array already contains a ReToken elision marker.
fn contains_elided_marker(arr: &[Value]) -> bool {
    arr.iter().any(|e| {
        e.as_object()
            .map(|o| o.contains_key(ELIDED_KEY))
            .unwrap_or(false)
    })
}

/// Collapses a long array, keeping signal-carrying elements and emitting
/// elision markers for dropped runs.
fn select_array(arr: &[Value], config: &JsonCompressorConfig, collapsed: &mut bool) -> Value {
    let n = arr.len();

    // Canonical per-element JSON strings for pattern matching
    let docs: Vec<String> = arr
        .iter()
        .map(|e| serde_json::to_string(e).unwrap_or_default())
        .collect();

    let mut keep = vec![false; n];

    // 1. Head/tail anchors
    for item in keep.iter_mut().take(config.keep_first.min(n)) {
        *item = true;
    }
    for item in keep.iter_mut().take(n).skip(n.saturating_sub(config.keep_last)) {
        *item = true;
    }

    // 2. Error-state elements (force-kept)
    for (i, doc) in docs.iter().enumerate() {
        if ERROR_VALUE_RE.is_match(doc) {
            keep[i] = true;
        }
    }

    // 3. Statistical anomalies (MAD-based robust z-score)
    let anomaly_indices = detect_anomalies(arr, &docs, config.anomaly_z);
    for i in anomaly_indices {
        if i < n {
            keep[i] = true;
        }
    }

    // Check if we actually drop anything
    let dropped_count = keep.iter().filter(|&&k| !k).count();
    if dropped_count == 0 {
        return Value::Array(arr.to_vec());
    }

    *collapsed = true;

    // Build output with elision markers for dropped runs
    let mut output: Vec<Value> = Vec::with_capacity(n);
    let mut elided_run = 0usize;
    let mut first_marker = true;

    for (i, element) in arr.iter().enumerate() {
        if keep[i] {
            if elided_run > 0 {
                let mut marker = serde_json::Map::new();
                marker.insert(ELIDED_KEY.to_string(), Value::Number(elided_run.into()));
                if first_marker {
                    marker.insert(
                        ELIDED_NOTE_KEY.to_string(),
                        Value::String("Elements elided by ReToken. Original recoverable via retoken_retrieve tool.".to_string()),
                    );
                    first_marker = false;
                }
                output.push(Value::Object(marker));
                elided_run = 0;
            }
            output.push(element.clone());
        } else {
            elided_run += 1;
        }
    }
    if elided_run > 0 {
        let mut marker = serde_json::Map::new();
        marker.insert(ELIDED_KEY.to_string(), Value::Number(elided_run.into()));
        if first_marker {
            marker.insert(
                ELIDED_NOTE_KEY.to_string(),
                Value::String("Elements elided by ReToken. Original recoverable via retoken_retrieve tool.".to_string()),
            );
        }
        output.push(Value::Object(marker));
    }

    debug!(
        "JSON compressor: {} elements -> {} kept + markers ({} dropped)",
        n,
        keep.iter().filter(|&&k| k).count(),
        dropped_count
    );

    Value::Array(output)
}


/// Original simple compressor (null stripping) preserved for backward compatibility.
pub fn compress_json_simple(value: &Value) -> String {
    let cleaned = strip_nulls(value);
    serde_json::to_string(&cleaned).unwrap_or_else(|_| value.to_string())
}

/// Recursively removes null values from JSON objects.
fn strip_nulls(value: &Value) -> Value {
    match value {
        Value::Object(map) => {
            let cleaned: serde_json::Map<String, Value> = map
                .iter()
                .filter(|(_, v)| !v.is_null())
                .map(|(k, v)| (k.clone(), strip_nulls(v)))
                .collect();
            Value::Object(cleaned)
        }
        Value::Array(arr) => Value::Array(arr.iter().map(strip_nulls).collect()),
        other => other.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn small_arrays_pass_through() {
        let input = json!([1, 2, 3, 4, 5]);
        let result = compress_json_heuristic(&input);
        assert_eq!(result, input);
    }

    #[test]
    fn large_homogeneous_array_gets_collapsed() {
        let arr: Vec<Value> = (0..20)
            .map(|i| json!({"id": i, "status": "ok", "value": 100}))
            .collect();
        let input = Value::Array(arr);
        let (result, did_compress) = compress_json_heuristic_tracked(&input);
        assert!(did_compress);
        // Should contain elision markers
        let result_str = serde_json::to_string(&result).unwrap();
        assert!(result_str.contains(ELIDED_KEY));
    }

    #[test]
    fn error_elements_are_kept() {
        let mut arr: Vec<Value> = (0..20)
            .map(|i| json!({"id": i, "status": "ok"}))
            .collect();
        arr[7] = json!({"id": 7, "status": "error", "message": "disk full"});
        arr[15] = json!({"id": 15, "status": "failed", "reason": "timeout"});

        let input = Value::Array(arr);
        let result = compress_json_heuristic(&input);
        let result_str = serde_json::to_string(&result).unwrap();
        assert!(result_str.contains("disk full"));
        assert!(result_str.contains("timeout"));
    }

    #[test]
    fn anomalies_are_kept() {
        let mut arr: Vec<Value> = (0..20)
            .map(|_| json!({"response_time": 100, "status": "ok"}))
            .collect();
        // Inject a dramatic anomaly
        arr[12] = json!({"response_time": 50000, "status": "ok"});

        let input = Value::Array(arr);
        let result = compress_json_heuristic(&input);
        let result_str = serde_json::to_string(&result).unwrap();
        assert!(result_str.contains("50000"));
    }

    #[test]
    fn error_subtrees_preserved() {
        let input = json!({
            "data": [1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12],
            "errors": ["err1", "err2", "err3", "err4", "err5", "err6", "err7", "err8", "err9", "err10"]
        });
        let result = compress_json_heuristic(&input);
        // The "errors" key subtree should be fully preserved
        let errors = result.get("errors").unwrap().as_array().unwrap();
        assert_eq!(errors.len(), 10);
    }

    #[test]
    fn idempotent() {
        let arr: Vec<Value> = (0..20)
            .map(|i| json!({"id": i, "val": 42}))
            .collect();
        let input = Value::Array(arr);
        let first = compress_json_heuristic(&input);
        let second = compress_json_heuristic(&first);
        assert_eq!(first, second);
    }
}
