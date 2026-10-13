#!/usr/bin/env python3
"""Read-only, repeatable Chrono Trigger USA boot probe for Starbyte.

Writes only to the requested diagnostic output directory; never writes SRAM,
save-states, ROMs, or the Starbyte config. ROM files are not uploaded anywhere.
"""
from __future__ import annotations

import argparse
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
        "last_ppu_display": report.get("ppu_display"),
        "last_ppu_write_activity": report.get("ppu_write_activity"),
        "last_apu_io_activity": report.get("apu_io_activity"),
        "milestones": history,
    }


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--rom", type=Path, default=DEFAULT_ROM)
    parser.add_argument("--binary", type=Path, default=Path("./target/release/starbyte"))
    parser.add_argument("--frames", type=int, default=1200)
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
    report = args.out / "run-report.json"
    frame_log = args.out / "frame-history.jsonl"
    print(f"=== READ-ONLY BOOT PROBE ({args.frames} frames) ===", flush=True)
    result = run_command([
        str(args.binary), "run", str(args.rom),
        "--frames", str(args.frames), "--no-save-ram",
        "--report-json", str(report), "--frame-log", str(frame_log),
    ], quiet=True)
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
    print(f"Local detailed reports: {args.out}")
    return result.returncode


if __name__ == "__main__":
    raise SystemExit(main())
