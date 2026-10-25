#!/usr/bin/env python3
"""Stage one boxpigma release as a directory the update mirror can serve."""

from __future__ import annotations

import argparse
import json
import shutil
import urllib.parse
from pathlib import Path

DEFAULT_BASE_URL = "https://updates.bionickernel.com"


def local_name(url: str) -> str:
    return urllib.parse.unquote(
        urllib.parse.urlparse(url).path.rsplit("/", 1)[-1]
    )


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--dist", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--tag", required=True, help="release tag, e.g. v1.8.0")
    parser.add_argument("--base-url", default=DEFAULT_BASE_URL)
    args = parser.parse_args()

    if not args.tag.startswith("v"):
        parser.error("tag must start with v")
    version = args.tag.removeprefix("v")
    source = args.dist / "boxpigma-updates.json"
    manifest = json.loads(source.read_text())
    if not isinstance(manifest, dict):
        parser.error("manifest is not an object")
    if manifest.get("version") != version:
        parser.error(
            f"manifest version {manifest.get('version')} does not match {version}"
        )

    release_name = f"boxpigma-{args.tag}"
    release = args.output / release_name
    release.mkdir(parents=True, exist_ok=True)
    for asset in args.dist.rglob("*"):
        if asset.is_file():
            shutil.copy2(asset, release / asset.name)

    base = f"{args.base_url.rstrip('/')}/{release_name}"
    platforms = manifest.get("platforms", {})
    if not isinstance(platforms, dict):
        parser.error("manifest has no platforms")
    for platform in platforms.values():
        if not isinstance(platform, dict):
            continue
        url = platform.get("url")
        if isinstance(url, str) and url:
            platform["url"] = f"{base}/{local_name(url)}"
        install_urls = platform.get("install_urls")
        if isinstance(install_urls, list):
            platform["install_urls"] = [
                f"{base}/{local_name(url)}"
                if isinstance(url, str) and url
                else url
                for url in install_urls
            ]

    for platform in platforms.values():
        if not isinstance(platform, dict):
            continue
        for key in ("url", "install_urls"):
            value = platform.get(key)
            urls = value if isinstance(value, list) else [value]
            for url in urls:
                if isinstance(url, str) and url:
                    name = local_name(url)
                    if not (release / name).is_file():
                        parser.error(f"manifest references missing asset {name}")

    (release / "boxpigma-updates.json").write_text(
        json.dumps(manifest, indent=2) + "\n"
    )
    print(f"staged {args.tag} in {release}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
