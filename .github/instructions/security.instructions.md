---
name: Security
description: Security practices for this project
globs: ["**/*.rs", "**/*.toml", "**/*.yml"]
---

# Security Instructions

## Command Injection Prevention

The `launch_cmd`, `launch_args`, `command`, and `default_args` config fields are passed to OS process spawning. These are user-controlled via config.toml.

Rules:
- **Never** construct shell commands by string concatenation from user input
- The `{command}` and `{cwd}` placeholders in `launch_args` are expanded via simple string replace — they must NOT be passed through a shell interpreter without escaping
- On Unix, the `sh -c` path in `launch_with_shortcut` needs proper shell escaping for CWD and command strings containing spaces, quotes, or special characters
- Prefer `std::process::Command` with explicit arg arrays over shell invocation

## Path Traversal

- `state_dir`, `data_dir`, and `startup_dir` are used directly from config for filesystem reads and process CWD
- Providers must not follow symlinks outside expected directories
- When constructing paths from JSONL data (e.g., decoded project paths), validate they don't escape the expected base directory

## Dependency Auditing

- Run `cargo audit` before releases to check for known vulnerabilities
- Consider adding `cargo audit` to CI workflow
- Keep dependencies minimal — this project intentionally uses few crates

## DLL Loading Surface

- The semantic search plugin (`ort` / ONNX Runtime) loads native DLLs from the executable directory. An attacker who can write to the exe directory can substitute a malicious DLL. Ensure the install directory has appropriate ACLs.
- The embedding model file is loaded from `data_dir/models/` — same trust boundary applies.

## Sensitive Data

- `config.toml` contains file paths and CLI commands — it's gitignored for a reason
- Log files (`%TEMP%/agent-session-tui.log`) may contain session IDs and file paths — they should not be shared publicly
- `archived.json` contains provider:session_id pairs — low sensitivity but still user data
- `embeddings_cache.json` in `data_dir/models/` stores cached embedding vectors — writable user data, same sensitivity as `archived.json`

## File Permissions

- On Unix, `archived.json`, `embeddings_cache.json`, and `config.toml` should be user-readable only (0600)
- Currently files are written with default permissions — a future improvement
