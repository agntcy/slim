#!/usr/bin/env python3
# Copyright AGNTCY Contributors (https://github.com/agntcy)
# SPDX-License-Identifier: Apache-2.0
"""Check that every crate release-plz publishes resolves its dependencies
from crates.io on its own.

This is the step of `release-plz update` that has broken releases on main:
cargo-semver-checks packages each crate and runs `cargo update` on it against
crates.io, which fails when a required version is yanked or missing. Doing
only that, without building anything, takes about a minute instead of the
half hour the full semver check takes.

A workspace crate whose current version isn't on crates.io yet (a release PR
bumped it) is taken from the workspace instead, since release-plz publishes
it first. A yanked version is still a failure.
"""

import json
import pathlib
import subprocess
import sys
import tarfile
import tempfile
import tomllib
import urllib.error
import urllib.request

ROOT = pathlib.Path(__file__).resolve().parents[2]


def publishable() -> list[str]:
    config = tomllib.loads((ROOT / ".release-plz.toml").read_text())
    return [p["name"] for p in config.get("package", []) if p.get("publish")]


def local_versions() -> dict[str, str]:
    metadata = subprocess.run(
        ["cargo", "metadata", "--no-deps", "--format-version", "1"],
        cwd=ROOT,
        check=True,
        capture_output=True,
        text=True,
    ).stdout
    return {p["name"]: p["version"] for p in json.loads(metadata)["packages"]}


def published_versions(name: str) -> set[str]:
    """Every version on crates.io, yanked ones included (sparse index)."""
    n = name.lower()
    prefix = {1: "1", 2: "2", 3: f"3/{n[0]}"}.get(len(n), f"{n[:2]}/{n[2:4]}")
    try:
        with urllib.request.urlopen(f"https://index.crates.io/{prefix}/{n}") as resp:
            lines = resp.read().decode().splitlines()
    except urllib.error.HTTPError as e:
        if e.code == 404:
            return set()
        raise
    return {json.loads(line)["vers"] for line in lines if line}


def main() -> int:
    names = publishable()
    versions = local_versions()
    pending = {n for n in names if versions[n] not in published_versions(n)}
    if pending:
        print(
            "Not on crates.io yet, taken from the workspace:",
            ", ".join(sorted(pending)),
        )

    package = ["cargo", "package", "--no-verify", "--allow-dirty"]
    subprocess.run(
        package + [a for n in names for a in ("-p", n)], cwd=ROOT, check=True
    )

    failed = []
    with tempfile.TemporaryDirectory() as tmp:
        unpacked = {n: pathlib.Path(tmp) / f"{n}-{versions[n]}" for n in names}
        for n in names:
            crate = ROOT / "target" / "package" / f"{n}-{versions[n]}.crate"
            with tarfile.open(crate) as archive:
                archive.extractall(tmp, filter="data")

        for n in names:
            manifest = unpacked[n] / "Cargo.toml"
            patches = [
                f'{p} = {{ path = "{unpacked[p]}" }}' for p in sorted(pending) if p != n
            ]
            if patches:
                with manifest.open("a") as f:
                    f.write("\n[patch.crates-io]\n" + "\n".join(patches) + "\n")
            # Run from the workspace root so rust-toolchain.toml still applies.
            result = subprocess.run(
                ["cargo", "generate-lockfile", "--manifest-path", str(manifest)],
                cwd=ROOT,
                capture_output=True,
                text=True,
            )
            if result.returncode != 0:
                failed.append(n)
                print(
                    f"::error::{n} {versions[n]} doesn't resolve its dependencies from crates.io"
                )
                print(result.stderr)

    if failed:
        return 1
    print(
        f"All {len(names)} published crates resolve their dependencies from crates.io."
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
