use std::collections::HashSet;

#[derive(Debug, Clone)]
pub struct ContextPackage {
    pub object_ids: HashSet<String>,
    pub estimated_tokens: usize,
    pub max_budget: usize,
}

impl ContextPackage {
    pub fn new(max_budget: usize) -> Self {
        Self {
            object_ids: HashSet::new(),
            estimated_tokens: 0,
            max_budget,
        }
    }

    pub fn add(&mut self, object_id: String, token_estimate: usize) -> bool {
        if self.estimated_tokens + token_estimate > self.max_budget {
            return false;
        }

        if self.object_ids.insert(object_id) {
            self.estimated_tokens += token_estimate;
        }
        true
    }
}
