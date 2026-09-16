# synctr

Named rclone profiles, gitignore-style ignores, and a TUI. rclone is a subprocess. synctr does not run `rclone config` and does not create Drive remotes.

Version **0.1.0**. Tag that as `v0.1.0`.

## You need this first

1. rclone, already installed. `synctr which-rclone` prints the binary it will use.
2. A remote that already exists. `remote:path` in a profile is passed straight to rclone. If `rclone lsd remote:` fails, `synctr sync` will fail the same way.
3. For **bisync** only: rclone wants a first `--resync` (or its current equivalent) before a normal bisync will run. synctr never passes that flag. The first `synctr sync` on a new bisync profile will fail with rclone's error until you resync yourself.

## Install

### Cargo, from a clone

Rust **1.88** or newer (workspace `rust-version`). 1.83 cannot parse the locked crates.

```
git clone git@github.com:luxus/synctr.git
cd synctr
cargo test --workspace --locked
cargo build --release -p synctr
./target/release/synctr --help
```

Put `target/release/synctr` on your PATH if you want `synctr` as a command.

### Flake, from a clone

```
nix run
# or
nix build
./result/bin/synctr --help
```

Another flake can take `inputs.synctr.packages.${system}.synctr`. The package is `rustPlatform.buildRustPackage` with the committed `Cargo.lock`, crates.io only.

### Release tarball

After you tag `v0.1.0`, Actions uploads:

- `synctr-0.1.0-x86_64-unknown-linux-gnu`
- `synctr-0.1.0-aarch64-apple-darwin`
- `synctr-0.1.0-x86_64-apple-darwin`

No aarch64-linux artifact, no notarization, no brew tap.

## First run

Pick a local folder and a remote that already works with rclone.

```
synctr which-rclone
synctr profile add docs \
  --local ~/docs \
  --remote b2:bucket/docs \
  --mode sync
synctr profile list
synctr sync docs --dry-run
synctr sync docs
synctr status --json
synctr tui
```

`--dry-run` uses the same argv as a real run and appends rclone `--dry-run`. rclone writes nothing. synctr still records last-run.

`--rclone PATH` overrides discovery for that process. `SYNCTR_RCLONE` and a profile `rclone` field do the same, in that order. `--config-dir DIR` overrides `~/.config/synctr`.

Modes on `profile add` / `profile edit`: `copy`, `sync`, `bisync`. `synctr sync <name>` runs whatever mode the profile has.

## TUI keys

Left pane is profiles, right is last run / rclone path / state, bottom is the rclone log.

| Key | Action |
| --- | --- |
| Enter | start or stop the selected profile |
| d | dry-run the selected profile |
| j / k | next / previous profile |
| pgup / pgdn | scroll the rclone log |
| r | reload profiles from disk |
| ? or h | help |
| q or Esc | quit |

One rclone child at a time. Enter on another profile stops the current one. Directory watch is `synctr watch`, not the TUI.

## `status --json`

This is the contract for the Noctalia plugin and any later menu extra. Do not invent a second format.

```
synctr status --json
```

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

The Noctalia widget lives at `contrib/noctalia/synctr` and shells that command. A real Mac menu bar extra is not in this release. `contrib/menubar/` is a sketch that parses the same JSON.

## Other commands

```
synctr profile show <name>
synctr profile edit <name> [--local PATH] [--remote remote:path] [--mode copy|sync|bisync]
                             [--rclone PATH | --clear-rclone]
                             [--flag ARG ...] [--clear-flags]
                             [--ignore PATTERN ...] [--clear-ignore]
synctr profile rename <old> <new>
synctr profile remove <name>
synctr watch <name> [--debounce-ms 1500]
synctr schedule generate <name> [--kind systemd|launchd] [--interval SECS] [--bin PATH]
synctr schedule install <name> [--kind systemd|launchd] [--interval SECS] [--bin PATH] [--dir DIR]
synctr schedule uninstall <name> [--kind systemd|launchd] [--dir DIR]
synctr which-rclone [--profile NAME]
```

`--json` also works on `which-rclone`, `profile list`, and `schedule generate`.

`watch` is opt-in. Changes under the profile's local directory debounce, then run the same sync path. Ignored paths do not wake it. Ctrl-C stops it. It is not a login daemon.

`schedule install` writes a systemd user unit+timer or a LaunchAgents plist. It prints the enable/load command. It does not run systemctl or launchctl.

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

1. `--rclone`, then the profile `rclone` field, then `SYNCTR_RCLONE`
2. `PATH`
3. `/etc/profiles/per-user/$USER/bin/rclone`
4. `/run/current-system/sw/bin/rclone`
5. `/opt/homebrew/bin/rclone`
6. `/usr/local/bin/rclone`

GUI-less sessions often have a short `PATH`. The nix and Homebrew paths are searched anyway.

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

## What 0.1.0 does not do

See CHANGELOG.md. Short version: no real Mac menu extra, no remote creation, no automatic bisync `--resync`, no synctr-started launchd/systemd, no brew tap.
