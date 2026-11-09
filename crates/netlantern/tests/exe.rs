//! Runs the real binary, for the behaviour that needs no server or pipe.

use std::path::Path;
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU32, Ordering};

fn run_exe(args: &[&str]) -> Output {
    // Unique per call: tests run in parallel and a child may still hold the
    // pipe when the next starts.
    static CALLS: AtomicU32 = AtomicU32::new(0);
    let pipe = format!(
        r"\\.\pipe\netlantern-test-{}-{}",
        std::process::id(),
        CALLS.fetch_add(1, Ordering::Relaxed)
    );
    Command::new(env!("CARGO_BIN_EXE_netlantern"))
        .args(args)
        // So a Net Lantern running on this machine cannot turn the run into
        // "already running".
        .env("NETLANTERN_PIPE_NAME", pipe)
        .output()
        .expect("run netlantern")
}

fn stderr(out: &Output) -> String {
    String::from_utf8_lossy(&out.stderr).into_owned()
}

#[test]
fn demo_scan_once_prints_the_table() {
    let out = run_exe(&["--demo", "--scan-once"]);
    assert!(out.status.success(), "{}", stderr(&out));
    let stdout = String::from_utf8(out.stdout).unwrap();
    assert!(stdout.starts_with("ip\tmac\tvendor\tname\ttype\tping\n"));
    assert!(stdout.contains("192.168.1.70\t00:11:32:AB:CD:70\tSynology\tDiskStation\tnas\t"));
    assert!(stdout
        .lines()
        .last()
        .unwrap()
        .starts_with("Found 13 devices in "));
    // Sorted by IP: .1 before .70.
    let ips: Vec<&str> = stdout
        .lines()
        .skip(1)
        .take_while(|l| !l.starts_with("Found"))
        .map(|l| l.split('\t').next().unwrap())
        .collect();
    assert_eq!(ips.len(), 13);
    assert_eq!(ips[0], "192.168.1.1");
}

fn scenario(name: &str) -> String {
    format!(
        "--demo={}/../../scenarios/{name}.json",
        env!("CARGO_MANIFEST_DIR")
    )
}

fn stdout(out: &Output) -> String {
    String::from_utf8_lossy(&out.stdout).into_owned()
}

#[test]
fn diagnose_once_on_the_cafe_scenario_says_sign_in() {
    let out = run_exe(&[&scenario("cafe"), "--diagnose-once"]);
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    let text = stdout(&out);
    assert!(text.contains("Verdict: Sign in to this network."), "{text}");
    assert_eq!(text.matches("SKIPPED").count(), 2, "{text}");
}

#[test]
fn diagnose_once_on_weak_wifi_gives_journey_a_verdict() {
    let out = run_exe(&[&scenario("weak-wifi"), "--diagnose-once"]);
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    let text = stdout(&out);
    assert!(
        text.contains("Verdict: Your Wi-Fi signal is weak and the channel is crowded."),
        "{text}"
    );
}

#[test]
fn wifi_once_on_the_demo_prints_the_connection_and_nearby_table() {
    let out = run_exe(&["--demo", "--wifi-once"]);
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    let text = stdout(&out);
    assert!(text.contains("Connected to \"Lantern Home\""), "{text}");
    assert!(text.contains("SSID\tBSSID"), "{text}");
    assert!(text.contains("Advice: "), "{text}");
}

#[test]
fn wifi_once_with_location_denied_is_a_normal_report() {
    let out = run_exe(&[&scenario("location-denied"), "--wifi-once"]);
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    assert!(stdout(&out).contains("Windows is hiding Wi-Fi details from Net Lantern."));
}

#[test]
fn demo_with_a_data_dir_warns_and_leaves_it_alone() {
    let dir = tempfile::tempdir().unwrap();
    let data = dir.path().join("data");
    let out = run_exe(&[
        "--demo",
        "--scan-once",
        "--data-dir",
        data.to_str().unwrap(),
    ]);
    assert!(out.status.success(), "{}", stderr(&out));
    assert!(stderr(&out).contains("--data-dir is ignored in demo mode"));
    assert!(!data.exists());
}

fn make_db_with_user_version(path: &Path, version: u32) {
    let conn = rusqlite::Connection::open(path).unwrap();
    conn.pragma_update(None, "user_version", version).unwrap();
}

#[cfg(windows)]
#[test]
fn a_database_from_a_newer_version_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    make_db_with_user_version(&dir.path().join("netlantern.db"), 99);
    let out = run_exe(&["--data-dir", dir.path().to_str().unwrap(), "--no-browser"]);
    assert_eq!(out.status.code(), Some(1), "{}", stderr(&out));
    assert!(out.stdout.is_empty());
    let err = stderr(&out);
    assert!(err.contains("newer Net Lantern"), "{err}");
    assert!(err.contains("--data-dir"), "{err}");
}

#[cfg(windows)]
#[test]
fn an_unusable_data_dir_is_named_in_the_error() {
    let dir = tempfile::tempdir().unwrap();
    // A file where the folder should be.
    let blocker = dir.path().join("blocker");
    std::fs::write(&blocker, "x").unwrap();
    let out = run_exe(&["--data-dir", blocker.to_str().unwrap(), "--no-browser"]);
    assert_eq!(out.status.code(), Some(1));
    assert!(stderr(&out).contains("Could not open Net Lantern's data in"));
}

#[cfg(not(windows))]
#[test]
fn without_demo_it_explains_and_exits_with_2() {
    let out = run_exe(&[]);
    assert_eq!(out.status.code(), Some(2));
    assert!(out.stdout.is_empty());
    assert_eq!(
        stderr(&out).trim_end(),
        "Real network scanning only works on Windows so far. Run netlantern --demo instead."
    );
}
