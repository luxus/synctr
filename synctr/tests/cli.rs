use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use synctr_engine::{
    assert_status_json_contract, build_sync_argv, load_filters, status_json, status_snapshot,
    Paths, ProfileStore, StatusSnapshot,
};

static SEQ: AtomicU64 = AtomicU64::new(0);

fn bin() -> Command {
    Command::new(env!("CARGO_BIN_EXE_synctr"))
}

fn scratch() -> PathBuf {
    let n = SEQ.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("synctr-cli-{}-{}", std::process::id(), n));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn isolated(home: &Path) -> Command {
    let mut c = bin();
    c.env("HOME", home);
    c.env("XDG_CONFIG_HOME", home.join(".config"));
    c.env("XDG_STATE_HOME", home.join(".local/state"));
    c.env("XDG_CACHE_HOME", home.join(".cache"));
    c.env_remove("SYNCTR_RCLONE");
    c
}

fn paths_for(root: &Path) -> Paths {
    Paths::from_config_dir(root.to_path_buf())
}

fn store_for(root: &Path) -> ProfileStore {
    ProfileStore::new(paths_for(root))
}

fn assert_recorded_argv_matches_engine(
    root: &Path,
    recorded: &[String],
    rclone: &Path,
    profile_name: &str,
    dry_run: bool,
) {
    let profile = store_for(root).get(profile_name).unwrap();
    let filter_idx = recorded
        .iter()
        .position(|a| a == "--filter-from")
        .expect("--filter-from");
    let filter_path = Path::new(&recorded[filter_idx + 1]);
    let expected = build_sync_argv(rclone, &profile, filter_path, dry_run);
    assert_eq!(recorded[0], rclone.to_str().unwrap());
    assert_eq!(&recorded[1..], expected.args_lossy().as_slice());
    let filters = load_filters(&paths_for(root), &profile).unwrap();
    assert_eq!(
        fs::read_to_string(filter_path).unwrap(),
        filters.to_filter_from()
    );
}

fn stub_rclone(dir: &Path) -> PathBuf {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/rclone-stub.sh");
    let dest = dir.join("rclone");
    fs::copy(&src, &dest).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut p = fs::metadata(&dest).unwrap().permissions();
        p.set_mode(0o755);
        fs::set_permissions(&dest, p).unwrap();
    }
    dest
}

fn add_docs(home: &Path, local: &Path, rclone: Option<&Path>) {
    let mut args = vec![
        "--config-dir".to_string(),
        home.display().to_string(),
        "profile".into(),
        "add".into(),
        "docs".into(),
        "--local".into(),
        local.display().to_string(),
        "--remote".into(),
        "b2:bucket/docs".into(),
        "--mode".into(),
        "sync".into(),
        "--flag".into(),
        "--checksum".into(),
        "--ignore".into(),
        "*.key".into(),
    ];
    if let Some(bin) = rclone {
        args.push("--rclone".into());
        args.push(bin.display().to_string());
    }
    let out = isolated(home).args(&args).output().unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
}

fn parse_stub_line(line: &str) -> Vec<String> {
    line.split('\t').map(str::to_string).collect()
}

#[test]
fn help_lists_commands() {
    let out = bin().arg("--help").output().unwrap();
    assert!(out.status.success());
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(text.contains("profile"));
    assert!(text.contains("sync"));
    assert!(text.contains("watch"));
    assert!(text.contains("schedule"));
    assert!(text.contains("which-rclone"));
    assert!(text.contains("status"));
    assert!(text.contains("tui"));
}

#[test]
fn tui_help_does_not_need_a_tty() {
    let out = bin().args(["tui", "--help"]).output().unwrap();
    assert!(out.status.success());
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(text.contains("Interactive profile list"));
}

