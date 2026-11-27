use std::fs;
use std::path::Path;

/// Project skill for Claude Code (`.claude/skills/<name>/SKILL.md` is where Claude Code looks
/// for project skills). It only explains ReToken's markers; it does not change reply style.
const CLAUDE_SKILL_CONTENT: &str = r#"---
name: retoken
description: 'Use when tool output contains a "[ReToken: ... handle "ccr:..." ...]" marker. ReToken shortened that output and the original can be fetched.'
---

# ReToken

This project's model traffic may go through ReToken, a local proxy that shortens large tool output (build and test logs, JSON arrays, file reads the conversation has moved past) to save tokens.

When tool output contains a marker such as:

    [ReToken: full original elided; call `retoken_retrieve` with handle "ccr:..." to retrieve it]

- Call the tool the marker names with that handle to get the original. Never guess the elided content.
- For large originals, pass `pattern` to get only matching lines, or `start_line` and `end_line` to read one part.
- If the tool is deferred, load it with ToolSearch first.
"#;

pub fn run_init() -> anyhow::Result<()> {
    println!("Initializing ReToken agent guidance in the current directory...");
    for line in write_guidance(Path::new("."))? {
        println!("{line}");
    }
    println!(
        "\nDone. Claude Code loads project skills from .claude/skills when it starts in this directory."
    );
    Ok(())
}

/// Writes the guidance files under `root`, never overwriting existing ones. Returns one status
/// line per file.
fn write_guidance(root: &Path) -> anyhow::Result<Vec<String>> {
    let skill_dir = root.join(".claude").join("skills").join("retoken");
    let skill_path = skill_dir.join("SKILL.md");
    if skill_path.exists() {
        return Ok(vec![format!(
            "{} already exists, skipping.",
            skill_path.display()
        )]);
    }
    fs::create_dir_all(&skill_dir)?;
    fs::write(&skill_path, CLAUDE_SKILL_CONTENT)?;
    Ok(vec![format!("Created {}", skill_path.display())])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_root(name: &str) -> std::path::PathBuf {
        let dir =
            std::env::temp_dir().join(format!("retoken-init-test-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn writes_claude_skill_with_frontmatter() {
        let root = temp_root("write");
        write_guidance(&root).unwrap();
        let written = fs::read_to_string(root.join(".claude/skills/retoken/SKILL.md")).unwrap();
        assert!(written.starts_with("---\nname: retoken\ndescription: "));
        assert!(written.contains("retoken_retrieve"));
        assert!(
            !root.join(".gemini").exists(),
            "Gemini traffic is not supported, so no Gemini guidance is written"
        );
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn never_overwrites_an_existing_skill() {
        let root = temp_root("keep");
        let dir = root.join(".claude/skills/retoken");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("SKILL.md"), "mine").unwrap();
        let status = write_guidance(&root).unwrap();
        assert!(status[0].contains("already exists"));
        assert_eq!(fs::read_to_string(dir.join("SKILL.md")).unwrap(), "mine");
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn skill_marker_example_matches_the_real_marker() {
        let marker = compressors::ccr::ccr_marker("ccr:abc", "retoken_retrieve");
        let prefix = "[ReToken: full original elided; call `retoken_retrieve` with handle";
        assert!(marker.starts_with(prefix));
        assert!(CLAUDE_SKILL_CONTENT.contains(prefix));
    }
}
