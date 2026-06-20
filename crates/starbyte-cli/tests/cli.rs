//! CLI integration tests.

use assert_cmd::Command;

#[test]
fn print_config_toml_succeeds() {
    Command::cargo_bin("starbyte")
        .unwrap()
        .args(["print-config", "toml"])
        .assert()
        .success();
}
