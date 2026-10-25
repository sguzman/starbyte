# Emulation scheduler migration

## Battery SRAM verified across separate application processes — 2026-10-24

The original-window Weston SRAM smoke previously closed `StarbyteApp`, joined
the emulation owner, and reopened the battery cartridge in the same executable
process. The Linux CI smoke now uses isolated temporary `HOME` and XDG roots,
closes that writer application process, then starts a second `starbyte-egui`
process to reload the copyright-free battery demo and verify persisted byte
`0x5A`. The reader runs without creating a GUI window. This directly tests
process-boundary persistence in addition to the existing owner-shutdown and
SRAM tests. It does not establish physical game SRAM use or close D-15/P-19;
the evidence-gated acceptance count remains **71/77 (92%)**.

## 2026-10-24 non-interference safety gate after failed second focused run

Local Codex retested `real-focused-clock` on `fd522d4` and reported 0/1
accepted: its unverified app log counted 181 frames and texture submissions
over 3,016 ms (60.006 FPS), but the compositor foreground/native-client
check failed. The owner confirmed the focused-only wrapper **forced the
desktop onto the QA workspace** despite promising not to. The original
workspace was restored afterward, but that does not undo the interruption.

**Corrective action:** the unattended wrapper and direct Rust collector
now fail closed before creating GUI windows or switching Hyprland
workspaces. The non-GUI `--check-only` build/contract/ledger path remains
available. Physical GUI QA may be re-enabled only after a separate compositor
and input seat have been demonstrated to preserve the owner's workspace
and keyboard focus. D-12 remains pending; previously accepted A-08 and
D-14 retain their real-desktop evidence. **Overall 71/77 (92%)**.
The historical procedure below must not be executed on the owner's active
desktop.


## 2026-10-24 first actual desktop Hyprland acceptance — 11/12 physical probes

