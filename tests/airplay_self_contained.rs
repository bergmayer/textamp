#![cfg(unix)]
use std::process::{Command, Stdio};

#[test]
fn copied_executable_runs_airplay_without_an_external_install_or_app_initialization() {
    let root = tempfile::tempdir().unwrap();
    let binary = root.path().join("textamp");
    std::fs::copy(env!("CARGO_BIN_EXE_textamp"), &binary).unwrap();
    let command = |args: &[&str]| {
        Command::new(&binary)
            .args(args)
            .current_dir(root.path())
            .env("PATH", "")
            .env("TEXTAMP_AIRPLAY_HELPER", "/does-not-exist")
            .env("XDG_CONFIG_HOME", root.path().join("config"))
            .env("XDG_DATA_HOME", root.path().join("data"))
            .env("XDG_CACHE_HOME", root.path().join("cache"))
            .env("XDG_STATE_HOME", root.path().join("state"))
            .stdin(Stdio::null())
            .output()
            .unwrap()
    };
    let output = command(&["--internal-airplay", "--check"]);
    assert!(output.status.success());
    assert_eq!(
        String::from_utf8(output.stdout).unwrap().trim(),
        "cliairplay v0.5.8 check"
    );
    let output = command(&[
        "--internal-airplay",
        "--protocol",
        "raop",
        "--pw",
        "true",
        "127.0.0.1",
    ]);
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("auth_required"));
    let output = command(&["--licenses"]);
    assert!(output.status.success());
    assert!(String::from_utf8_lossy(&output.stdout).contains("GNU GENERAL PUBLIC LICENSE"));
    assert!(
        !root.path().join("state").exists(),
        "native entry must bypass the app lock"
    );
    assert!(!root.path().join("textamp-airplay").exists());
}
