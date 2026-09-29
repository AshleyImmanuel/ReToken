pub mod schema;
pub mod store;
pub mod entry;
pub mod paths;

pub use schema::{ccr_tool_schema, ccr_marker};
pub use entry::CcrEntry;
pub use store::CcrStore;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn store_and_retrieve() {
        let store = CcrStore::open_in_memory().unwrap();
        let data = b"hello world this is some test data";
        let handle = store.store(data, "text").unwrap();
        assert!(handle.starts_with("ccr:"));

        let retrieved = store.retrieve(&handle).unwrap().unwrap();
        assert_eq!(retrieved, data);
    }

    #[test]
    fn missing_handle_returns_none() {
        let store = CcrStore::open_in_memory().unwrap();
        let result = store.retrieve("ccr:nonexistent").unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn duplicate_stores_are_idempotent() {
        let store = CcrStore::open_in_memory().unwrap();
        let data = b"same content";
        let h1 = store.store(data, "text").unwrap();
        let h2 = store.store(data, "text").unwrap();
        assert_eq!(h1, h2);
        assert_eq!(store.count(), 1);
    }

    #[test]
    fn handle_is_deterministic() {
        let h1 = CcrStore::handle(b"test");
        let h2 = CcrStore::handle(b"test");
        assert_eq!(h1, h2);
        let h3 = CcrStore::handle(b"different");
        assert_ne!(h1, h3);
    }

    #[test]
    fn tool_schema_is_valid_json() {
        let schema = ccr_tool_schema();
        assert!(schema.get("name").is_some());
        assert!(schema.get("input_schema").is_some());
    }
}
