#!/usr/bin/env python3
"""Bump the Quasar workspace version in the root Cargo.toml."""
import argparse
import re
import subprocess
import sys
from pathlib import Path

VERSION_RE = re.compile(r"^\d+\.\d+\.\d+$")
SECTION_RE = re.compile(r"(?ms)^\[workspace\.package\]\s*$(.*?)(?=^\[|\Z)")
VERSION_LINE_RE = re.compile(r'(?m)^version\s*=\s*"([^"]+)"')
REFERENCE_FILES = (
    "README.md",
    "crates/quasar-render-2d/README.md",
    "crates/quasar-render-3d/README.md",
    "crates/quasar-frame/README.md",
    "packaging/PONG-RUNTIME.txt",
)


def repository_root() -> Path:
    try:
        output = subprocess.check_output(
            ["git", "rev-parse", "--show-toplevel"],
            text=True,
            stderr=subprocess.DEVNULL,
        )
        return Path(output.strip())
    except (subprocess.CalledProcessError, FileNotFoundError):
        return Path.cwd()


def bump(current: str, level: str) -> str:
    major, minor, patch = (int(part) for part in current.split("."))
    if level == "major":
        return f"{major + 1}.0.0"
    if level == "minor":
        return f"{major}.{minor + 1}.0"
    return f"{major}.{minor}.{patch + 1}"


def update_references(root: Path, current: str, new_version: str) -> None:
    pattern = re.compile(rf"(?<![\d.]){re.escape(current)}(?![\d.])")
    for relative in REFERENCE_FILES:
        path = root / relative
        if not path.exists():
            continue
        text = path.read_text()
        updated, count = pattern.subn(new_version, text)
        if count:
            path.write_text(updated)
            print(f"updated {relative} ({count} reference(s))")


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("level", nargs="?", choices=["major", "minor", "patch"], default="patch")
    parser.add_argument("--version", help="Use an exact X.Y.Z version instead of bumping")
    parser.add_argument("--root", type=Path, help="Repository root (defaults to git root)")
    args = parser.parse_args()

    root = args.root or repository_root()
    manifest = root / "Cargo.toml"
    text = manifest.read_text()

    section_match = SECTION_RE.search(text)
    if section_match is None:
        print("error: [workspace.package] not found in Cargo.toml", file=sys.stderr)
        return 1
    section = section_match.group(1)
    version_match = VERSION_LINE_RE.search(section)
    if version_match is None:
        print("error: version not found under [workspace.package]", file=sys.stderr)
        return 1

    current = version_match.group(1)
    if not VERSION_RE.fullmatch(current):
        print(f"error: current version {current!r} is not X.Y.Z", file=sys.stderr)
        return 1

    if args.version is not None:
        if not VERSION_RE.fullmatch(args.version):
            print(f"error: {args.version!r} is not X.Y.Z", file=sys.stderr)
            return 1
        new_version = args.version
    else:
        new_version = bump(current, args.level)

    replacement = version_match.group(0).replace(current, new_version, 1)
    new_section = section[: version_match.start()] + replacement + section[version_match.end() :]
    text = text[: section_match.start(1)] + new_section + text[section_match.end(1) :]
    manifest.write_text(text)
    update_references(root, current, new_version)
    print(f"{current} -> {new_version}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