The locally running Codex executor fast-forwarded a clean Starbyte checkout to [`b1448b3`](https://github.com/sguzman/starbyte/commit/b1448b392556b03c963cc273744826a0d7affbbb) and ran the existing full native harness in the owner's **real** Hyprland/Wayland login. Its sanitized report records **11 of 12 probes passed**: all nine isolated original-window smokes and both compositor workspace transitions passed; focused-clock observation **failed** on `lost_keyboard_focus`, with no valid focused FPS measurement. The original workspace was restored, no QA window remained, and Git was clean.

Real background-play transition: **142 independently completed frames, 59.81 FPS over 2,374 ms**, texture catch-up at frame **154**, away-workspace sampling for **2,289 ms across 9 compositor observations**. This satisfies the explicitly off-workspace alternative of **A-08** (not occlusion or sound); now accepted.

Real pause-on-blur transition: **2 frames** advanced while away, **1 suspension**, **1 resume**, and **10 resumed frames in 166 ms**, with no catch-up burst. This satisfies **D-14**; now accepted. These results were reported by local Codex, with private raw receipts retained locally; the full 12/12 sanitized aggregate was not emitted because the focused probe failed.

**Evidence-scoped progress is now 71/77 (92%)**, milestone A 8/8, milestone D 12/15. **Still pending (six):** P-18 physical background speaker/device recovery; P-19 hardware gamepad/menu/save acceptance; C-16 production cutover; D-12 focused FPS; D-13 off-workspace **sound** (video-only evidence is insufficient); D-15 complete native saves, inputs, cheats, screenshots and SRAM. No physical speaker/gamepad/persistence/cutover claim is made. The old 69/77 references below are historical checkpoints.


**2026-10-24 focused evidence integrity:** [`dede0ce`](https://github.com/sguzman/starbyte/commit/dede0ceb325739edf5b86bdfa32f537cd1687600) now rejects focused physical-clock receipts with an inconsistent FPS/frame-duration combination or zero worker frames. [CI 40131591925](https://github.com/sguzman/starbyte/actions/runs/40131591925) passed four code jobs; these are not physical measurements. The desktop runner remains unattached and no task was newly accepted (69/77).

**Status: in progress. Earlier native-QA source [`e693be2`](https://github.com/sguzman/starbyte/commit/e693be2b717bed82dfb2156d359caad1dff8bcc8) adds a redacted, machine-readable physical clock/pause evidence receipt, gated on the complete twelve-probe local Hyprland run. [`25e8165`](https://github.com/sguzman/starbyte/commit/25e8165ac74307e440e312e5bb1a7275ad221adc) prioritizes exact-current-main physical requests without interrupting active probes; [`84bee78`](https://github.com/sguzman/starbyte/commit/84bee78ba01cb977a739b045fd732c874a2e2215) reaps probe children on compositor errors. [Exact-code CI 40130239670](https://github.com/sguzman/starbyte/actions/runs/40130239670) passed all four jobs. Actual physical execution is **still blocked** by lack of an attached local Hyprland runner; a pending GitHub job is not evidence. Accepted checklist remains 69/77 (90%): A 7/8, B 18/18, P 18/20, C 15/16, D 11/15. Production worker stays opt-in until direct physical/audio/gamepad/cutover evidence.**
**Issue:** [#7](https://github.com/sguzman/starbyte/issues/7).

## Latest-only desktop execution and auditable numeric evidence (2026-10-24)

[`25e8165`](https://github.com/sguzman/starbyte/commit/25e8165ac74307e440e312e5bb1a7275ad221adc) updates the opt-in ephemeral runner bootstrap. It waits for GitHub to index the workflow_dispatch at the exact requested `main` SHA and retires obsolete *queued/pending* physical runs without canceling an in-progress desktop QA session. A live compositor can be left on a temporary workspace if its orchestrator is forcibly terminated, so cancellation of an active run is prohibited. [`84bee78`](https://github.com/sguzman/starbyte/commit/84bee78ba01cb977a739b045fd732c874a2e2215) independently fixes Rust probe lifecycle: all early compositor/IO error paths now kill and reap launched emulator children. Linux regression confirms reaping.

[`e693be2`](https://github.com/sguzman/starbyte/commit/e693be2b717bed82dfb2156d359caad1dff8bcc8) extracts exact original-window success-marker numbers and local native Hyprland observation receipts into an identity-free `public-evidence.json`. On a **fully passing twelve-probe physical** run, the shell wrapper publishes precisely those numeric data to Actions logs as `STARBYTE_NATIVE_PHYSICAL_EVIDENCE=` for remote evidence review. Local raw log contents, workspace IDs, PID, device names and filesystem paths remain private. It never declares unmeasured audio audibility/gamepad/cutover accepted. [CI 40130239670](https://github.com/sguzman/starbyte/actions/runs/40130239670) passed all four jobs **for code and parser regressions only**.

The self-hosted [physical workflow request 40127765927](https://github.com/sguzman/starbyte/actions/runs/40127765927) was unassigned when checked. No GitHub-only connector can register or execute a runner inside the owner's desktop. **A-08, P-18, P-19, C-16 and D-12–D-15 remain unaccepted: 69/77 (90%).**

## Native Hyprland QA execution and focused-rate evidence (2026-10-24)

[Source `4304e20`](https://github.com/sguzman/starbyte/commit/4304e200501bf7759253c2dd73a2176bb46e6aef) expands the existing isolated native collector from nine synthetic original-window checks plus two physical workspace probes to **twelve** checks: nine synthetic adjuncts, one real visible/focused clock-and-texture probe, one actual off-workspace progression probe, and one actual background-pause probe. The focused smoke measures original-window callbacks, worker-completed emulation frames, elapsed monotonic time, and fresh framebuffer uploads over at least three seconds. The collector independently samples `hyprctl` to verify the test process is both a native Wayland client and the active Hyprland window, then records a private machine-readable receipt with measured FPS and sample counts. Missing/false success markers, invalid metrics, lost foreground status or an inaccessible compositor result in failure, not acceptance.

The dedicated [native workflow](https://github.com/sguzman/starbyte/actions/workflows/native-hyprland.yml) is limited to trusted main-branch dispatch, requires a self-hosted Linux runner, and never uploads personal host logs. [`scripts/qa-native-hyprland.sh`](../scripts/qa-native-hyprland.sh) performs the isolated build and actual 12-probe run. [`scripts/qa-runner-ephemeral.sh`](../scripts/qa-runner-ephemeral.sh) is an unattended *local-agent* bootstrap: when executed inside an already-authenticated owner's interactive Hyprland shell, it retrieves a checksum-verified official runner, dispatches the latest physical workflow, processes already queued runs in a bounded loop, and deregisters each runner after its one job. The bootstrap **cannot** remotely enter the user's desktop or run from a hosted CI job.

[CI 40127403253](https://github.com/sguzman/starbyte/actions/runs/40127403253) passed four jobs including native synthetic Weston smoke and focused receipt unit tests. Older physical dispatch attempts were queued without a connected runner; **no real desktop observations, speaker audibility or gamepad results were obtained**. D-12, A-08, P-18/P-19 and D-13–D-15/C-16 remain pending. Keep the evidence-gated ledger at **69/77 (90%)**.

## Robust battery SRAM autosave retry, including while paused (2026-10-24)

[`af46bc3`](https://github.com/sguzman/starbyte/commit/af46bc34ef509d848daafc85b0a3cf7e224d36e2) fixes the owner advancing its 30-second autosave timestamp **before** attempting a save. A failed write previously postponed the next opportunity by another full 30 seconds. The owner now schedules success for 30 seconds and failure for 5 seconds; a failed ROM load no longer resets the current cartridge's timer. The integration regression deliberately makes the destination `.srm` path a directory, checks a real atomic write failure and short retry deadline, restores the path, and confirms the battery byte is persisted on retry.

[`9b62b85`](https://github.com/sguzman/starbyte/commit/9b62b85b862fbcc52f7f3c4414275adb24790f5d) extends persistence to **manual/background pauses**. The generic exclusive scheduler adds an optional owner-thread idle maintenance hook while the frame clock is stopped. `OwnedSession` uses it only when a save actually failed, retrying about five seconds after failure without depending on repaint callbacks or executing additional emulation frames. Both an independent idle-hook test and a paused-session failed-write/recovery regression pass. The successful owner's usual autosave interval and SRAM-on-shutdown behavior remain intact. [CI 40120984331](https://github.com/sguzman/starbyte/actions/runs/40120984331) passed all four jobs.

## Manual audio device resets survive a full PCM queue (2026-10-24)

[`2260374`](https://github.com/sguzman/starbyte/commit/22603740d86dd50fe08bc91a5b1027e6d2f467e2) fixes a real output recovery weakness: `AudioOutput::request_reopen` previously failed whenever the six-message PCM queue was full, so a user-initiated speaker route re-selection could be refused precisely during active playback. The worker now reads an atomic manual-reopen flag before processing PCM; the bounded `Reopen` message merely wakes it if there is room. Repeated requests coalesce; a manual reset takes precedence over automatic stream-fault handling and never fabricates a recovered-speaker receipt. An offline regression fills the queue with PCM and confirms the reset request persists without dropping or blocking.

[Exact-source CI 40121513425](https://github.com/sguzman/starbyte/actions/runs/40121513425) passed all four jobs, including Linux virtual Wayland and Windows. These checks prove the commands and synthetic file/device paths, **not real output routing, physically audible game audio, full gameplay persistence or Hyprland workspace behavior**. P-18/P-19 and the six other physical/cutover gates remain unaccepted: **69/77 (90%)**.

## Live sound settings coalesced under worker backlog (2026-10-24)

[`fceb8e1`](https://github.com/sguzman/starbyte/commit/fceb8e1fbd433631e384ca24fcd3890d3d34acc0) completes the earlier mute-queue repair without allowing an unbounded queue for repeated volume-slider changes. The scheduler's `LiveControl` command now references a shared latest-only slot. A subsequent pending setting replaces the previous closure without posting another message. The owner takes and executes the latest setting at its serialized turn. Updates arriving after the owner took that closure receive a new control message, so they are not lost. The normal `MAX_PENDING_POSTS` quota remains enforced for file, screenshot, cheat and save operations, while a user mute change is not refused by that unrelated quota.

A deterministic stress regression deliberately blocks the owner, submits 500 volume changes, then releases it: only one pending live-control closure executes and the owner sees the final value (499). The existing full-file-queue mute regression also passes. [Exact-source CI 40119339861](https://github.com/sguzman/starbyte/actions/runs/40119339861) passed all four Linux native Weston, Windows, formatting/Clippy and Rust 1.88 jobs. This is source and synthetic evidence, **not** physical speaker mute latency, perceptual quality or reconnect testing. P-18 and the other seven physical/cutover gates remain open at **69/77 (90%)**.

## Atomic playback configuration and non-droppable live mute (2026-10-24)

[`63aa5da`](https://github.com/sguzman/starbyte/commit/63aa5da8cc2a55142ab0aa7a3acc4c2ebfe308af) closes a real synchronization error. Previously `configure_playback_speed` posted a clock speed change and a separate queued `OwnedSession::set_speed` update; one frame could execute at the new rate with stale audio speed/pitch, or a saturated file/cheat queue could refuse the second operation after the clock was already changed. The generic scheduler now accepts one bounded `SpeedAndApply` command, updates the owner and clock policy between frames, and refuses both changes together when saturated. A regression covers the atomic commit and another intentionally fills the bounded queue before a rejected update.

[`626f9e6`](https://github.com/sguzman/starbyte/commit/626f9e68b7b357e69429597dc4a1faab39acd32c) makes infrequent live sound settings use an ordered `LiveControl` command exempt from that file/cheat job quota. Without it, pressing Mute on a full async queue could log an error but leave the output audible. A synthetic saturation test checks that the mute operation still reaches the worker and the normal bounded job quota stays intact. This is eventual serialized application, not instantaneous real-hardware mute latency measurement.

## Rebinding suppression survives gamepad focus recovery (2026-10-24)

[`bcfc3c6`](https://github.com/sguzman/starbyte/commit/bcfc3c6d126492b6de2efc7c3400e4c9987f9442) fixes a follow-on bug in the cached gamepad resynchronization: a button/axis used to rebind an SNES action must remain suppressed until physical release, even if the user switches away from Starbyte and returns while still holding it. The focus-loss code previously discarded the capture suppression map, allowing that still-held rebinding gesture to trigger gameplay on return. It now retains previous captures while unfocused, then intersects them with actual currently held connected-pad button, stick and D-pad state when focus returns, pruning released or unplugged controls. Synthetic regression checks held, partially released and fully released captures.

All three changes and the concurrently committed keyboard-edge expiration path passed [exact-revision CI 40118580865](https://github.com/sguzman/starbyte/actions/runs/40118580865): Ubuntu workspace plus native Weston, Windows tests/benchmark compile, formatting/Clippy and Rust 1.88. **The source fixes do not satisfy physical A-08, P-18/P-19 or D-12–D-15; accepted checklist remains 69/77 (90%).**

## Effective heartbeat-resume clock rebasing and idle mixer gain (2026-10-24)

[`0c3744f`](https://github.com/sguzman/starbyte/commit/0c3744fb3aff62f33f272e0d44856fa53456d5ae) fixes a scheduler transition defect: the old worker loop assigned `was_running = running` before checking `!was_running` to decide whether to rebase its frame deadline. That made a resume detected only by a fresh compositor heartbeat (with unchanged Run/Focus policy) miss its intended timing reset. The worker now saves `just_resumed` before publishing the current clock state and starts a fresh monotonic origin on every effective resume, as it already does for speed changes. Tests cover a missed heartbeat and resuming without a catch-up frame burst.

The same verified revision fixes an idle mixer-volume path. `AudioOutput::set_volume` correctly publishes the desired gain atomically, but its nonblocking VolumeChanged message can be dropped when the bounded frame/control queue is full. If gameplay then pauses and no new PCM arrives, the Rodio sink could retain the previous volume indefinitely. The detached audio worker now reconciles current sink gain on its existing 250 ms idle health polls. An offline Rodio test covers a full queue with a dropped notification and the mixer receiving subsequent gain changes. No new device opens solely for volume polling. [Exact-source CI 40116531381](https://github.com/sguzman/starbyte/actions/runs/40116531381) passed all four jobs. **A-08, P-18, D-12–D-14 remain physically unaccepted.**

## Restore held gamepad state without replaying stale taps (2026-10-24)

[`f94ae9e`](https://github.com/sguzman/starbyte/commit/f94ae9e799df7590965037d5b526906bfa449800) extends the previous backlog-discard fix in both the original-window and worker-preview routes. After draining old Gilrs events, it snapshots connected pads' *current* pressed buttons and live analog/D-pad axes; only these currently held values are restored, while the queued press-edge buffer stays clear. A tap fully pressed and released on another workspace is not replayed; a control still held when the user returns is no longer forgotten until repress. Resynchronization now happens on quick focus return even when the window kept repainting and the callback gap never exceeded 500 ms. The tests check all named Gilrs button enumeration excluding Unknown, short/long poll behavior and the quick blur/refocus transition. [Exact-source CI 40117342690](https://github.com/sguzman/starbyte/actions/runs/40117342690) passed Linux native Weston and workspace tests, Windows, lint and pinned Rust 1.88.

These are real code and synthetic regression results, not assertions of physical controller movement, audible game sound or a production cutover. **P-19/D-15 and all other unaccepted gates remain pending; the checklist stays 69/77 (90%).**

## Off-workspace controller event backlog no longer replays on return

[Code `0f1ef09`](https://github.com/sguzman/starbyte/commit/0f1ef09c1653a43afd458eb2d166d99dfa209d2d) fixes a gameplay-input failure mode in both input implementations. Gilrs accumulates button and axis events even while an egui window receives no redraw callback. On return, previously hidden-workspace button taps could be interpreted as fresh press edges. The worker preview already cleared its *held* state after a long update gap, but still interpreted the queued events that followed; the original-window path had the same backlog problem. Both paths now drain pending Gilrs events without applying them after focus loss or a gap of at least 500 ms, clear held/queued input and wait for genuinely new input. Normal short-interval polls retain press-and-release edge behavior.

[Exact-source CI 40056557353](https://github.com/sguzman/starbyte/actions/runs/40056557353) passed Linux native Weston and workspace tests, Windows tests/benchmark compilation, formatting/Clippy and pinned Rust 1.88. New device-free regressions confirm the normal, unfocused and stale-poll decision boundary; they do **not** simulate a physical gamepad or an actual compositor-focus transition. A direct physical pad press/release and post-workspace-return test is still required for P-19/D-15, so **69/77 (90%) accepted** is unchanged.

## Silent playback: real CPAL-fault polling and honest recovery counts

[Code `fe1d16b`](https://github.com/sguzman/starbyte/commit/fe1d16bf97d664af454c95ca4c878ff88adba5da) fixes a worker lifetime gap in the existing audio recovery path. Previously the Rodio/CPAL worker waited indefinitely for its next PCM/control message; when emulation was paused, an asynchronous stream-failure callback could go unnoticed until gameplay resumed. The worker now checks for already-open stream faults on a bounded 250 ms idle interval and retries failed outputs using the existing 10-second cooldown, even with no new PCM. A previously unused sound route is not opened simply because the poll fired. Manual route re-selection also cancels outstanding automatic-recovery attribution so subsequent reopening is not mislabeled as fault recovery.

[Exact-code CI 40055135458](https://github.com/sguzman/starbyte/actions/runs/40055135458) passed Linux tests including original-window Weston, Windows tests and benchmarks, formatting/Clippy and the pinned Rust 1.88 build. Device-independent tests cover idle retry prerequisites, cooldown and the separation of manual resets from automatic fault recovery. They cannot prove that a particular real-world unplug produces a CPAL callback, that a recovered OS stream routes to the desired speaker, or that game PCM is audible after reconnection. **P-18 is pending; total remains 69/77 (90%).**

## Mixer gain during paused play and manual default-route reset

[`f72b672`](https://github.com/sguzman/starbyte/commit/f72b67215de38727e029d02f4da04bc5e2bc5134) fixes the live audio mixer gain so a volume-only action takes effect without waiting for the next PCM buffer, and adds a bounded, asynchronous Sound-panel action to reset a stale default output route even when CPAL did not report a stream fault. The action preserves emulator/SRAM ownership, handles full worker queues, and does not count manual resets as automatic stream-error recoveries. Offline Rodio and real audio-worker message tests cover queueing, idle gain application, and status reset; [CI 40054055103](https://github.com/sguzman/starbyte/actions/runs/40054055103) covers source and virtual Weston only. Physical audible sound, device disconnect/reconnect and hardware acceptance remain pending; **69/77 accepted**.

## Session-panel owner transition diagnostics

[`d1b950f`](https://github.com/sguzman/starbyte/commit/d1b950f06150d3ee97a1d2f96964e31f06f6e3fb) displays worker-owned suspension and resumption totals in the original session diagnostics panel, without expanding the small-window performance overlay. [CI 40052422079](https://github.com/sguzman/starbyte/actions/runs/40052422079) verifies the code; real compositor blur, physical speaker and gamepad operation remain separately gated. The accepted ledger stays 69/77.

## Owner-clock suspend/resume receipts and real focus-loss QA

[`e9c0cc5`](https://github.com/sguzman/starbyte/commit/e9c0cc5d7a36e5484949620e52c018361d1daee7) adds monotonic worker-owned suspend/resume transition counters, with tests for manual Pause, compositor heartbeat expiry, and focus policy. The independent no-repaint worker smoke rejects unexpected clock suspensions even when frame totals could otherwise suggest uninterrupted emulation. A second **opt-in physical Hyprland probe** now arms background pause from inside the original window and requires the local compositor to leave and return to its workspace before crediting suspend/resume counts and no-catchup pacing. Nine default synthetic probes remain isolated; the two physically orchestrated cases are never invoked by hosted CI. [CI 40052095971](https://github.com/sguzman/starbyte/actions/runs/40052095971) validates source and virtual Weston, not hardware. All eight hardware/production gates remain **pending** until actual device/workspace observations satisfy their complete task wording.

## Opt-in real Hyprland workspace QA adjunct

[`9cab61a`](https://github.com/sguzman/starbyte/commit/9cab61a569698b1e98d9c1946f36800f76d35dc5) adds `starbyte-native-qa --physical-workspace` to the nine isolated synthetic demo probes. On a real compositor using the current Lua dispatcher API, this optional tenth probe verifies the spawned Starbyte window is native Wayland and on a regular numeric workspace; it switches to a private named workspace during an independent emulation gap, restores the original workspace even on ordinary failures, and records a minimal local JSON receipt. This follows the accepted source test pattern but no physical execution occurred in hosted CI. [CI 40048375872](https://github.com/sguzman/starbyte/actions/runs/40048375872) checks code and virtual Weston only. **The 8 hardware/cutover checklist gates remain pending** until actual physical evidence and gate-specific observations exist.

## Native Hyprland QA runner (source milestone)

[`f6e3890`](https://github.com/sguzman/starbyte/commit/f6e3890316bea574bb401d8b8bd608cd7e16fcc6) introduced an opt-in Rust binary for nine original-window worker demo checks in a local Hyprland session, with local-only receipts. Its first CI run was superseded during iteration; the subsequent [fully green 40048755437](https://github.com/sguzman/starbyte/actions/runs/40048755437) includes and verifies the runner's source along with native virtual Weston. No physical Hyprland run is claimed; workspace transitions, gamepads, speaker audibility, actual output-device reconnect and production cutover remain unaccepted. The source ledger remains **69/77**.

## 2026-10-23 stream-fault accounting and candidate isolation

The audio worker now distinguishes **CPAL stream faults** from **successful output reopenings** and publishes both counts through the opt-in original-window Sound panel and the legacy worker's audio output. A successful initial open, or an unsuccessful retry after a reported fault, never counts as a recovery. Each default/alternate-device candidate owns its own fault flag; a callback from a rejected candidate cannot mark the ultimately selected stream failed and trigger a spurious second restart.

[`ae9ce76`](https://github.com/sguzman/starbyte/commit/ae9ce76602758eac3e2abbfea1abf081adc53af5) includes device-free tests for these distinctions; [CI 40045761526](https://github.com/sguzman/starbyte/actions/runs/40045761526) is the exact-code verification run. These counters measure software events only. They **do not** verify that a physically disconnected output is detected, reconnection is audible or a background game continues making sound. P-18 stays pending, as do all seven other hardware/production tasks.

## 2026-10-23 post-handoff receipt and audio recovery hardening

- The full-library opt-in worker preserves separately ordered save and screenshot operations when another ROM is selected. A completed screenshot owns its captured pixels and still writes its PNG after a later cartridge installation, but outdated save/screenshot acknowledgements cannot replace the newer ROM-selection status. The same installed-owner/selection split also fences asynchronous speaker-test status. The source regression and original-window native Weston suite passed on [`baf5a02`](https://github.com/sguzman/starbyte/commit/baf5a025a40ce8bf70c151819aba22decda0bac9), [CI 40043507022](https://github.com/sguzman/starbyte/actions/runs/40043507022).
- The background Rodio output worker can now observe a CPAL stream error after an otherwise successful device open, drop the invalidated stream, clear stale PCM/stretch continuity and retry an output device on its own thread. This preserves default/alternate-output fallback and keeps all OS audio operations away from egui callbacks. [`7839b30`](https://github.com/sguzman/starbyte/commit/7839b30b2440f4189cd0c4929bd89f30b41dca20) adds a device-free callback regression; [CI 40044092562](https://github.com/sguzman/starbyte/actions/runs/40044092562) checks format, Rust 1.88, Linux native Weston and Windows. The physical unplug/replug, speaker audibility and background audio quality acceptance still require real-device observations.
- These changes improve source robustness and diagnostics; they do not complete any of the eight pending physical/production gates. **Accepted source ledger: 69/77 (90%)**. The legacy player remains default and the worker-backed original window remains opt-in.


**Machine-readable source checklist:** [scheduler-progress.json](scheduler-progress.json) (77 stable task IDs and checked-status parity tested by Rust CI).
**Branch:** `main`. **Scope:** Rust/egui game-loop ownership and window-independent SNES timing.

**Physical acceptance checklist:** [qa-native-hyprland.md](qa-native-hyprland.md). This is an unexecuted procedure, not acceptance evidence; the production default remains legacy.

## Speaker-test acknowledgement ordering — 2026-10-23

The original-window Sound menu sends an asynchronous 440 Hz tone to the shared game audio worker. Previously a slow acknowledgement had no game-generation or ROM-selection identity and could replace newer cartridge-loading feedback on the status line. `a35ced5` records both identities with each pending tone; the GUI displays a completion only while the selected cartridge and latest request still match and no installation is pending. No-game tests remain supported (matching absent generations). A Rust regression covers current, superseded, newer-cartridge and installation-pending cases. This fixes stale user feedback, not physical speaker audibility. It does not close an existing scheduler acceptance task.

## Verified superseded-install worker/UI reconciliation — 2026-10-23

Code [`32cc83c`](https://github.com/sguzman/starbyte/commit/32cc83c2909facb54d0378fee52387b7a2bb4a61) passed [CI 40040187151](https://github.com/sguzman/starbyte/actions/runs/40040187151): Windows workspace tests/benchmark compilation, Linux workspace tests and actual original-window native Weston, formatting/Clippy, and pinned Rust 1.88 workspace compile.

- A newer ROM *request* supersedes an older job's user-facing status, but an older successful owner transaction may have **already installed** a cartridge. Previously the full-window GUI discarded that successful receipt entirely, leaving displayed session identity, video, controls, save namespace and generation stale if the newer request then failed.
- The full-window completion handler now **reconciles every successful installed cartridge in owner command order**, including superseded successes, while retaining the superseded job status instead of reporting an obsolete user-facing success. Superseded failures cannot overwrite newer errors.
- The native original-window acceptance extends the real failed-ROM screenshot/paused-step race: it queues a successful same-ROM reinstallation and then a newer invalid ROM request. Its final marker recorded `STARBYTE_FULL_LIBRARY_SUPERSEDED_INSTALL_SMOKE_OK active_owner_synced=true newer_failure_reported=true advanced_frames=11`; the old receipt race remained green. The original worker FPS test also measured `122 frames / 2020 ms = 60.38 FPS` and verified no shadow legacy session.
- The separate progress JSON tracks 77 stable task IDs with source parity checked by Rust tests. This fix and additional test strengthen opt-in worker correctness; no physical Hyprland, audible-device, gamepad or production cutover acceptance was performed. **Total remains 69/77 (90%).**

## Verified independent worker NTSC pacing during no-redraw gap — 2026-10-23

Code commit [`7950e27`](https://github.com/sguzman/starbyte/commit/7950e27a6ef32bfe9be02c67330af3c5238447dc) passed [CI 40036684106](https://github.com/sguzman/starbyte/actions/runs/40036684106) across all three Linux/Weston, Windows and formatting/Clippy jobs.

- The original worker-backed window now measures **completed owner frames per actual monotonic wall-clock second** during a two-second gap with no egui callback, excluding fake UI repaint FPS. Native Weston recorded `STARBYTE_FULL_LIBRARY_CLOCK_PACING_SMOKE_OK advanced_frames=121 fps=59.91 wall_ms=2019`; the nominal hardware NTSC target is approximately 60.10 FPS.
- Its check tolerates 48–72 FPS to allow CI host contention while rejecting serious timing regressions. It separately requires a new framebuffer upload when original-window callbacks resume, and still proves no shadow legacy emulator was instantiated.
- The check is **virtual-Wayland pacing only**, not a visible/focused physical Hyprland, real GPU present, or audible-sound measurement. D 11/15 and total 69/77 (90%) remain unchanged.

## Compiler floor integrity — 2026-10-23

The source uses Rust 2024 `if let` chains, stabilized with Rust **1.88**. The previously declared workspace minimum of 1.85 was inconsistent with that existing source. The workspace declaration is corrected to **1.88**, and a separate CI job checks `cargo check --workspace --all-targets --locked` with Rust **1.88.0**. This is a compiler compatibility gate, not a newly accepted emulator scheduling task.

## Verified native single-owner gate and failed-replacement step accounting — 2026-10-23

[`3817af9`](https://github.com/sguzman/starbyte/commit/3817af966f3f4a3d5b970a594078e04a52f0458f) passed [CI 40034370165](https://github.com/sguzman/starbyte/actions/runs/40034370165), all three jobs: Linux tests and native original-window Weston, Windows tests/benchmark compile, and formatting/Clippy.

- The actual opt-in original eframe window checks that it has **no shadow legacy session** after a worker-owned video frame has been uploaded, and the CI gate requires `STARBYTE_FULL_LIBRARY_SINGLE_OWNER_SMOKE_OK legacy_session_absent=true worker_frame=30`.
- Queuing a new ROM installation no longer clears a previously queued manual single-step on the surviving cartridge. Only an **accepted installation** resets that step count; failed replacements preserve it until the owner delivers a valid paused-frame receipt.
- The native failed-ROM replacement regression now verifies the pending count remained **1 immediately after queuing the invalid replacement**, and fell to **0 upon receiving and uploading the old cartridge's stepped still**. Weston logged `STARBYTE_FULL_LIBRARY_RECEIPT_RACE_SMOKE_OK retained_rom=true screenshot_written=true step_uploaded=true pending_step_cleared=true`.
- These are acceptance-strengthening checks, **not newly completed roadmap tasks**: B stays 18/18 and the overall verified count stays 69/77 (90%). The worker remains opt-in, and physical Hyprland/device acceptance remains open.

## Verified single-session ownership in original window — 2026-10-23

Commit [`f4e6369`](https://github.com/sguzman/starbyte/commit/f4e63699d75dd2c0f7090dd3a14f79830176f188) passed [CI 40032643086](https://github.com/sguzman/starbyte/actions/runs/40032643086): Linux original-window native Weston and all workspace tests, Windows tests/benchmark compilation, and formatting/Clippy.

- The original egui app stores the legacy `FrontendSession` in an `Option` that is **None when the complete original window is started in worker mode**. Startup creates only the `OwnedSession` within the scheduler in that mode, not a shadow legacy emulator. Legacy mode retains the existing synchronous session path.
- `SessionViewBridge` receives `Option<&FrontendSession>`, using its worker snapshot, controls and input diagnostics with no fallback session when a worker is attached; the worker-mode unit test intentionally passes `None`. Direct mutation calls use explicitly named legacy-only accessors.
- Failed partial worker setup can drop an app with no legacy session without panicking, and native screenshot dispatch reads the published view before mutably borrowing worker command queues.
- Native Weston logs reconfirmed independent no-redraw advancement (`advanced_frames=122 caught_up_frame=133`), failed-ROM replacement screenshot + stepped-still receipts, actual F5/F8/F9/Escape shortcuts, shell menus/cover/status rendering and owner-thread shutdown (`finalized=true audio_released=true`).
- **Source milestone B is complete: 18/18.** Total 69/77 (90%). This does **not** authorize default cutover, or claim physical Hyprland/GPU/audio/gamepad acceptance; those separate tasks remain open.

## Verified independent worker and renderer telemetry acceptance — 2026-10-23

[`4020d64`](https://github.com/sguzman/starbyte/commit/4020d643847773833d39673ac22e5a65f1840add) passed [CI 40030994642](https://github.com/sguzman/starbyte/actions/runs/40030994642), including all Linux and Windows tests, lint/Clippy, headless independent-clock checks and native Weston original-window smokes.

- Owner-generated frames now carry a monotonic `ready_at` timestamp. The complete original library's worker path reports **worker-completed FPS, frame gaps and emulation CPU time** separately from **GUI frame-delivery age and successful egui texture submission CPU time**. A restore-only still does not erase the last genuine emulation CPU sample.
- One shared worker-only upload adapter handles installed cartridge stills, ordinary video frames, restored save-state stills and paused manual steps. Invalid or missing RGBA buffers must not advance the recorded texture frame or success count; unit tests explicitly exercise failed and successful uploads.
- The worker publishes a nonblocking, atomic count of outstanding bounded asynchronous operations, including the currently executing operation, and the full original-window overlay and Session panel label it distinctly. The backpressure regression verifies a 128-operation maximum and no spurious count increase after rejected submissions.
- Native original-window regressions remained green: `STARBYTE_FULL_LIBRARY_NO_REPAINT_SMOKE_OK advanced_frames=121 caught_up_frame=133`, `STARBYTE_FULL_LIBRARY_RECEIPT_RACE_SMOKE_OK retained_rom=true screenshot_written=true step_uploaded=true`, and `STARBYTE_FULL_LIBRARY_SHELL_SMOKE_OK menu=true recent_cards=true cover_texture=true library=true status_metrics=true view_toggles=true`.
- **Source-task acceptance:** This closes the opt-in worker telemetry ownership item in Milestone B, **17/18 (94%)**; total **68/77 (88%)**. It does **not** establish real GPU presentation timing, speaker audibility, physical Hyprland off-workspace playback or production-default worker cutover.

## Verified native failed-replacement receipt-race acceptance — 2026-10-23

The new opt-in original-window extension at [`c74ed82`](https://github.com/sguzman/starbyte/commit/c74ed8282caa50ef01e43bbaa9e0cc58149e6e39) issues an asynchronous screenshot capture and a paused one-frame step, then attempts a second invalid ROM installation before processing their receipts. A failing installation must not invalidate either receipt belonging to the surviving cartridge, including the PNG writer completion and actual uploaded still texture. The corresponding [CI gate](https://github.com/sguzman/starbyte/commit/24bd74ea110c2d009eddd6aa4d96a3c73acf6531) requires `STARBYTE_FULL_LIBRARY_RECEIPT_RACE_SMOKE_OK` alongside the existing replacement regression. The final corrected change at [`8c455a5`](https://github.com/sguzman/starbyte/commit/8c455a5aada8eeb8c582e356d41cca8ff6ae7b61) passed [CI 40025522747](https://github.com/sguzman/starbyte/actions/runs/40025522747) across Linux, Windows, and lint/Clippy. The native Weston original-window log recorded `STARBYTE_FULL_LIBRARY_REPLACEMENT_SMOKE_OK retained_rom=true advanced_frames=11` and `STARBYTE_FULL_LIBRARY_RECEIPT_RACE_SMOKE_OK retained_rom=true screenshot_written=true step_uploaded=true`. The worker-backed legacy shell remains **opt-in**, and the extra regression closes no additional tracked roadmap task: **67/77 (87%)**.

## Verified original-window shell and owner shutdown — 2026-10-23

Commit [`cd8c3f5`](https://github.com/sguzman/starbyte/commit/cd8c3f54dd5c1b7c988f7a5253a4bef9554498c6) passed [CI 40021161371](https://github.com/sguzman/starbyte/actions/runs/40021161371) with native original-window View/status toggle, recent-cover texture, and library rendering acceptance. The original application's `Drop` also consumes and joins the exclusive worker exactly once; [`d15fcf4`](https://github.com/sguzman/starbyte/commit/d15fcf49937143f98f4b6679c565f196e82eb25f) passed [CI 40016711374](https://github.com/sguzman/starbyte/actions/runs/40016711374) with SRAM reopen and `STARBYTE_FULL_LIBRARY_OWNER_SHUTDOWN_SMOKE_OK finalized=true audio_released=true`. These close two opt-in shell tasks but **not** production default cutover.

## Verified original-window shortcut and save namespace parity — 2026-10-23

Source commit [`432b221`](https://github.com/sguzman/starbyte/commit/432b2219ac9c89dc82c9183b2f2ef7c31b8a66d8)
passed [CI 40015689046](https://github.com/sguzman/starbyte/actions/runs/40015689046)
with **all Linux, Windows and format/Clippy jobs green**, including the
expanded native Weston full-library suite.

- **Actual shortcut event path:** synthetic egui key events enter the same
  `raw_input_hook` and ordinary application hotkey handlers as physical
  F5, F8, F9 and Escape input. The complete original window successfully
  quick-saves, restores, visits its library while the independent SNES worker
  continues making progress, returns to the game, and uploads a fresh
  framebuffer. The native result was
  `STARBYTE_FULL_LIBRARY_SHORTCUT_SMOKE_OK f5=true f8=true f9=true escape=true frame=44`.
  The earlier real-owner SNES auto-joypad integration separately verified
  pressed **and released** button bits at the emulated hardware register.
- **Original-window save mode switching:** starting with persistent sound,
  the complete library saves disk slot 1, reinstalls the same cartridge in
  compatibility mode, verifies the persistent slot is **not available in
  that separate namespace**, saves a compatibility slot, returns to
  persistent sound, and verifies the original persistent slot reappears
  without an inherited quick save. The native result was
  `STARBYTE_FULL_LIBRARY_MODE_SMOKE_OK persistent_slot=true compatibility_slot=true separate_slots=true`.
  Existing worker integration tests additionally verify isolated diagnostic
  mode rejects durable saves; previous native full-window SRAM acceptance
  reopened a battery-backed cartridge after UI shutdown and recovered its
  written byte `0x5A`.

These two native checks close the opt-in complete-window Milestone C
controller/shortcut and save-namespace tasks, lifting C to **13/16 (81%)**.
Tracked source coverage is **65/77 (84%)**: A 7/8 (88%), B 16/18 (89%),
P 18/20 (90%), C 13/16 (81%), D 11/15 (73%).

**Acceptance boundary:** synthesized egui keys and virtual Weston are not
proof of a specific physical keyboard or gamepad, actual audible output or
off-workspace Hyprland behavior. Original window library View toggles/covers
still require explicit parity acceptance. The stable legacy player remains
the production default; independent emulation in the complete original
library is still opt-in.

## Verified full-window failure-safe cartridge replacement — 2026-10-23

Commit [`5d5ef8e`](https://github.com/sguzman/starbyte/commit/5d5ef8e3a57e9966cb6d725f9e3c9905afc64f36)
passed [CI 40013247831](https://github.com/sguzman/starbyte/actions/runs/40013247831)
with **Linux, Windows and formatting/Clippy all green**, including the full
original Starbyte library window under native Weston.

The hidden replacement acceptance begins actual original-window gameplay,
queues an asynchronous ROM installation for a deliberately absent cartridge,
and verifies the worker's failure is acknowledged in the ordinary UI while
the **previous valid ROM path and active installed-cartridge generation**
remain unchanged. The surviving game must continue at least 10 more frames.
The test observed `STARBYTE_FULL_LIBRARY_REPLACEMENT_SMOKE_OK
retained_rom=true advanced_frames=10`. Both successful original-window
installation and failed replacement now have native-window acceptance, so
the worker launch reply/error integration item is checked.

**Progress:** 63/77 tracked source tasks (**82%**), with full native GUI
migration C 11/16 (**69%**). Milestones A 7/8 (88%), B 16/18 (89%),
P 18/20 (90%), D 11/15 (73%) are unchanged. The legacy runtime remains
the default. This is Weston virtual-Wayland proof, not verified physical
Hyprland audio/workspace behavior, real-game compatibility or a production
default-owner switch.

## Verified manual-pause independence and audio-policy handoff — 2026-10-23

Commit [`51e729b`](https://github.com/sguzman/starbyte/commit/51e729bc106fea64c34cfb01040f5afa798bf80f)
passed [CI 40011629443](https://github.com/sguzman/starbyte/actions/runs/40011629443)
with **Linux, Windows, formatting and Clippy all green**, including the
original full-library native Weston smoke.

The full-window focus acceptance now tests a complete sequence: automatic
focus pause, automatic focus resume, **deliberate manual Pause followed by a
second focus loss/restoration**, and an explicit manual Play. It checks that
manual Pause cannot be overridden by regaining focus. The native result was
`STARBYTE_FULL_LIBRARY_FOCUS_PAUSE_SMOKE_OK paused_frames=0 resumed_frames=11 manual_paused_frames=0 manual_resumed_frames=11`.
This closes Milestone C's independent manual/background pause-state task in
the opt-in original-window worker path; **C advances to 10/16 (63%)**.

A separate integration regression submits worker-owned playback speed and
pitch changes (preserved pitch, speed-following pitch and custom pitch at
constant speed) through the actual scheduler/owner command queue, along
with mute and volume updates, in each of the compatibility, persistent and
isolated sound/save modes. It verifies the exclusive owner receives the
expected values and does not lose its selected sound-mode namespace.
Together with the existing original-app pitch-selection mapping and
pitch-only-change regression, this closes Milestone B's **control-path
speed/pitch handoff** task: **B advances to 16/18 (89%)**.

Neither check proves audible device fidelity or physical Hyprland behavior;
the worker-backed complete library remains opt-in. Current evidence-backed
source total is **62/77 (81%)**: A 7/8 (88%), B 16/18 (89%), P 18/20
(90%), C 10/16 (63%), D 11/15 (73%). Production-default cutover is not
complete.

## Verified complete-window video catch-up after compositor callback gap — 2026-10-23

Commit [`c981f64`](https://github.com/sguzman/starbyte/commit/c981f6416696724a63513675ca47964454e6f582)
passed [CI 40009459833](https://github.com/sguzman/starbyte/actions/runs/40009459833):
Linux, Windows, formatting, Clippy and native Weston all green.

The previous original full-library no-redraw smoke closed immediately after
confirming emulation progressed with no egui callbacks. It now **returns to
normal egui processing** and requires a texture upload carrying a frame
number at least as new as the emulation count observed after the two-second
gap. The acceptance log reports
`STARBYTE_FULL_LIBRARY_NO_REPAINT_SMOKE_OK advanced_frames=121 caught_up_frame=134`.
Only an actual `upload_rgba` updates the recorded texture frame, so this
cannot be satisfied by a worker frame counter or cached status alone.

This **closes the full-app latest-completed RGBA task** in Milestone C, on
the current opt-in complete-library path, moving that milestone to **9/16
(56%)**. Source-task total is now **60/77 (78%)**: A 7/8 (88%), B 15/18
(83%), P 18/20 (90%), C 9/16 (56%) and D 11/15 (73%).

**Not established:** physical Hyprland off-workspace behavior, audio
audibility, complete original-menu/input parity, and production-default
worker ownership. A native virtual-Wayland successful texture upload is
evidence for publication/catch-up, not for the default migration.

## Verified original-window async status, sound modes and screenshot ownership — 2026-10-23

Commit [`a0fbdd8`](https://github.com/sguzman/starbyte/commit/a0fbdd8cb10494352d6a782c447f814287d4c37a)
passed [CI run 40007158503](https://github.com/sguzman/starbyte/actions/runs/40007158503)
with **Linux, Windows and lint/Clippy green**, including the existing
native Weston original-window tests. The new source/test coverage:

- **Delayed PNG completions:** an asynchronous screenshot retains its
  selected-cartridge generation through PNG encoding. A late completion
  still writes its requested image but cannot overwrite a newer game's UI
  status; the generation predicate has a focused regression.
- **Exclusive SRAM owner:** the original window skips its legacy UI-thread
  30-second SRAM flush whenever the independent worker owns the game.
  The worker already flushes on its own 30-second cadence, pause and
  shutdown; native battery-backed SRAM shutdown acceptance remains green.
- **Sound/save namespaces:** an actual worker-mode integration test
  switches the same built-in synthetic cartridge among persistent sound,
  compatibility and isolated diagnostic modes. Persistent and legacy
  disk-slot paths are distinct; switching back finds the appropriate
  slot; memory quick saves are invalidated; isolated diagnostic mode
  refuses durable saves.
- **SPC700 panel detail:** the original opt-in full-library sound panel
  no longer substitutes a generic placeholder for emulated SPC700
  diagnostics. It requests owned APU information asynchronously only
  while visible, at most once per second, and ignores replies from
  superseded ROM generations. A worker-side test verifies the typed
  query reflects sound-driver enablement across a mode replacement.

**Evidence boundary:** CI demonstrates compilation, unit/integration
coverage and native virtual-Wayland regression stability. It has *not*
visually exercised every open/close state of this sound panel or reproduced
a delayed PNG race during a native ROM replacement. Physical Hyprland
focus transitions, speaker audibility and full default runtime cutover
are still unverified. Source-task progress stays A 7/8 (88%), B 15/18
(83%), P 18/20 (90%), C 8/16 (50%), D 11/15 (73%). The established
legacy player remains the default.

## Verified worker input through SNES auto joypad — 2026-10-23

Commit [`f7114d9`](https://github.com/sguzman/starbyte/commit/f7114d9032356594e6250aab9a38d3ab1b42ed36)
passed [CI 40002095002](https://github.com/sguzman/starbyte/actions/runs/40002095002):
Linux, Windows and lint/Clippy green, including the native Weston GUI suite.

A new integration regression writes a copyright-free cartridge that enables
the SNES automatic joypad polling register (`$4200`). With emulation and its
input port owned by a separate worker thread, it submits Start+Right,
executes frames through the asynchronous one-frame worker command, then
checks **both host and actual emulated latched joypad bits**. It then submits
a released state and verifies both registers clear after subsequent frames.
This proves input reaches the actual virtual SNES hardware, rather than
only updating an egui diagnostic overlay or testing a bitset in isolation.

It does **not** verify physical keyboards/gamepads or Hyprland focus
behavior, and the default GUI has not switched owners.
Source counts remain A 7/8 (88%), B 15/18 (83%), P 18/20 (90%), C 8/16
(50%) and D 11/15 (73%).

## Verified battery-backed SRAM across original GUI shutdown — 2026-10-23

Commit [`66d7147`](https://github.com/sguzman/starbyte/commit/66d71471b7b7a1faf268ff8c75fd4205bab7e08c)
passed [CI 40000854342](https://github.com/sguzman/starbyte/actions/runs/40000854342):
**Linux, Windows and lint/Clippy all green**, including the original-library
native Weston smoke.

A new headless owner regression modifies a small copyright-free synthetic
SNES cartridge to expose battery-backed SRAM. The exclusive emulation worker
writes byte `0x5A` at bus address `0x700000`, with no GUI-side save call,
shuts down, flushes SRAM from the owner finalizer, then reopens the cartridge
to confirm the on-disk value survived.

A separate **native original Starbyte window** test exercises the real eframe
close path (not merely an isolated worker shutdown): the same synthetic
cartridge is launched through the opt-in complete library UI, a worker-owned
cheat writes the SRAM byte, the window closes, `StarbyteApp::drop` joins the
worker, and the executable reopens the persisted cartridge. Weston recorded
`STARBYTE_FULL_LIBRARY_SRAM_SMOKE_OK persisted_byte=5A`.

This verifies a real synthetic battery-backed SRAM write on *normal*
shutdown, not general behavior for all commercial games, sudden process
termination, power loss, storage failure or physical Hyprland. The source
ledger counts remain A 7/8 (88%), B 15/18 (83%), P 18/20 (90%), C 8/16
(50%), D 11/15 (73%); the original full-library worker path is still opt-in,
and the legacy player remains the default.

## Verified original-window owner failure reporting — 2026-10-23

Commit [`f201823`](https://github.com/sguzman/starbyte/commit/f2018235217838bff01647d8ff153dc6446b6f3b)
passed [CI 39998845470](https://github.com/sguzman/starbyte/actions/runs/39998845470)
with green Linux, Windows and lint/Clippy jobs. The opt-in **complete
original library window** now detects a finished emulation-owner thread,
reports a playback error even if the player was manually paused, and
does not repeatedly log the same fatal exit on subsequent redraws.

A new hidden native acceptance mode begins playback, injects a synthetic
panic into *only* the dedicated emulation owner, and waits for the ordinary
window to mark playback stopped. Linux Weston recorded
`STARBYTE_FULL_LIBRARY_WORKER_EXIT_SMOKE_OK playback_stopped=true`.
The earlier focus policy acceptance passed again in the same run:
`STARBYTE_FULL_LIBRARY_FOCUS_PAUSE_SMOKE_OK paused_frames=0 resumed_frames=11`.

This intentionally fatal test never runs in normal desktop launches.
It proves the original GUI sees an unexpected worker exit; it does **not**
prove recovery after an emulator crash, physical Hyprland off-workspace
sound quality, SRAM integrity during a panic, or default worker readiness.
Roadmap counts remain A 7/8 (88%), B 15/18 (83%), P 18/20 (90%),
C 8/16 (50%) and D 11/15 (73%). The legacy player remains the default.

## Verified native focus pause and background continuation — 2026-10-23

Commit [`88da6d7`](https://github.com/sguzman/starbyte/commit/88da6d7b4f542392ae670e346da39db6dac8af2a)
passed [CI run 39997413540](https://github.com/sguzman/starbyte/actions/runs/39997413540):
all three Linux, Windows and lint/Clippy jobs succeeded, including
the complete native Weston GUI smoke suite.

The scheduler publishes an atomic **effective running** flag from its sole
emulation-clock thread, after applying manual Play, focus state, background
pause preference and the GUI heartbeat policy. The publication resets even
when that worker unwinds from a panic. The original full-library window's
status bar, game overlay and Session diagnostics now read that effective
state rather than inferring it from UI frame cadence or stale FPS windows.

The newly added **original full-window** Weston focus-pause smoke starts
gameplay, applies the real worker focus-pause policy, confirms that the
owner is stopped, then deliberately withholds **every egui callback for two
seconds**. It resumes the owner through the same focus policy and demands
new game frames. The log records:

- `STARBYTE_FULL_LIBRARY_FOCUS_PAUSE_SMOKE_OK paused_frames=0 resumed_frames=11`
- `STARBYTE_FULL_LIBRARY_NO_REPAINT_SMOKE_OK advanced_frames=121` in the separate background-allowed case

The focused scheduler unit regression additionally checks the effective
running flag across manual pause, focus loss and missing UI heartbeats. This
is **virtual Wayland/Weston evidence**, not physical Hyprland off-workspace
testing, and no claim about speaker audibility or device recovery follows
from it. All existing source-task counts remain A 7/8 (88%), B 15/18 (83%),
P 18/20 (90%), C 8/16 (50%) and D 11/15 (73%). The established synchronous
player remains the default; worker ownership in the complete library GUI
is still opt-in.

## Latest full-library worker integration — 2026-10-23

Commit [`08c5802`](https://github.com/sguzman/starbyte/commit/08c5802ee644b54e6327292212fb607e9615675a)
passed [CI 39994543237](https://github.com/sguzman/starbyte/actions/runs/39994543237)
with **all three jobs green** (Linux, Windows and lint/Clippy). Native Weston
logged:

- `STARBYTE_FULL_LIBRARY_WORKER_SMOKE_OK frame=31`
- `STARBYTE_FULL_LIBRARY_SAVE_SMOKE_OK frame=83 slot=1 quick=true screenshot=true step_frames=60`
- `STARBYTE_FULL_LIBRARY_NO_REPAINT_SMOKE_OK advanced_frames=121`

In the original library UI's opt-in worker path, the Session panel now uses
worker-produced FPS, frame gaps, emulation CPU, coalesced video frames and
audio-drop diagnostics rather than its unused legacy frame-loop counters.
The two frame-step buttons are disabled during exclusive cartridge
installation so their requests cannot be redirected to the newly installed
game. Library cheat refreshes no longer queue old cartridge patches behind
an in-flight replacement; successful and failed installation responses
resynchronize enabled cheats against whichever game is actually installed.

A real owner-thread regression queues a quick save and then reinstalls the
**same ROM path** with a different sound/save policy. It verifies that the
receipt from the earlier namespace must be filtered by the successfully
installed generation even though its ROM path still matches. The Windows
and Linux test suites passed this regression.

**Limits:** This is code and virtual-Wayland acceptance, not actual sound
audibility, physical Hyprland off-workspace behavior or production-default
cutover. Roadmap source-task counts stay at A 7/8 (88%), B 15/18 (83%),
P 18/20 (90%), C 8/16 (50%), D 11/15 (73%).

## Verified 60-step and save-generation checkpoint — 2026-10-23

Commit [`93d3720`](https://github.com/sguzman/starbyte/commit/93d3720b4abbb4d6720e5ef381dc66e0fff9d152)
passed all jobs in [CI 39991451457](https://github.com/sguzman/starbyte/actions/runs/39991451457).
The original library-window Weston acceptance advanced **60 contiguous, worker-owned,
paused steps**, checking each successive frame number and the restored still
texture before reporting
`STARBYTE_FULL_LIBRARY_SAVE_SMOKE_OK frame=82 slot=1 quick=true screenshot=true step_frames=60`.
The independent no-egui-callback test separately advanced 122 frames in two
seconds. This is native virtual-Wayland evidence, not physical Hyprland validation.

Commit [`28fb308`](https://github.com/sguzman/starbyte/commit/28fb308244a0a5f0b89923b2d934ef7d95038ba7)
also passed Linux, Windows and lint in
[CI 39992289325](https://github.com/sguzman/starbyte/actions/runs/39992289325).
A save/restore receipt is now tagged with the **last successfully installed**
worker cartridge generation, not merely its ROM path. An acknowledgement from
an older cartridge or older sound/save namespace cannot repaint a newer worker
session, even when the path is the same. A failed replacement retains the active
generation, so receipts for the surviving original cartridge remain valid.
The generation guard has focused unit coverage and the full suite plus native
Weston passes; a deliberately interleaved same-ROM replacement/saving
**native race test is not yet available**.

The established player remains the default; the complete-window worker path
is still opt-in. The existing roadmap source counts remain A 7/8 (88%),
B 15/18 (83%), P 18/20 (90%), C 8/16 (50%) and D 11/15 (73%). Neither
virtual Wayland acceptance nor 60-frame stepping establishes real device
audio, physical Hyprland focus changes, or default-runtime cutover.

## Verified original-library worker parity checkpoint — 2026-10-23

The full native [CI run 39990152770](https://github.com/sguzman/starbyte/actions/runs/39990152770)
on source commit [`729aab9`](https://github.com/sguzman/starbyte/commit/729aab95024fb79a138a9cb4d1f93fb81bf0c43d)
**passed all Linux, Windows and lint jobs**, including the headless scheduler
and virtual-Wayland (Weston) integration. The original full-library window
(`--native-worker-preview`) emitted these native acceptance records:

- `STARBYTE_FULL_LIBRARY_WORKER_SMOKE_OK frame=30` — the original game view displayed worker-produced frames.
- `STARBYTE_FULL_LIBRARY_SAVE_SMOKE_OK frame=23 slot=1 quick=true screenshot=true step=true` — worker-owned quick/disk save+restore, PNG screenshot and a **paused single-frame step** returned still video through the original window.
- `STARBYTE_FULL_LIBRARY_NO_REPAINT_SMOKE_OK advanced_frames=121` — emulation continued for two seconds without egui callbacks.

The existing `Step Frame` / `Step 60 Frames` commands now submit asynchronous,
silent frame-step requests to the exclusive worker, one at a time. A unit
regression asserts a paused worker advances exactly one frame, returns an
RGBA still and produces no PCM submissions. Another worker-backed regression
associates installed direct and ZIP-extracted ROMs with library metadata even
though the opt-in full library window's legacy session is empty. An audio
control regression guards **pitch-only** changes at constant game speed.
Intermediate completed-frame mailboxes are drained after a newer explicit
launch/save/restore/step receipt to avoid repainting obsolete still video.

**Acceptance limits:** the new step native smoke exercises one paused frame;
it does not yet prove 60-frame sequence completion on physical Hyprland.
Emulation remains worker-owned only in the opt-in window. Audio audibility,
physical workspace switching, comprehensive gamepad/cheat/menu parity,
shutdown persistence under real gameplay, and the default-owner handoff
remain unverified. **Keep the source task counts unchanged** (A 7/8, B 15/18,
P 18/20, C 8/16, D 11/15); these additions are incremental evidence, not
the completion of their remaining acceptance gates.

## Why a worker, not another repaint timer

The current player runs `FrontendSession::run_frame_with_timeout` inside
`eframe::App::update`. When Hyprland/Wayland withholds a hidden window's
presentation callbacks, a queued egui repaint may not run the application
again. Disabling `pause_when_unfocused` alone does not create a game clock.
The correct long-term owner of ROM state, frame timing and save mutations is a
dedicated emulation thread. The renderer merely consumes completed frames.

## Milestone A: clock and ownership foundation — 8/8 accepted tasks (100%)

- [x] ⏱️ Move deadline pacing into `scheduler.rs` with a dedicated OS thread.
- [x] 🎮 Give the worker exclusive ownership of a generic machine object.
- [x] 🖼️ Replace an unbounded video queue with a one-frame mailbox.
- [x] 💤 Add independent Play, Focus, Background-pause and Speed commands.
- [x] 🛑 Make a failed emulation frame halt the worker and expose its error.
- [x] 📦 Bound outstanding asynchronous operations to 128 without blocking the GUI or losing separately ordered policy commands.
- [x] 🧪 Confirm format, Clippy and full workspace tests on code commit `d0254a7` (CI run 39951239026).
- [x] 🖥️ Verify worker scheduling while Hyprland is off-workspace or occluded.

## Milestone B: thread-owned frontend session — 18/18 source tasks (100%; source-only)

- [x] 🧠 Define `OwnedSession` wrapping `FrontendSession` in `session_engine.rs`.
- [x] 🎵 Produce PCM on the worker and enqueue it to the existing bounded Rodio worker.
- [x] 💾 Serialize quick saves, durable slots and SRAM operations on the worker.
- [x] ⏳ Flush SRAM periodically on the worker and at manual/background pause transitions, independent of UI redraws.
- [x] 🎮 Publish an atomic held-controller state with latched short-press edges.
- [x] 🖼️ Skip unnecessary RGBA copies after 300 ms without UI repaint heartbeats, even when Wayland never reports the window as minimized.
- [x] 📬 Expose typed ROM, save, cheat, audio and input command methods.
- [x] 🖼️ Build a renderer-only adapter that consumes published snapshots and updates egui textures.
- [x] ⌨️ Expire stale input heartbeat after missing window callbacks, preventing endlessly held keys.
- [x] 💾 Publish restored still frames after quick-save and disk-state loads, including while paused.
- [x] 🎮 Support nonblocking ROM replacement with old-game preservation after failed cartridge loads.
- [x] 📸 Share an owned screenshot payload with the established frontend instead of borrowing emulator pixels from the UI.
- [x] 🔊 Publish actual audio-device, submitted PCM, and dropped-buffer diagnostics alongside video frames.
- [x] 💾 Run final SRAM flush and PCM invalidation on the emulation owner thread before shutdown/join.
- [x] 📬 Process command bursts in bounded batches; surface fatal worker exits and queue cheat/save commands without stalling UI frames.
- [x] 🔄 Replace all live `StarbyteApp.session` access in opt-in worker mode with worker commands and snapshots. Optional legacy session is never constructed with the worker; bridge reads published state without fallback. CI 40032643086 verified Linux, Windows and original-window native Weston, including owner finalization.
- [x] 📊 Move opt-in original-window emulation telemetry (FPS, frame gaps, overlays) to worker-produced truth, with separately labeled renderer delivery and texture submission measurements and bounded asynchronous owner-job depth (CI 40030994642). Production default cutover remains separate.
- [x] 🎚️ Preserve current speed/pitch/mute/volume control semantics across sound modes through the opt-in original-app worker handoff (worker command test and pitch-mode mapping, CI 40011629443; physical audibility separately gated).

## Milestone P: opt-in threaded preview parity — 18/20 source tasks (90%)

- [x] 🖼️ Display coalesced worker frames using egui textures without advancing the emulator from UI redraws.
- [x] 🎮 Reuse configured twelve-button keyboard bindings and support several gamepads with analog D-pad emulation.
- [x] ⏱️ Follow configured playback speed and independent audio-pitch defaults.
- [x] 💤 Apply background pause and compositor heartbeat policies separately from manual Pause.
- [x] 💾 Route memory quick saves/loads and three persistent disk slots through worker ownership.
- [x] 📬 Queue preview saves and loads asynchronously to avoid freezing menus.
- [x] 📸 Capture screenshots through the worker and write PNG files on a separate I/O thread.
- [x] 📊 Show actual worker FPS, hitches, CPU time and superseded video-frame counts.
- [x] 🎚️ Add worker-controlled mute, audio enable, volume and independent pitch-mode selectors.
- [x] 🧩 Honor the existing individual View → Status bar metric visibility choices.
- [x] 📂 Open a replacement cartridge asynchronously without blocking menus or discarding the old game on load failure.
- [x] 🔉 Add real device/sink diagnostics and a mute-aware 440 Hz speaker check.
- [x] 🎮 Show worker-produced hardware input and cheat-interception diagnostics.
- [x] ⏩ Use Tab for temporary configured fast-forward speed, with transition-only audio/scheduler updates.
- [x] 🛡️ Release stale gamepad holds on lost compositor focus and expose a worker-owned clear-cheats action.
- [x] 📚 Start the opt-in preview with no cartridge, browse installed games from the real asynchronous library index, and launch each selected ROM/ZIP plus matching cheats on the sole emulation owner.
- [x] 🔎 Search the installed library while rendering only visible rows; disable save/pause controls until a cartridge is installed.
- [ ] 🎼 Verify actual background audio quality and speaker/device recovery on Hyprland.
- [ ] 🧪 Verify preview menus, gamepads, screenshots and save slots natively on Hyprland.
- [x] ✅ Confirm format, compile, Clippy and native Weston preview smoke at code commit `d0254a7`.

**Isolation:** The preview is explicitly selected with `--threaded-preview`. It is
not a separate repository, not the default UI, and not a substitute for the
complete Starbyte library application.

## Milestone C: native GUI migration — 15/16 source tasks (94%; production-default worker cutover still pending)

- [x] 🧱 Introduce frontend-neutral SessionControlsSnapshot and migrate the existing disk-slot menu to it; worker can serialize the same summary.
- [x] 🎮 Migrate the existing input overlay to frontend-neutral SessionInputDiagnostics that the worker can also publish.
- [x] 📸 Migrate the existing screenshot action to a shared owned SessionScreenshot payload that the worker can capture as well.
- [x] 🔊 Use shared frontend PCM metrics and sound diagnostics in both legacy app and worker, without direct emulator access from ordinary menus.
- [x] 🛡️ Isolate deep blackout-monitor emulator inspection behind the FrontendSession boundary for later migration into worker ownership.
- [x] 💾 Route the established F5/F8 and durable-slot actions through frontend-neutral SessionSaveAction; the worker uses the same action dispatcher for quick save/load, durable slots and SRAM.
- [x] 🖼️ Share validated nearest-filtered RGBA texture uploads between the legacy player and threaded preview, without adding an intermediate copy to the legacy loop.
- [x] 🎬 Share one typed ROM launch request (resolved path, sound/save mode and matching cheats) across library, direct-file, startup, sound-mode restart and worker APIs, without executing a frame between installation and cheat selection.
- [x] 🎬 Route game-launch success/error replies through the emulation worker, with native full-window Weston failure/survivor acceptance (CI 40013247831).
- [x] ⌨️ Send controller inputs without losing key-down/up edges; preserve F5/F8/F9/Escape. Native original-window key-event smoke and actual SNES joypad press/release regression passed (CI 40015689046, 40002095002).
- [x] 🪟 Display the latest completed RGBA texture without blocking the worker (opt-in full library, Weston catch-up after a two-second callback gap, CI 40009459833).
- [x] 🏠 Keep the full library, menus, recent covers and View status-bar toggles intact (opt-in original-window Weston acceptance, CI 40021161371).
- [x] ⏸️ Keep manual Pause, automatic focus pause, background-allowed gameplay and their resumes independent (original full-library Weston, CI 40011629443).
- [x] 💾 Preserve original disk-slot, SRAM and sound-mode namespaces. Native original-window persistent/compatibility slot switch, disk/quick save/restore and battery SRAM restart tests passed (CI 40015689046, 40000854342).
- [x] 🧹 Complete the opt-in original-window worker shutdown handoff: stop clock, flush SRAM, clear audio and join exactly once (CI 40016711374; production-default cutover remains separately gated).
- [ ] 🚀 Make the worker the default runtime only after passing acceptance gates.

## Milestone D: validation — 12/15 accepted checkpoints (80%; focused FPS, sound and full persistence still pending)

- [x] 🧪 Add a display-server-free, real built-in ROM smoke path (`--demo --scheduler-smoke`) to CI.
- [x] 🖥️ Add an opt-in `--threaded-preview` native egui shell and Weston smoke that requires a rendered texture.
- [x] 🧪 Add a `cargo test` built-in demo ROM integration test for background frames, hidden-window copy suppression and reappearance, neutral controls, screenshot capture and quick-save state.
- [x] 🎬 Add direct/worker launch-request tests for failed ROM preservation, restored save-mode namespaces and memory quick-save invalidation after a successful ROM replacement.
- [x] 🔗 Add a compositor-free integration test from an installed library entry through ROM materialization, exclusive worker launch, unfocused gameplay frames and a worker-owned quick save.
- [x] ✅ Confirm Linux CI format/Clippy and Linux+Windows tests pass on `d0254a7`.
- [x] 🟩 Confirm the headless scheduler smoke and both native Weston smokes complete on `d0254a7`.
- [x] 🎮 Test the opt-in library selection coordinator from installed entry through asynchronous ROM preparation, owned-session launch, still-frame upload and compositor-independent frame progress (Linux test run on `adddfa9`).
- [x] 🪟 Add and pass a dedicated native Weston smoke for `--threaded-preview --exit-after-library-scan` without a startup ROM, with a real indexed built-in SNES demo file (Linux CI run 39955310386).
- [x] 🎮 Add and pass native Weston indexed-library game launch: scan the local demo, select the installed entry through the ROM preparation worker, install it on the exclusive SNES worker, and upload 30 rendered egui frames (`STARBYTE_THREADED_LIBRARY_PLAY_SMOKE_OK`; run 39959433627).
- [x] 🌙 In native Weston, suppress egui callback work and heartbeats for two seconds with background pause disabled; verify the worker advances at least 45 independent frames (observed 121 on commit `135cf2e`, run 39962498642).
- [ ] 🪟 Run Wayland window visible/focused and confirm accurate ~60.1 emulated FPS.
- [ ] 🌌 Switch to another Hyprland workspace; confirm continued emulation and sound when allowed.
- [x] 💤 Repeat with background pause checked; verify true stop, no catch-up burst and clean resume.
- [ ] 🛡️ Confirm manual Pause, gamepad, cheats, screenshot, quick/disk saves and SRAM across restarts.

**Implementation note:** These source milestones measure tested opt-in ownership,
not demonstrated production readiness. The original complete StarbyteApp can
execute on the independent owner behind `--native-worker-preview`, but its
normal/default runtime still retains the synchronous `FrontendSession`.
The remaining eight tasks across milestones A/P/C/D include physical Hyprland,
audible audio/device quality, native input/save verification and deliberate default cutover. The workspace declares Rust 1.88, with a dedicated pinned-version CI compile gate;
this does not count as physical Hyprland acceptance.

## First confirmed CI evidence (2026-10-23)

The GitHub Actions run [39887087222](https://github.com/sguzman/starbyte/actions/runs/39887087222),
built from commit `6acc6d8`, compiled the entire workspace and passed the
Windows Rust tests, Linux Rust test suites, and the display-free
`--demo --scheduler-smoke`. Its actual test output included
`STARBYTE_SCHEDULER_SMOKE_OK background_frames=5 paused_at=5
total_frames=8 restored_frame=8`. This validates the experimental worker
clock, deliberate pause, resume, memory restore and PNG smoke **on that
specific commit**, not yet on the current tip of `main`.

That run also identified a failing **legacy** Weston smoke: the headless
window had no keyboard focus, so its default `pause_when_unfocused=true`
prevented progress until the 90-second timeout. The explicit bounded
`--exit-after-frames` test session now disables focus pause without changing
the default for real players. Source formatter failures were addressed with
exact `rustfmt` diffs recovered from the lint logs.

The subsequent [**green run 39887694659**](https://github.com/sguzman/starbyte/actions/runs/39887694659)
at commit `4be0094` passed `cargo fmt`, Clippy, Windows and Linux tests, the
headless worker smoke, and both native Weston checks. Its verified output
included `STARBYTE_NATIVE_SMOKE_OK frame=30` (legacy window) and
`STARBYTE_THREADED_PREVIEW_SMOKE_OK worker_frames=30 rendered_frame=30`
(threaded window + real texture upload). **This is tested implementation
evidence at that SHA**, not a claim that the default library has been
migrated, nor that the newest commit or a Hyprland desktop is validated.

The dedicated worker also now has a real lost-heartbeat test:
default focus pause must suspend frames when Wayland stops repainting,
while disabling focus pause must continue frames without any UI update.
Full Hyprland native performance and speaker audibility remain untested.

**Latest source note:** The normal application's `WorkerEvent::RomReady`,
file/ZIP open, sound-mode restart and startup ROM flows now use the same
`SessionLaunchRequest` type as `OwnedSession::queue_launch_with_frame`.
This reduces divergence in cheat timing and save namespaces but **does not**
move the legacy app's frame clock or `FrontendSession` to another thread.
Outstanding cutover tasks remain blocking. The command-burst test now
explicitly accepts quota-based backpressure (instead of assuming an
unbounded queue), and CI formatter corrections have been applied.

## Partial handoff interfaces (2026-10-23)

The independent session owner now exposes a single nonblocking save/restore
receipt with the authoritative `SessionControlsSnapshot` and a restored
still framebuffer when applicable. A successful launch can similarly return
`WorkerLaunchResponse` containing both the new cartridge's menu state and
a still frame. The opt-in threaded preview uses these APIs, disables
unavailable quick/disk loads, and refreshes availability from acknowledgements
without synchronous emulator inspection.

The existing full-library ROM-materialization worker now prepares
`SessionLaunchRequest` (resolved path, persistent sound/save namespace and
selected game's cheats) before returning it to `StarbyteApp`. This keeps
request formation out of the GUI event callback and prepares the same
request for the emulation owner. **The established `StarbyteApp` still applies
the request to its UI-owned `FrontendSession`; its frame loop has not
been moved.** No Milestone C checkbox is claimed for these preparatory
interfaces, and source percentages remain unchanged until the actual
worker-owned full application cutover passes acceptance.


## Architectural invariants

1. **One mutable emulator owner.** Only the worker touches `FrontendSession`.
   No `Arc<Mutex<FrontendSession>>` held while drawing UI.
2. **Compositor-independent clock.** `Instant` deadlines and command-receive
   timeouts are enough to drive the worker. No `request_repaint` dependency.
3. **Bounded publication.** The UI sees the newest frame, never a growing video
   backlog; emulation progress must not wait for hidden windows.
4. **Independent audio.** PCM is delivered directly from worker to existing
   bounded audio thread; video is not an audio clock.
5. **Serialized disk state.** ROM load/cheats/save-state/SRAM mutations never race
   an executing SNES frame.
6. **Conservative defaults.** `pause_when_unfocused = true` and existing
   saved settings remain valid. A hidden status bar does not stop emulation.
7. **Falsifiable acceptance.** A successful synthetic clock test cannot prove
   Hyprland behavior, audio fidelity or commercial ROM compatibility.

## Reproduction / developer smoke

```fish
cargo run --locked -p starbyte-egui -- --demo --scheduler-smoke
```

Developers can also launch the opt-in `--demo --threaded-preview` shell to
exercise real native input, video texture upload, frame execution and
background pause. It is deliberately **not** the full game library or the
default desktop app, and its normal functionality is not a substitute for
Milestone C. For Weston/headless acceptance, `--preview-background` explicitly
disables the preview's focus pause without mutating persisted preferences.

This headless path is a diagnostic of the new worker only. Ordinary
`cargo run --release -p starbyte-egui` **still runs the existing UI-owned
scheduler until Milestone C is complete**. Never describe this intermediate
state as a fixed background-pause regression.

## Source evidence

- `scheduler.rs`: dedicated clock, mailbox, command and transaction tests.
- `session_engine.rs`: worker-owned SNES session, audio, input port, periodic SRAM
  flush and framebuffer copying controlled by renderer-update heartbeats; built-in demo integration tests run without a compositor.
- `scheduler_video.rs`: renderer-only texture and telemetry adapter (not yet attached to the live GUI).
- `video_upload.rs`: the same framebuffer upload boundary is already used by the established local game view and opt-in worker renderer.
- `main.rs`: explicit headless worker smoke (including a memory quick-save/restore roundtrip, failed ROM load preservation and temporary PNG validation) and opt-in native-preview routes, no production default switch.
- `threaded_preview.rs`: opt-in worker-powered egui player with configured twelve-button keyboard/gamepad input, independent FPS, audio controls and speaker check, live clock policies, status visibility, asynchronous save/load, ROM replacement, disk slots and F12 screenshots.
- `preview_gamepads.rs`: multi-controller button and analog-axis input aggregation, independent of SNES frame pacing.
- `starbyte-frontend::SessionControlsSnapshot`: common typed summary of ROM, mode, quick-save, durable slots, and cheat-read diagnostics; adopted by the legacy slot menu as a low-risk UI migration seam.
- `starbyte-frontend::SessionInputDiagnostics`, `SessionSoundDiagnostics`, `SessionAudioFrameMetrics`, and `SessionScreenshot`: neutral hardware/audio readouts and owned RGBA capture shared by the established UI and the worker.
- `starbyte-frontend::SessionSaveAction`: one typed save/load/SRAM action dispatcher consumed synchronously by the current application and asynchronously by the worker. A built-in ROM test asserts compatibility and sound-mode disk slot isolation.
- `.github/workflows/ci.yml`: headless worker (including memory quick-save roundtrip) and Weston threaded-preview smoke tasks; all passed on `d0254a7`.

## 2026-10-23 continuation — asynchronous normal-app ROM preparation

The established library UI now dispatches direct ROM/ZIP preparation through
its dedicated ROM worker, instead of extracting ZIP archives in egui callbacks.
Prepared `SessionLaunchRequest` values preserve the selected game's
cheats and persistent sound/save mode, and are applied only after a successful
worker reply. Outdated selection replies/errors cannot replace the newest
selected game; queued superseded ROM jobs are skipped before ZIP extraction.
The legacy `StarbyteApp` still owns and advances `FrontendSession` after
materialization. This is **not** the complete ownership handoff.

Worker-only blackout/CPU/PPU diagnostics now observe `FrontendSession` on the
emulation thread, including when Wayland does not repaint. Worker shutdown
finalizes SRAM and releases the audio output on that owning thread instead
of flushing SRAM a second time when the returned owner is dropped on the UI
thread. The headless library-to-worker integration test exercises the full
materialization, cartridge launch, background clock and quick-save command
chain using the built-in copyright-free demo. Bounded nonblocking diagnostic
publication now writes QA reports on a separate I/O thread, dropping reports
rather than stalling emulation if the two-item queue fills.

**Verified 2026-10-23:** [CI run 39951239026](https://github.com/sguzman/starbyte/actions/runs/39951239026)
at code commit `d0254a7` passed Rust formatting, Clippy, all Linux and Windows
workspace tests, benchmark compilation on both platforms, the independent
background scheduler smoke and native Weston legacy + threaded-preview checks.
This verifies the new worker-side integrations and existing opt-in native
preview under CI, but does **not** establish real Hyprland focus/background
audio behavior or worker-backed execution in the full `StarbyteApp`.

## 2026-10-23 opt-in library bridge

The threaded preview can now boot without a cartridge (`--threaded-preview`),
scan the same configured installed-library sources as Starbyte's regular UI
using `AppWorker`, display a filtered, virtualized list, and pass a selected
entry's typed ROM/cheat launch request directly to the exclusive emulation
worker. Normal `StarbyteApp` still owns its own `FrontendSession`: this
feature exercises a working library-to-worker boundary but does **not**
fulfill Milestone C's main-UI handoff. The `--exit-after-library-scan` flag
is reserved for bounded native Wayland CI acceptance, and is not a normal
player option. Native Hyprland scheduling, audio quality, and complete GUI
feature parity remain mandatory before default-runtime migration.

## Latest worker-owned preview startup change

Commit [`407f30f`](https://github.com/sguzman/starbyte/commit/407f30f637929309683e2a31bcde4b94ff87596f)
now queues preview startup `--rom` and direct File → Open ROM requests
through the dedicated ROM preparation worker, including ZIP handling, before
submitting the typed launch to the emulation owner. The native UI constructor
no longer synchronously installs a cartridge or loads its SRAM. This also adds
two regression tests for startup and direct-file preparation. **These new
source changes are pending their own CI result**; the verified expanded CI
checkpoint below applies specifically to the preceding `adddfa9` SHA.

## Verified expanded CI checkpoint (2026-10-23)

[Run 39955310386](https://github.com/sguzman/starbyte/actions/runs/39955310386)
for `adddfa9` passed format, Clippy, Linux/Windows tests and benchmark
compilation, independent-scheduler smoke, legacy and threaded-preview native
Weston smokes, and the no-startup-cartridge indexed-library Weston smoke.
This does **not** prove physical Hyprland off-workspace playback, speaker
quality or full original library-GUI parity/default readiness.

## 2026-10-23 native indexed-library playback evidence

The fully green [run 39959433627](https://github.com/sguzman/starbyte/actions/runs/39959433627)
from `3a808ff` passed Linux and Windows tests, formatting, Clippy,
headless worker scheduling, and native Weston smokes. In particular, the
`--threaded-preview --preview-background --exit-after-library-play` check
started with **no ROM**, scanned the built-in demo from `--rom-dir`, selected
that indexed game on the asynchronous ROM-preparation lane, and launched it
through the exclusive emulation worker. The log reported
`STARBYTE_THREADED_LIBRARY_PLAY_SMOKE_OK worker_frames=30 rendered_frame=30`.
This is stronger than the earlier index-only smoke: the worker ran and egui
uploaded actual resulting video in one native UI session. It is **not**
physical Hyprland/audio acceptance or proof that the original full-library
`StarbyteApp` has transferred ownership. The subsequent held/press input
sampling separation is preparatory source work and remains unverified until
its own CI completes; it does not close Milestone C's input handoff.

## 2026-10-23 worker install handoff and missing-redraw regression

A new preview launch request arriving while the worker is installing a
cartridge now replaces the pending-next slot rather than returning a busy
error. The preparation lane remains independent; acknowledgements and video
from superseded intermediate installations are not painted. Save/load,
screenshots, cheat changes and pause controls are unavailable while an
exclusive owner install is pending, avoiding cross-cartridge action races.
Regression tests exercise preparation during installation, back-to-back
worker launches, and the save guard. These changes do **not** transfer the
original full-library `StarbyteApp` to the worker.

A further native Weston test launches the built-in demo with background
pause explicitly disabled, then deliberately stops processing egui callbacks,
input and display polling for two seconds. The log on commit `135cf2e`
reported `STARBYTE_NO_REPAINT_SMOKE_OK advanced_frames=121`. That is
evidence for compositor-callback-independent game progression in virtual
Wayland, **not** proof of physical Hyprland focus transitions or speaker
output. The default legacy application is unchanged.

## 2026-10-23 full-library worker integration (opt-in)

The **existing complete** `StarbyteApp` library shell now has a hidden
`--native-worker-preview` integration path. Unlike the smaller
`--threaded-preview`, it retains the original library, covers, menus, cheat
browser, recent games and status controls. Its own `FullLibraryWorker`
owns `OwnedSession`, accepts prepared typed launch requests, publishes
coalesced RGBA frames and menu snapshots, and handles worker input on the
independent clock. The original synchronous `FrontendSession` remains
present but does not execute game frames in that opt-in mode.

The native Weston test `STARBYTE_FULL_LIBRARY_WORKER_SMOKE_OK` reached 31
frames on [run 39981233316](https://github.com/sguzman/starbyte/actions/runs/39981233316).
That same run completed the original-window save sequence
`STARBYTE_FULL_LIBRARY_SAVE_SMOKE_OK frame=18 slot=1 quick=true`
for quick-save, quick-load, persistent disk slot save and disk slot restore.
Linux and Windows test jobs passed, but that commit's lint failed solely for
formatting. Its native Weston evidence is valid for the tested code at that
commit; it is **not** a clean full-pipeline result.

Further opt-in work includes worker-owned cheats, sound mode restart, audio
volume/mute controls, owner-side PCM diagnostics, a worker sound-check
receipt, and screenshot pixels captured by the emulator worker then encoded
to PNG by a separate I/O thread. A separate native Weston test verifies
PNG creation in the full-library path. CI verification for the newest
combined code remains tied to its specific commit and must not be inferred
from earlier passes.

**Acceptance boundary:** default runtime remains the original player.
These changes do not demonstrate physical Hyprland background audio, every
menu command, gamepad handling or a complete shutdown/SRAM regression across
real ROMs. Do not claim the Milestone C default cutover is complete until
the original full-library worker path and its remaining feature parity are
validated and the default switch is deliberately made.
