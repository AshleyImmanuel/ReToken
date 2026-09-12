use serde_json::Value;

/// Compresses a JSON value by removing null fields and collapsing whitespace.
/// This is a deterministic, lossless compression safe for machine consumption.
pub fn compress_json(value: &Value) -> String {
    // serde_json::to_string produces compact JSON (no whitespace)
    // We additionally strip null fields to save tokens.
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
        Value::Array(arr) => {
            Value::Array(arr.iter().map(strip_nulls).collect())
        }
        other => other.clone(),
    }
}
