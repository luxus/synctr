# synctr

CLI and TUI for rclone folder sync. Profiles live as TOML. Ignore rules are gitignore-style and become rclone `--filter-from` when a sync runs. The engine library has no clap or ratatui; `synctr` is the frontend.

Works on macOS and Linux/NixOS. No GUI, launchd, telemetry, or File Provider.

## Commands

```
synctr profile add <name> --local PATH --remote remote:path --mode copy|sync|bisync
synctr profile list
synctr profile show <name>
synctr profile remove <name>
synctr sync <name>
synctr which-rclone [--profile NAME]
synctr status
synctr tui
```

`--json` on `status`, `which-rclone`, and `profile list` prints structs a later Luau widget can `runAsync`. `--rclone PATH` overrides discovery. `--config-dir DIR` overrides XDG. `which-rclone --profile NAME` uses that profile's `rclone` field.

`synctr tui`: left pane is profiles, right is last run / rclone path / state, bottom is the rclone log. Enter starts or stops the selected profile through the engine. `q` quits.

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

`synctr sync` builds argv as separate arguments (no shell). It always passes `--filter-from`, `--verbose`, and `--use-json-log`, then the profile's extra flags. Exit status is rclone's.

## Config

`$XDG_CONFIG_HOME/synctr` or `~/.config/synctr`.

```
~/.config/synctr/profiles/<name>.toml
~/.config/synctr/profiles/<name>.ignore
~/.config/synctr/ignore
~/.local/state/synctr/runs/<name>.toml
```

Profile fields: `name`, `local`, `remote`, `mode`, optional `rclone`, `extra_flags`, `extra_ignore`.

## Build

```
cargo test --workspace
cargo build -p synctr
nix build
nix run
```

Flake outputs, each of `aarch64-darwin`, `x86_64-darwin`, `x86_64-linux`, `aarch64-linux`:

- `packages.<system>.default`
- `packages.<system>.synctr`
- `apps.<system>.default`

`Cargo.lock` is committed. The package is `rustPlatform.buildRustPackage` with that lock file, crates.io only, no git crates. Another flake can take `inputs.synctr.packages.${system}.synctr`.
