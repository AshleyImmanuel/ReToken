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

/// Antigravity workspace rule (`.agents/rules/*.md` needs a `trigger` in its frontmatter or
/// Antigravity discards it). Antigravity's model traffic cannot go through the proxy, so the
/// savings come from keeping noisy command output out of the conversation.
const ANTIGRAVITY_RULE_CONTENT: &str = r#"---
trigger: always_on
---

# ReToken

Run commands that print a lot of output (builds, test suites, installs, linters) as `retoken exec -- <command>`, for example `retoken exec -- cargo test`. It prints short output unchanged. Long output keeps its first and last lines and every error, failure, warning and test-summary line; the exit code is the command's own.

When the output ends with a `[ReToken: ... handle "ccr:..." ...]` marker, call the `retoken_retrieve` tool with that handle if you need lines that were removed. Pass `pattern` or `start_line` and `end_line` to fetch only part. Never guess removed content.

Run commands directly when you need their exact full output: `git diff`, `git show`, reading files, or anything you will parse.
"#;

/// The `mcp_config.json` entry that registers the retrieval server in Antigravity.
const ANTIGRAVITY_MCP_SNIPPET: &str =
    r#"{ "mcpServers": { "retoken": { "command": "retoken", "args": ["mcp"] } } }"#;

pub fn run_init(antigravity: bool) -> anyhow::Result<()> {
    println!("Initializing ReToken agent guidance in the current directory...");
    for line in write_guidance(Path::new("."), antigravity)? {
        println!("{line}");
    }
    println!(
        "
Done. Claude Code loads project skills from .claude/skills when it starts in this directory."
    );
    if antigravity {
        println!(
            "Antigravity also needs the retrieval tool. Add this to ~/.gemini/config/mcp_config.json              (all projects) or .agents/mcp_config.json (this project), then restart Antigravity:
  {ANTIGRAVITY_MCP_SNIPPET}"
        );
    }
    Ok(())
}

/// Writes the guidance files under `root`, never overwriting existing ones. Returns one status
/// line per file.
fn write_guidance(root: &Path, antigravity: bool) -> anyhow::Result<Vec<String>> {
    let mut status = vec![write_new(
        &root.join(".claude/skills/retoken/SKILL.md"),
        CLAUDE_SKILL_CONTENT,
    )?];
    if antigravity {
        status.push(write_new(
            &root.join(".agents/rules/retoken.md"),
            ANTIGRAVITY_RULE_CONTENT,
        )?);
    }
    Ok(status)
}

/// Creates `path` with `content` unless it already exists.
fn write_new(path: &Path, content: &str) -> anyhow::Result<String> {
    if path.exists() {
        return Ok(format!("{} already exists, skipping.", path.display()));
    }
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    fs::write(path, content)?;
    Ok(format!("Created {}", path.display()))
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
        write_guidance(&root, false).unwrap();
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
        let status = write_guidance(&root, false).unwrap();
        assert!(status[0].contains("already exists"));
        assert_eq!(fs::read_to_string(dir.join("SKILL.md")).unwrap(), "mine");
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn antigravity_rule_is_opt_in_and_has_a_trigger() {
        let root = temp_root("agy");
        write_guidance(&root, false).unwrap();
        assert!(!root.join(".agents").exists());
        write_guidance(&root, true).unwrap();
        let rule = fs::read_to_string(root.join(".agents/rules/retoken.md")).unwrap();
        assert!(rule.starts_with(
            "---
trigger: always_on
---
"
        ));
        assert!(rule.contains("retoken exec -- "));
        let mcp: serde_json::Value = serde_json::from_str(ANTIGRAVITY_MCP_SNIPPET).unwrap();
        assert_eq!(mcp["mcpServers"]["retoken"]["args"][0], "mcp");
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
