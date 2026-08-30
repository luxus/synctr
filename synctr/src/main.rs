use std::io::{self, Write};
use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Parser, Subcommand, ValueEnum};
use synctr_engine::{
    resolve_rclone_live, status_snapshot, Mode, Paths, Profile, ProfileStore, RcloneJson,
};

mod tui;

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
    /// Add, list, show, or remove profiles
    Profile {
        #[command(subcommand)]
        command: ProfileCmd,
    },
    /// Run rclone for a profile
    Sync {
        name: String,
    },
    /// Print the rclone binary that would be used
    WhichRclone {
        /// Apply this profile's rclone override
    },
    /// Profiles, last run, and rclone path
    Status,
    /// Interactive profile list, state, and rclone log
    Tui,
}

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
    /// Delete a profile
    Remove { name: String },
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
        Command::Sync { name } => {
            let profile = store.get(&name)?;
            let outcome =
                synctr_engine::run_sync(store.paths(), &profile, cli.rclone.as_deref(), true)?;
            Ok(ExitCode::from(outcome.exit_code as u8))
        }
        Command::WhichRclone { profile } => which_rclone(&store, cli.rclone.as_deref(), cli.json, profile),
        Command::Status => status_cmd(&store, cli.rclone.as_deref(), cli.json),
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
                    println!(
                        "{}\t{}\t{}\t->\t{}",
                        p.name,
                        p.mode,
                        p.local.display(),
                        p.remote
                    );
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
        ProfileCmd::Remove { name } => {
            store.remove(&name)?;
            if json {
                print_json(&serde_json::json!({ "removed": name }))?;
            } else {
                println!("removed {name}");
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
    match resolve_rclone_live(rclone_flag, profile_rclone.as_deref()) {
        Ok(found) => {
            if json {
                print_json(&RcloneJson::from(&found))?;
            } else {
                println!("{}", found.path.display());
                println!("source: {}", found.source.explain());
            }
            Ok(ExitCode::SUCCESS)
        }
        Err(synctr_engine::Error::RcloneNotFound) => {
            if json {
                print_json(&RcloneJson::missing())?;
                Ok(ExitCode::from(1))
            } else {
                Err(synctr_engine::Error::RcloneNotFound)
            }
        }
        Err(e) => Err(e),
    }
}

fn status_cmd(
    store: &ProfileStore,
    rclone_flag: Option<&std::path::Path>,
    json: bool,
) -> synctr_engine::Result<ExitCode> {
    let snap = status_snapshot(store, rclone_flag, None)?;
    if json {
        print_json(&snap)?;
        return Ok(ExitCode::SUCCESS);
    }
    match (&snap.rclone.found, &snap.rclone.path, &snap.rclone.detail) {
        (true, Some(path), Some(detail)) => println!("rclone: {} ({})", path.display(), detail),
        _ => println!("rclone: not found"),
    }
    if snap.profiles.is_empty() {
        println!("no profiles");
        return Ok(ExitCode::SUCCESS);
    }
    for p in snap.profiles {
        let last = match p.last_run {
            Some(run) if run.ok => format!("ok {}", run.finished_at),
            Some(run) => format!("exit {} {}", run.exit_code, run.finished_at),
            None => "never".into(),
        };
        println!(
            "{}\t{}\t{}\t->\t{}\t{}",
            p.name,
            p.mode,
            p.local.display(),
            p.remote,
            last
        );
    }
    Ok(ExitCode::SUCCESS)
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
