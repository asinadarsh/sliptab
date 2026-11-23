#!/usr/bin/env python3
"""Writes the open-source licences of everything Sliptab ships, for the Licences screen.

    python3 scripts/licenses.py <cargo workspace dir> <out.json> [node_modules dir] [npm package ...]

Rust: every crate reachable through normal (not dev or build) dependencies from the workspace
members, with the licence files from its source folder. npm: only the named packages, the ones whose
code or fonts end up in the app (the rest are build tools). Identical licence texts are stored once.
Our own crates are left out. Run it again when dependencies change.
"""
import json
import pathlib
import subprocess
import sys

NAMES = ("LICENSE", "LICENCE", "COPYING", "NOTICE", "OFL", "UNLICENSE")


def texts(folder: pathlib.Path) -> list[str]:
    files = sorted(p for p in folder.iterdir() if p.is_file() and p.name.upper().startswith(NAMES))
    return [p.read_text(errors="replace").strip() for p in files]


def rust(workspace: str) -> list[dict]:
    meta = json.loads(subprocess.check_output(
        ["cargo", "metadata", "--format-version", "1", "--locked"], cwd=workspace))
    pkgs = {p["id"]: p for p in meta["packages"]}
    nodes = {n["id"]: n for n in meta["resolve"]["nodes"]}
    seen, todo = set(), list(meta["workspace_members"])
    while todo:
        i = todo.pop()
        if i in seen:
            continue
        seen.add(i)
        for d in nodes[i]["deps"]:
            if any(k["kind"] is None for k in d["dep_kinds"]):
                todo.append(d["pkg"])
    out = []
    for i in seen:
        p = pkgs[i]
        if p["source"] is None or "asinadarsh/" in p["source"]:  # ours (path or our git repos)
            continue
        out.append({"name": p["name"], "version": p["version"], "license": p["license"] or "",
                    "url": p.get("repository") or "",
                    "texts": texts(pathlib.Path(p["manifest_path"]).parent)})
    return out


def npm(modules: str, names: list[str]) -> list[dict]:
    out = []
    for n in names:
        d = pathlib.Path(modules) / n
        pj = json.loads((d / "package.json").read_text())
        repo = pj.get("repository")
        out.append({"name": n, "version": pj["version"], "license": pj.get("license", ""),
                    "url": repo.get("url", "") if isinstance(repo, dict) else (repo or ""),
                    "texts": texts(d)})
    return out


def main() -> None:
    workspace, dest, *rest = sys.argv[1:]
    items = rust(workspace) + (npm(rest[0], rest[1:]) if rest else [])
    items.sort(key=lambda x: (x["name"].lower(), x["version"]))
    pool: dict[str, int] = {}
    for it in items:
        it["texts"] = [pool.setdefault(t, len(pool)) for t in it["texts"]]
    missing = [f'{it["name"]} {it["version"]}' for it in items if not it["texts"]]
    json.dump({"packages": items, "texts": list(pool)}, open(dest, "w"), separators=(",", ":"))
    print(f"{len(items)} packages, {len(pool)} distinct texts, {len(missing)} without a licence file")
    if missing:
        print("  no file (licence id only):", ", ".join(missing[:40]))


if __name__ == "__main__":
    main()
