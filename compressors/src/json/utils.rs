use serde_json::Value;

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
