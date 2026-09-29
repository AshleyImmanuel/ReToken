use serde_json::Value;
use crate::stats::{median_absolute_deviation, percentile};

/// Extracts all numeric leaf values from a JSON element, keyed by their JSON path.
pub fn extract_numeric_leaves(value: &Value, path: &str, out: &mut Vec<(String, f64)>) {
    match value {
        Value::Number(n) => {
            if let Some(f) = n.as_f64() {
                out.push((path.to_string(), f));
            }
        }
        Value::Object(map) => {
            for (k, v) in map {
                let child_path = if path.is_empty() {
                    k.clone()
                } else {
                    format!("{}.{}", path, k)
                };
                extract_numeric_leaves(v, &child_path, out);
            }
        }
        Value::Array(arr) => {
            for (i, v) in arr.iter().enumerate() {
                let child_path = format!("{}[{}]", path, i);
                extract_numeric_leaves(v, &child_path, out);
            }
        }
        _ => {}
    }
}

/// Detects anomalous array elements using Median Absolute Deviation (MAD) on
/// numeric fields. Returns indices of elements with robust |z| >= threshold.
///
/// For each numeric "path" that appears across elements (e.g., "response_time",
/// "size"), we collect values, compute median and MAD, then flag elements whose
/// value deviates significantly.
pub fn detect_anomalies(arr: &[Value], docs: &[String], z_threshold: f64) -> Vec<usize> {
    let n = arr.len();
    if n < 4 {
        return vec![];
    }

    // Collect numeric fields per path across all elements
    let mut fields_by_path: std::collections::HashMap<String, Vec<(usize, f64)>> =
        std::collections::HashMap::new();

    for (i, element) in arr.iter().enumerate() {
        let mut leaves = Vec::new();
        extract_numeric_leaves(element, "", &mut leaves);
        for (path, val) in leaves {
            fields_by_path.entry(path).or_default().push((i, val));
        }
    }

    // Also use element serialized length as a numeric feature
    for (i, doc) in docs.iter().enumerate() {
        fields_by_path
            .entry("__len__".to_string())
            .or_default()
            .push((i, doc.len() as f64));
    }

    let mut anomalous = std::collections::HashSet::new();

    for values in fields_by_path.values() {
        // Only consider paths that cover most elements (at least half)
        if values.len() < n / 2 {
            continue;
        }

        let nums: Vec<f64> = values.iter().map(|(_, v)| *v).collect();
        let median = percentile(&nums, 0.5);
        let mad = median_absolute_deviation(&nums, median);

        if mad < 1e-10 {
            // MAD is 0, meaning the majority of elements are exactly the median.
            // Any element that differs from the median is infinitely anomalous.
            for &(idx, val) in values {
                if (val - median).abs() > 1e-10 {
                    anomalous.insert(idx);
                }
            }
            continue;
        }

        // Modified z-score: 0.6745 * (x - median) / MAD
        for &(idx, val) in values {
            let z = 0.6745 * (val - median).abs() / mad;
            if z >= z_threshold {
                anomalous.insert(idx);
            }
        }
    }

    let mut result: Vec<usize> = anomalous.into_iter().collect();
    result.sort_unstable();
    result
}
