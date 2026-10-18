#!/usr/bin/env python3
"""Fail when tracked files cross the public source snapshot boundary."""

from pathlib import Path
import subprocess
import sys


ROOT = Path(__file__).resolve().parent.parent
BLOCKED_PARTS = {
    ".build-cache",
    "target",
    "patches",
    "application-drafts",
    "__pycache__",
}
BLOCKED_SUFFIXES = {
    ".app", ".bin", ".dmg", ".dll", ".dylib", ".exe", ".log",
    ".pkg", ".so", ".tar", ".zip",
}
MAX_SOURCE_BYTES = 2_000_000


def main() -> int:
    names = subprocess.check_output(["git", "ls-files", "-z"], cwd=ROOT).split(b"\0")
    failures = []
    checked = 0
    for raw_name in names:
        if not raw_name:
            continue
        relative = Path(raw_name.decode("utf-8", "surrogateescape"))
        path = ROOT / relative
        checked += 1
        if (
            any(part in BLOCKED_PARTS for part in relative.parts)
            or relative.parts[:2] == ("docs", "evidence")
            or relative.suffix.lower() in BLOCKED_SUFFIXES
            or relative.name.endswith(".local.conf")
        ):
            failures.append(f"{relative}: excluded from public source")
            continue
        if path.is_symlink() or not path.is_file():
            failures.append(f"{relative}: must be a regular file")
            continue
        size = path.stat().st_size
        if size > MAX_SOURCE_BYTES:
            failures.append(f"{relative}: {size} bytes exceeds source review limit")
            continue
        if b"\0" in path.read_bytes():
            failures.append(f"{relative}: contains NUL bytes; binary files need separate review")
    if failures:
        for failure in failures:
            print(failure, file=sys.stderr)
        return 1
    print(f"public source boundary: {checked} tracked text files checked")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
