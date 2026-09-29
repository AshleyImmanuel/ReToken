use serde::{Deserialize, Serialize};

/// A single CCR entry stored on disk.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CcrEntry {
    /// Base64-encoded original content.
    pub content_b64: String,
    pub content_type: String,
    pub created_at: u64,
    pub access_count: u64,
    pub size_bytes: usize,
}
