use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::time::{SystemTime, UNIX_EPOCH};
use uuid::Uuid;
use ring::digest::{Context, SHA256};
use hex;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum StateType {
    File,
    Command,
    ToolResult,
    Error,
    SearchResult,
    Decision,
    Symbol,
    FileStructure,
    Generic(String),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StateObject {
    pub object_id: String,
    pub content_hash: String,
    pub version: u32,
    pub state_type: StateType,
    pub size: usize,
    pub created_at: u64,
    pub updated_at: u64,
    pub source: Option<String>,
    pub dependencies: Vec<String>,
    pub confidence: f32,
    pub recovery_location: Option<String>,
    pub original_content: Option<Vec<u8>>,
}

pub struct StateEngine {
    /// Maps object_id to the most recent version of a StateObject
    objects_by_id: HashMap<String, StateObject>,
    /// Maps content_hash to the corresponding StateObject to avoid duplicate storage/transmission
    objects_by_hash: HashMap<String, StateObject>,
}

impl Default for StateEngine {
    fn default() -> Self {
        Self::new()
    }
}

impl StateEngine {
    pub fn new() -> Self {
        Self {
            objects_by_id: HashMap::new(),
            objects_by_hash: HashMap::new(),
        }
    }

    pub fn hash_content(content: &[u8]) -> String {
        let mut context = Context::new(&SHA256);
        context.update(content);
        let digest = context.finish();
        hex::encode(digest.as_ref())
    }

    fn current_timestamp() -> u64 {
        SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs()
    }

    pub fn insert_or_update(&mut self, state_type: StateType, content: &[u8], source: Option<String>) -> StateObject {
        let hash = Self::hash_content(content);
        let size = content.len();
        let now = Self::current_timestamp();

        // If we already have this exact content, return the existing object
        if let Some(existing) = self.objects_by_hash.get(&hash) {
            return existing.clone();
        }

        // We create a new state object.
        let new_id = Uuid::new_v4().to_string();
        let obj = StateObject {
            object_id: new_id.clone(),
            content_hash: hash.clone(),
            version: 1,
            state_type,
            size,
            created_at: now,
            updated_at: now,
            source,
            dependencies: Vec::new(),
            confidence: 1.0,
            recovery_location: None,
            original_content: Some(content.to_vec()),
        };

        self.objects_by_id.insert(new_id, obj.clone());
        self.objects_by_hash.insert(hash, obj.clone());

        obj
    }

    pub fn get_by_id(&self, object_id: &str) -> Option<&StateObject> {
        self.objects_by_id.get(object_id)
    }

    pub fn get_by_hash(&self, content_hash: &str) -> Option<&StateObject> {
        self.objects_by_hash.get(content_hash)
    }

    pub fn get_all(&self) -> Vec<&StateObject> {
        self.objects_by_id.values().collect()
    }
}
