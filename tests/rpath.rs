//! Run with: JLRS_JULIA_DIR=<julia directory> cargo nextest run
#![cfg(target_os = "linux")]

use std::process::Command;

#[test]
fn test_binary_has_the_julia_rpath() {
    let Some(julia_dir) = option_env!("JLRS_JULIA_DIR") else {
        eprintln!("rpath: skipped, JLRS_JULIA_DIR was not set when this test was built");
        return;
    };
    let expected = format!("{julia_dir}/lib");

    let exe = std::env::current_exe().unwrap();
    let output = Command::new("readelf")
        .arg("-d")
        .arg(&exe)
        .output()
        .unwrap_or_else(|error| panic!("this test needs readelf, which did not run: {error}"));
    assert!(
        output.status.success(),
        "readelf -d {}: {}; stderr {:?}",
        exe.display(),
        output.status,
        String::from_utf8_lossy(&output.stderr)
    );
    let dynamic = String::from_utf8_lossy(&output.stdout);
    let entries: Vec<&str> = dynamic
        .lines()
        .filter(|line| line.contains("(RUNPATH)") || line.contains("(RPATH)"))
        .filter_map(|line| line.split_once('[')?.1.trim_end().strip_suffix(']'))
        .flat_map(|paths| paths.split(':'))
        .collect();

    assert!(
        entries.contains(&expected.as_str()),
        "RUNPATH and RPATH entries of the test binary: got {entries:?}, expected one equal to {expected:?}"
    );
}
