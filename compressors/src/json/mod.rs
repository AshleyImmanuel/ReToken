pub mod array;
pub mod config;
pub mod constants;
pub mod utils;

use serde_json::Value;

pub use config::JsonCompressorConfig;
pub use constants::{ELIDED_KEY, ELIDED_NOTE_KEY, ERROR_KEY_RE, ERROR_VALUE_RE};
pub use utils::compress_json_simple;

/// Compresses a JSON value in-place, collapsing repetitive arrays while
/// preserving structure, error subtrees, and statistical anomalies.
pub fn compress_json_heuristic(value: &Value) -> Value {
    let config = JsonCompressorConfig::default();
    let mut collapsed = false;
    transform(value, false, &config, &mut collapsed)
}

/// Returns (compressed_value, did_compress).
pub fn compress_json_heuristic_tracked(value: &Value, elision_note: &str) -> (Value, bool) {
    let config = JsonCompressorConfig {
        elision_note: elision_note.to_string(),
        ..Default::default()
    };
    let mut collapsed = false;
    let result = transform(value, false, &config, &mut collapsed);
    (result, collapsed)
}

/// Recursively transforms the value. `preserve` is set inside error/message subtrees.
fn transform(
    v: &Value,
    preserve: bool,
    config: &JsonCompressorConfig,
    collapsed: &mut bool,
) -> Value {
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
                array::select_array(arr, config, collapsed)
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
        let (result, did_compress) = compress_json_heuristic_tracked(&input, "test elision");
        assert!(did_compress);
        // Should contain elision markers
        let result_str = serde_json::to_string(&result).unwrap();
        assert!(result_str.contains(ELIDED_KEY));
    }

    #[test]
    fn error_elements_are_kept() {
        let mut arr: Vec<Value> = (0..20).map(|i| json!({"id": i, "status": "ok"})).collect();
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
        let arr: Vec<Value> = (0..20).map(|i| json!({"id": i, "val": 42})).collect();
        let input = Value::Array(arr);
        let first = compress_json_heuristic(&input);
        let second = compress_json_heuristic(&first);
        assert_eq!(first, second);
    }
}
