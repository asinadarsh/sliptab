# Contributing to Sliptab

Thanks for helping. Bug reports, fixes, tests on new desktops and small features are all welcome.

## Before you start

- **Bugs:** open an [issue](https://github.com/asinadarsh/sliptab/issues/new?template=bug.yml) with your
  OS, desktop (e.g. GNOME on Wayland, KDE on X11, Windows 11) and steps to reproduce.
- **Features:** open an [idea](https://github.com/asinadarsh/sliptab/issues/new?template=idea.yml) first,
  so we agree on it before you write code.
- **Security problems:** don't open a public issue. Email support@sliptab.com with "Security" in the subject.

## Set up

Needs Rust (stable), Node 24 and the [Tauri 2 prerequisites](https://v2.tauri.app/start/prerequisites/).

```bash
git clone https://github.com/asinadarsh/sliptab && cd sliptab
cargo test --workspace
cd desktop && npm ci && npm run tauri dev
```

## Making a change

1. Fork, then branch from `main`.
2. Keep the pull request to one change, and follow the code around it.
3. Run what CI runs:
   ```bash
   cargo clippy --workspace --all-targets -- -D warnings
   cargo test --workspace
   cd desktop && npm run check && npm test
   ```
4. For anything visible, add a screenshot or short clip to the pull request.
5. If you changed how the edge bar behaves on a desktop, update `desktop/PLATFORMS.md`.

## Review and releases

Every pull request from outside the team is reviewed by a maintainer before it is merged. Releases
are built only by this repository's GitHub Actions, and each one is approved by hand before it is
signed (see the [code signing policy](README.md#code-signing-policy)).

## Licence of contributions

Sliptab is GPL-3.0-only. By opening a pull request you agree that your contribution is licensed
under the same licence. Only submit code you wrote or have the right to share under it.

## Be kind

Assume good intent, keep feedback about the code, and help newcomers find their way.
