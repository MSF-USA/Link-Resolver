#!/usr/bin/env python3
"""Set the crate version in Cargo.toml and refresh its entry in Cargo.lock.

Used by the release workflow so that the binaries report the release version
(for example ``1.20261106.0`` for tag ``v1.20261106``). Only this crate's own
line changes; dependencies stay exactly as locked, so ``cargo build --locked``
still works afterwards.

Usage: packaging/stamp-version.py 1.20261106.0
       packaging/stamp-version.py v1.20261106.2   (a tag name is accepted too)
"""

import pathlib
import re
import subprocess
import sys

ROOT = pathlib.Path(__file__).resolve().parent.parent
TAG_PATTERN = re.compile(r"^v?1\.(\d{8})(?:\.(\d+))?$")
CRATE_PATTERN = re.compile(r"^\d+\.\d+\.\d+$")


def crate_version(argument: str) -> str:
    """Turn a release tag or a plain version into the MAJOR.MINOR.PATCH Cargo needs.

    ``v1.20261106`` becomes ``1.20261106.0`` and ``v1.20261106.2`` becomes
    ``1.20261106.2``; a value that is already three numbers is returned as is.
    Raises ``ValueError`` for anything else.
    """
    tag = TAG_PATTERN.match(argument)
    if tag:
        return f"1.{tag.group(1)}.{tag.group(2) or 0}"
    if CRATE_PATTERN.match(argument):
        return argument
    raise ValueError(
        f"{argument!r} is neither a v1.YYYYMMDD[.N] tag nor a MAJOR.MINOR.PATCH version"
    )


def stamp(version: str) -> None:
    """Rewrite the ``version`` line of the ``[package]`` table and update Cargo.lock."""
    manifest = ROOT / "Cargo.toml"
    text = manifest.read_text(encoding="utf-8")
    updated, count = re.subn(
        r'^version = "[^"]*"', f'version = "{version}"', text, count=1, flags=re.MULTILINE
    )
    if count != 1:
        raise RuntimeError("Cargo.toml has no version line to replace")
    manifest.write_text(updated, encoding="utf-8")
    subprocess.run(["cargo", "update", "--workspace", "--offline"], cwd=ROOT, check=True)
    print(f"Cargo.toml and Cargo.lock now say {version}")


def main(arguments: list[str]) -> int:
    if len(arguments) != 1:
        print(__doc__, file=sys.stderr)
        return 2
    try:
        stamp(crate_version(arguments[0]))
    except (ValueError, RuntimeError, subprocess.CalledProcessError) as error:
        print(f"Error: {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
