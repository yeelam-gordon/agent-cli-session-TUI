#!/usr/bin/env python3
"""Generate a platform-specific config.toml from config.toml.template.

Usage: generate_config.py <target-triple> <template-path> <output-path>

The target triple is matched against substrings to pick per-OS launcher
defaults. Release workflow calls this once per matrix target.
"""
from __future__ import annotations

import sys
from pathlib import Path

# (launch_method, launch_fallback, launch_cmd_line, launch_args_line)
WINDOWS = (
    "wt",
    "cmd",
    '# launch_cmd = "wt"  # uncomment for custom launcher',
    '# launch_args = ["-w", "0", "new-tab", "--startingDirectory", "{cwd}", "cmd", "/k", "{command}"]',
)
LINUX = (
    "wt",  # ignored on non-Windows; launch_cmd below takes priority
    "cmd",
    'launch_cmd = "gnome-terminal"',
    'launch_args = ["--working-directory={cwd}", "--", "bash", "-c", "{command}; exec bash"]',
)
MACOS = (
    "wt",  # ignored on non-Windows; launch_cmd below takes priority
    "cmd",
    'launch_cmd = "osascript"',
    'launch_args = ["-e", "tell application \\"Terminal\\" to do script \\"cd {cwd} && {command}\\""]',
)


def pick(target: str) -> tuple[str, str, str, str]:
    t = target.lower()
    if "windows" in t:
        return WINDOWS
    if "linux" in t:
        return LINUX
    if "apple" in t or "darwin" in t:
        return MACOS
    raise SystemExit(f"Unknown target triple: {target}")


def main() -> int:
    if len(sys.argv) != 4:
        print(__doc__, file=sys.stderr)
        return 2
    target, tpl_path, out_path = sys.argv[1], Path(sys.argv[2]), Path(sys.argv[3])
    method, fallback, cmd_line, args_line = pick(target)
    text = tpl_path.read_text(encoding="utf-8")
    text = (
        text.replace("{{LAUNCH_METHOD}}", method)
            .replace("{{LAUNCH_FALLBACK}}", fallback)
            .replace("{{LAUNCH_CMD_LINE}}", cmd_line)
            .replace("{{LAUNCH_ARGS_LINE}}", args_line)
    )
    if "{{" in text:
        raise SystemExit(f"Unsubstituted placeholder remains:\n{text}")
    out_path.parent.mkdir(parents=True, exist_ok=True)
    out_path.write_text(text, encoding="utf-8")
    print(f"Generated {out_path} for {target}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
