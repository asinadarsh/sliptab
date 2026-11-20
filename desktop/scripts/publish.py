#!/usr/bin/env python3
"""Publishes a desktop release to the sync server, for the apps' auto-updater.

  publish.py <version> <dist-dir>            installers + latest.json
  publish.py --android <version> <apk>       the phone app (until it is on the Play Store)
  publish.py --min <platform> <version>      oldest version still allowed to sync

Uploads every installer and update package found in dist-dir (as laid out by the desktop-build
workflow), then writes latest.json (the Tauri updater manifest) last, so apps never see a
manifest pointing at files that are not there yet. Needs EDGE_ADMIN_TOKEN.
"""
import datetime
import json
import os
import pathlib
import sys
import urllib.request

SERVER = "https://edge-sync.forcesync.in"
UPDATE_PACKAGE = {  # which file the updater installs on each platform
    "linux-x86_64": ".AppImage",
    "windows-x86_64": "-setup.exe",
    "darwin-aarch64": ".app.tar.gz",
}


def put(name: str, data: bytes) -> None:
    req = urllib.request.Request(
        f"{SERVER}/v1/admin/releases/{name}",
        data=data,
        method="PUT",
        headers={"Authorization": f"Bearer {os.environ['EDGE_ADMIN_TOKEN']}"},
    )
    with urllib.request.urlopen(req, timeout=300) as r:
        assert r.status == 204, (name, r.status)
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
        with urllib.request.urlopen(f"{SERVER}/v1/app/status?platform={p}&version=0.0.0", timeout=30) as r:
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
