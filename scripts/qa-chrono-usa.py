#!/usr/bin/env python3
"""Read-only, repeatable Chrono Trigger USA boot probe for Starbyte.

Writes only to the requested diagnostic output directory; never writes SRAM,
save-states, ROMs, or the Starbyte config. ROM files are not uploaded anywhere.
"""
from __future__ import annotations

import argparse
from collections import Counter
import hashlib
import json
import os
from pathlib import Path
import subprocess
import sys
import zipfile

DEFAULT_ROM = Path("/home/sguzman/Games/Roms/SNES/Chrono Trigger (USA).zip")
MILESTONES = {1, 30, 60, 90, 120, 180, 240, 360, 600, 900, 1200, 1800}


def cartridge_hashes(path: Path) -> dict:
    if path.suffix.lower() == ".zip":
        with zipfile.ZipFile(path) as archive:
            candidates = [
                info for info in archive.infolist()
                if not info.is_dir()
                and Path(info.filename).suffix.lower() in (".sfc", ".smc", ".swc", ".fig")
            ]
            if len(candidates) != 1:
                raise ValueError(
                    f"Expected exactly one ROM member, found {len(candidates)}. "
                    "Select a single-game ZIP before probing."
                )
            member = candidates[0]
            if member.file_size > 64 * 1024 * 1024:
                raise ValueError("ROM archive member exceeds 64 MiB limit")
            data = archive.read(member)
            name = member.filename
    else:
        data = path.read_bytes()
        name = path.name
    if len(data) % 1024 == 512:
        data = data[512:]
    return {
        "archive_member": name,
        "unheadered_bytes": len(data),
        "unheadered_sha1": hashlib.sha1(data).hexdigest(),
    }


def run_command(argv: list[str], quiet: bool = False) -> subprocess.CompletedProcess:
    env = dict(os.environ)
    env["STARBYTE_LOG"] = "error"
    result = subprocess.run(argv, capture_output=True, text=True, env=env, check=False)
    if result.returncode:
        print(f"FAILURE: {' '.join(argv[:3])}: exit {result.returncode}")
        if result.stdout:
            print(result.stdout[-2500:])
        if result.stderr:
            print(result.stderr[-4500:], file=sys.stderr)
    elif not quiet and result.stdout.strip():
        print(result.stdout.strip()[-3000:])
    return result


def summarize(rows: list[dict], report: dict, frames: int) -> dict:
    first_pixels = next(
        (row["completed_frame"] for row in rows
         if row.get("framebuffer", {}).get("nonblack_pixels", 0) > 0),
        None,
    )
    first_unblank = next(
        (row["completed_frame"] for row in rows
         if row.get("ppu_display", {}).get("forced_blank") is False
         and row.get("ppu_display", {}).get("brightness", 0) > 0),
        None,
    )
    history = []
    visible = [row for row in rows if row.get("framebuffer", {}).get("nonblack_pixels", 0) > 0]
    visible_hashes = [row.get("framebuffer", {}).get("hash") for row in visible]
    hash_changes = sum(1 for older, newer in zip(visible_hashes, visible_hashes[1:]) if older != newer)
    maximum_pixels = max(
        (row.get("framebuffer", {}).get("nonblack_pixels", 0) for row in rows),
        default=0,
    )
    for row in rows:
        frame = row.get("completed_frame")
        if frame not in MILESTONES and frame != frames:
            continue
        cpu = row.get("cpu", {})
        ppu = row.get("ppu_display", {})
        w = row.get("ppu_write_activity", {})
        history.append({
            "frame": frame,
            "pc": f"{cpu.get('pbr', 0):02X}:{cpu.get('pc', 0):04X}",
            "nonblack_pixels": row.get("framebuffer", {}).get("nonblack_pixels"),
            "distinct_colors": row.get("framebuffer", {}).get("distinct_rgb_colors"),
            "framebuffer_hash": row.get("framebuffer", {}).get("hash"),
            "blank": ppu.get("forced_blank"),
            "brightness": ppu.get("brightness"),
            "mode": ppu.get("background_mode"),
            "ppu_writes": w.get("total_writes"),
            "apu_reads": row.get("apu_io_activity", {}).get("cpu_read_counts"),
        })
    return {
        "status": report.get("status", "ok"),
        "failure": report.get("failure"),
        "requested_frames": frames,
        "completed_frames": report.get("frame_counter"),
        "first_nonblack_frame": first_pixels,
        "first_display_enabled_frame": first_unblank,
        "max_nonblack_pixels": maximum_pixels,
        "nonblack_frame_count": len(visible),
        "distinct_visible_frame_hashes": len(set(visible_hashes)),
        "visible_frame_hash_changes": hash_changes,
        "last_ppu_display": report.get("ppu_display"),
        "last_ppu_write_activity": report.get("ppu_write_activity"),
        "last_apu_io_activity": report.get("apu_io_activity"),
        "milestones": history,
    }


