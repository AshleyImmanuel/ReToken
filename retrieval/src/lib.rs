//! Context retrieval, prioritization, and packing subsystem for ReToken.

pub mod package;
pub mod packer;
pub mod planner;

pub use package::ContextPackage;
pub use packer::{inject_cache_breakpoint, inject_terse_mode, reorder_for_cache};
pub use planner::ContextPlanner;
