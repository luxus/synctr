# Changelog

## 0.1.0 - 2026-08-30

First release. Tag it `v0.1.0`. Cargo.toml is already `0.1.0`.

This is the first cut that compiles and is meant to be usable: named rclone profiles, gitignore-style ignores, `sync --dry-run`, a TUI, optional watch, and unit-file install. rclone stays a subprocess. synctr does not create remotes.

### What you get

- `synctr profile add|list|show|edit|rename|remove`. Edit rewrites the existing toml. Rename moves the toml, ignore file, and last-run together.
- Profile modes `copy`, `sync`, and `bisync`. `synctr sync <name>` runs that mode.
- `synctr sync --dry-run` uses the same argv path as a real run and appends rclone `--dry-run`.
- Default ignores (`node_modules/`, `.git/`, `target/`, `dist/`, `.DS_Store`, `._*`, `*.partial`) plus per-profile and shared ignore files, emitted as rclone `--filter-from`.
- rclone discovery: `--rclone` / profile field / `SYNCTR_RCLONE` → PATH → nix-darwin paths → Homebrew.
- TUI: left profiles, right last run/state, bottom rclone log. Enter start/stop, `d` dry-run, `j`/`k` move, pgup/pgdn scroll the log, `r` reload profiles, `?` help, `q` quit.
- `synctr watch` (opt-in, debounced). The TUI still starts jobs with Enter. Not a daemon.
- `synctr schedule generate|install|uninstall` writes systemd user units or a launchd plist. It does not enable, load, or start them.
- `synctr status --json` is the contract for the Noctalia plugin and any later menu extra. There is no second status format.
- `contrib/noctalia/synctr`: small Luau widget that shells `synctr status --json`.
- GitHub Actions: `cargo test --workspace --locked` on PR and push to main. Release artifacts on `main`, `v*` tags, and `workflow_dispatch` for linux x86_64 and darwin arm+intel, named `synctr-<version>-<target>`. No aarch64-linux, notarization, or brew.

### How we got here

main's first cut (engine + CLI + TUI + flake) did not compile. The TUI reconstruction left an undefined `len` in `move_sel`, a leftover `sync_selected` fragment, a broken `render_stateful_widget`, and a truncated `age_label`. `which-rclone` was missing `--profile`. That compile fix is in this release. Then edit, dry-run, watch, schedule, TUI keys, docs, and the status JSON contract.

### Still out

- A real Mac menu bar extra. `contrib/menubar/` is a `synctr status --json` consumer sketch. No Swift app in this tree.
- Creating rclone remotes or Drive folders. The remote in `remote:path` must already exist.
- Automatic bisync `--resync`. The first bisync run will fail with rclone's error until you resync yourself.
- synctr starting launchd or systemd for you. `schedule install` only writes files.
- Notarized macOS binaries, a brew tap, File Provider, telemetry.

See README.md for install and a first `profile add` / `sync` / TUI session.
