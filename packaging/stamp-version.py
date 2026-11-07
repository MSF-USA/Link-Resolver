#!/usr/bin/env python3
"""Set the crate version in Cargo.toml and in the crate's own entry in Cargo.lock.

Used by the release workflow so that the binaries report the release version
(for example ``1.20261106.0`` for tag ``v1.20261106``). Only this crate's own
version changes; every dependency line in Cargo.lock stays byte-for-byte as
locked, so ``cargo build --locked`` still works afterwards. No cargo command is
run and no network or registry cache is needed, so the step behaves the same
on a fresh CI runner as on a developer machine.

Usage: packaging/stamp-version.py 1.20261106.0
       packaging/stamp-version.py v1.20261106.2   (a tag name is accepted too)
"""

import pathlib
import re
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


def package_name(manifest_text: str) -> str:
    """The ``name`` from the ``[package]`` table of a Cargo.toml. Raises ``ValueError``."""
    match = re.search(r'^\[package\]\s*$(.*?)(?=^\[|\Z)', manifest_text, re.MULTILINE | re.DOTALL)
    name = match and re.search(r'^name = "([^"]+)"', match.group(1), re.MULTILINE)
    if not name:
        raise ValueError("Cargo.toml has no [package] name")
    return name.group(1)


def stamp_manifest(manifest_text: str, version: str) -> str:
    """Return ``manifest_text`` with the first ``version = "..."`` line set to ``version``.

    Raises ``ValueError`` when there is no version line.
    """
    updated, count = re.subn(
        r'^version = "[^"]*"', f'version = "{version}"', manifest_text, count=1, flags=re.MULTILINE
    )
    if count != 1:
        raise ValueError("Cargo.toml has no version line to replace")
    return updated


def stamp_lockfile(lock_text: str, name: str, version: str) -> str:
    """Return ``lock_text`` with the version of package ``name`` set to ``version``.

    Only the ``version`` line directly under ``name = "<name>"`` inside a
    ``[[package]]`` block changes; nothing else in the file is touched. Raises
    ``ValueError`` when the package has no entry.
    """
    pattern = re.compile(
        r'(^\[\[package\]\]\nname = "' + re.escape(name) + r'"\nversion = ")[^"]*(")',
        re.MULTILINE,
    )
    updated, count = pattern.subn(r'\g<1>' + version + r'\g<2>', lock_text, count=1)
    if count != 1:
        raise ValueError(f"Cargo.lock has no [[package]] entry for {name}")
    return updated


def stamp(version: str, root: pathlib.Path = ROOT) -> None:
    """Rewrite ``root/Cargo.toml`` and ``root/Cargo.lock`` in place to ``version``."""
    manifest = root / "Cargo.toml"
    lockfile = root / "Cargo.lock"
    manifest_text = manifest.read_text(encoding="utf-8")
    name = package_name(manifest_text)
    new_manifest = stamp_manifest(manifest_text, version)
    new_lock = stamp_lockfile(lockfile.read_text(encoding="utf-8"), name, version)
    manifest.write_text(new_manifest, encoding="utf-8")
    lockfile.write_text(new_lock, encoding="utf-8")
    print(f"{name}: Cargo.toml and Cargo.lock now say {version}")


def main(arguments: list[str]) -> int:
    if len(arguments) != 1:
        print(__doc__, file=sys.stderr)
        return 2
    try:
        stamp(crate_version(arguments[0]))
    except (ValueError, OSError) as error:
        print(f"Error: {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
