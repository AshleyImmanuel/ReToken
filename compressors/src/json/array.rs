use serde_json::Value;
use tracing::debug;

use crate::anomaly::detect_anomalies;
use crate::json::config::JsonCompressorConfig;
use crate::json::constants::{ELIDED_KEY, ELIDED_NOTE_KEY, ERROR_VALUE_RE};

/// Collapses a long array, keeping signal-carrying elements and emitting
/// elision markers for dropped runs.
pub fn select_array(arr: &[Value], config: &JsonCompressorConfig, collapsed: &mut bool) -> Value {
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
    for item in keep
        .iter_mut()
        .take(n)
        .skip(n.saturating_sub(config.keep_last))
    {
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
                        Value::String(config.elision_note.clone()),
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
                Value::String(config.elision_note.clone()),
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
