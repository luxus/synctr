# synctr

CLI and TUI for rclone folder sync. Profiles live as TOML. Ignore rules are gitignore-style and become rclone `--filter-from` when a sync runs. The engine library has no clap or ratatui; `synctr` is the frontend.

Works on macOS and Linux/NixOS. rclone is a subprocess, not librclone. synctr does not run `rclone config` and does not create Drive remotes. The remote in `remote:path` must already exist.

bisync needs a first `--resync` (or rclone's equivalent) before it will run normally. synctr does not pass that automatically. The first `synctr sync` on a new bisync profile will fail with rclone's error until you resync yourself.

## Commands

```
synctr profile add <name> --local PATH --remote remote:path --mode copy|sync|bisync
synctr profile list
synctr profile show <name>
synctr profile edit <name> [--local PATH] [--remote remote:path] [--mode copy|sync|bisync]
                             [--rclone PATH | --clear-rclone]
                             [--flag ARG ...] [--clear-flags]
                             [--ignore PATTERN ...] [--clear-ignore]
synctr profile rename <old> <new>
synctr profile remove <name>
synctr sync <name> [--dry-run]
synctr watch <name> [--debounce-ms 1500]
synctr schedule generate <name> [--kind systemd|launchd] [--interval SECS] [--bin PATH]
synctr schedule install <name> [--kind systemd|launchd] [--interval SECS] [--bin PATH] [--dir DIR]
synctr schedule uninstall <name> [--kind systemd|launchd] [--dir DIR]
synctr which-rclone [--profile NAME]
synctr status [--json]
synctr tui
```

`--json` on `status`, `which-rclone`, `profile list`, and `schedule generate` prints structs. The bar/plugin contract is `synctr status --json` only. Do not add a second status format. `--rclone PATH` overrides discovery. `--config-dir DIR` overrides XDG. `which-rclone --profile NAME` uses that profile's `rclone` field.

`synctr sync --dry-run` uses the same argv path as a real run and appends rclone `--dry-run`. rclone writes nothing. Last-run is still recorded.

`synctr watch` is opt-in. It is not a daemon and the TUI still starts jobs with Enter. Changes under the profile's local directory debounce, then call the same `run_sync` path. Ignored paths (node_modules, .git, …) do not wake it.

`synctr schedule install` writes a systemd user unit+timer or a launchd plist. It does not enable, load, or start anything. `generate` prints the files. Tests assert contents; CI never talks to systemd or launchd.

`synctr tui`: left pane is profiles, right is last run / rclone path / state, bottom is the rclone log. Enter starts or stops the selected profile. `d` dry-runs. `j`/`k` move. `pgup`/`pgdn` scroll the log. `r` reloads profiles from disk. `?` help. `q` quits.

## Ignore

Defaults, always applied unless a user rule matches first:

```
node_modules/
.git/
target/
dist/
.DS_Store
._*
*.partial
```

More patterns, gitignore syntax, from:

1. profile `extra_ignore` / `synctr profile add --ignore PATTERN`
2. `~/.config/synctr/profiles/<name>.ignore`
3. `~/.config/synctr/ignore`

`!pattern` is an include (rclone `+`). First match wins, so a profile `!node_modules/` syncs that tree.

## rclone path

One resolver, this order:

1. `--rclone`, then the profile `rclone` field, then `SYNCTR_RCLONE`
2. `PATH`
3. `/etc/profiles/per-user/$USER/bin/rclone`
4. `/run/current-system/sw/bin/rclone`
5. `/opt/homebrew/bin/rclone`
6. `/usr/local/bin/rclone`

GUI-less sessions often have a short `PATH`. The nix and Homebrew paths are searched anyway. `synctr which-rclone` prints the path and why. Pass `--profile NAME` to apply that profile's override.

`synctr sync` builds argv as separate arguments (no shell). It always passes `--filter-from`, `--verbose`, and `--use-json-log`, then the profile's extra flags, then `--dry-run` when requested. Exit status is rclone's.

## Config

`$XDG_CONFIG_HOME/synctr` or `~/.config/synctr`.

```
~/.config/synctr/profiles/<name>.toml
~/.config/synctr/profiles/<name>.ignore
~/.config/synctr/ignore
~/.local/state/synctr/runs/<name>.toml
```

Profile fields: `name`, `local`, `remote`, `mode`, optional `rclone`, `extra_flags`, `extra_ignore`.

`profile edit` rewrites the existing toml. `profile rename` moves the toml, ignore file, and last-run file together. It does not delete and recreate.

## status --json

The Noctalia plugin (`contrib/noctalia/synctr`) and any later menu bar extra read this object and nothing else:

```
{
  "rclone": { "found": true, "path": "...", "source": "PATH", "detail": "..." },
  "profiles": [
    {
      "name": "docs",
      "local": "/home/luxus/docs",
      "remote": "remote:path",
      "mode": "sync",
      "extra_flags": [],
      "extra_ignore": [],
      "last_run": {
        "finished_at_unix": 0,
        "finished_at": "1970-01-01T00:00:00Z",
        "exit_code": 0,
        "ok": true
      }
    }
  ]
}
```

`last_run` is omitted when the profile has never run. `rclone.path` / `source` / `detail` are omitted when rclone is missing.

Point a Noctalia path source at `contrib/noctalia/synctr`. This tree cannot run Noctalia, so the plugin is shipped with a test that the fields it reads are the fields `status --json` emits.

A macOS menu bar extra is not in this crate. See `contrib/menubar/`.

## Build

```
cargo test --workspace
cargo build -p synctr
nix build
nix run
```

GitHub Actions: `cargo test --workspace --locked` on every PR and push to `main`. Release binaries on `main`, `v*` tags, and `workflow_dispatch` for `x86_64-unknown-linux-gnu`, `aarch64-apple-darwin`, and `x86_64-apple-darwin`, named `synctr-<version>-<target>`. No aarch64-linux, notarization, or brew tap.

Flake outputs, each of `aarch64-darwin`, `x86_64-darwin`, `x86_64-linux`, `aarch64-linux`:

- `packages.<system>.default`
- `packages.<system>.synctr`
- `apps.<system>.default`

`Cargo.lock` is committed. The package is `rustPlatform.buildRustPackage` with that lock file, crates.io only, no git crates. Another flake can take `inputs.synctr.packages.${system}.synctr`.
