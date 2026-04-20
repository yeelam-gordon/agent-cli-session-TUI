---
name: Cross-Platform
description: Cross-platform compatibility guidelines
globs: ["**/*.rs"]
---

# Cross-Platform Instructions

## Path Handling

- **Never hardcode path separators** — use `std::path::Path::join()`, not string formatting with `\\` or `/`
- Provider path decoders (`decode_project_path`) currently hardcode `:\\` and `\\` for Windows. These need platform-aware alternatives on Unix.
- Use `std::path::MAIN_SEPARATOR` when constructing display strings

## Process Detection

- **Windows**: WMI via PowerShell (`Get-CimInstance Win32_Process`) — reliable command-line reading
- **Unix**: sysinfo crate — works but may have limited command-line visibility for some processes
- Both paths exist in `src/process_info.rs` via `#[cfg(windows)]` / `#[cfg(not(windows))]`
- When adding new providers, ensure process matching works on both platforms

## Terminal Launch

- **Windows**: `wt`, `wtai`, `pwsh`, `cmd` shortcuts work. Custom `launch_cmd`/`launch_args` for full control.
- **Unix**: The current implementation uses `sh -c "xterm -e '...'"` which is a placeholder. Real Unix support should try `tmux`, `screen`, or the user's `$TERMINAL`.
- **Shell escaping**: On Unix, CWD and command strings passed to `sh -c` MUST be properly escaped (spaces, quotes, special chars). Use `shell-escape` crate or manual quoting.

## Provider Code

- Claude/Qwen path decoding (`C--Users-john` → `C:\Users\john`) is Windows-specific. On Unix, paths would encode differently (e.g., `-home-user` → `/home/user`).
- Copilot lock files (`inuse.<pid>.lock`) use Windows PID semantics — verify these exist on Unix too
- Config defaults (`launch_method = "wt"`) are Windows-centric. Default should be platform-aware.

## CI/Release

- CI builds on `ubuntu-latest` + `windows-latest` — good
- Release builds Windows, Linux, macOS — good
- Consider adding `aarch64-apple-darwin` (Apple Silicon) to the release matrix
