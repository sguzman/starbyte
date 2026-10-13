# Target SNES game compatibility

**Priority list agreed 2026-10-13.** This is Starbyte's named-game acceptance backlog, not a list of games already verified as playable. It combines the user's story-heavy SNES shortlist with their explicitly requested **Chrono Trigger** as the next target after Super Mario World.

## Main-cartridge track (priority order)

| Order | Game | Platform / testing notes | Evidence status |
| --- | --- | --- | --- |
| 1 | Super Mario World | Baseline LoROM; native controller, sound, saves and Cheats QA | **Playable on user hardware; intermittent native frame/audio stutter remains** |
| 2 | **Chrono Trigger** | **USA retail ZIP identified as 4 MiB HiROM, NTSC; 8 KiB SRAM.** Headless 360-frame startup and isolated APU driver pass, but graphics remain black with forced blank enabled. Next: H-blank timing regression and longer native boot probe; then intro, gameplay, saves | **ROM load/CPU/APU partial pass; graphics and gameplay unverified** |
| 3 | Final Fantasy VI (Final Fantasy III in North America) | Test clean version first; intro, world navigation, dialogue, ATB and SRAM | Not verified |
| 4 | Dragon Quest V | Original Japanese Super Famicom release, then English fan-patched ROM separately | Not verified |
| 5 | Shin Megami Tensei II | Original Japanese Super Famicom release, then English fan-patched ROM separately | Not verified |
| 6 | Tactics Ogre: Let Us Cling Together | Original Super Famicom release and translation build separately; tactical UI / battle | Not verified |
| 7 | Live A Live | Original Japanese Super Famicom release and translation build separately | Not verified |
| 8 | Front Mission | Original Japanese Super Famicom release and translation build separately | Not verified |
| 9 | Fire Emblem: Genealogy of the Holy War | Original Japanese Super Famicom release and translation build separately | Not verified |
| 10 | EarthBound | Cartridge / title / overworld / battles / saves, extended play | Not verified |
| 11 | Der Langrisser | Original Super Famicom release and translation build separately; story routes / battles | Not verified |

## Separate Satellaview track

- **Radical Dreamers (1996):** original Satellaview/SoundLink-era distribution rather than a regular retail SNES cartridge. Track this separately until the actual supplied image format and any BS-X / broadcast dependencies are understood. Do not assume a standard `.sfc` loader will support it, or mark it playable based only on other titles.

## Acceptance gates for each game

1. Inspect the actual user-owned ROM/container (do not bundle or commit commercial ROM bytes); note regional release, header, patch/version, mapper and enhancement chip as applicable.
2. Reproduce launch/boot milestone in a *local-only* command-line harness: reset, startup/titles, meaningful graphics and audio, controls and error output. Distinguish a failed boot from a ZIP import or mapper rejection.
3. Verify introduction/title UI, save/load and first input-driven gameplay scene; add per-game regression fixture or trace without copyrighted ROM data.
4. Validate gameplay after more than one scene, audio fidelity, save persistence, and frame pacing in native EndeavourOS/Hyprland. Automated Linux/Windows synthetic CI cannot establish real game compatibility.
5. Record exactly what was tested and the first blocked milestone in this matrix. Do not infer another title works based on Super Mario World.

**User priority:** finish Super Mario World frame/audio smoothness and then **Chrono Trigger**. Work through the named list rather than promising generic SNES compatibility without game-specific evidence.