def trace_busy_wait(trace_file: Path) -> dict:
    """Summarize hot guest PCs and MMIO reads without printing ROM instructions."""
    hot_pc: Counter[str] = Counter()
    apu_reads: Counter[str] = Counter()
    apu_values: Counter[str] = Counter()
    instructions = 0
    for line in trace_file.open(encoding="utf-8"):
        entry = json.loads(line)
        before = entry.get("before", {})
        if "pbr" not in before or "pc" not in before:
            continue
        instructions += 1
        hot_pc[f"{before['pbr']:02X}:{before['pc']:04X}"] += 1
        for event in entry.get("bus_events", []):
            if event.get("access") != "Read":
                continue
            address = event.get("address", -1)
            if address < 0 or address & 0xFFFF not in (0x2140, 0x2141, 0x2142, 0x2143):
                continue
            port = f"${address & 0xFFFF:04X}"
            apu_reads[port] += 1
            apu_values[f"{port}={event.get('value')}"] += 1
    return {
        "instructions": instructions,
        "hottest_pcs": hot_pc.most_common(8),
        "apu_reads": dict(apu_reads),
        "apu_read_values": dict(apu_values.most_common(12)),
    }


def run_driver_comparison(args: argparse.Namespace) -> int:
    """Run real uploaded SPC code and contrast results with baseline shim."""
    run_dir = args.out / "spc-driver"
    run_dir.mkdir(parents=True, exist_ok=True)
    report_path = run_dir / "run-report.json"
    history_path = run_dir / "frame-history.jsonl"
    # The ordinary two-mode comparison defaults to 360 frames; an explicitly
    # requested longer driver run can follow intro/title-screen progression.
    frames = args.driver_frames
    cmd = [
        str(args.binary), "run", str(args.rom),
        "--frames", str(frames), "--no-save-ram",
        "--apu-upload-probe", "--apu-execute-driver",
        "--report-json", str(report_path), "--frame-log", str(history_path),
    ]
    if args.driver_images:
        frame_dir = run_dir / "frames"
        cmd += [
            "--frame-images-dir", str(frame_dir),
            "--frame-image-every", "120", "--max-frame-images", "20",
        ]
    print(f"=== REAL SPC DRIVER COMPARISON ({frames} frames) ===", flush=True)
    result = run_command(cmd, quiet=True)
    if not report_path.is_file() or not history_path.is_file():
        return result.returncode or 1
    data = json.loads(report_path.read_text(encoding="utf-8"))
    rows = [
        json.loads(line)
        for line in history_path.read_text(encoding="utf-8").splitlines()
        if line.strip()
    ]
    details = summarize(rows, data, frames)
    print(json.dumps({
        "completed_frames": details["completed_frames"],
        "first_nonblack_frame": details["first_nonblack_frame"],
        "first_display_enabled_frame": details["first_display_enabled_frame"],
        "max_nonblack_pixels": details["max_nonblack_pixels"],
        "nonblack_frame_count": details["nonblack_frame_count"],
        "distinct_visible_frame_hashes": details["distinct_visible_frame_hashes"],
        "visible_frame_hash_changes": details["visible_frame_hash_changes"],
        "failure": data.get("failure"),
        "apu_upload_timeline": data.get("apu_upload_timeline"),
        "apu_io_activity": data.get("apu_io_activity"),
        "ppu_write_totals": data.get("ppu_write_activity", {}).get("total_writes"),
        "audio_run": data.get("audio_run"),
        "selected_milestones": [
            row for row in details["milestones"] if row["frame"] in MILESTONES
        ],
    }, indent=2))
    (run_dir / "summary.json").write_text(
        json.dumps(details, indent=2) + "\n", encoding="utf-8"
    )
    return result.returncode


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--rom", type=Path, default=DEFAULT_ROM)
    parser.add_argument("--binary", type=Path, default=Path("./target/release/starbyte"))
    parser.add_argument("--frames", type=int, default=1200)
    parser.add_argument("--compare-apu", action="store_true",
                        help="compare normal bootstrap with real uploaded SPC driver")
    parser.add_argument("--driver-only", action="store_true",
                        help="skip stalled legacy-Apu bootstrap and test real SPC driver alone")
    parser.add_argument("--driver-frames", type=int, default=360,
                        help="real-SPC-driver frame count (default 360, up to 3000)")
    parser.add_argument("--driver-images", action="store_true",
                        help="write bounded PPM screenshots every 120 driver frames under --out")
    parser.add_argument("--trace-stall-frame", type=int, default=None,
                        help="trace one normal-mode frame of the suspected APU busy-wait")
    parser.add_argument(
        "--out", type=Path, default=Path("/tmp/starbyte-chrono-usa/boot-probe")
    )
    args = parser.parse_args()
    if not args.rom.is_file():
        parser.error(f"ROM file not found: {args.rom}")
    if not args.binary.is_file():
        parser.error(f"Starbyte CLI binary not found: {args.binary}")
    if not (1 <= args.frames <= 3000):
        parser.error("frames must be in range 1..3000")
    if not (1 <= args.driver_frames <= 3000):
        parser.error("driver-frames must be in range 1..3000")
    if args.driver_only and args.trace_stall_frame is not None:
        parser.error("--trace-stall-frame traces the legacy comparison; cannot combine with --driver-only")
    if args.trace_stall_frame is not None and not (1 <= args.trace_stall_frame <= args.frames):
        parser.error("trace-stall-frame must be in range 1..frames")

    args.out.mkdir(parents=True, exist_ok=True)
    print("=== ROM IDENTIFICATION (local, never uploaded) ===")
    print(json.dumps(cartridge_hashes(args.rom), indent=2))
    inspected = run_command([
        str(args.binary), "inspect", str(args.rom), "--json"
    ])
    if inspected.returncode:
        return inspected.returncode
    try:
        inspected_obj = json.loads(inspected.stdout)
    except json.JSONDecodeError:
        print("Failed to parse ROM inspect JSON", file=sys.stderr)
        return 1
    (args.out / "inspection.json").write_text(
        json.dumps(inspected_obj, indent=2) + "\n", encoding="utf-8"
    )
    if args.driver_only:
        return run_driver_comparison(args)
    report = args.out / "run-report.json"
    frame_log = args.out / "frame-history.jsonl"
    print(f"=== READ-ONLY BOOT PROBE ({args.frames} frames) ===", flush=True)
    cmd = [
        str(args.binary), "run", str(args.rom),
        "--frames", str(args.frames), "--no-save-ram",
        "--report-json", str(report), "--frame-log", str(frame_log),
    ]
    trace_path = args.out / "stall-frame-trace.jsonl"
    if args.trace_stall_frame is not None:
        cmd += ["--trace-frame", str(args.trace_stall_frame), "--trace-out", str(trace_path)]
    result = run_command(cmd, quiet=True)
    if not report.is_file() or not frame_log.is_file():
        print("Boot report missing; inspect stderr above.", file=sys.stderr)
        return result.returncode or 1
    rows = [
        json.loads(line)
        for line in frame_log.read_text(encoding="utf-8").splitlines()
        if line.strip()
    ]
    report_obj = json.loads(report.read_text(encoding="utf-8"))
    digest = summarize(rows, report_obj, args.frames)
    summary_path = args.out / "summary.json"
    summary_path.write_text(
        json.dumps(digest, indent=2) + "\n", encoding="utf-8"
    )
    print(json.dumps(digest, indent=2))
    if args.trace_stall_frame is not None and trace_path.is_file():
        print("=== BUSY-WAIT TRACE (counts, no ROM bytes) ===")
        trace_summary = trace_busy_wait(trace_path)
        print(json.dumps(trace_summary, indent=2))
        (args.out / "stall-trace-summary.json").write_text(
            json.dumps(trace_summary, indent=2) + "\n", encoding="utf-8"
        )
    if args.compare_apu:
        comparison_result = run_driver_comparison(args)
    else:
        comparison_result = 0
    print(f"Local detailed reports: {args.out}")
    return result.returncode or comparison_result


if __name__ == "__main__":
    raise SystemExit(main())
