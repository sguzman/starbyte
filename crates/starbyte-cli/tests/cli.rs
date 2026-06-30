//! CLI integration tests.

use std::fs;
use std::io::Write;
use std::path::Path;

use assert_cmd::Command;
use tempfile::tempdir;
use zip::write::SimpleFileOptions;

const SPC700_IPL_ROM_LEN: usize = 64;

fn write_test_rom(path: &Path) {
    let mut rom = vec![0_u8; 0x10000];
    let base = 0x7FC0;
    rom[base..base + 21].copy_from_slice(b"STARBYTE CLI TEST    ");
    rom[base + 0x15] = 0x20;
    rom[base + 0x16] = 0x00;
    rom[base + 0x17] = 0x09;
    rom[base + 0x18] = 0x01;
    rom[base + 0x19] = 0x01;
    rom[base + 0x1C] = 0x00;
    rom[base + 0x1D] = 0xFF;
    rom[base + 0x1E] = 0xFF;
    rom[base + 0x1F] = 0x00;
    rom[0x7FFC] = 0x00;
    rom[0x7FFD] = 0x80;
    rom[0x0000] = 0xEA;
    fs::write(path, rom).unwrap();
}

fn write_test_rom_with_rom_type(path: &Path, rom_type: u8, title: &[u8; 21]) {
    let mut rom = vec![0_u8; 0x10000];
    let base = 0x7FC0;
    rom[base..base + 21].copy_from_slice(title);
    rom[base + 0x15] = 0x20;
    rom[base + 0x16] = rom_type;
    rom[base + 0x17] = 0x09;
    rom[base + 0x18] = 0x01;
    rom[base + 0x19] = 0x01;
    rom[base + 0x1C] = 0x00;
    rom[base + 0x1D] = 0xFF;
    rom[base + 0x1E] = 0xFF;
    rom[base + 0x1F] = 0x00;
    rom[0x7FFC] = 0x00;
    rom[0x7FFD] = 0x80;
    rom[0x0000] = 0xEA;
    fs::write(path, rom).unwrap();
}

fn write_test_ipl(path: &Path) {
    fs::write(path, vec![0_u8; SPC700_IPL_ROM_LEN]).unwrap();
}

fn write_zip_roms(path: &Path, members: &[(&str, Vec<u8>)]) {
    let file = fs::File::create(path).unwrap();
    let mut zip = zip::ZipWriter::new(file);
    let options = SimpleFileOptions::default();
    for (name, bytes) in members {
        zip.start_file(name, options).unwrap();
        zip.write_all(bytes).unwrap();
    }
    zip.finish().unwrap();
}

fn write_regression_suite(dir: &Path) {
    let rom = dir.join("regression.sfc");
    write_test_rom(&rom);
    fs::write(
        dir.join("suite.json"),
        r#"[{
          "name":"ppu regression",
          "rom":"regression.sfc",
          "frames":1,
          "setup_writes":[[8481,0],[8482,0],[8482,124],[8492,1]],
          "expected_reads":[],
          "expected":{
            "frame":1,
            "save_ram_len":2048,
            "min_apu_steps":1,
            "first_pixel_rgba":[248,0,0,255]
          }
        }]"#,
    )
    .unwrap();
}

fn write_coprocessor_regression_suite(dir: &Path) {
    let mut rom = vec![0_u8; 0x10000];
    let base = 0x7FC0;
    rom[base..base + 21].copy_from_slice(b"STARBYTE DSP-1 TEST  ");
    rom[base + 0x15] = 0x20;
    rom[base + 0x16] = 0x03;
    rom[base + 0x17] = 0x09;
    rom[base + 0x18] = 0x00;
    rom[base + 0x19] = 0x01;
    rom[base + 0x1C] = 0x00;
    rom[base + 0x1D] = 0xFF;
    rom[base + 0x1E] = 0xFF;
    rom[base + 0x1F] = 0x00;
    rom[0x7FFC] = 0x00;
    rom[0x7FFD] = 0x80;
    rom[0x0000] = 0xEA;
    fs::write(dir.join("coprocessor.sfc"), rom).unwrap();

    fs::write(
        dir.join("coprocessor-suite.json"),
        r#"[{
          "name":"dsp multiply",
          "rom":"coprocessor.sfc",
          "frames":1,
          "setup_writes":[[3178496,0],[3178497,0],[3178496,0],[3178497,64],[3178496,0],[3178497,64]],
          "expected_reads":[[3194880,192],[3178496,0],[3178497,32],[3194880,128]],
          "expected":{
            "frame":1,
            "save_ram_len":0,
            "min_apu_steps":1
          }
        }]"#,
    )
    .unwrap();
}