#[test]
fn profile_add_list_show_remove_and_json() {
    let root = scratch();
    let add = isolated(&root)
        .args([
            "--config-dir",
            root.to_str().unwrap(),
            "profile",
            "add",
            "docs",
            "--local",
            "/tmp/docs",
            "--remote",
            "b2:bucket/docs",
            "--mode",
            "sync",
            "--flag",
            "--checksum",
            "--ignore",
            "*.key",
        ])
        .output()
        .unwrap();
    assert!(
        add.status.success(),
        "{}",
        String::from_utf8_lossy(&add.stderr)
    );

    let list = isolated(&root)
        .args([
            "--config-dir",
            root.to_str().unwrap(),
            "--json",
            "profile",
            "list",
        ])
        .output()
        .unwrap();
    assert!(list.status.success());
    let v: serde_json::Value = serde_json::from_slice(&list.stdout).unwrap();
    assert_eq!(v["profiles"][0]["name"], "docs");
    assert_eq!(v["profiles"][0]["remote"], "b2:bucket/docs");
    assert_eq!(v["profiles"][0]["mode"], "sync");

    let show = isolated(&root)
        .args([
            "--config-dir",
            root.to_str().unwrap(),
            "profile",
            "show",
            "docs",
        ])
        .output()
        .unwrap();
    let text = String::from_utf8_lossy(&show.stdout);
    assert!(text.contains("b2:bucket/docs"));
    assert!(text.contains("checksum"));

    let status = isolated(&root)
        .args(["--config-dir", root.to_str().unwrap(), "--json", "status"])
        .output()
        .unwrap();
    assert!(status.status.success());
    let v: serde_json::Value = serde_json::from_slice(&status.stdout).unwrap();
    assert_eq!(v["profiles"][0]["name"], "docs");
    assert!(v["rclone"].is_object());
    assert_status_json_contract(&v);

    let rm = isolated(&root)
        .args([
            "--config-dir",
            root.to_str().unwrap(),
            "profile",
            "remove",
            "docs",
        ])
        .output()
        .unwrap();
    assert!(rm.status.success());
    let list = isolated(&root)
        .args(["--config-dir", root.to_str().unwrap(), "profile", "list"])
        .output()
        .unwrap();
    assert!(String::from_utf8_lossy(&list.stdout).contains("no profiles"));
}

#[test]
fn which_rclone_json_with_explicit_binary() {
    let root = scratch();
    let fake = stub_rclone(&root);
    let out = isolated(&root)
        .args(["--rclone", fake.to_str().unwrap(), "--json", "which-rclone"])
        .output()
        .unwrap();
    assert!(out.status.success());
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["found"], true);
    assert_eq!(v["source"], "flag");
    assert_eq!(v["path"], fake.to_str().unwrap());
}

#[test]
fn which_rclone_help_lists_profile_flag() {
    let out = bin().args(["which-rclone", "--help"]).output().unwrap();
    assert!(out.status.success());
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(text.contains("--profile"));
}

#[test]
fn which_rclone_profile_uses_profile_rclone_field() {
    let root = scratch();
    let fake = stub_rclone(&root);
    add_docs(&root, Path::new("/tmp/docs"), Some(&fake));

    let out = isolated(&root)
        .args([
            "--config-dir",
            root.to_str().unwrap(),
            "--json",
            "which-rclone",
            "--profile",
            "docs",
        ])
        .output()
        .unwrap();
    assert!(out.status.success());
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["found"], true);
    assert_eq!(v["source"], "profile");
    assert_eq!(v["path"], fake.to_str().unwrap());
}

#[test]
fn which_rclone_order_flag_then_env_then_path() {
    let root = scratch();
    let flag_dir = root.join("flag");
    let env_dir = root.join("env");
    let path_dir = root.join("pathbin");
    fs::create_dir_all(&flag_dir).unwrap();
    fs::create_dir_all(&env_dir).unwrap();
    fs::create_dir_all(&path_dir).unwrap();
    let flag_bin = stub_rclone(&flag_dir);
    let env_bin = stub_rclone(&env_dir);
    let path_bin = stub_rclone(&path_dir);

    let flag = isolated(&root)
        .env("SYNCTR_RCLONE", &env_bin)
        .env("PATH", path_dir.to_str().unwrap())
        .args([
            "--rclone",
            flag_bin.to_str().unwrap(),
            "--json",
            "which-rclone",
        ])
        .output()
        .unwrap();
    let v: serde_json::Value = serde_json::from_slice(&flag.stdout).unwrap();
    assert_eq!(v["source"], "flag");
    assert_eq!(v["path"], flag_bin.to_str().unwrap());

    let env = isolated(&root)
        .env("SYNCTR_RCLONE", &env_bin)
        .env("PATH", path_dir.to_str().unwrap())
        .args(["--json", "which-rclone"])
        .output()
        .unwrap();
    let v: serde_json::Value = serde_json::from_slice(&env.stdout).unwrap();
    assert_eq!(v["source"], "env");
    assert_eq!(v["path"], env_bin.to_str().unwrap());

    let path = isolated(&root)
        .env("PATH", path_dir.to_str().unwrap())
        .args(["--json", "which-rclone"])
        .output()
        .unwrap();
    let v: serde_json::Value = serde_json::from_slice(&path.stdout).unwrap();
    assert_eq!(v["source"], "PATH");
    assert_eq!(v["path"], path_bin.to_str().unwrap());
}

