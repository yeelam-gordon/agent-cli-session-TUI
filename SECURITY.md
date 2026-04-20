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
- **Archive tampering** — `archived.json` is the only file written by the tool

## Supported Versions

| Version | Supported |
|---------|-----------|
| 0.1.x   | ✅ Current |