fn write_library_rom(path: &Path) {
    let mut rom = vec![0_u8; 0x10000];
    let base = 0x7FC0;
    rom[base..base + 21].copy_from_slice(b"STARBYTE LIBRARY ROM ");
    rom[base + 0x15] = 0x20;
    rom[base + 0x16] = 0x00;
    rom[base + 0x17] = 0x09;
    rom[base + 0x18] = 0x01;
    rom[base + 0x19] = 0x01;
    rom[base + 0x1C] = 0x00;
    rom[base + 0x1D] = 0xFF;
    rom[base + 0x1E] = 0xFF;
    rom[base + 0x1F] = 0x00;
    rom[0x7FFC] = 0x00;
    rom[0x7FFD] = 0x80;
    fs::write(path, rom).unwrap();
}

#[test]
fn print_config_toml_succeeds() {
    Command::cargo_bin("starbyte")
        .unwrap()
        .args(["print-config", "toml"])
        .assert()
        .success();
}

#[test]
fn inspect_reports_detected_coprocessor() {
    let dir = tempdir().unwrap();
    let rom = dir.path().join("dsp.sfc");
    write_test_rom_with_rom_type(&rom, 0x03, b"STARBYTE DSP TEST    ");

    let assert = Command::cargo_bin("starbyte")
        .unwrap()
        .args(["inspect", rom.to_str().unwrap()])
        .assert()
        .success();

    let stdout = String::from_utf8_lossy(&assert.get_output().stdout);
    assert!(stdout.contains("Coprocessor: DSP"));
}

#[test]
fn inspect_accepts_zip_backed_roms() {
    let dir = tempdir().unwrap();
    let zip_path = dir.path().join("sample.zip");
    let mut rom = vec![0_u8; 0x10000];
    let base = 0x7FC0;
    rom[base..base + 21].copy_from_slice(b"STARBYTE ZIP TEST    ");
    rom[base + 0x15] = 0x20;
    rom[base + 0x16] = 0x00;
    rom[base + 0x17] = 0x09;
    rom[base + 0x18] = 0x01;
    rom[base + 0x19] = 0x01;
    rom[base + 0x1C] = 0x00;
    rom[base + 0x1D] = 0xFF;
    rom[base + 0x1E] = 0xFF;
    rom[base + 0x1F] = 0x00;
    rom[0x7FFC] = 0x00;
    rom[0x7FFD] = 0x80;
    rom[0x0000] = 0xEA;
    write_zip_roms(
        &zip_path,
        &[("notes.txt", b"ignore me".to_vec()), ("sample.smc", rom)],
    );

    let assert = Command::cargo_bin("starbyte")
        .unwrap()
        .args(["inspect", zip_path.to_str().unwrap()])
        .assert()
        .success();

    let stdout = String::from_utf8_lossy(&assert.get_output().stdout);
    assert!(stdout.contains("Title: STARBYTE ZIP TEST"));
}

#[test]
fn library_scan_json_reports_installed_entries() {
    let dir = tempdir().unwrap();
    let rom_dir = dir.path().join("roms");
    fs::create_dir_all(&rom_dir).unwrap();
    write_library_rom(&rom_dir.join("library.sfc"));

    let assert = Command::cargo_bin("starbyte")
        .unwrap()
        .args([
            "library",
            "scan",
            "--rom-dir",
            rom_dir.to_str().unwrap(),
            "--json",
        ])
        .assert()
        .success();

    let stdout = String::from_utf8_lossy(&assert.get_output().stdout);
    assert!(stdout.contains("\"installed_count\": 1"));
    assert!(stdout.contains("STARBYTE LIBRARY ROM"));
}