#[test]
fn profile_edit_and_rename_are_in_place() {
    let root = scratch();
    add_docs(&root, Path::new("/tmp/docs"), None);

    let empty = isolated(&root)
        .args([
            "--config-dir",
            root.to_str().unwrap(),
            "profile",
            "edit",
            "docs",
        ])
        .output()
        .unwrap();
    assert!(!empty.status.success());
    assert!(String::from_utf8_lossy(&empty.stderr).contains("nothing to change"));

    let edit = isolated(&root)
        .args([
            "--config-dir",
            root.to_str().unwrap(),
            "--json",
            "profile",
            "edit",
            "docs",
            "--remote",
            "b2:bucket/other",
            "--mode",
            "copy",
            "--flag",
            "--fast-list",
        ])
        .output()
        .unwrap();
    assert!(
        edit.status.success(),
        "{}",
        String::from_utf8_lossy(&edit.stderr)
    );
    let v: serde_json::Value = serde_json::from_slice(&edit.stdout).unwrap();
    assert_eq!(v["remote"], "b2:bucket/other");
    assert_eq!(v["mode"], "copy");
    assert_eq!(v["extra_flags"][0], "--fast-list");
    assert!(root.join("profiles/docs.toml").is_file());

    let rename = isolated(&root)
        .args([
            "--config-dir",
            root.to_str().unwrap(),
            "profile",
            "rename",
            "docs",
            "notes",
        ])
        .output()
        .unwrap();
    assert!(rename.status.success());
    assert!(!root.join("profiles/docs.toml").exists());
    assert!(root.join("profiles/notes.toml").is_file());
    let show = isolated(&root)
        .args([
            "--config-dir",
            root.to_str().unwrap(),
            "profile",
            "show",
            "notes",
        ])
        .output()
        .unwrap();
    assert!(String::from_utf8_lossy(&show.stdout).contains("b2:bucket/other"));
}

#[test]
fn sync_dry_run_and_filter_from_use_stub_argv() {
    let root = scratch();
    let local = root.join("local");
    fs::create_dir_all(&local).unwrap();
    let fake = stub_rclone(&root);
    add_docs(&root, &local, Some(&fake));
    let log = root.join("stub.log");

    let dry = isolated(&root)
        .env("SYNCTR_STUB_LOG", &log)
        .args([
            "--config-dir",
            root.to_str().unwrap(),
            "--rclone",
            fake.to_str().unwrap(),
            "sync",
            "docs",
            "--dry-run",
        ])
        .output()
        .unwrap();
    assert!(
        dry.status.success(),
        "{}",
        String::from_utf8_lossy(&dry.stderr)
    );
    let line = fs::read_to_string(&log).unwrap();
    let args = parse_stub_line(line.trim());
    assert_recorded_argv_matches_engine(&root, &args, &fake, "docs", true);
    let filter = fs::read_to_string(paths_for(&root).filter_file("docs")).unwrap();
    assert!(filter.contains("- node_modules/**"));
    assert!(filter.contains("- .git/**"));
    assert!(filter.contains("- target/**"));
    assert!(filter.contains("- dist/**"));
    assert!(filter.contains("- .DS_Store"));
    assert!(filter.contains("- *.key"));

    fs::write(&log, "").unwrap();
    let real = isolated(&root)
        .env("SYNCTR_STUB_LOG", &log)
        .args([
            "--config-dir",
            root.to_str().unwrap(),
            "--rclone",
            fake.to_str().unwrap(),
            "sync",
            "docs",
        ])
        .output()
        .unwrap();
    assert!(real.status.success());
    let args = parse_stub_line(fs::read_to_string(&log).unwrap().trim());
    assert_recorded_argv_matches_engine(&root, &args, &fake, "docs", false);
}

