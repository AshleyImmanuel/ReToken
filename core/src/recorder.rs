use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FlightRecord {
    pub task_id: String,
    pub session_id: String,
    pub provider: String,
    pub model: String,
    pub timestamp: String,
    pub request_size: usize,
    pub input_tokens: Option<usize>,
    pub output_tokens: Option<usize>,
    pub cached_tokens: Option<usize>,
    pub reasoning_tokens: Option<usize>,
    pub network_duration_ms: u64,
    pub cache_hits: usize,
    pub cache_misses: usize,
    pub compression_ratio: Option<f64>,
    pub context_reduction: Option<f64>,
    pub errors: usize,
    pub final_success: Option<bool>,
}

impl Default for FlightRecord {
    fn default() -> Self {
        Self {
            task_id: uuid::Uuid::new_v4().to_string(),
            session_id: uuid::Uuid::new_v4().to_string(),
            provider: "unknown".into(),
            model: "unknown".into(),
            timestamp: chrono::Utc::now().to_rfc3339(),
            request_size: 0,
            input_tokens: None,
            output_tokens: None,
            cached_tokens: None,
            reasoning_tokens: None,
            network_duration_ms: 0,
            cache_hits: 0,
            cache_misses: 0,
            compression_ratio: None,
            context_reduction: None,
            errors: 0,
            final_success: None,
        }
    }
}

/// Aggregated runtime telemetry metrics across the gateway process lifetime.
#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct FlightTelemetry {
    pub total_requests: u64,
    pub total_errors: u64,
    pub total_bytes_transferred: u64,
    pub total_input_tokens: u64,
    pub total_output_tokens: u64,
    pub total_cached_tokens: u64,
    pub total_reasoning_tokens: u64,
    pub total_cache_hits: u64,
    pub total_cache_misses: u64,
}

impl FlightTelemetry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn record(&mut self, record: &FlightRecord) {
        self.total_requests += 1;
        self.total_errors += record.errors as u64;
        self.total_bytes_transferred += record.request_size as u64;
        if let Some(tokens) = record.input_tokens {
            self.total_input_tokens += tokens as u64;
        }
        if let Some(tokens) = record.output_tokens {
            self.total_output_tokens += tokens as u64;
        }
        if let Some(tokens) = record.cached_tokens {
            self.total_cached_tokens += tokens as u64;
        }
        if let Some(tokens) = record.reasoning_tokens {
            self.total_reasoning_tokens += tokens as u64;
        }
        self.total_cache_hits += record.cache_hits as u64;
        self.total_cache_misses += record.cache_misses as u64;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_flight_record_default_and_serialization() {
        let record = FlightRecord::default();
        assert_eq!(record.provider, "unknown");
        assert_eq!(record.errors, 0);
        assert!(record.input_tokens.is_none());

        let json = serde_json::to_string(&record).expect("serialization should succeed");
        let deserialized: FlightRecord =
            serde_json::from_str(&json).expect("deserialization should succeed");
        assert_eq!(deserialized.task_id, record.task_id);
        assert_eq!(deserialized.session_id, record.session_id);
    }
}
