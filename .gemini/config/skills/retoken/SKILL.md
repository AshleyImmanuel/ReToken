---
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