#[test]
fn status_json_survives_missing_rclone_override() {
    let root = scratch();
    add_docs(&root, Path::new("/tmp/docs"), None);
    let missing = root.join("no-such-rclone");
    let out = isolated(&root)
        .args([
            "--config-dir",
            root.to_str().unwrap(),
            "--rclone",
            missing.to_str().unwrap(),
            "--json",
            "status",
        ])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_status_json_contract(&v);
    assert_eq!(v["rclone"]["found"], false);
    assert!(v["rclone"].get("path").is_none());
    assert!(v["rclone"].get("source").is_none());
    assert!(v["rclone"].get("detail").is_none());
    let parsed: StatusSnapshot = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(parsed.profiles[0].name, "docs");
    assert!(!parsed.rclone.found);
}

#[test]
fn status_json_skips_bad_profile_toml_and_corrupt_last_run() {
    let root = scratch();
    add_docs(&root, Path::new("/tmp/docs"), None);
    fs::write(root.join("profiles/zzz-bad.toml"), "this is not toml {{{").unwrap();
    fs::create_dir_all(root.join("state/runs")).unwrap();
    fs::write(root.join("state/runs/docs.toml"), "not a last-run {{{").unwrap();
    let out = isolated(&root)
        .args([
            "--config-dir",
            root.to_str().unwrap(),
            "--json",
            "status",
        ])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_status_json_contract(&v);
    assert_eq!(v["profiles"].as_array().unwrap().len(), 1);
    assert_eq!(v["profiles"][0]["name"], "docs");
    assert!(v["profiles"][0].get("last_run").is_none());
}

#[test]
fn status_json_matches_noctalia_plugin_fields() {
    let root = scratch();
    add_docs(&root, Path::new("/tmp/docs"), None);
    let fake = stub_rclone(&root);
    let out = isolated(&root)
        .args([
            "--config-dir",
            root.to_str().unwrap(),
            "--rclone",
            fake.to_str().unwrap(),
            "--json",
            "status",
        ])
        .output()
        .unwrap();
    assert!(out.status.success());
    let snap = status_snapshot(&store_for(&root), Some(&fake), None).unwrap();
    assert_eq!(
        String::from_utf8(out.stdout.clone()).unwrap(),
        status_json(&snap).unwrap()
    );
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_status_json_contract(&v);
    let parsed: StatusSnapshot = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(parsed.profiles[0].name, "docs");
    assert!(parsed.profiles[0].last_run.is_none());
    assert_eq!(parsed.rclone.path.as_deref(), Some(fake.as_path()));

    let plugin = include_str!("../../contrib/noctalia/synctr/widget.luau");
    assert!(plugin.contains("synctr status --json"));
    assert!(plugin.contains("data.rclone"));
    assert!(plugin.contains("rclone.found"));
    assert!(plugin.contains("data.profiles"));
    assert!(plugin.contains("p.last_run"));
    assert!(plugin.contains("last.ok"));
    assert!(plugin.contains("noctalia.json.decode"));
    assert!(!plugin.contains("status --toml"));
}

