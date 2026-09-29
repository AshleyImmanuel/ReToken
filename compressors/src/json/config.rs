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
    /// Note inserted into elided arrays.
    pub elision_note: String,
}

impl Default for JsonCompressorConfig {
    fn default() -> Self {
        Self {
            array_threshold: std::env::var("RETOKEN_JSON_ARRAY_THRESHOLD").ok().and_then(|v| v.parse().ok()).unwrap_or(8),
            keep_first: std::env::var("RETOKEN_JSON_KEEP_FIRST").ok().and_then(|v| v.parse().ok()).unwrap_or(3),
            keep_last: std::env::var("RETOKEN_JSON_KEEP_LAST").ok().and_then(|v| v.parse().ok()).unwrap_or(2),
            anomaly_z: std::env::var("RETOKEN_JSON_ANOMALY_Z").ok().and_then(|v| v.parse().ok()).unwrap_or(2.0),
            elision_note: "Elements elided by ReToken. Original recoverable via retoken_retrieve tool.".to_string(), // Overridden by AppConfig anyway
        }
    }
}
