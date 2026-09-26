#!/usr/bin/env python3
"""Summarize one thread from macOS `sample` output without overclaiming stacks."""

from __future__ import annotations

import argparse
import json
import re
from pathlib import Path


THREAD = re.compile(r"^    (?P<samples>\d+) Thread_\d+(?:: (?P<name>.*))?$")
DIRECT_BRANCH = re.compile(r"^    \+ (?P<samples>\d+) (?P<frame>.*)$")
MARKERS = {
    "rosetta_jit": "Rosetta JIT",
    "rosetta_runtime": "Rosetta Runtime Routines",
    "dxvk": "dxvk",
    "moltenvk": "MoltenVK",
    "ntdll": "ntdll.so",
    "mach_wait": "mach_msg",
    "semaphore_wait": "semaphore_wait",
    "ulock_wait": "__ulock_wait",
}


def summarize(contents: str, requested_thread: str) -> dict[str, object]:
    lines = contents.splitlines()
    process = _header_value(lines, "Process")
    code_type = _header_value(lines, "Code Type")
    selected: tuple[int, str, int, int] | None = None
    headings: list[tuple[int, str, int]] = []
    for index, line in enumerate(lines):
        match = THREAD.match(line)
        if match:
            headings.append((index, match.group("name") or "unnamed", int(match.group("samples"))))
    for heading_index, (start, name, samples) in enumerate(headings):
        if name == requested_thread:
            end = headings[heading_index + 1][0] if heading_index + 1 < len(headings) else len(lines)
            selected = (start, name, samples, end)
            break
    if selected is None:
        available = sorted({name for _, name, _ in headings})
        raise ValueError(
            f"thread {requested_thread!r} not found; available named threads: {', '.join(available)}"
        )

    start, name, total_samples, end = selected
    branches: list[dict[str, object]] = []
    current: dict[str, object] | None = None
    for line in lines[start + 1 : end]:
        match = DIRECT_BRANCH.match(line)
        if match:
            if current is not None:
                branches.append(current)
            current = {
                "samples": int(match.group("samples")),
                "frame": match.group("frame"),
                "stack": [line],
            }
        elif current is not None:
            current["stack"].append(line)  # type: ignore[union-attr]
    if current is not None:
        branches.append(current)

    marker_samples = {marker: 0 for marker in MARKERS}
    public_branches: list[dict[str, object]] = []
    for branch in branches:
        stack = "\n".join(branch.pop("stack"))
        markers = [marker for marker, needle in MARKERS.items() if needle.lower() in stack.lower()]
        for marker in markers:
            marker_samples[marker] += int(branch["samples"])
        branch["markers"] = markers
        public_branches.append(branch)

    return {
        "schema": 1,
        "process": process,
        "code_type": code_type,
        "thread": name,
        "total_samples": total_samples,
        "direct_branch_samples": sum(int(branch["samples"]) for branch in public_branches),
        "marker_branch_samples": marker_samples,
        "branches": public_branches,
        "evidence": (
            "marker counts sum top-level thread branches containing each string; "
            "markers can overlap and do not identify guest functions"
        ),
    }


def _header_value(lines: list[str], name: str) -> str | None:
    prefix = f"{name}:"
    for line in lines:
        if line.startswith(prefix):
            return line.split(":", 1)[1].strip().split(" [", 1)[0]
    return None


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("sample", type=Path)
    parser.add_argument("--thread", required=True)
    parser.add_argument("--json", action="store_true")
    args = parser.parse_args()
    try:
        result = summarize(args.sample.read_text(errors="replace"), args.thread)
    except (OSError, ValueError) as error:
        parser.error(str(error))
    if args.json:
        print(json.dumps(result, indent=2, sort_keys=True))
    else:
        print(f"Process: {result['process'] or 'unknown'}")
        print(f"Code type: {result['code_type'] or 'unknown'}")
        print(f"Thread: {result['thread']}")
        print(f"Samples: {result['total_samples']}")
        for marker, samples in result["marker_branch_samples"].items():
            print(f"{marker}: {samples}")
        print(f"Evidence: {result['evidence']}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
