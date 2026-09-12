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
    pub tool_calls: usize,
    pub tool_duration_ms: u64,
    pub network_duration_ms: u64,
    pub ttft_ms: Option<u64>,
    pub generation_duration_ms: Option<u64>,
    pub cache_hits: usize,
    pub cache_misses: usize,
    pub compression_ratio: Option<f64>,
    pub context_reduction: Option<f64>,
    pub agent_turns: usize,
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
            tool_calls: 0,
            tool_duration_ms: 0,
            network_duration_ms: 0,
            ttft_ms: None,
            generation_duration_ms: None,
            cache_hits: 0,
            cache_misses: 0,
            compression_ratio: None,
            context_reduction: None,
            agent_turns: 0,
            errors: 0,
            final_success: None,
        }
    }
}