#[test]
fn library_download_rom_json_reports_unsupported() {
    let assert = Command::cargo_bin("starbyte")
        .unwrap()
        .args(["library", "download-rom", "--json"])
        .assert()
        .success();

    let stdout = String::from_utf8_lossy(&assert.get_output().stdout);
    assert!(stdout.contains("\"supported\": false"));
    assert!(stdout.contains("unsupported"));
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

#[test]
fn run_persists_save_ram_to_configured_directory() {
    let dir = tempdir().unwrap();
    let rom = dir.path().join("sample.sfc");
    let ipl = dir.path().join("spc700.rom");
    let save_dir = dir.path().join("saves/nested");
    write_test_rom(&rom);
    write_test_ipl(&ipl);

    Command::cargo_bin("starbyte")
        .unwrap()
        .args([
            "--spc700-ipl",
            ipl.to_str().unwrap(),
            "--save-dir",
            save_dir.to_str().unwrap(),
            "run",
            rom.to_str().unwrap(),
            "--frames",
            "0",
        ])
        .assert()
        .success();

    let save_path = save_dir.join("sample.srm");
    assert!(save_path.exists());
    assert_eq!(fs::read(save_path).unwrap().len(), 0x800);
}

#[test]
fn run_fails_for_mismatched_existing_save_ram() {
    let dir = tempdir().unwrap();
    let rom = dir.path().join("sample.sfc");
    let ipl = dir.path().join("spc700.rom");
    let save_dir = dir.path().join("saves");
    write_test_rom(&rom);
    write_test_ipl(&ipl);
    fs::create_dir_all(&save_dir).unwrap();
    fs::write(save_dir.join("sample.srm"), [0xAA]).unwrap();

    Command::cargo_bin("starbyte")
        .unwrap()
        .args([
            "--spc700-ipl",
            ipl.to_str().unwrap(),
            "--save-dir",
            save_dir.to_str().unwrap(),
            "run",
            rom.to_str().unwrap(),
            "--frames",
            "0",
        ])
        .assert()
        .failure();
}

#[test]
fn rom_regression_summary_works() {
    let dir = tempdir().unwrap();
    write_regression_suite(dir.path());

    Command::cargo_bin("starbyte")
        .unwrap()
        .args(["compliance", "rom-summary", dir.path().to_str().unwrap()])
        .assert()
        .success();
}

#[test]
fn rom_regression_run_current_works() {
    let dir = tempdir().unwrap();
    write_regression_suite(dir.path());

    Command::cargo_bin("starbyte")
        .unwrap()
        .args([
            "compliance",
            "rom-run-current",
            dir.path().to_str().unwrap(),
        ])
        .assert()
        .success();
}

#[test]
fn run_writes_screenshot_report_and_state_artifacts() {
    let dir = tempdir().unwrap();
    let rom = dir.path().join("sample.sfc");
    let ipl = dir.path().join("spc700.rom");
    let save_dir = dir.path().join("saves");
    let state_dir = dir.path().join("states");
    let screenshot = dir.path().join("artifacts/frame.ppm");
    let report = dir.path().join("artifacts/run.json");
    write_test_rom(&rom);
    write_test_ipl(&ipl);

    Command::cargo_bin("starbyte")
        .unwrap()
        .args([
            "--spc700-ipl",
            ipl.to_str().unwrap(),
            "--save-dir",
            save_dir.to_str().unwrap(),
            "--state-dir",
            state_dir.to_str().unwrap(),
            "run",
            rom.to_str().unwrap(),
            "--frames",
            "1",
            "--controller1",
            "start,a",
            "--screenshot",
            screenshot.to_str().unwrap(),
            "--report-json",
            report.to_str().unwrap(),
        ])
        .assert()
        .success();

    let screenshot_bytes = fs::read(&screenshot).unwrap();
    assert!(screenshot_bytes.starts_with(b"P6\n256 224\n255\n"));

    let report_json: serde_json::Value =
        serde_json::from_slice(&fs::read(&report).unwrap()).unwrap();
    assert_eq!(report_json["frame_counter"], 1);
    assert!(report_json["audio_sample_count"].as_u64().unwrap() > 0);
    assert_eq!(report_json["framebuffer"]["first_pixel_rgba"][3], 255);

    let auto_state = state_dir.join("sample.state.json");
    assert!(auto_state.exists());
}

#[test]
fn rom_regression_run_current_can_dump_artifacts() {
    let dir = tempdir().unwrap();
    let artifact_dir = dir.path().join("regression-artifacts");
    write_regression_suite(dir.path());

    Command::cargo_bin("starbyte")
        .unwrap()
        .args([
            "compliance",
            "rom-run-current",
            dir.path().to_str().unwrap(),
            "--artifact-dir",
            artifact_dir.to_str().unwrap(),
        ])
        .assert()
        .success();

    let summary: serde_json::Value =
        serde_json::from_slice(&fs::read(artifact_dir.join("summary.json")).unwrap()).unwrap();
    assert_eq!(summary["failed"], 0);
    assert_eq!(summary["passed"], 1);
}

#[test]
fn rom_regression_run_current_supports_expected_host_reads() {
    let dir = tempdir().unwrap();
    write_coprocessor_regression_suite(dir.path());

    Command::cargo_bin("starbyte")
        .unwrap()
        .args([
            "compliance",
            "rom-run-current",
            dir.path().to_str().unwrap(),
        ])
        .assert()
        .success();
}
