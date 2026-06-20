//! CLI integration tests.

use std::fs;

use assert_cmd::Command;
use tempfile::tempdir;

#[test]
fn print_config_toml_succeeds() {
    Command::cargo_bin("starbyte")
        .unwrap()
        .args(["print-config", "toml"])
        .assert()
        .success();
}

#[test]
fn compliance_summary_works_for_synthetic_65816_suite() {
    let dir = tempdir().unwrap();
    fs::write(dir.path().join("00.n.json"), "[]").unwrap();

    Command::cargo_bin("starbyte")
        .unwrap()
        .args([
            "compliance",
            "summary",
            "cpu-65816",
            dir.path().to_str().unwrap(),
        ])
        .assert()
        .success();
}

#[test]
fn compliance_verify_format_works_for_synthetic_spc700_suite() {
    let dir = tempdir().unwrap();
    fs::write(
        dir.path().join("00.json"),
        r#"[{
          "initial":{"pc":4096,"a":1,"x":2,"y":3,"sp":255,"psw":18,"ram":[[4096,0]]},
          "final":{"pc":4097,"a":1,"x":2,"y":3,"sp":255,"psw":18,"ram":[[4097,0]]},
          "cycles":[[4096,0,"read"]]
        }]"#,
    )
    .unwrap();

    Command::cargo_bin("starbyte")
        .unwrap()
        .args([
            "compliance",
            "verify-format",
            "spc700",
            dir.path().to_str().unwrap(),
            "--opcode",
            "00",
        ])
        .assert()
        .success();
}

#[test]
fn compliance_run_current_passes_for_matching_synthetic_65816_vector() {
    let dir = tempdir().unwrap();
    fs::write(
        dir.path().join("ea.n.json"),
        r#"[{
          "name":"placeholder pass",
          "initial":{"a":0,"x":0,"y":0,"s":0,"d":0,"pc":4660,"pbr":0,"dbr":0,"p":0,"e":0,"ram":[[4660,234]]},
          "final":{"a":0,"x":0,"y":0,"s":0,"d":0,"pc":4661,"pbr":0,"dbr":0,"p":0,"e":0,"ram":[[4660,234]]},
          "cycles":[[4660,234,"pcrr"],[4661,null,"pcrr"]]
        }]"#,
    )
    .unwrap();

    Command::cargo_bin("starbyte")
        .unwrap()
        .args([
            "compliance",
            "run-current",
            "cpu-65816",
            dir.path().to_str().unwrap(),
            "--opcode",
            "ea",
            "--mode",
            "native",
        ])
        .assert()
        .success();
}

#[test]
fn compliance_run_current_fails_for_mismatching_synthetic_spc700_vector() {
    let dir = tempdir().unwrap();
    fs::write(
        dir.path().join("00.json"),
        r#"[{
          "name":"placeholder fail",
          "initial":{"pc":4096,"a":1,"x":2,"y":3,"sp":255,"psw":18,"ram":[]},
          "final":{"pc":4100,"a":1,"x":2,"y":3,"sp":255,"psw":18,"ram":[]},
          "cycles":[[4096,0,"read"]]
        }]"#,
    )
    .unwrap();

    Command::cargo_bin("starbyte")
        .unwrap()
        .args([
            "compliance",
            "run-current",
            "spc700",
            dir.path().to_str().unwrap(),
            "--opcode",
            "00",
        ])
        .assert()
        .failure();
}
