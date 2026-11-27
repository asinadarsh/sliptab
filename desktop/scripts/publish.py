#!/usr/bin/env python3
"""Publishes a desktop release to the sync server, for the apps' auto-updater.

  publish.py <version> <dist-dir>            installers + latest.json
  publish.py --android <version> <apk>       the phone app (until it is on the Play Store)
  publish.py --min <platform> <version>      oldest version still allowed to sync

Uploads every installer and update package found in dist-dir (as laid out by the desktop-build
workflow), then writes latest.json (the Tauri updater manifest) last, so apps never see a
manifest pointing at files that are not there yet. Needs EDGE_PUBLISH_TOKEN.
"""
import datetime
import json
import os
import pathlib
import sys
import time
import urllib.error
import urllib.request

SERVER = "https://edge-sync.forcesync.in"
UPDATE_PACKAGE = {  # which file the updater installs on each platform
    "linux-x86_64": ".AppImage",
    "windows-x86_64": "-setup.exe",
    "darwin-aarch64": ".app.tar.gz",
}


PIECE = 16 * 1024 * 1024  # the tunnel breaks one big request ("broken pipe"); send pieces


def _send(url: str, data: bytes, expect: tuple) -> None:
    for attempt in range(4):
        req = urllib.request.Request(url, data=data, method="PUT",
                                     headers={"Authorization": f"Bearer {os.environ['EDGE_PUBLISH_TOKEN']}",
                                              # Cloudflare's bot rules refuse Python's default agent (403)
                                              "User-Agent": "sliptab-publish/1"})
        try:
            with urllib.request.urlopen(req, timeout=300) as r:
                assert r.status in expect, (url, r.status)
                return
        except (OSError, urllib.error.URLError) as e:
            if attempt == 3:
                raise
            print(f"  retry {attempt + 1} after: {e}")
            time.sleep(3 * (attempt + 1))


def put(name: str, data: bytes) -> None:
    url = f"{SERVER}/v1/admin/releases/{name}"
    if len(data) <= PIECE or name.endswith(".json"):
        _send(url, data, (204,))
    else:
        for off in range(0, len(data), PIECE):
            chunk = data[off:off + PIECE]
            last = off + len(chunk) == len(data)
            _send(f"{url}?offset={off}&total={len(data)}", chunk, (204,) if last else (202,))
    print(f"published {name} ({len(data) // 1024} KB)")


def safe(name: str) -> str:
    """Server names are letters, digits, . _ - (an .app.tar.gz may contain spaces)."""
    return "".join(c if c.isalnum() or c in "._-" else "_" for c in name)


def release(version: str, dist: pathlib.Path) -> None:
    platforms = {}
    for key, suffix in UPDATE_PACKAGE.items():
        files = sorted((dist / f"edge-{key}").rglob("*"))
        for f in files:
            if f.is_file() and not f.name.endswith(".sig"):
                put(safe(f.name), f.read_bytes())
        pkg = next((f for f in files if f.name.endswith(suffix)), None)
        sig = pkg and pkg.with_name(pkg.name + ".sig")
        if pkg and sig.exists():
            platforms[key] = {"signature": sig.read_text().strip(), "url": f"{SERVER}/downloads/{safe(pkg.name)}"}
        else:
            print(f"warning: no signed update package for {key}")
    if not platforms:
        sys.exit("nothing to publish")
    manifest = {
        "version": version,
        "notes": f"Sliptab {version}",
        "pub_date": datetime.datetime.now(datetime.timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ"),
        "platforms": platforms,
    }
    put("latest.json", json.dumps(manifest, indent=2).encode())


def set_min(platform: str, version: str) -> None:
    mins = {}
    for p in ("desktop", "android"):  # keep the other platform's minimum as it is
        req = urllib.request.Request(f"{SERVER}/v1/app/status?platform={p}&version=0.0.0&nc=1",
                                     headers={"User-Agent": "sliptab-publish/1"})
        with urllib.request.urlopen(req, timeout=30) as r:
            if m := json.load(r).get("min"):
                mins[p] = m
    mins[platform] = version
    put("policy.json", json.dumps({"min": mins}).encode())
    print(f"{platform} older than {version} can no longer sync")


def android(version: str, apk: pathlib.Path) -> None:
    name = f"Edge_{safe(version)}.apk"
    put(name, apk.read_bytes())
    put("android.json", json.dumps({"version": version, "url": f"{SERVER}/downloads/{name}"}).encode())


if __name__ == "__main__":
    if sys.argv[1:2] == ["--android"]:
        android(sys.argv[2], pathlib.Path(sys.argv[3]))
    elif sys.argv[1:2] == ["--min"]:
        set_min(sys.argv[2], sys.argv[3])
    else:
        release(sys.argv[1], pathlib.Path(sys.argv[2]))
