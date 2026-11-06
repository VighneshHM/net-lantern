//! Runs the real binary, for the behaviour that needs no server or pipe.

use std::process::Command;

#[test]
fn without_demo_it_explains_and_exits_with_2() {
    let output = Command::new(env!("CARGO_BIN_EXE_netlantern"))
        .output()
        .expect("run netlantern");

    assert_eq!(output.status.code(), Some(2));
    assert!(output.stdout.is_empty());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_eq!(
        stderr.trim_end(),
        "Real network scanning arrives in phase 1. For now run: netlantern --demo"
    );
}
