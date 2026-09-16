use std::io::{self, Write};
use std::path::PathBuf;
use std::process::ExitCode;
use std::time::Duration;

use clap::{Parser, Subcommand, ValueEnum};
use synctr_engine::{
    default_install_dir, doctor_report, enable_hint, generate_schedule, install_schedule,
    last_run_short, make_absolute, status_snapshot, test_remote, uninstall_schedule,
    write_status_json, Mode, Paths, Profile, ProfileEdit, ProfileStore, RcloneJson, ScheduleKind,
};

mod tui;
mod watch;

#[derive(Debug, Parser)]
#[command(
    name = "synctr",
    version = env!("CARGO_PKG_VERSION"),
    about = "rclone profiles, ignore rules, and a TUI"
)]
struct Cli {
    /// rclone binary (overrides profile, SYNCTR_RCLONE, and discovery)
    #[arg(long, global = true, value_name = "PATH")]
    rclone: Option<PathBuf>,

    /// Config directory (default: $XDG_CONFIG_HOME/synctr or ~/.config/synctr)
    #[arg(long, global = true, value_name = "DIR")]
    config_dir: Option<PathBuf>,

    /// JSON on stdout
    #[arg(long, global = true)]
    json: bool,

    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Add, list, show, edit, rename, or remove profiles
    Profile {
        #[command(subcommand)]
        command: ProfileCmd,
    },
    /// Run rclone for a profile
    Sync {
        name: String,
        /// Same argv as a real run, plus rclone --dry-run (writes nothing)
        #[arg(long)]
        dry_run: bool,
        /// rclone --resync (bisync only). First pass on a new bisync profile.
        #[arg(long)]
        resync: bool,
    },
    /// Sync when files under the profile's local directory change
    Watch {
        name: String,
        /// Quiet period before a change triggers rclone
        #[arg(long, default_value_t = 1500, value_name = "MS")]
        debounce_ms: u64,
    },
    /// Generate or install a launchd plist / systemd user unit
    Schedule {
        #[command(subcommand)]
        command: ScheduleCmd,
    },
    /// Print the rclone binary that would be used
    WhichRclone {
        /// Apply this profile's rclone override
        #[arg(long, value_name = "NAME")]
        profile: Option<String>,
    },
    /// Profiles, last run, and rclone path
    Status,
    /// rclone, config, and per-profile health (does not start rclone sync)
    Doctor,
    /// Probe a profile's remote with rclone lsd (timeout, no sync)
    TestRemote {
        name: String,
        /// Seconds before the probe is killed
        #[arg(long, default_value_t = 3, value_name = "SECS")]
        timeout: u64,
    },
    /// Interactive profile list, state, and rclone log
    Tui,
}

#[derive(Debug, Subcommand)]
enum ProfileCmd {
    /// Create a profile
    Add {
        name: String,
        #[arg(long)]
        local: PathBuf,
        #[arg(long)]
        remote: String,
        #[arg(long, value_enum)]
        mode: ModeArg,
        #[arg(long, value_name = "PATH")]
        rclone: Option<PathBuf>,
        #[arg(long = "flag", value_name = "ARG", allow_hyphen_values = true)]
        extra_flags: Vec<String>,
        #[arg(long = "ignore", value_name = "PATTERN")]
        extra_ignore: Vec<String>,
    },
    /// List profiles
    List,
    /// Print one profile
    Show { name: String },
    /// Change fields in place (does not delete and recreate)
    Edit {
        name: String,
        #[arg(long)]
        local: Option<PathBuf>,
        #[arg(long)]
        remote: Option<String>,
        #[arg(long, value_enum)]
        mode: Option<ModeArg>,
        #[arg(long, value_name = "PATH")]
        rclone: Option<PathBuf>,
        #[arg(long)]
        clear_rclone: bool,
        #[arg(long = "flag", value_name = "ARG", allow_hyphen_values = true)]
        extra_flags: Vec<String>,
        #[arg(long)]
        clear_flags: bool,
        #[arg(long = "ignore", value_name = "PATTERN")]
        extra_ignore: Vec<String>,
        #[arg(long)]
        clear_ignore: bool,
    },
    /// Move the toml, ignore file, and last-run together
    Rename { old: String, new: String },
    /// Delete a profile
    Remove { name: String },
    /// Allow sync / watch / TUI / schedule generate for this profile
    Enable { name: String },
    /// Refuse sync / watch / TUI Enter until enabled again
    Disable { name: String },
}

