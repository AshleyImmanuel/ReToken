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
        if self.object_ids.contains(&object_id) {
            return true;
        }

        if self.estimated_tokens + token_estimate > self.max_budget {
            return false;
        }

        self.object_ids.insert(object_id);
        self.estimated_tokens += token_estimate;
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_context_package_budget_limit() {
        let mut pkg = ContextPackage::new(100);
        assert!(pkg.add("file1.rs".into(), 40));
        assert_eq!(pkg.estimated_tokens, 40);
        assert!(pkg.add("file2.rs".into(), 50));
        assert_eq!(pkg.estimated_tokens, 90);
        // Exceeds budget:
        assert!(!pkg.add("file3.rs".into(), 20));
        assert_eq!(pkg.estimated_tokens, 90);
        // Duplicate object does not increase tokens:
        assert!(pkg.add("file1.rs".into(), 40));
        assert_eq!(pkg.estimated_tokens, 90);
    }
}