#[test]
fn schedule_generate_and_install_do_not_start_anything() {
    let root = scratch();
    let units = root.join("units");
    let gen = isolated(&root)
        .args([
            "--json",
            "schedule",
            "generate",
            "docs",
            "--kind",
            "systemd",
            "--interval",
            "1800",
            "--bin",
            "/opt/synctr/bin/synctr",
        ])
        .output()
        .unwrap();
    assert!(
        gen.status.success(),
        "{}",
        String::from_utf8_lossy(&gen.stderr)
    );
    let v: serde_json::Value = serde_json::from_slice(&gen.stdout).unwrap();
    assert_eq!(v["kind"], "systemd");
    assert_eq!(v["files"][0]["name"], "synctr-docs.service");
    let service = v["files"][0]["body"].as_str().unwrap();
    assert!(service.contains("ExecStart=/opt/synctr/bin/synctr sync docs"));
    let timer = v["files"][1]["body"].as_str().unwrap();
    assert!(timer.contains("OnUnitActiveSec=1800s"));
    assert!(!service.contains("systemctl"));

    let launchd = isolated(&root)
        .args([
            "--json",
            "schedule",
            "generate",
            "docs",
            "--kind",
            "launchd",
            "--interval",
            "600",
            "--bin",
            "/opt/synctr/bin/synctr",
        ])
        .output()
        .unwrap();
    let v: serde_json::Value = serde_json::from_slice(&launchd.stdout).unwrap();
    let plist = v["files"][0]["body"].as_str().unwrap();
    assert!(plist.contains("<string>sync</string>"));
    assert!(plist.contains("<integer>600</integer>"));
    assert!(!plist.contains("launchctl"));

    let install = isolated(&root)
        .args([
            "--json",
            "schedule",
            "install",
            "docs",
            "--kind",
            "systemd",
            "--interval",
            "3600",
            "--bin",
            "/opt/synctr/bin/synctr",
            "--dir",
            units.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(install.status.success());
    assert!(units.join("synctr-docs.service").is_file());
    assert!(units.join("synctr-docs.timer").is_file());
    let uninstall = isolated(&root)
        .args([
            "schedule",
            "uninstall",
            "docs",
            "--kind",
            "systemd",
            "--dir",
            units.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(uninstall.status.success());
    assert!(!units.join("synctr-docs.service").exists());

    let agents = root.join("agents");
    let human = isolated(&root)
        .args([
            "schedule",
            "install",
            "docs",
            "--kind",
            "launchd",
            "--interval",
            "600",
            "--bin",
            "/opt/synctr/bin/synctr",
            "--dir",
            agents.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(human.status.success());
    let text = String::from_utf8_lossy(&human.stdout);
    let plist = agents.join("dev.luxus.synctr.docs.plist");
    assert!(plist.is_file());
    assert!(text.contains(&format!("launchctl load {}", plist.display())));
    assert!(!text.contains("~/Library/LaunchAgents"));
    assert!(text.contains(agents.to_str().unwrap()));
}

#[test]
fn schedule_generate_bakes_absolute_config_dir() {
    let root = scratch();
    let cfg = root.join("cfg");
    fs::create_dir_all(&cfg).unwrap();

    let gen = isolated(&root)
        .args([
            "--config-dir",
            cfg.to_str().unwrap(),
            "--json",
            "schedule",
            "generate",
            "docs",
            "--kind",
            "systemd",
            "--bin",
            "/opt/synctr/bin/synctr",
        ])
        .output()
        .unwrap();
    assert!(
        gen.status.success(),
        "{}",
        String::from_utf8_lossy(&gen.stderr)
    );
    let v: serde_json::Value = serde_json::from_slice(&gen.stdout).unwrap();
    let service = v["files"][0]["body"].as_str().unwrap();
    let expected = format!(
        "ExecStart=/opt/synctr/bin/synctr --config-dir {} sync docs",
        cfg.display()
    );
    assert!(
        service.contains(&expected),
        "systemd unit must bake --config-dir so the timer finds profiles; got {service}"
    );

    let rel = isolated(&root)
        .current_dir(&root)
        .args([
            "--config-dir",
            "rel-cfg",
            "--json",
            "schedule",
            "generate",
            "docs",
            "--kind",
            "launchd",
            "--bin",
            "bin/synctr",
        ])
        .output()
        .unwrap();
    assert!(rel.status.success());
    let v: serde_json::Value = serde_json::from_slice(&rel.stdout).unwrap();
    let plist = v["files"][0]["body"].as_str().unwrap();
    let abs_cfg = root.join("rel-cfg");
    let abs_bin = root.join("bin/synctr");
    assert!(
        plist.contains(&format!("<string>{}</string>", abs_bin.display())),
        "relative --bin must be stored absolute; got {plist}"
    );
    assert!(plist.contains("<string>--config-dir</string>"));
    assert!(
        plist.contains(&format!("<string>{}</string>", abs_cfg.display())),
        "relative --config-dir must be stored absolute; got {plist}"
    );
}

#[test]
fn watch_runs_sync_after_temp_dir_change() {
    let root = scratch();
    let local = root.join("local");
    fs::create_dir_all(&local).unwrap();
    let fake = stub_rclone(&root);
    add_docs(&root, &local, Some(&fake));
    let log = root.join("stub.log");

    let err_path = root.join("watch.err");
    let err_file = fs::File::create(&err_path).unwrap();
    let child = isolated(&root)
        .env("SYNCTR_STUB_LOG", &log)
        .args([
            "--config-dir",
            root.to_str().unwrap(),
            "--rclone",
            fake.to_str().unwrap(),
            "watch",
            "docs",
            "--debounce-ms",
            "150",
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::from(err_file))
        .spawn()
        .unwrap();

    struct Kill(std::process::Child);
    impl Drop for Kill {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
    let _guard = Kill(child);

    std::thread::sleep(Duration::from_millis(200));
    fs::write(local.join("note.txt"), "hello\n").unwrap();

    let start = Instant::now();
    let mut woke = false;
    while start.elapsed() < Duration::from_secs(4) {
        if let Ok(text) = fs::read_to_string(&log) {
            if text
                .lines()
                .any(|l| parse_stub_line(l).get(1).map(String::as_str) == Some("sync"))
            {
                woke = true;
                break;
            }
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    assert!(
        woke,
        "watch did not invoke stub rclone; log={:?} stderr={} local={}",
        fs::read_to_string(&log).ok(),
        fs::read_to_string(&err_path).unwrap_or_default(),
        local.display()
    );
    let args = parse_stub_line(fs::read_to_string(&log).unwrap().lines().next().unwrap());
    assert_recorded_argv_matches_engine(&root, &args, &fake, "docs", false);
}

#[test]
fn watch_does_not_sync_on_ignored_node_modules() {
    let root = scratch();
    let local = root.join("local");
    fs::create_dir_all(local.join("node_modules/pkg")).unwrap();
    let fake = stub_rclone(&root);
    add_docs(&root, &local, Some(&fake));
    let log = root.join("stub.log");

    let err_path = root.join("watch.err");
    let err_file = fs::File::create(&err_path).unwrap();
    let child = isolated(&root)
        .env("SYNCTR_STUB_LOG", &log)
        .args([
            "--config-dir",
            root.to_str().unwrap(),
            "--rclone",
            fake.to_str().unwrap(),
            "watch",
            "docs",
            "--debounce-ms",
            "150",
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::from(err_file))
        .spawn()
        .unwrap();

    struct Kill(std::process::Child);
    impl Drop for Kill {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
    let _guard = Kill(child);

    std::thread::sleep(Duration::from_millis(200));
    fs::write(local.join("node_modules/pkg/index.js"), "ignored\n").unwrap();
    std::thread::sleep(Duration::from_millis(500));
    let after_ignored = fs::read_to_string(&log).unwrap_or_default();
    assert!(
        after_ignored.trim().is_empty(),
        "ignored node_modules write woke watch; log={:?} stderr={}",
        after_ignored,
        fs::read_to_string(&err_path).unwrap_or_default()
    );

    fs::write(local.join("note.txt"), "hello\n").unwrap();
    let start = Instant::now();
    let mut woke = false;
    while start.elapsed() < Duration::from_secs(4) {
        if let Ok(text) = fs::read_to_string(&log) {
            if text
                .lines()
                .any(|l| parse_stub_line(l).get(1).map(String::as_str) == Some("sync"))
            {
                woke = true;
                break;
            }
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    assert!(
        woke,
        "watch should still sync a non-ignored file; log={:?} stderr={}",
        fs::read_to_string(&log).ok(),
        fs::read_to_string(&err_path).unwrap_or_default()
    );
}

#[test]
fn watch_preflights_rclone_before_printing_watching() {
    let root = scratch();
    let local = root.join("local");
    fs::create_dir_all(&local).unwrap();
    add_docs(&root, &local, None);
    let missing = root.join("no-such-rclone");
    let out = isolated(&root)
        .args([
            "--config-dir",
            root.to_str().unwrap(),
            "--rclone",
            missing.to_str().unwrap(),
            "watch",
            "docs",
            "--debounce-ms",
            "50",
        ])
        .output()
        .unwrap();
    assert!(!out.status.success());
    let err = String::from_utf8_lossy(&out.stderr);
    let combined = format!("{err}{}", String::from_utf8_lossy(&out.stdout));
    assert!(
        err.contains("rclone override not found") || err.contains("not found"),
        "stderr={err}"
    );
    assert!(
        !combined.contains("watching "),
        "must fail before the watching banner; out={combined}"
    );
}
