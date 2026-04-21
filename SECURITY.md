# Security Policy

## Reporting a Vulnerability

If you discover a security vulnerability, please report it responsibly:

1. **Do NOT open a public issue.**
2. Email the maintainer or use [GitHub's private vulnerability reporting](https://github.com/yeelam-gordon/agent-cli-session-TUI/security/advisories/new).
3. Include a description of the vulnerability, steps to reproduce, and potential impact.

## Scope

This tool reads session data from local CLI state directories (read-only) and launches terminal processes. Security concerns include:

- **Config injection** — malicious `config.toml` could specify arbitrary commands in `launch_cmd` / `command` fields
- **Path traversal** — provider `state_dir` paths are used for filesystem reads
- **File writes** — the tool writes `archived.json` and `embeddings_cache.json` (in `data_dir/models/`); both are JSON and writable by the current user
- **Semantic plugin DLL loading** — if a `semantic_plugin.dll` / `libsemantic_plugin.so` is found at startup, it is loaded via `libloading`. A malicious DLL in the search path can execute arbitrary code. Only load DLLs you built yourself or obtained from a trusted source
- **Model download trust** — the semantic plugin uses `fastembed-rs`, which downloads ONNX models from Hugging Face on first use. Ensure you trust the model repository and that your network is not intercepting the download

## Supported Versions

| Version | Supported |
|---------|-----------|
| 0.1.x   | ✅ Current |