#[derive(Debug, Subcommand)]
enum ScheduleCmd {
    /// Print unit file contents (does not talk to launchd or systemd)
    Generate {
        name: String,
        #[arg(long, value_name = "KIND")]
        kind: Option<String>,
        /// Seconds between runs
        #[arg(long, default_value_t = 3600, value_name = "SECS")]
        interval: u64,
        /// synctr binary baked into the unit
        #[arg(long, value_name = "PATH")]
        bin: Option<PathBuf>,
    },
    /// Write unit files to a directory. Does not enable or start them.
    Install {
        name: String,
        #[arg(long, value_name = "KIND")]
        kind: Option<String>,
        #[arg(long, default_value_t = 3600, value_name = "SECS")]
        interval: u64,
        #[arg(long, value_name = "PATH")]
        bin: Option<PathBuf>,
        /// Destination directory (default: systemd user dir or LaunchAgents)
        #[arg(long, value_name = "DIR")]
        dir: Option<PathBuf>,
    },
    /// Remove unit files previously written by install
    Uninstall {
        name: String,
        #[arg(long, value_name = "KIND")]
        kind: Option<String>,
        #[arg(long, value_name = "DIR")]
        dir: Option<PathBuf>,
    },
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum ModeArg {
    Copy,
    Sync,
    Bisync,
}

impl From<ModeArg> for Mode {
    fn from(m: ModeArg) -> Self {
        match m {
            ModeArg::Copy => Mode::Copy,
            ModeArg::Sync => Mode::Sync,
            ModeArg::Bisync => Mode::Bisync,
        }
    }
}

fn main() -> ExitCode {
    match try_main() {
        Ok(code) => code,
        Err(e) => {
            eprintln!("{e}");
            ExitCode::from(1)
        }
    }
}

fn try_main() -> synctr_engine::Result<ExitCode> {
    let cli = Cli::parse();
    let paths = match &cli.config_dir {
        Some(dir) => Paths::from_config_dir(dir.clone()),
        None => Paths::from_env(),
    };
    let store = ProfileStore::new(paths);

    match cli.command {
        Command::Profile { command } => {
            profile_cmd(&store, cli.json, command)?;
            Ok(ExitCode::SUCCESS)
        }
        Command::Sync {
            name,
            dry_run,
            resync,
        } => {
            let profile = store.get(&name)?;
            profile.require_enabled()?;
            if resync && profile.mode != Mode::Bisync {
                return Err(synctr_engine::Error::ResyncNotBisync(
                    profile.name,
                    profile.mode.as_str(),
                ));
            }
            let outcome = synctr_engine::run_sync(
                store.paths(),
                &profile,
                cli.rclone.as_deref(),
                true,
                dry_run,
                resync,
            )?;
            Ok(ExitCode::from(outcome.exit_code as u8))
        }
        Command::Watch { name, debounce_ms } => {
            watch::run(&store, cli.rclone.as_deref(), &name, debounce_ms)?;
            Ok(ExitCode::SUCCESS)
        }
        Command::Schedule { command } => {
            let baked_config = cli
                .config_dir
                .as_ref()
                .map(|_| store.paths().config_dir.clone());
            schedule_cmd(&store, cli.json, baked_config.as_deref(), command)?;
            Ok(ExitCode::SUCCESS)
        }
        Command::WhichRclone { profile } => {
            which_rclone(&store, cli.rclone.as_deref(), cli.json, profile)
        }
        Command::Status => status_cmd(&store, cli.rclone.as_deref(), cli.json),
        Command::Doctor => doctor_cmd(&store, cli.rclone.as_deref(), cli.json),
        Command::TestRemote { name, timeout } => {
            test_remote_cmd(&store, cli.rclone.as_deref(), cli.json, name, timeout)
        }
        Command::Tui => {
            tui::run(&store, cli.rclone.as_deref())?;
            Ok(ExitCode::SUCCESS)
        }
    }
}

fn profile_cmd(store: &ProfileStore, json: bool, cmd: ProfileCmd) -> synctr_engine::Result<()> {
    match cmd {
        ProfileCmd::Add {
            name,
            local,
            remote,
            mode,
            rclone,
            extra_flags,
            extra_ignore,
        } => {
            let profile = Profile::new(
                name,
                local,
                remote,
                mode.into(),
                rclone,
                extra_flags,
                extra_ignore,
            )?;
            store.add(&profile)?;
            if json {
                print_json(&profile)?;
            } else {
                println!("added {}", profile.name);
            }
        }
        ProfileCmd::List => {
            let profiles = store.list()?;
            if json {
                print_json(&serde_json::json!({ "profiles": profiles }))?;
            } else if profiles.is_empty() {
                println!("no profiles");
            } else {
                for p in profiles {
                    if p.enabled {
                        println!(
                            "{}\t{}\t{}\t->\t{}",
                            p.name,
                            p.mode,
                            p.local.display(),
                            p.remote
                        );
                    } else {
                        println!(
                            "{}\t{}\t{}\t->\t{}\tdisabled",
                            p.name,
                            p.mode,
                            p.local.display(),
                            p.remote
                        );
                    }
                }
            }
        }
        ProfileCmd::Show { name } => {
            let profile = store.get(&name)?;
            if json {
                print_json(&profile)?;
            } else {
                print!("{}", toml_pretty(&profile)?);
            }
        }
        ProfileCmd::Edit {
            name,
            local,
            remote,
            mode,
            rclone,
            clear_rclone,
            extra_flags,
            clear_flags,
            extra_ignore,
            clear_ignore,
        } => {
            let edit = ProfileEdit {
                local,
                remote,
                mode: mode.map(Into::into),
                rclone: if clear_rclone {
                    Some(None)
                } else {
                    rclone.map(Some)
                },
                extra_flags: if clear_flags {
                    Some(Vec::new())
                } else if extra_flags.is_empty() {
                    None
                } else {
                    Some(extra_flags)
                },
                extra_ignore: if clear_ignore {
                    Some(Vec::new())
                } else if extra_ignore.is_empty() {
                    None
                } else {
                    Some(extra_ignore)
                },
                enabled: None,
            };
            let profile = store.edit(&name, edit)?;
            if json {
                print_json(&profile)?;
            } else {
                println!("edited {}", profile.name);
            }
        }
        ProfileCmd::Rename { old, new } => {
            let profile = store.rename(&old, &new)?;
            if json {
                print_json(&profile)?;
            } else {
                println!("renamed {old} -> {}", profile.name);
            }
        }
        ProfileCmd::Remove { name } => {
            store.remove(&name)?;
            if json {
                print_json(&serde_json::json!({ "removed": name }))?;
            } else {
                println!("removed {name}");
            }
        }
        ProfileCmd::Enable { name } => {
            let profile = store.set_enabled(&name, true)?;
            if json {
                print_json(&profile)?;
            } else {
                println!("enabled {}", profile.name);
            }
        }
        ProfileCmd::Disable { name } => {
            let profile = store.set_enabled(&name, false)?;
            if json {
                print_json(&profile)?;
            } else {
                println!("disabled {}", profile.name);
            }
        }
    }
    Ok(())
}

fn schedule_kind(kind: Option<String>) -> synctr_engine::Result<ScheduleKind> {
    match kind {
        Some(s) => ScheduleKind::parse(&s),
        None => Ok(ScheduleKind::default_for_host()),
    }
}

fn synctr_bin(bin: Option<PathBuf>) -> synctr_engine::Result<PathBuf> {
    match bin {
        Some(p) => Ok(make_absolute(&p)),
        None => std::env::current_exe().map_err(synctr_engine::Error::from),
    }
}

fn require_enabled_if_present(store: &ProfileStore, name: &str) -> synctr_engine::Result<()> {
    if let Ok(profile) = store.get(name) {
        profile.require_enabled()?;
    }
    Ok(())
}

fn schedule_cmd(
    store: &ProfileStore,
    json: bool,
    config_dir: Option<&std::path::Path>,
    cmd: ScheduleCmd,
) -> synctr_engine::Result<()> {
    match cmd {
        ScheduleCmd::Generate {
            name,
            kind,
            interval,
            bin,
        } => {
            require_enabled_if_present(store, &name)?;
            let spec = generate_schedule(
                schedule_kind(kind)?,
                &name,
                &synctr_bin(bin)?,
                interval,
                config_dir,
            )?;
            if json {
                print_json(&spec)?;
            } else {
                for file in spec.files {
                    println!("=== {} ===", file.name);
                    print!("{}", file.body);
                    if !file.body.ends_with('\n') {
                        println!();
                    }
                }
            }
        }
        ScheduleCmd::Install {
            name,
            kind,
            interval,
            bin,
            dir,
        } => {
            require_enabled_if_present(store, &name)?;
            let kind = schedule_kind(kind)?;
            let spec = generate_schedule(kind, &name, &synctr_bin(bin)?, interval, config_dir)?;
            let used_default = dir.is_none();
            let dest = match dir {
                Some(d) => d,
                None => default_install_dir(kind)?,
            };
            let written = install_schedule(&dest, &spec)?;
            if json {
                print_json(&serde_json::json!({
                    "kind": kind.as_str(),
                    "dir": dest,
                    "written": written,
                }))?;
            } else {
                println!("wrote {} file(s) under {}", written.len(), dest.display());
                for path in &written {
                    println!("{}", path.display());
                }
                println!("{}", enable_hint(kind, &dest, &name, used_default));
            }
        }
        ScheduleCmd::Uninstall { name, kind, dir } => {
            let kind = schedule_kind(kind)?;
            let dest = match dir {
                Some(d) => d,
                None => default_install_dir(kind)?,
            };
            let removed = uninstall_schedule(&dest, kind, &name)?;
            if json {
                print_json(&serde_json::json!({
                    "kind": kind.as_str(),
                    "dir": dest,
                    "removed": removed,
                }))?;
            } else if removed.is_empty() {
                println!("nothing to remove under {}", dest.display());
            } else {
                for path in removed {
                    println!("removed {}", path.display());
                }
            }
        }
    }
    Ok(())
}

fn which_rclone(
    store: &ProfileStore,
    rclone_flag: Option<&std::path::Path>,
    json: bool,
    profile: Option<String>,
) -> synctr_engine::Result<ExitCode> {
    let profile_rclone = match profile {
        Some(name) => store.get(&name)?.rclone,
        None => None,
    };
    let rclone = RcloneJson::from_live(rclone_flag, profile_rclone.as_deref())?;
    if json {
        print_json(&rclone)?;
        return Ok(if rclone.found {
            ExitCode::SUCCESS
        } else {
            ExitCode::from(1)
        });
    }
    match (rclone.found, rclone.path, rclone.detail) {
        (true, Some(path), Some(detail)) => {
            println!("{}", path.display());
            println!("source: {detail}");
            Ok(ExitCode::SUCCESS)
        }
        _ => Err(synctr_engine::Error::RcloneNotFound),
    }
}

fn status_cmd(
    store: &ProfileStore,
    rclone_flag: Option<&std::path::Path>,
    json: bool,
) -> synctr_engine::Result<ExitCode> {
    let snap = status_snapshot(store, rclone_flag, None)?;
    if json {
        write_status_json(io::stdout().lock(), &snap)?;
        return Ok(ExitCode::SUCCESS);
    }
    println!("rclone: {}", snap.rclone.human_line());
    if snap.profiles.is_empty() {
        println!("no profiles");
        return Ok(ExitCode::SUCCESS);
    }
    for p in snap.profiles {
        let last = match p.progress.as_ref() {
            Some(xfer) => format!("running {}", xfer.summary()),
            None => last_run_short(p.last_run.as_ref()),
        };
        if p.enabled {
            println!(
                "{}\t{}\t{}\t->\t{}\t{}",
                p.name,
                p.mode,
                p.local.display(),
                p.remote,
                last
            );
        } else {
            println!(
                "{}\t{}\t{}\t->\t{}\t{}\tdisabled",
                p.name,
                p.mode,
                p.local.display(),
                p.remote,
                last
            );
        }
    }
    Ok(ExitCode::SUCCESS)
}

fn doctor_cmd(
    store: &ProfileStore,
    rclone_flag: Option<&std::path::Path>,
    json: bool,
) -> synctr_engine::Result<ExitCode> {
    let report = doctor_report(store, rclone_flag)?;
    if json {
        print_json(&report)?;
    } else {
        report
            .write_human(io::stdout().lock())
            .map_err(synctr_engine::Error::from)?;
    }
    Ok(if report.ok {
        ExitCode::SUCCESS
    } else {
        ExitCode::from(1)
    })
}

fn test_remote_cmd(
    store: &ProfileStore,
    rclone_flag: Option<&std::path::Path>,
    json: bool,
    name: String,
    timeout_secs: u64,
) -> synctr_engine::Result<ExitCode> {
    let timeout = Duration::from_secs(timeout_secs.max(1));
    let probe = test_remote(store, rclone_flag, &name, timeout)?;
    if json {
        print_json(&probe)?;
    } else if probe.ok {
        println!("ok\t{}\t{}\t{}", probe.name, probe.remote, probe.detail);
    } else {
        println!("fail\t{}\t{}\t{}", probe.name, probe.remote, probe.detail);
    }
    Ok(if probe.ok {
        ExitCode::SUCCESS
    } else {
        ExitCode::from(1)
    })
}

fn print_json(value: &impl serde::Serialize) -> synctr_engine::Result<()> {
    let mut out = io::stdout().lock();
    serde_json::to_writer(&mut out, value).map_err(ser_err)?;
    out.write_all(b"\n")?;
    Ok(())
}

fn toml_pretty(value: &impl serde::Serialize) -> synctr_engine::Result<String> {
    toml::to_string_pretty(value).map_err(ser_err)
}

fn ser_err<E: std::fmt::Display>(e: E) -> synctr_engine::Error {
    synctr_engine::Error::Io(io::Error::new(io::ErrorKind::Other, e.to_string()))
}
