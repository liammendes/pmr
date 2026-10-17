#!/usr/bin/env python3
"""Require every pull request to advance the package's Semantic Version."""
import argparse
import pathlib
import re
import subprocess
import tomllib


def parse_version(text):
    match = re.fullmatch(
        r"(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)"
        r"(?:-([0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*))?"
        r"(?:\+[0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*)?", text)
    if not match:
        raise ValueError(f"Invalid Semantic Version: {text}")
    major, minor, patch, pre = match.groups()
    identifiers = pre.split(".") if pre else []
    if any(part.isdigit() and len(part) > 1 and part.startswith("0") for part in identifiers):
        raise ValueError(f"Invalid numeric prerelease identifier: {text}")
    return (int(major), int(minor), int(patch)), identifiers


def compare(left, right):
    a, ap = parse_version(left)
    b, bp = parse_version(right)
    if a != b:
        return (a > b) - (a < b)
    if not ap or not bp:
        return (not ap) - (not bp)
    for x, y in zip(ap, bp):
        if x == y:
            continue
        if x.isdigit() and y.isdigit():
            return (int(x) > int(y)) - (int(x) < int(y))
        if x.isdigit() != y.isdigit():
            return -1 if x.isdigit() else 1
        return (x > y) - (x < y)
    return (len(ap) > len(bp)) - (len(ap) < len(bp))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--base-ref", required=True)
    args = parser.parse_args()
    root = pathlib.Path(__file__).resolve().parent.parent
    base = tomllib.loads(subprocess.check_output(
        ["git", "show", f"{args.base_ref}:Cargo.toml"], cwd=root, text=True))["package"]["version"]
    current = tomllib.loads((root / "Cargo.toml").read_text())["package"]["version"]
    locked = [p["version"] for p in tomllib.loads((root / "Cargo.lock").read_text())["package"]
              if p["name"] == "pmr" and "source" not in p]
    if locked != [current]:
        raise SystemExit("Cargo.lock must match the PMR package version")
    if compare(current, base) <= 0:
        raise SystemExit(f"Every PR needs a SemVer bump: base={base}, head={current}")
    print(f"SemVer bump verified: {base} -> {current}")


if __name__ == "__main__":
    main()
