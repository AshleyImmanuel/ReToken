use std::fs;
use std::path::Path;

const CLAUDE_CODE_CONTENT: &str = r#"# ReToken Terse Optimization Configuration (Inspired by Caveman)
# This configures Claude Code to automatically be aware of ReToken.

# ReToken is an intercepting proxy. It will automatically elide large logs and JSON.
# It injects a tool called `retoken_retrieve` into your schema.
# DO NOT hallucinate elided data. If you see <<ccr:...>>, call `retoken_retrieve`.

# Keep your outputs extremely terse. No conversational filler. Provide only raw code or data.

## Available Tool Commands
- **`retoken_retrieve(handle)`**: When you see data replaced with `<<ccr:...>>`, invoke the `retoken_retrieve` tool with the handle as the argument to fetch the original, uncompressed bytes.
- **CLI Wrap**: If you need to spawn a sub-agent or script that uses an LLM, prefix it with `retoken wrap -- ` (e.g. `retoken wrap -- aider` or `retoken wrap -- python script.py`). This forces all sub-agent LLM traffic through ReToken.
- **CLI Analyze**: Run `retoken analyze .` to gather intelligence about a codebase.
"#;

const ANTIGRAVITY_SKILL_CONTENT: &str = r#"---
name: retoken
description: Instructions for the agent on how to interact with the ReToken proxy.
---

# ReToken Proxy

You are currently running behind the ReToken proxy.

ReToken is a local-first Agent Runtime Optimizer designed for AI coding agents. It saves tokens by intercepting your requests and eliding large logs, JSON arrays, and file contents with `<<ccr:...>>` handles.

## Available Tool Commands
- **`retoken_retrieve(handle)`**: When you see data replaced with `<<ccr:...>>`, invoke the injected `retoken_retrieve` tool with the handle as the argument to fetch the original bytes.
- **CLI Wrap**: If you need to spawn a sub-agent or run an LLM script, prefix the command with `retoken wrap -- ` (e.g. `retoken wrap -- python main.py`). This transparently routes its traffic through ReToken.
- **CLI Analyze**: Run `retoken analyze <path>` to gather structural intelligence about a directory.

## Rules
1. **Never guess elided data**: If you see a handle like `<<ccr:...>>` and need the original contents, you MUST use the `retoken_retrieve` tool to fetch it.
2. **Terse Mode (Caveman Inspired)**: ReToken enforces extreme brevity, drawing inspiration from the mature Caveman ecosystem. You must output only raw technical data, code, or direct answers. Omit all pleasantries and explanations.
"#;

pub fn run_init() -> anyhow::Result<()> {
    println!("Initializing ReToken Agent configurations in current directory...");

    // Claude Code
    let claude_path = Path::new(".claudecode");
    if !claude_path.exists() {
        fs::write(claude_path, CLAUDE_CODE_CONTENT)?;
        println!("Created {}", claude_path.display());
    } else {
        println!("{} already exists, skipping.", claude_path.display());
    }

    // Antigravity (Gemini)
    let agy_dir = Path::new(".gemini").join("config").join("skills").join("retoken");
    if !agy_dir.exists() {
        fs::create_dir_all(&agy_dir)?;
    }
    let agy_path = agy_dir.join("SKILL.md");
    if !agy_path.exists() {
        fs::write(&agy_path, ANTIGRAVITY_SKILL_CONTENT)?;
        println!("Created {}", agy_path.display());
    } else {
        println!("{} already exists, skipping.", agy_path.display());
    }

    println!("\nConfiguration complete! Agents running in this directory will automatically adapt to ReToken.");
    Ok(())
}
