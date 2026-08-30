use std::fs;
use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

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

#[test]
fn help_lists_commands() {
    let out = bin().arg("--help").output().unwrap();
    assert!(out.status.success());
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(text.contains("profile"));
    assert!(text.contains("sync"));
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
    let add = bin()
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

    let list = bin()
        .args(["--config-dir", root.to_str().unwrap(), "--json", "profile", "list"])
        .output()
        .unwrap();
    assert!(list.status.success());
    let v: serde_json::Value = serde_json::from_slice(&list.stdout).unwrap();
    assert_eq!(v["profiles"][0]["name"], "docs");
    assert_eq!(v["profiles"][0]["remote"], "b2:bucket/docs");
    assert_eq!(v["profiles"][0]["mode"], "sync");

    let show = bin()
        .args(["--config-dir", root.to_str().unwrap(), "profile", "show", "docs"])
        .output()
        .unwrap();
    let text = String::from_utf8_lossy(&show.stdout);
    assert!(text.contains("b2:bucket/docs"));
    assert!(text.contains("checksum"));

    let status = bin()
        .args(["--config-dir", root.to_str().unwrap(), "--json", "status"])
        .output()
        .unwrap();
    assert!(status.status.success());
    let v: serde_json::Value = serde_json::from_slice(&status.stdout).unwrap();
    assert_eq!(v["profiles"][0]["name"], "docs");
    assert!(v["rclone"].is_object());

    let rm = bin()
        .args(["--config-dir", root.to_str().unwrap(), "profile", "remove", "docs"])
        .output()
        .unwrap();
    assert!(rm.status.success());
    let list = bin()
        .args(["--config-dir", root.to_str().unwrap(), "profile", "list"])
        .output()
        .unwrap();
    assert!(String::from_utf8_lossy(&list.stdout).contains("no profiles"));
}

#[test]
fn which_rclone_json_with_explicit_binary() {
    let root = scratch();
    let fake = root.join("rclone");
    fs::write(&fake, "#!/bin/sh\nexit 0\n").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut p = fs::metadata(&fake).unwrap().permissions();
        p.set_mode(0o755);
        fs::set_permissions(&fake, p).unwrap();
    }
    let out = bin()
        .args([
            "--rclone",
            fake.to_str().unwrap(),
            "--json",
            "which-rclone",
        ])
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
    let fake = root.join("rclone-from-profile");
    fs::write(&fake, "#!/bin/sh\nexit 0\n").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut p = fs::metadata(&fake).unwrap().permissions();
        p.set_mode(0o755);
        fs::set_permissions(&fake, p).unwrap();
    }
    let add = bin()
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
            "--rclone",
            fake.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(
        add.status.success(),
        "{}",
        String::from_utf8_lossy(&add.stderr)
    );

    let out = bin()
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
