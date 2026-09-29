/// Robust statistical functions for anomaly detection.

/// Computes the median of a slice of f64.
/// The slice must not be empty.
pub fn percentile(nums: &[f64], p: f64) -> f64 {
    let mut sorted = nums.to_vec();
    sorted.sort_unstable_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let idx = (sorted.len() as f64 * p).floor() as usize;
    let idx = idx.min(sorted.len().saturating_sub(1));
    sorted[idx]
}

/// Computes the Median Absolute Deviation (MAD).
pub fn median_absolute_deviation(nums: &[f64], median: f64) -> f64 {
    let mut devs: Vec<f64> = nums.iter().map(|x| (x - median).abs()).collect();
    percentile(&mut devs, 0.5)
}
