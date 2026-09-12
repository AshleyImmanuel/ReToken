use similar::TextDiff;

/// Generates a Unified Diff representing the delta between `old_content` and `new_content`.
pub fn generate_diff(old_content: &str, new_content: &str, filename: &str) -> String {
    let diff = TextDiff::from_lines(old_content, new_content);
    let mut output = String::new();

    let mut unified = diff.unified_diff();
    let formatted = unified.context_radius(3).header(filename, filename);
        
    output.push_str(&formatted.to_string());
    
    // Fallback if no lines actually changed but function called
    if output.is_empty() {
        return "No changes detected.".to_string();
    }
    
    output
}
