#!/usr/bin/env python3
"""Publish stable, beta, and alpha boxpigma update feeds from staged releases.

Each staged release is a `boxpigma-v<version>/boxpigma-updates.json` directory
under the mirror root. The newest release allowed by each channel is copied to
the root feed file; the manifest inside the release already points at the
mirror's assets.
"""

from __future__ import annotations

import argparse
import json
import os
import re
from dataclasses import dataclass
from pathlib import Path

SEMVER = re.compile(
    r"^(\d+)\.(\d+)\.(\d+)(?:-([0-9A-Za-z.-]+))?(?:\+[0-9A-Za-z.-]+)?$"
)
CHANNELS = ("stable", "beta", "alpha")
ALLOWED = {
    "stable": {"stable"},
    "beta": {"stable", "beta"},
    "alpha": {"stable", "beta", "alpha"},
}
MANIFESTS = {
    "stable": "boxpigma-updates.json",
    "beta": "boxpigma-updates-beta.json",
    "alpha": "boxpigma-updates-alpha.json",
}


@dataclass(frozen=True)
class Release:
    directory: Path
    version: str
    channel: str


def version_key(version: str) -> tuple[object, ...]:
    match = SEMVER.fullmatch(version)
    if match is None:
        raise ValueError(f"invalid release version: {version}")
    main = tuple(int(part) for part in match.group(1, 2, 3))
    prerelease = match.group(4)
    if prerelease is None:
        return (main, 1, ())
    identifiers = tuple(
        (0, int(part)) if part.isdigit() else (1, part)
        for part in prerelease.split(".")
    )
    return (main, 0, identifiers)


def channel_for_version(version: str) -> str:
    match = SEMVER.fullmatch(version)
    if match is None:
        raise ValueError(f"invalid release version: {version}")
    prerelease = match.group(4)
    if prerelease is None:
        return "stable"
    return "alpha" if prerelease.startswith("alpha") else "beta"


def load_releases(root: Path) -> list[Release]:
    releases: list[Release] = []
    for directory in root.iterdir():
        if not directory.is_dir() or not directory.name.startswith("boxpigma-v"):
            continue
        version = directory.name.removeprefix("boxpigma-v")
        manifest_path = directory / "boxpigma-updates.json"
        if not manifest_path.is_file():
            continue
        try:
            channel = channel_for_version(version)
            manifest = json.loads(manifest_path.read_text())
        except (OSError, ValueError):
            continue
        if not isinstance(manifest, dict) or manifest.get("version") != version:
            continue
        releases.append(Release(directory, version, channel))
    return releases


def latest_for(releases: list[Release], channel: str) -> Release | None:
    candidates = [
        release
        for release in releases
        if release.channel in ALLOWED[channel]
    ]
    return max(
        candidates,
        key=lambda release: version_key(release.version),
        default=None,
    )


def write_atomic(path: Path, content: str) -> None:
    temp = path.with_name(f".{path.name}.{os.getpid()}.tmp")
    temp.write_text(content)
    os.replace(temp, path)


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument(
        "--root",
        type=Path,
        default=Path(
            os.environ.get("BOXPIGMA_UPDATE_ROOT", "/var/www/ferriso-updates")
        ),
    )
    args = parser.parse_args()
    root = args.root
    if not root.is_dir():
        parser.error(f"mirror root is missing: {root}")

    releases = load_releases(root)
    changed = {}
    for channel in CHANNELS:
        target = latest_for(releases, channel)
        if target is None:
            continue
        manifest = target.directory / "boxpigma-updates.json"
        destination = root / MANIFESTS[channel]
        write_atomic(destination, manifest.read_text())
        changed[channel] = target.version
    for channel, version in changed.items():
        print(f"{channel}: {version}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
