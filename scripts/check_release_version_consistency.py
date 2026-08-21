#!/usr/bin/env python3
"""Keep hand-written version literals in step with the workspace version.

The workspace version in the root ``Cargo.toml`` is the single source of truth: binaries, the release manifest, the
Homebrew formula, and the npm and pip packages all derive from it. Most of that derivation happens at package time
into a staging directory, so the copies committed in the repository are never consulted by a release -- which is
precisely why they drift unnoticed, and why a reader cannot tell whether they are authoritative.

This checks only the files that are *supposed* to mirror the workspace version, in the version format each one
uses. It deliberately ignores version literals in tests and fixtures: several exist to exercise release-candidate
handling itself (PEP 440 normalization of ``0.4.0-rc1``, for instance), and demanding they track the workspace
would make this check something people switch off.

Example ``incan.lock`` files record the compiler that last wrote them and update when the examples are rebuilt, so
they are reported for awareness but never fail the check.
"""

from __future__ import annotations

import re
import subprocess
import sys
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent


def workspace_version() -> str:
    """Read the single source-of-truth version from the root manifest."""
    manifest = (REPO_ROOT / "Cargo.toml").read_text(encoding="utf-8")
    match = re.search(r'^version = "([^"]+)"$', manifest, re.MULTILINE)
    if not match:
        raise SystemExit("check_release_version_consistency: no workspace version in Cargo.toml")
    return match.group(1)


def pep440(version: str) -> str:
    """Convert a Cargo version to the PEP 440 spelling the pip package uses.

    Mirrors ``pep440_version`` in ``workspaces/release/pip/prepare_package.py``; the two must agree or this check
    would report a false difference on every pre-release.
    """
    normalized = version.replace("-dev.", ".dev")
    return re.sub(r"-(a|b|rc)(\d+)$", r"\1\2", normalized)


def mirrors(version: str) -> list[tuple[Path, re.Pattern[str], str]]:
    """Files carrying a literal that must equal the workspace version, with the spelling each one expects."""
    cargo_form = version
    pip_form = pep440(version)
    return [
        (Path("workspaces/release/npm/package.json"), re.compile(r'^\s*"version":\s*"([^"]+)"', re.MULTILINE), cargo_form),
        (Path("workspaces/release/pip/pyproject.toml"), re.compile(r'^version = "([^"]+)"', re.MULTILINE), pip_form),
        (
            Path("workspaces/release/pip/src/incan_toolchain/__init__.py"),
            re.compile(r'^__version__ = "([^"]+)"', re.MULTILINE),
            pip_form,
        ),
    ]


def example_lock_versions() -> list[tuple[Path, str]]:
    """Compiler versions recorded in tracked example lockfiles, which follow whenever examples are rebuilt."""
    listing = subprocess.run(
        ["git", "ls-files", "-z", "*incan.lock"], cwd=REPO_ROOT, capture_output=True, text=True, check=True
    )
    recorded: list[tuple[Path, str]] = []
    for name in listing.stdout.split("\0"):
        if not name:
            continue
        path = REPO_ROOT / name
        match = re.search(r'^incan-version = "([^"]+)"', path.read_text(encoding="utf-8"), re.MULTILINE)
        if match:
            recorded.append((Path(name), match.group(1)))
    return recorded


def main() -> int:
    version = workspace_version()
    failures: list[str] = []

    for relative, pattern, expected in mirrors(version):
        path = REPO_ROOT / relative
        if not path.is_file():
            failures.append(f"{relative}: expected to mirror the workspace version, but the file is missing")
            continue
        match = pattern.search(path.read_text(encoding="utf-8"))
        if not match:
            failures.append(f"{relative}: no version literal found to compare against {expected}")
        elif match.group(1) != expected:
            failures.append(f"{relative}: reads {match.group(1)!r}, workspace is {version!r} (expected {expected!r})")

    if failures:
        print(f"check_release_version_consistency: workspace is {version}, but mirrored literals disagree:")
        for failure in failures:
            print(f"  {failure}")
        print("\nThese are overwritten during packaging, so a release still publishes the right version — but the")
        print("committed values look authoritative and are not. Update them to match the workspace version.")
        return 1

    print(f"check_release_version_consistency: workspace is {version}, mirrored literals agree")

    stale = [(path, recorded) for path, recorded in example_lock_versions() if recorded != version]
    if stale:
        print(f"  note: {len(stale)} example lockfile(s) still record an older compiler; they update when rebuilt:")
        for path, recorded in stale:
            print(f"    {path}: {recorded}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
