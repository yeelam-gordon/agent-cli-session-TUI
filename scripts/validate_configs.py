#!/usr/bin/env python3
"""Validate generated per-OS config TOML files.

Usage: validate_configs.py <dir>

Checks each .toml file in <dir>:
  - parses as valid TOML
  - contains exactly the EXPECTED_PROVIDERS set
  - copilot provider has the OS-appropriate launcher
    (Windows -> launch_method=wt, no launch_cmd;
     Linux   -> launch_cmd=gnome-terminal;
     macOS   -> launch_cmd=osascript)

Exits 0 if all pass, non-zero otherwise.
"""
from __future__ import annotations

import sys
import tomllib
from pathlib import Path

# Update this set when adding/removing a provider in config.toml.template.
EXPECTED_PROVIDERS = {"copilot", "claude", "codex", "qwen", "gemini", "kimi"}


def check(path: Path) -> list[str]:
    errors: list[str] = []
    try:
        data = tomllib.loads(path.read_text(encoding="utf-8"))
    except tomllib.TOMLDecodeError as e:
        return [f"TOML parse error: {e}"]

    provs = data.get("providers", {})
    actual = set(provs.keys())
    missing = EXPECTED_PROVIDERS - actual
    extra = actual - EXPECTED_PROVIDERS
    if missing:
        errors.append(f"missing providers: {sorted(missing)}")
    if extra:
        errors.append(f"unexpected providers: {sorted(extra)}")

    cop = provs.get("copilot")
    if not cop:
        errors.append("missing copilot provider")
        return errors

    stem = path.stem.lower()
    if "windows" in stem:
        if cop.get("launch_method") != "wt":
            errors.append(f"windows: launch_method={cop.get('launch_method')!r}, want 'wt'")
        if cop.get("launch_cmd") is not None:
            errors.append(f"windows: launch_cmd should be absent, got {cop.get('launch_cmd')!r}")
    elif "linux" in stem:
        if cop.get("launch_cmd") != "gnome-terminal":
            errors.append(f"linux: launch_cmd={cop.get('launch_cmd')!r}, want 'gnome-terminal'")
    elif "apple" in stem or "darwin" in stem:
        if cop.get("launch_cmd") != "osascript":
            errors.append(f"macos: launch_cmd={cop.get('launch_cmd')!r}, want 'osascript'")
    else:
        errors.append(f"unrecognized OS in filename {path.name!r}")

    return errors


def main() -> int:
    if len(sys.argv) != 2:
        print(__doc__, file=sys.stderr)
        return 2

    root = Path(sys.argv[1])
    files = sorted(root.glob("*.toml"))
    if not files:
        print(f"No .toml files in {root}", file=sys.stderr)
        return 2

    failed = 0
    for p in files:
        errs = check(p)
        if errs:
            failed += 1
            print(f"FAIL {p.name}")
            for e in errs:
                print(f"     {e}")
        else:
            print(f"OK   {p.name}")

    return 0 if failed == 0 else 1


if __name__ == "__main__":
    sys.exit(main())
