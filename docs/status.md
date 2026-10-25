# Starbyte: current state

## Hosted-only bootless virtual GPU/input device-model QA (2026-10-25)

Added a separate GitHub-hosted QEMU smoke that rejects live desktop or
self-hosted execution, starts paused TCG-only emulation with a private QMP
socket, and checks that 2D virtio GPU and virtio keyboard PCI devices
instantiate without host display/input/network/storage or KVM access.
This is a virtual-device **model** prerequisite; it does not boot Linux
or prove guest render-node/input-seat initialization. The six physical
scheduler and production gates remain **71/77 (92%)**.

## Nested Hyprland startup blocked by upstream render-allocator requirement (2026-10-25)

Inspection of Aquamarine v0.15.1 source confirmed that a nested Wayland
backend still requires a `zwp_linux_dmabuf_v1` connection and DRM-backed
GBM allocator. The host-device-free Bubblewrap namespace cannot provide
one, even when private Weston headless/Pixman itself starts successfully.
The experimental nested supervisor is therefore removed: the explicitly
requested `--hyprland-nested-smoke` route now fails *before* checking
installed tools or launching a compositor, with a reason suitable for CI
assertion. The previously attempted headless Hyprland mode now fails
early for the same missing allocator instead of repeating its observed
abort. `--weston-smoke` and the physical GUI collector's fail-closed
guard remain intact. This retires an impossible startup path without
weakening the user's input/device isolation. The practical next dependency
is a truly private virtual DRM/render device and input seat; no VM or
physical run was executed and all six gates remain open **71/77 (92%)**.
The remaining Weston-only smoke also has a four-second `wayland-info`
handshake timeout and 20-second whole-sandbox deadline, preventing a
stalled private compositor from hanging unattended QA indefinitely.
The no-GUI namespace `--check-only` audit is capped at ten seconds.
The obsolete proposal to nest Hyprland without any virtual DRM device
was replaced in the execution plan with explicit, unverified VM device
prerequisites: no machine installation or physical test was attempted.
See `docs/native-qa-execution.md` for pinned upstream source evidence.

## Account for game PCM lost while the output device is unavailable (2026-10-25)

When Rodio had no usable OS output, accepted nonzero SNES game PCM fell
through the audio-worker loop without reaching the mixer or incrementing
the worker-drop diagnostic. The Sound panel could therefore report zero
worker losses while the device-open status showed a failure. The worker
now counts those lost nonzero buffers as unroutable and resets streaming
WSOLA overlap, but leaves silent startup PCM and diagnostic speaker
checks out of the game-loss counter. Both original and full-library
worker Sound panels label this broader loss category accurately. The
full-worker test-tone command receipt is also now labeled **queued**, not
**submitted**: it confirms an owner-thread enqueue but cannot certify
Rodio delivery. A device-free regression verifies the loss count exclusions
and recovered time-stretch state. This fixes diagnostics and truthful
feedback, not output device availability, physical audibility or native
Hyprland acceptance.
The scheduler ledger remains **71/77 (92%)**.

## Retry initially unavailable desktop audio even while manually paused (2026-10-25)

When a fresh nonzero game buffer or speaker check first attempted to open
Rodio output and no OS device was available, the worker previously retried
only if another game frame arrived. That leaves paused users stuck in a
failure state after plugging in a device. The audio-thread health loop now
retries a *previously attempted* initial open after the existing ten-second
cooldown, without a GUI frame or emulator advancement. Muted output holds
the retry until sound is enabled; unused workers, manual route resets and
healthy outputs remain lazy. Initial-open recovery does not increment the
separate CPAL stream-fault recovery count. Deterministic synthetic tests
cover the retry deadline, paused/muted behavior and unused-device guard.
A real speaker was not opened in tests, so P-18 and the other physical
acceptance gates remain pending: **71/77 (92%)**.

## Avoid synthesizing expired speaker-check waveforms (2026-10-25)

The dedicated audio worker previously generated every 8,000-frame stereo
440 Hz diagnostic tone before checking whether its queued request had
expired or belonged to an earlier audio generation. Under a saturated queue
the sine-wave work served no purpose and delayed fresh game PCM. Checks now
reject an expired or superseded tone before synthesis and still retain the
existing final validity checks after OS output setup and sample conversion.
A hardware-free regression confirms that rejected requests never invoke
the sample generator, while one valid request invokes it exactly once.
Existing expired-tone counters and device-failure behavior remain intact.
This is playback-thread latency prevention, not evidence of physical audio
or completed native Hyprland gates. Scheduler acceptance remains
**71/77 (92%)**.

## Preserve WSOLA continuity across bounded-queue overflow (2026-10-25)

A producer-side queue overflow formerly incremented diagnostics but supplied
no message to the playback worker, which could continue merging overlapping
samples across the missing SNES PCM chunk. Each accepted game-audio frame
now carries the count of *preceding* producer-side drops. The dedicated
playback worker detects a change in that counter and resets WSOLA and
mixer-gap continuity before processing the first post-drop frame, without
blocking the GUI or clearing retained already-queued Rodio samples.
A no-device bounded-channel regression creates a real producer queue
overflow, verifies the before/after markers, and shows that the recovered
WSOLA output matches a fresh processor. A lost sample cannot be recreated;
this prevents stale blending but does not establish physical speaker or
Hyprland background acceptance. Ledger: **71/77 (92%)**.

## Discard obsolete WSOLA overlap after lost game PCM (2026-10-25)

A frame rejected as over-age, superseded or intentionally suppressed during
a speaker check now resets the streaming WSOLA overlap history and mixer-gap
continuity. Previously, dropping input without resetting WSOLA could carry
pre-gap audio into newly accepted buffers, especially at non-unity tempo.
The second age check now uses the same recovery for expired gameplay PCM.
The delayed-device-open invalidation path also clears the tone exclusion
window and mixer continuity. A hardware-free regression feeds real WSOLA
history, drops a frame, then proves the next output matches a fresh instance
rather than blended pre-gap samples. This is a source-level stale-audio
prevention fix; audible output and native background timing are not yet
physically accepted. Scheduler acceptance remains **71/77 (92%)**.

## Final mixer-entry checkpoint after audio processing (2026-10-25)

A game pause, mute, save restore, cartridge change or output route reset can
invalidate an audio generation while the worker is still converting PCM or
running WSOLA pitch/tempo processing. The worker now rechecks both the live
audio generation and the 150ms gameplay / two-second diagnostic freshness
limits immediately before sending processed samples to the Rodio mixer.
Superseded buffers clear the old sink and reset time stretching; expired
buffers are discarded and counted without a stale append. Mixer gain is also
read after processing rather than before it. A deterministic hardware-free
regression covers a generation change and both age limits at the second
checkpoint. This narrows the in-flight audio race but does not prove
hardware-level silence or cancel audio that the OS has already accepted.
Physical audio and background gates remain pending; accepted scheduler
completion is **71/77 (92%)**.

## Paused Sound panel refreshes live audio-worker counters (2026-10-25)

The full-worker Sound panel now reads its detailed SPC700 diagnostics and
audio-device/mixer counters together through the existing bounded asynchronous
owner query once per second while open. Previously, it only refreshed mixer
counts from completed video frames, so a speaker check or stream failure while
paused could leave its numbers stale indefinitely. A no-ROM, hardware-free
regression queries the actual emulation owner after the real playback worker
expires a test tone, confirms the counter without any game frames, and verifies
that no OS device was opened. Results remain generation-scoped to the active
game. This improves observability, **not** speaker audibility or physical
native-acceptance evidence. The scheduler ledger remains **71/77 (92%)**.

## Show expired speaker checks in the live Sound diagnostics (2026-10-25)

The Sound panel now displays both the number of test tones submitted to
Rodio and the number rejected as expired, for the original library player
and the independent emulator worker. The latter is carried in the existing
`WorkerAudioDiagnostics` snapshot; no UI-thread emulator calls were
introduced. A hardware-free test injects an expired diagnostic into an
actual playback worker, observes it through the owner's snapshot, and
confirms no OS audio device was attempted. Worker metrics are the last
published snapshot and may remain unchanged while the game is paused.
These counters are not evidence of physical audibility. P-18 and D-13
remain pending and the scheduler ledger stays **71/77 (92%)**.


## Expired speaker checks now report a precise outcome (2026-10-25)

A stale speaker-check tone is now counted as expired when the real audio
worker rejects it either before attempting device initialization or after
a slow successful device open. The standalone speaker diagnostic reports
the expiration rather than waiting for an ambiguous five-second timeout.
A hardware-free test injects an already-expired tone into the real worker
and verifies no OS device was opened, no tone was submitted, and the
expiration count increased. A receiver-channel acceptance is **not**
physical speaker verification; the scheduler stays at 71/77 (92%).


## Expire speaker checks after delayed device initialization (2026-10-25)

Unlike 150ms gameplay PCM, pending 440 Hz checks previously had no freshness
limit. A blocked device-open operation could eventually deliver a long-stale
beep after its user-visible five-second diagnostic had already timed out.
Speaker-check messages now have a separate two-second lifetime, verified
both before attempting an OS device open and again before appending sound
to Rodio. Fresh checks still tolerate slower startup than game PCM. A
hardware-free regression verifies the exact age boundaries. This is not
evidence of speaker audibility or physical route recovery. **P-18 and D-13
remain pending, 71/77 scheduler gates accepted (92%).**


## Audio owner shutdown invalidates pending PCM and test tones (2026-10-25)

Releasing the last `AudioOutput` handle now mutes shared gain and advances
the PCM generation before dropping the bounded sender. Otherwise, queued
frames or a speaker-check tone might be drained while the audio thread
shuts down. A no-device regression verifies that queued tone and PCM epochs
are invalidated on owner drop, without claiming that hardware speaker
shutdown timing was physically measured.


## Faulted output retries despite continuous silent PCM (2026-10-25)

An automatically detected CPAL stream failure is now retried on the audio
worker's regular receive loop, rather than only after a 250ms receive
timeout. A game supplying silent PCM continuously can previously prevent
that timeout forever and suppress background recovery. The retry still
honors the 10-second cooldown, does not open an unused/healthy device,
and does not require any GUI repaint or physical QA. Regression tests
model a continuously nonempty silent queue and verify retry eligibility.
No physical output recovery or audibility is asserted. Scheduler acceptance
remains **71/77 (92%)**.


## Speaker check distinguishes stopped workers from a full queue (2026-10-24)

A live 440 Hz tone request now reports whether the bounded producer queue is
full or the playback receiver has exited, instead of describing both as a
busy queue. The Sound panel directs the user to the existing output
re-selection action on a stopped worker. Hardware-free channel regressions
cover both outcomes; no audible speaker verification is claimed.

## Paused audio output re-selection after playback-thread exit (2026-10-24)

Manual output re-selection now detects a disconnected playback receiver even
when a previous reset was still pending. Both the worker-backed player and
the original library UI replace the dead software audio thread on explicit
re-selection, without waiting for a new emulation frame or opening an OS
audio device. Hardware-free regressions terminate real playback receivers,
exercise paused recovery and verify the legacy menu request; queue saturation
still behaves as a nonblocking, nonfatal drop. This is *not* a
physical speaker-recovery acceptance. The six open scheduler gates, including
P-18 and D-13, remain pending at **71/77 (92%)**.

## Owner desktop focus: physical QA suspended (2026-10-24)

The first run's 11/12 successes did not justify disrupting the owner's
working workspace. Codex's second focused-only run at `fd522d4` opened one
window on another workspace but **forcibly switched focus to it**, violating
the owner's non-interruption requirement. The collector rejected focused
active-window/native-client verification (0/1), despite an unverified
application log of 181 frames, 181 textures, 60.006 FPS in 3,016 ms.
**D-12 is still pending; acceptance stays 71/77 (92%).**

The QA wrapper and direct native collector are now **blocked from launching
any GUI probes in the owner's active session**. `--check-only` performs
non-GUI build/unit/ledger checks. This is intentionally a safety restriction
until a separately verified compositor **and input seat** can perform
real GUI QA without workspace or keyboard-focus hijacking. No workaround
may simply restore the previous focus after stealing it.

`scripts/qa-isolated-wayland.sh --check-only` verifies a Bubblewrap
process/mount namespace with host display/session sockets, input/DRM devices,
and user configuration excluded. The Weston 15.0.1 headless/Pixman smoke now
passes using signed, temporary package extraction: `wayland-info` observed
the private compositor global, `weston-simple-shm` ran, and success plus
forced startup/client-failure and SIGTERM paths cleaned up the socket,
process and runtime directory. No separate Hyprland compositor or virtual
input seat is verified. The GUI hard-stops remain in force, and scheduler
acceptance stays **71/77 (92%)**.

Standalone isolated Hyprland 0.56.2 / Aquamarine 0.15.1 was attempted once
with no host DRM/input/session sockets and a private runtime/config. It aborted
during startup (134) before readiness; the private log was removed with the
runtime before cause classification, so the status alone is not treated as
proof of a backend error. Source inspection identifies the blocker: the
headless backend returns no DRM FD, while Aquamarine's backend startup
requires a render allocator unless using the Null backend; Hyprland does not
request Null. Its renderer also initializes EGL through the backend DRM FD.
The private QA runtime was removed and no checkout core artifact remained.
No IPC identity, backend readiness, or `HEADLESS-0` output was verified.
Normal compositor shutdown and timeout cleanup therefore remain untested.
The isolation preflight also tests that host Hyprland IPC, personal config
and session-manager environment are inaccessible.


## Targeted focused QA option (2026-10-24)

After the 11/12 local run, the native executor supports a focused-clock-only
`--focused-only` mode, preserving real compositor and focus checks while
avoiding eleven repeated GUI launches. The dispatcher must use the dedicated
workspace wrapper. A successful 1/1 focused run would be evidence to review
D-12, not proof of audio/device/gamepad/cutover or a complete 12/12 run.
This newly authored mode has not yet received another physical on-host run.

## First real native QA and isolated workspace protection (2026-10-24)

Local Codex ran the native harness on physical Hyprland at `b1448b3`: **11/12 passed**. Focused FPS failed after keyboard-focus loss; the two genuine workspace checks passed. Background: 142 frames in 2,374 ms (59.81 FPS), with 9 compositor samples across 2,289 ms off-workspace. Pause/resume: 2 frames while away, 1 suspension, 1 resume, and 10 resumed frames across 166 ms. The local collector restored the original workspace, left no emulator window running and retained private raw evidence. This supports **A-08 and D-14 accepted, 71/77 (92%)**; all speaker, controller, complete persistence and cutover gates remain pending.

Subsequent isolated QA launcher changes on main reserve an unused numeric Hyprland workspace only for the live GUI probe phase, then conditionally restore the previous workspace; the Rust collector refuses to spawn additional windows if that QA workspace is no longer active. The source/CI tests of those new guard paths must not be mistaken for a second physical run. Foreground-FPS observation still temporarily requires actual input focus, so zero visual/focus disturbance is not guaranteed.


**Updated:** 2026-10-24. **Project direction:** active development. **Product maturity:** experimental Rust SNES emulator, intended for comfortable Linux/Wayland play rather than cycle-perfect hardware accuracy.

**Evidence rule:** passing synthetic tests is not evidence that a specific commercial title is playable. [GitHub Actions on `main`](https://github.com/sguzman/starbyte/actions/workflows/ci.yml) runs Linux formatting/Clippy and Ubuntu/Windows workspace tests. Confirm that the **latest completed run matches the exact commit** before making verification claims. Hosted CI does not constitute a native Wayland desktop test.

## Focused-FPS evidence integrity (2026-10-24)

[`dede0ce`](https://github.com/sguzman/starbyte/commit/dede0ceb325739edf5b86bdfa32f537cd1687600) rejects the physical focused-clock success receipt when advertised FPS contradicts actual completed worker frames and elapsed wall time, or when frame evidence is missing. Regression tests include zero frames, inconsistent rates and normal timestamp rounding. [Exact-code CI 40131591925](https://github.com/sguzman/starbyte/actions/runs/40131591925) passed Linux/virtual Wayland, Windows, lint and Rust 1.88. This improves **evidence validity**, not physical gate acceptance. The native self-hosted run remains unclaimed; accepted is still 69/77 (90%).

## Native QA runner prioritization, failure cleanup and sanitized evidence (2026-10-24)

[`25e8165`](https://github.com/sguzman/starbyte/commit/25e8165ac74307e440e312e5bb1a7275ad221adc) makes the unattended local ephemeral runner select the exact current `main` workflow_dispatch before retiring obsolete *queued* physical requests. Active native jobs are never deliberately canceled; a missing latest dispatch fails instead of quietly running stale code. The queue filters were independently checked with a synthetic mixed-state run-index fixture. [`84bee78`](https://github.com/sguzman/starbyte/commit/84bee78ba01cb977a739b045fd732c874a2e2215) ensures Rust's native probe collector kills and reaps an emulator child if a compositor query or early observation fails, with a Linux process-lifetime regression.

[`e693be2`](https://github.com/sguzman/starbyte/commit/e693be2b717bed82dfb2156d359caad1dff8bcc8) adds an **identity-free, machine-readable clock and pause evidence receipt** that is emitted only if all twelve local probes succeed. The receipt carries focused worker FPS, worker/texture frames, compositor sample counts, real off-workspace scheduling and resume counters. It explicitly marks actual speaker audibility, physical gamepad and production cutover untested; no raw compositor metadata, device names, process IDs, personal paths or game content is published. [Exact-source CI 40130239670](https://github.com/sguzman/starbyte/actions/runs/40130239670) passed all four jobs. **This verifies the implementation, not the physical run.** Native Actions [40127765927](https://github.com/sguzman/starbyte/actions/runs/40127765927) remains pending without a desktop runner; the accepted checklist remains **69/77 (90%)**.

## Native Hyprland acceptance executor prepared and source verified (2026-10-24)

[`4304e20`](https://github.com/sguzman/starbyte/commit/4304e200501bf7759253c2dd73a2176bb46e6aef) adds a physical focused-window worker FPS/texture measurement and compositor-active-window evidence receipt to the opt-in native QA harness. It extends the physical plan to 12 checks and provides an ephemeral local runner bootstrap that can dispatch the latest trusted main workflow without installing a permanent desktop service. [CI 40127403253](https://github.com/sguzman/starbyte/actions/runs/40127403253) passed Linux, Windows, lint/Clippy and Rust 1.88. **Hosted success verifies code only.** Actual physical workflow attempts remain queued without a connected local runner, so speaker, gamepad, off-workspace behavior and default cutover are not accepted; scheduler ledger stays **69/77 (90%)**.

## Verified paused SRAM retries and robust audio-route resets (2026-10-24)

[`af46bc3`](https://github.com/sguzman/starbyte/commit/af46bc34ef509d848daafc85b0a3cf7e224d36e2) repairs a 30-second retry delay after failed battery-SRAM writes. Failed saves retry after 5 seconds while successful writes retain the 30-second cadence; an integration test exercises an actual failed atomic write and subsequent recovery. [`9b62b85`](https://github.com/sguzman/starbyte/commit/9b62b85b862fbcc52f7f3c4414275adb24790f5d) adds a paused-owner maintenance hook so failed SRAM persists without emulating frames or waiting for Wayland redraws; paused retry and idle hook regressions pass [CI 40120984331](https://github.com/sguzman/starbyte/actions/runs/40120984331).

[`2260374`](https://github.com/sguzman/starbyte/commit/22603740d86dd50fe08bc91a5b1027e6d2f467e2) makes manual audio-device re-selection survive a saturated six-message PCM queue via a worker-checked atomic signal and best-effort wakeup. Duplicate resets coalesce, and synthetic queue-saturation tests preserve stream fault/recovery accounting. [Exact-source CI 40121513425](https://github.com/sguzman/starbyte/actions/runs/40121513425) passed all four jobs. These are source/virtual-Wayland results, not real speaker or Hyprland acceptance; scheduler remains **69/77 (90%)**.

## Verified latest-only live sound controls (2026-10-24)

[`fceb8e1`](https://github.com/sguzman/starbyte/commit/fceb8e1fbd433631e384ca24fcd3890d3d34acc0) coalesces rapid mute/volume-slider updates into a single pending serialized owner-thread command instead of enqueuing unbounded volume mutations behind file/cheat jobs. The new 500-update stress regression confirms the final setting wins and executes once. The existing saturated-file-queue mute test still passes. [Exact-source CI 40119339861](https://github.com/sguzman/starbyte/actions/runs/40119339861) passed all four jobs including virtual-Wayland smoke. It does not measure real speaker mute latency. Remaining scheduler acceptance **69/77 (90%)**.

## Verified atomic playback and focus-recovery capture safety (2026-10-24)

[`63aa5da`](https://github.com/sguzman/starbyte/commit/63aa5da8cc2a55142ab0aa7a3acc4c2ebfe308af) applies frame clock speed and session audio tempo/pitch together in one bounded owner-thread command. When the async file/cheat quota is saturated, both are rejected without creating a mismatch. [`626f9e6`](https://github.com/sguzman/starbyte/commit/626f9e68b7b357e69429597dc4a1faab39acd32c) routes infrequent Mute/sound settings through an ordered live-control path that is not refused due to that unrelated queue's saturation. [`bcfc3c6`](https://github.com/sguzman/starbyte/commit/bcfc3c6d126492b6de2efc7c3400e4c9987f9442) keeps rebinding gesture suppression through hidden-window focus interruptions until button or axis release. A concurrent keyboard-edge expiration change also prevents stale queued keys from replaying on return.

All of these source changes passed [the exact main revision's four CI jobs, run 40118580865](https://github.com/sguzman/starbyte/actions/runs/40118580865), including virtual-Wayland smoke. Native speakers, input devices, focus transitions and default-runtime cutover remain untested by that hosted run; scheduler acceptance remains **69/77 (90%)**.

## Verified owner-clock/audio and gamepad focus recovery (2026-10-24)

[`0c3744f`](https://github.com/sguzman/starbyte/commit/0c3744fb3aff62f33f272e0d44856fa53456d5ae) fixes the owner clock's deadline reset on heartbeat-only resume; the effective stopped-to-running edge now rebases scheduling even if no Run/Focus command changed. It also reconciles Rodio's idle mixer gain against the latest atomic mute/volume when a bounded-queue VolumeChanged message was dropped. Synthetic scheduler and offline sink regressions passed [four-job CI 40116531381](https://github.com/sguzman/starbyte/actions/runs/40116531381).

[`f94ae9e`](https://github.com/sguzman/starbyte/commit/f94ae9e799df7590965037d5b526906bfa449800) repairs gamepad input after hidden-workspace or quick focus transitions in both the original GUI and preview. Backlogged Gilrs events are discarded, but the actual held buttons/stick positions are rebuilt from cached connected-device state rather than lost. Completed off-workspace tap edges are not replayed. Linux native Weston, Windows, lint and MSRV checks all passed [CI 40117342690](https://github.com/sguzman/starbyte/actions/runs/40117342690). Physical gamepad, speaker and real Hyprland QA are still pending. Scheduler acceptance **69/77 (90%)** remains unchanged.

## Hidden-workspace controller backlog suppression (2026-10-23)

The original complete Starbyte window and opt-in worker-preview now discard old Gilrs button/axis event queues after a focus loss or a 500 ms-or-longer gap between UI polls. Clearing only held-button state was insufficient: pending button-tap and stick-crossing events could still become *new* SNES input when the compositor resumed rendering. The repaired paths drain old events and clear the held/pressed buffers without disturbing the ordinary short-press behavior during active play. [`0f1ef09`](https://github.com/sguzman/starbyte/commit/0f1ef09c1653a43afd458eb2d166d99dfa209d2d) passed [four-job CI 40056557353](https://github.com/sguzman/starbyte/actions/runs/40056557353), including Linux native Weston. This is source and synthetic test evidence, **not** a physical gamepad test or real off-workspace input validation. Pending hardware/cutover gates keep the scheduler at **69/77 (90%)**.

## Idle audio device-fault handling (2026-10-23)

The detached Rodio/CPAL worker now observes failed output-stream callbacks while emulation is paused, polling every 250 ms instead of waiting indefinitely for the next synthesized PCM or UI control message. A faulted output is released and retried with the existing 10-second cooldown; startup with no prior output remains lazy, and manual desktop-route re-selection is kept separate from *automatic* post-fault recovery accounting. [Source `fe1d16b`](https://github.com/sguzman/starbyte/commit/fe1d16bf97d664af454c95ca4c878ff88adba5da) passed [all four CI jobs in run 40055135458](https://github.com/sguzman/starbyte/actions/runs/40055135458), including native virtual-Wayland smoke. Actual CPAL callback behavior across physical unplug/replug, speaker audibility and perceived sound continuity remain untested; **scheduler 69/77 (90%)** remains the accepted checkpoint.

## Live mixer volume and desktop route recovery

The Rodio audio worker now applies volume changes to its **existing sink** even when emulation is paused and no new PCM arrives. Muting still clears queued game audio, but repeated already-muted updates do not re-clear the sink. The Sound panel adds **Reselect desktop audio output**: it releases a stale Rodio/CPAL route when changing the default OS output fails to trigger a stream-error callback, without altering the current game or save state. The next fresh game PCM or in-game speaker check attempts to open the current default output. The UI distinguishes an accepted reset request from a successful device reopen; manually selecting a route does not invent a CPAL failure or automatic recovery.

[`f72b672`](https://github.com/sguzman/starbyte/commit/f72b67215de38727e029d02f4da04bc5e2bc5134) is covered by [CI 40054055103](https://github.com/sguzman/starbyte/actions/runs/40054055103) (software and virtual compositor only). Real speaker audibility, actual cable/device unplug/replug and listening quality remain untested here. **Scheduler acceptance remains 69/77 (90%).**

## Session-panel owner-clock diagnostic receipts

The session diagnostics panel in the opt-in worker-backed original window now shows **suspension** and **resumption** transition totals recorded by the independent worker, not UI paint counts. This keeps the existing small-window performance overlay unchanged while making hardware QA observations inspectable within the application. [`d1b950f`](https://github.com/sguzman/starbyte/commit/d1b950f06150d3ee97a1d2f96964e31f06f6e3fb) is checked by [CI 40052422079](https://github.com/sguzman/starbyte/actions/runs/40052422079). Real speaker/gamepad/workspace execution is still pending; the migration remains at 69/77 accepted.

## Owner clock transition receipts and physical background-pause QA

The emulation scheduler now records run-to-suspended and suspended-to-running transitions on the **exclusive worker thread**, publishing these alongside its effective running state. The original-window no-redraw smoke rejects even brief unexpected suspensions when background play is enabled. The opt-in `starbyte-native-qa --physical-workspace` mode additionally includes a **separate real focus-loss pause scenario**, driven by the compositor rather than fabricated focus flags. A passing local result must show an actual suspend transition, substantially reduced frame advance while away, a real resume transition, and no instantaneous burst of ten frames upon return. [`e9c0cc5`](https://github.com/sguzman/starbyte/commit/e9c0cc5d7a36e5484949620e52c018361d1daee7) contains the owner instrumentation and CLI mode, with exact-revision CI checked through [run 40052095971](https://github.com/sguzman/starbyte/actions/runs/40052095971). **No physical Hyprland run has been observed in this GitHub-only session, and no checklist acceptance status has changed: 69/77 (90%).**

## Native Hyprland QA adjunct and real-workspace option (2026-10-23)

The standalone Rust `starbyte-native-qa` binary runs nine original-window demo worker checks using disposable **HOME** and XDG directories; the child cannot fall back to X11. The user's ROM collection, live saves and settings are not used by the synthetic checks. A separate opt-in `--physical-workspace` scenario uses Hyprland's current Lua dispatcher to verify the Starbyte PID belongs to a native Wayland client, switch away from the original numeric workspace during a worker-only gap, then return. It writes a minimal local `workspace-transition.json` receipt, and restores the original workspace on normal error paths. [`9cab61a`](https://github.com/sguzman/starbyte/commit/9cab61a569698b1e98d9c1946f36800f76d35dc5) is validated by [CI 40048375872](https://github.com/sguzman/starbyte/actions/runs/40048375872). The code has **not** been executed inside the user's actual compositor here; even a local pass cannot establish real speaker audibility, gamepad behavior or production cutover. See [native acceptance procedure](qa-native-hyprland.md). **69/77 accepted (90%); worker preview remains opt-in.**

## Stream fault/recovery diagnostics (2026-10-23)

The dedicated Rodio/CPAL audio worker now counts detected post-open stream faults separately from successful recovery opens. A new output attempt is not treated as recovered until opening succeeds; callbacks for alternate output candidates are isolated, preventing false failures from a rejected device. Counters are visible in the original Sound panel for worker and legacy audio. [`ae9ce76`](https://github.com/sguzman/starbyte/commit/ae9ce76602758eac3e2abbfea1abf081adc53af5) is checked by [CI 40045761526](https://github.com/sguzman/starbyte/actions/runs/40045761526), with device-free tests. Actual USB/audio-route disconnect/reconnect, PipeWire behavior, speaker audibility and gamepad evidence are still unverified; no acceptance task can be completed from these counters alone. **69/77 accepted; worker preview still opt-in.**

## Asynchronous ROM feedback and stream recovery (2026-10-23)

The opt-in original-window worker now carries request identities through save and screenshot completions so older acknowledgements do not replace newer ROM-loading status. Screenshot pixels already captured before a successful replacement are still written to disk rather than discarded. [`baf5a02`](https://github.com/sguzman/starbyte/commit/baf5a025a40ce8bf70c151819aba22decda0bac9) passed [all four CI jobs](https://github.com/sguzman/starbyte/actions/runs/40043507022), including native virtual Weston.

Rodio/CPAL output-stream errors after initial device open now set an atomic fault flag. The dedicated audio worker owns device disposal/reopening and resets PCM/stretch continuity; the audio callback and egui remain nonblocking. [`7839b30`](https://github.com/sguzman/starbyte/commit/7839b30b2440f4189cd0c4929bd89f30b41dca20) includes a synthetic device-disconnect callback test. Passing host CI does **not** establish real USB/headphone unplug/replug recovery, PipeWire routing, actual speaker audibility or background audio quality. Those acceptance gates remain open in [the native QA procedure](qa-native-hyprland.md). Source milestone counts remain **69/77 (90%)** and production defaults remain unchanged.

## Worker-owned original-window scheduler checkpoint (2026-10-23)

The original Starbyte library window now supports an **opt-in** exclusive emulation worker via `--native-worker-preview`; no redundant synchronous `FrontendSession` exists in that mode. Hosted Linux/Weston, Windows, Clippy and pinned Rust 1.88 CI at [`a35ced5`](https://github.com/sguzman/starbyte/commit/a35ced5ddd05e6f4908967ebe4b322bbf6b42ef6) includes direct owner pacing without egui redraws, cartridge-replacement and save receipt ordering, original-window menus/shortcuts, native virtual compositor checks and stale speaker-test receipt fencing. A pending speaker-test acknowledgement cannot replace status from a newer ROM request or installation. **The code CI must be confirmed green at its exact run before reporting all jobs verified.**

Source checklist: **69/77 accepted (90%)**, with eight separate physical Hyprland, audible-device/gamepad and eventual production-default gates. They are documented in [scheduler-migration.md](scheduler-migration.md), the [machine-readable ledger](scheduler-progress.json), and [native Hyprland acceptance procedure](qa-native-hyprland.md). Virtual Weston does not prove actual off-workspace GPU presentation, speaker audibility or gamepad behavior. The legacy player remains the production default.

## Selectable playback speed and Tab fast-forward (2026-10-13)

Desktop speed preferences now persist in `RuntimeConfig.video`: selectable normal speed 50%, 75%, 100%, 125%, 150%, 200%, 300%; held-Tab temporary boost 200%, 300% or 400%. Tab does not activate while a text field or keybinding capture has keyboard focus. Each speed gets an actual NTSC-relative frame deadline and target FPS display. Multiple frames may be emulated on a single GUI update when needed; regular playback limits catch-up to one extra frame and oversized stalls resync. An egui repaint is requested a small amount before the deadline. Audio is cleared on speed/pitch transitions so old-rate PCM cannot play into the new selection. The Speed menu now independently controls audio pitch: **Preserve original pitch** (default, using an in-tree pure-Rust streaming WSOLA implementation), **Follow game speed** (classic pitch-linked varispeed), and **Custom pitch** (50–200% independent of tempo). The worker receives both tempo and pitch and runs WSOLA at tempo/pitch followed by Rodio output speed pitch; the 440 Hz diagnostic tone stays at 1x. Generated-tone Linux/Windows unit tests check duration and near-original pitch across slow/fast speeds without physical audio hardware. **Audio fidelity, transient artifacts, extreme speed behavior, and the existing native stutter still require real-machine QA.** Existing saves and gamepad control pathways remain unchanged. Native performance, speed UX, and full-speed audio resumption remain acceptance gates; synthetic timing/config tests are not evidence of successful native gameplay at modified rates.

## Commercial game compatibility acceptance (2026-10-13)

The user reports that **some other games do not work at all**. This is an important unresolved compatibility class, not a claim that any specific unprovided title fails for a particular reason. Existing commercial-ROM bring-up and native play have been heavily centered on Super Mario World, while synthetic subsystem tests and generic GUI smoke cannot establish cross-title playability. Need titles and observed boot/gameplay failures to reproduce, classify CPU/PPU/APU/coproc issues, and add separate local-only reproducible regression fixtures. Do not claim broad compatibility.

## Native idle-game FPS and audio stutter (2026-10-13)

The user confirmed Starbyte now **plays Super Mario World audio automatically** after the sound-default change, but also observed intermittent idle overworld dips to approximately **52.6 / 60.1 emulated FPS**, with simultaneous audio lag. Later captured idle-game timing: 3.9 ms emulation average / 6.5 ms peak (0 slow emulation frames); UI wake 1.2 ms last / 31.2 ms peak (53 late wakeups); peak frame gap 35.3 ms; 53 possible mixer-empty-on-PCM observations (cumulative, not proof of individual audible underruns). This is a real native report, not a synthetic benchmark. Root cause remains unresolved: the core may be over budget, the egui frame wake may be delayed, texture submission may spike, or audio playback may starve as a consequence. Do not assume the audio worker or host scheduling is the cause without measurements.

The guarded CPU loop now checks its 1.5-second wall-clock timeout once every 256 instructions instead of twice per CPU instruction, retaining the existing instruction-count guard, zero-duration error behavior, and a bounded deadline check interval. This removes redundant OS timer polling but is not proof of improved native FPS. The GUI timing overlay now shows its most recent frame gap, and rate-limited warning logs include emulated FPS, frame gaps, emulation milliseconds, UI wake lateness, texture submission cost, producer/worker audio queue drops, and empty-sink observations when a pacing gap exceeds 20 ms or rolling FPS drops below 55. These can distinguish likely causes in the next native run. CI and measured native results must still be separated.

## Searchable and sortable Cheatarium play menu (2026-10-13)

The in-game Cheats menu has case-insensitive, multiword search across name, code, type and source, plus sort by name/type/source/code/enabled status, ascending/descending direction, and All/Enabled/Usable/Unsupported filters. Results show total and enabled counts, and virtualized scrolling avoids materializing large offscreen lists. Selecting a checkbox still uses the original enabled-by-game persistence and live emulator patch sync. No new cheat codes are automatically activated. The new controls and search behavior have deterministic unit coverage; physical GUI acceptance is pending.

## Desktop sound enabled by default (2026-10-13)

Ordinary desktop game loads now select the sound-enabled Persistent policy, including startup ROM, library Play, and direct Open ROM. This keeps writable battery SRAM and isolates sound-mode save-state slots; it does not silently switch to the read-only diagnostic. Silent compatibility remains available as a manual fallback from Sound, and the explicit experimental sound-driver CLI flag remains read-only. Added a desktop-path regression to verify default driver activation and fallback. This policy change still requires CI confirmation and native QA for stuttering; it does not establish broad sound accuracy.

## Audio queue loss visibility (2026-10-13)

The dedicated Rodio worker retains a bounded, nonblocking PCM queue. The Sound menu now exposes **full-queue producer drops** and **stale/expired worker drops** separately from generated nonzero PCM, OS device-open status, and successfully submitted mixer buffers. A producer drop means the UI-to-audio queue reached its capacity; a worker drop means an outdated generation/late frame or deliberate diagnostic tone exclusion prevented playback. These are direct queue accounting counters, not proof of a physical underrun or audio quality. A device-free regression uses a deliberately full bounded channel to verify producer accounting. Cross-correlate these counts with the independent emulation, UI-wake and texture timing counters when investigating intermittent sound stutter.

## Sound with normal save persistence (2026-10-13)

The frontend now distinguishes three sound/save policies: **Compatibility** (original silent gameplay with existing SRAM and disk slots), **Isolated** (explicit sound-driver test with read-only SRAM and no disk slots), and **Persistent** (explicit sound-enabled preview with battery SRAM persistence and a separate disk-slot namespace). Sound execution is no longer intrinsically tied to disabling saves. Existing ordinary disk slots retain their `<rom-id>.slotN.state.json` filenames; sound-mode slots are stored as `<rom-id>.sound.slotN.state.json`. Mode changes flush pending writable SRAM, and switching into a read-only diagnostic imports the last saved SRAM without ever writing the diagnostic's mutations.

The Sound menu exposes Sound (on) for the default desktop session and keeps silent compatibility as an explicit fallback. Sound-with-saves has automated regression coverage, but audible artifacts, game compatibility, slow frames, and physical save-state restore still need broader native acceptance. Synthetic regression tests check SRAM updates, reloading under compatibility mode, preserving legacy disk-slot bytes, saving/loading distinct sound-mode slots, and avoiding writes from isolated mode. Validate the final Linux/Windows/lint run against the exact implementation revision.

## Native audible Super Mario World sound confirmed (2026-10-13)

User independently confirmed that **actual Super Mario World game sound is audible** on their native EndeavourOS/Hyprland desktop after selecting **Restart isolated with sound**. This is the first user-verified physical listening result; previous synthetic PCM/mixer probes proved only software behavior. The user additionally reported **a little intermittent lag with audio enabled**, compared to essentially smooth gameplay near 60.1 FPS in the normal, intentionally silent compatibility mode. The difference suggests sound CPU/DSP execution overhead, but no quantitative mode-to-mode timing comparison has yet been captured. Keep both the verified audible outcome and the measured-performance uncertainty explicit.

**Product direction:** desktop sound now runs by default without experimental confirmation or read-only save isolation. Battery SRAM continues to persist and sound-mode state slots remain separate from legacy compatibility snapshots; silent fallback remains available for unsupported titles. Native QA is still needed to characterize stutter and quality before broader compatibility claims.

A conservative sound scheduling optimization is now under CI: 64-bit checked fractional clock arithmetic on the frequent six-master-clock SPC700 and DSP paths, with an exact 128-bit fallback for artificially large advances. The arithmetic remains bit-identical including remainder and overflow scenarios; it is not yet evidence that the user's lag is resolved. The latest known implementation commit is `2ef408ea` (verify CI on that exact revision).

## Live shared-worker audio output check (2026-10-13)

The Sound menu now offers **Play 440 Hz through game audio worker**. This 250 ms original, low-amplitude stereo sine burst takes the exact same live `AudioOutput`, Rodio default stream, mixer, sink and volume settings used for experimental game PCM. The worker opens its own output device if needed, briefly clears stale game audio before the burst, suppresses incoming game audio for 300 ms so queue trimming cannot cut off the tone, then resumes normal PCM playback; video, input and emulation are not paused. The UI shows a dedicated `Live-worker test tones submitted to mixer` counter. Test-tone submissions are **excluded** from both ordinary and nonzero **game PCM** counters so a synthetic speaker test cannot be mistaken for a successful SPC700 sound-driver result. Unlike ordinary game PCM, a requested diagnostic tone remains eligible after a slow OS-device open or a backed-up queue.

The independent 440 Hz probe remains available as a *separate* OS stream check. A separate-stream tone audible while live-worker tone fails suggests worker/queue routing. If both test tones are silent despite reported stream opening, inspect OS routing/volume. If live-worker tone is audible while experimental game PCM remains zero, investigate SNES game-driver upload/DSP. If live-worker tone is audible and nonzero game PCM reaches mixer but music is silent or distorted, investigate PCM amplitude/shape and audio routing before changing the emulated CPU. These are diagnostic possibilities, **not** claims of physical audibility. Unit regressions verify same-worker queue submission, bounded tone waveform and distinct counter accounting without requiring live speakers; native acceptance still requires human listening.

## Native silent-gameplay report and sound pipeline diagnostics (2026-10-13)

The player rebuilt the latest release and confirmed native Super Mario World remains responsive on the overworld, with **59.6 / 60.1 emulated FPS** visible in Play View and **no noticeable lag** in that session. They reported **no audible audio**. The screenshot's `Audio samples: 1066` is an unconditional **sample count**, not evidence of any nonzero PCM or speaker output. Crucially, the captured Play View did not expose which APU mode was running. The normal compatibility bootstrap is intentionally nonexecuting/silent, so a successful release rebuild does not by itself make game audio available. Do not infer the user activated isolated audio without evidence.

To prevent further blind sound tests, the GUI now displays `Sound: SILENT (compatibility)` in the in-game status bar when the ordinary shim is selected, and labels the menu `Sound (silent)`. In explicitly opt-in mode, it displays whether output is disabled/muted, whether **any nonzero DSP PCM has been generated**, whether the default OS sound device opened or failed, and whether the background worker actually **submitted** any fresh buffers to Rodio's mixer. The Sound menu reports lifetime-to-session nonzero PCM pairs, last-frame nonzero pairs, device-open status and mixer-submitted frame count. The worker submission counter advances only after a nonstale buffer is appended; synthesizing PCM or successfully opening an OS device is not itself counted as submission. Counters and worker state are reset on ROM change or sound-mode restart so stale data from prior games cannot masquerade as success. Even mixer submission does not prove that the intended physical speaker played music. Deterministic tests verify the distinct audio-stage labels without requiring real hardware.

**Remaining acceptance:** opt into the isolated game sound-driver mode (without writing battery SRAM) and distinguish `no SPC program` / `all-zero DSP output` / `device failed` / `no mixer submission` from `submitted but inaudible`. Only then prioritize SNES timing or Linux output routing accordingly. Do not promote experimental sound to default, or certify it audible, on synthetic CI results alone.

## User gameplay feedback and rolling FPS instrumentation (2026-10-12; CI pending)

After playing Super Mario World again on the native Hyprland build, the user reported **fewer heavy lag spikes**, with occasional stutter remaining; locally enabled cheats were remembered and applied on restart. Game audio was still not audible. This is user-reported progress, not a measured framerate, a resolved performance regression, or confirmation of commercial-game sound.

Starbyte now measures **completed emulated frames per wall-clock second**, averaged over a rolling two-second window. The bottom status bar reports actual emulation FPS against the ~60.1 Hz NTSC target without requiring a diagnostic overlay. The optional Play View **Timing** overlay and Session panel report frame-completion gaps >=40 ms and peak frame gap alongside already separate core, scheduling and CPU texture-submit timings. The measurement deliberately does not claim GPU-present FPS, and it resets on pause/resume, new ROMs, isolated audio restarts and save-state loads. Deterministic tests cover stable cadence, hitch detection, stale samples and reset behavior. **Native timing improvement and actual sound remain unverified; full CI is pending.**

## Isolated game audio: read-only progress and opted-in DSP key-on timing (2026-10-13)

To let a later single native sound acceptance test start from the player's actual battery-backed progress, the **explicitly opted-in** sound-driver mode now imports a private copy of the latest on-disk SRAM. The frontend flushes any pending normal-play SRAM before the copy, then forbids disk writes and persistent-slot access throughout the isolated test; all test-mode RAM changes remain in memory. Returning to normal mode reloads normal saved progress. Synthetic frontend regressions cover reading existing progress and preventing modified probe RAM from overwriting the disk or slot files. This does **not** enable sound execution in ordinary gameplay.

The experimental sound-driver mode now opts the DSP into the S-DSP's five empty 32 kHz sample periods after a voice's KON. The pending voice does not advance its source pitch/envelope or signal ENDX during that startup interval, and the delay survives serialization. The historical immediate-start behavior is retained for default synthetic DSP fixtures and ordinary compatibility mode to avoid unrelated regressions. Held KOFF/FLG reset can cancel a pending note. This is a bounded accuracy improvement, **not** a complete 32-phase DSP pipeline or a proof of correct commercial-game music. Hardware reference: https://patrickjohnston.org/ASM/ROM%20data/snestek.htm . Linux/Windows/Clippy acceptance is checked separately against the final implementation commit, not against earlier formatting/test-failing intermediate commits.

## Speaker-test feedback and experimental SPC700 visibility (2026-10-12; CI pending)

The in-game Sound menu now receives the **result** of its asynchronous, independent 440 Hz speaker probe rather than reporting only thread launch. The probe button is disabled while the test runs, then displays either a device/playback initialization error or that the test finished without an OS error. This cannot establish what the person actually heard, nor the fidelity of game-generated PCM. It does not enable experimental sound drivers, access saved games, or block the GUI on a device open. Deterministic tests cover pending, successful, failed and disconnected probe notifications without requiring audio hardware.

In explicit isolated-game-audio mode only, the Sound menu additionally displays uploaded sound-program bytes and real SPC700 instruction count, PC and halt state, or a message that the ROM has not handed off a driver. Normal-mode silence remains expected until commercial sound-driver execution is graduated safely. Native audible result and stutter reduction remain **unverified**, as tracked by [Issue #3](https://github.com/sguzman/starbyte/issues/3).

## Offline Cheatarium consumption and single-code SNES Game Genie (2026-10-12; latest CI pending)

The desktop's Settings → Cheats can persist an optional user-selected local Cheatarium checkout in `[cheats].cheatarium_root`. Refresh Cheats now reads `generated/v1/snes.json.gz` from that checkout without downloading codes, and the CLI's read-only title search can use the same configured source without repeatedly passing `--index`. Title matching supplies **unverified candidates**, not identified ROM revisions. Source descriptions, formats and attribution remain visible. Original Cheatarium files and cartridges are never rewritten.

The core's new opt-in cartridge-ROM read substitution maps a single valid SNES Game Genie code to a full 24-bit bus address/replacement byte and changes the result **only on matching cartridge reads**, not WRAM, MMIO, or SRAM. User toggling immediately enables/disables the replacement; host-managed cheats are not serialized into guest save states, and are reapplied after state loads. Known Cheatarium decoder vectors and synthetic bus/65816 runs exercise the execution path. Simple individual 7E/7F WRAM codes remain supported. Unknown composition joins, other formats and ROM build ambiguity still prevent automatic activation; successful effects in a commercial ROM have not been demonstrated. The earlier implementation reached a full green Linux/Windows/lint CI at `58f4d3b`; later source/test refinements require checking their exact runs before claiming full verification.

## Native gamepad mapping feedback (2026-10-12; CI pending)

The existing controller rebinding UI now shows names of controllers enumerated by gilrs and live held-button names in Settings → Input. This helps verify an already USB-recognized pad within Starbyte without relying on a browser gamepad tester. These are **source/UI changes only**; physical PXN button mapping, focus, and sustained gameplay acceptance remain untested.

## Live speaker volume and mute controls (2026-10-12; CI pending)

The desktop's previously inert Audio preferences are now connected to Rodio's real mixer through a nonblocking atomic gain value. **Sound enabled**, in-session **Mute**, and the persisted 0–100% **Volume** slider are accessible in the game toolbar's **Sound** menu and the library Audio settings. Disabling or muting immediately clears the queued device buffers; restoring audible output resumes on newly generated PCM without restarting or mutating emulated sound CPU/DSP state. The persisted `mute_on_startup` setting seeds only the initial session mute flag and is not overwritten by toggling live mute. The obsolete editable `sample_rate_hz` control was removed from the GUI because Starbyte's DSP always emits 32 kHz stereo PCM and the operating system selects device playback rate. Original sound generation and ROM-free diagnostic metrics remain unaffected by mixer settings. Audio-device-open success is not proof of audible or accurate game music.

## Real user gameplay acceptance and remaining sound/stutter work (2026-10-13)

The user played **Super Mario World (Europe) (Rev 1)** on Hyprland using their USB PXN P50L generic pad and custom Starbyte mapping. The in-game Cheatarium menu found 748 title candidates. The user observed functional 99 coins and 99 lives codes, and SRAM-backed progress survived reloading. This verifies **those concrete combinations**, not every controller edge case or all indexed cheats. The same user reported intermittent gameplay lag and no audible game sound.

Source audit explains the silence: regular cartridge startup intentionally keeps the SPC700 sound CPU on a nonexecuting placeholder, so real commercial-game DSP driver uploads are never run. The existing Rodio/CPAL output mixer, volume controls, and previously green 500-frame *explicitly isolated* sound probe do not amount to working normal gameplay audio. The opt-in probe produced real nonzero samples and opened the host sound device; **audible playback has not yet been confirmed**.

The Sound menu now reports normal versus isolated mode, per-frame DSP nonzero sample counts, and device-open status. A synthetic 440 Hz speaker test is available separately. The opt-in **Try experimental game audio** menu requires an explicit confirmation, restarts the currently loaded game in isolated mode, leaves the user's original SRAM and durable slots untouched and disables their use during the test; a return button reloads normal gameplay and saved progress. It is not enabled by default, and may stutter or fail. Native testing is pending.

The Timing overlay now separates core emulation milliseconds from late UI wakeups and CPU texture submission, enabling bottleneck discrimination during the user's reported stutter. Redundant per-update `egui::Context::set_visuals` calls were removed as a UI-side overhead reduction; **no real stutter fix has yet been verified**. Remaining acceptance is tracked in [Issue #3](https://github.com/sguzman/starbyte/issues/3).

## First physical Hyprland Super Mario World GUI success (2026-10-12)

A local **500-frame** opt-in GUI smoke at commit `bafbd83` successfully opened an actual Starbyte native window under the user's Hyprland session and automatically exited with code **0** at the requested frame limit. The user directly reported seeing gameplay on screen for several seconds; the log contains `STARBYTE_NATIVE_SMOKE_OK frame=500`, not a crash. The 500-frame duration intentionally bounds the test and must not be confused with an unexpected shutdown.

The native sound metrics were identical to the earlier 500-frame headless SMW probe: **266,229 stereo sample pairs**, **131,848 nonzero pairs**, peak magnitude **11,743**, first sound frame **71**, **2,177,389 SPC700 instructions** and **11,974 uploaded bytes**. The background Rodio/CPAL worker reported `audio_device_status=opened` and `audio_worker_spawned=true`, establishing that the default OS audio output device was successfully initialized, **not** that audible music was heard. The native gamepad subsystem enumerated a **Nintendo Pro Controller**, but no physical button input was tested. The library scan independently found 675 ROM entries with two rejected files; neither interrupted play. The egui-winit clipboard emitted a nonfatal X11-backend timeout warning in this Wayland-only invocation, but the GUI continued normally.

The original checkout and modified lockfile were preserved by the temporary detached worktree. No copyrighted ROM, generated audio, personal filesystem path or private log was committed. Remaining native acceptance gates are audible speaker output, longer open-ended play, controller mapping, fullscreen/focus and pacing on real hardware.

## Background OS audio-device feedback (2026-10-12; CI pending)

The speaker worker now publishes a read-only, atomic last-open result to the native GUI smoke report: `opened`, `open_failed`, or `not_attempted` (with `worker_not_started` if playback never needed a worker). It reports successful initialization of the default Rodio/CPAL output separately from nonzero DSP PCM and from worker-thread creation. An unavailable device continues to be nonfatal and is retried under the existing cooldown. The status neither certifies audible output nor measures latency, dropout or physical Hyprland playback. A device-free unit test exercises initial, failure and success visibility.

## Bounded native sound acceptance metrics (2026-10-12; CI pending)

The opt-in `--exit-after-frames N` GUI smoke now prints cumulative stereo pairs, nonzero pairs, absolute peak, first nonzero frame, uploaded SPC bytes and dispatched SPC700 instructions alongside its completion marker. The initial startup frame is included, so a 500-frame bounded graphical probe can be compared directly with the successful 500-frame headless SMW sound diagnostic. A separate field reports whether the asynchronous audio worker was spawned, **not** whether a physical speaker device opened. Synthetic tests verify silence followed by stereo activity, peak saturation-safe accounting, and first-sound-frame retention. Native Hyprland/speaker behavior remains untested.

## Automated native Wayland smoke (2026-10-12; CI pending)

A bounded `starbyte-egui --demo --exit-after-frames 30` probe now requests a real native egui window under a **Weston headless Wayland compositor** and closes cleanly after 30 completed SNES frames. The Linux CI job builds the GUI and runs this probe with Mesa software rendering, no X11 DISPLAY and an isolated temporary config/cache, rejecting launch failures, hangs, early emulator errors and missing completion markers. The same frame limit can be paired with an explicit user-owned `--rom` and `--experimental-sound-driver` for later native acceptance; it is never persisted or automatically enabled.

**Boundary:** virtual Weston checks native Wayland window startup and frame progression, not the user's real Hyprland session, physical controllers, full-screen/tiled behavior, live speakers, or SMW sound fidelity. The new CI job has not yet completed.

## First commercial-ROM-generated sound (2026-10-12; local acceptance probe)

A user-provided local, no-save-RAM **Super Mario World** ROM completed all **500 of 500 headless frames** at commit `18cc86b`, with `--apu-upload-probe --apu-execute-driver --audio-wav` explicitly enabled. The attached private diagnostics contain 500 `starbyte.frame_log.v1` records, a completed `starbyte.run_report.v1` and a valid 32 kHz, 16-bit, stereo WAV of 266,229 sample pairs (8.320 seconds). No proprietary ROM bytes, game audio or private filesystem paths are committed.

The real 65816 mailbox sequence transferred **11,974 bytes in three SPC blocks** and selected sound-CPU entrypoint **$0500** by frame 8. The uploaded SPC700 program actually ran **2,177,389 instructions** and **8,398,177 cycles** by frame 500, with the sound engine setting DSP volume, directory, key-on/off and envelope-related registers. Its DSP produced **131,848 nonzero stereo PCM pairs** across the run, first on **frame 71**; peak absolute PCM was **11,743** (well below the signed 16-bit limit). Both stereo channels contain signal and are not bit-identical. The final frame happens to be silent, so `apu_io_activity.isolated_ipl_upload.audio_frame.nonzero_pairs = 0` is compatible with the cumulative `audio_run.nonzero_pairs = 131848`.

**Evidence boundary:** this is the first observed **real SMW sound-driver execution and generated audio**, not validated song identity, instrument accuracy, timing, speaker playback or sound quality. The 500-frame trace shows one initial **three-block upload and one handoff**, **not** a second post-re-entry IPL transfer; the repeat-upload feature still has only synthetic tests. Production playback and ordinary GUI sessions still use the compatibility bootstrap and do not enable this sound driver by default. The next priorities are a listening/quality comparison of the actual WAV, a longer bounded input-driven run if needed, and native Linux/Wayland speaker acceptance before promoting opt-in commercial sound playback.

## 65816-to-SPC700 second-upload integration (October 12; CI pending)

An original synthetic 65816 LoROM now drives a complete two-stage upload through actual $2140-$2143 MMIO: it transfers an SPC700 driver which jumps to the IPL's $FFC0 entry, observes the renewed $AA/$BB handshake, and uploads a second SPC700 driver into the same target address. The test checks the handshake from a 65816 bus read, resumed execution and mailbox output from the second program, preserved unrelated audio RAM, and deterministic save-state recovery between the two uploads. This exercises SystemBus, CPU instruction dispatch, opt-in host-clocked SPC700 and IPL capture together, rather than directly writing APU ports in the test. No commercial ROM or IPL firmware is shipped or used. Commercial Super Mario World audio remains unverified.

## Repeat IPL upload after sound-driver re-entry (October 12; CI pending)

The opt-in sound-CPU executor can now recognize an uploaded driver's jump back to the IPL bootstrap at $FFC0 or $FFC9 if the boot overlay is active and no user-supplied IPL firmware is installed. It re-arms the byte-transfer decoder and publishes the $AA/$BB ready handshake so the host CPU can upload another SPC driver or BRR/music bank in the **same cartridge session**. The $FFC0 path clears only SPC zero page $00-$EF and restores the IPL stack/start registers, preserving previously uploaded BRR samples and DSP state; $FFC9 preserves existing zero-page contents. Both host-clocked and standalone runners stop at re-entry rather than erroneously executing ARAM filler as firmware or spending a backlog. Explicit diagnostic opt-in remains in effect; the production bootstrap is unchanged. Original synthetic tests cover a second post-reentry upload, save/restore between transfers, $FFC9 preservation and the disabled-overlay case. This closes a sound initialization blocker visible in the [Super Mario World SPC uploader disassembly](https://github.com/IsoFrieze/SMWDisX/blob/master/bank_00.asm), but real game-audio acceptance and actual Linux speaker output remain unverified.

## Transient gamepad press delivery (October 12; CI pending)

The native gilrs controller poller now retains each physical button press across intermediate UI repaint updates until an actual emulated frame samples the controller. This prevents a press+release in a single poll, or between two SNES frames, from disappearing because only the final held-button set remained. The queue is scoped to each connected physical gamepad, is consumed once per emulated frame, is cleared when paused and is removed for the disconnected controller without disturbing another pad. Rebinding a gamepad button does not queue that same press as gameplay. Tests check transient release, one-time consumption and isolated disconnect. Native hardware validation is still outstanding.

## Queued keyboard taps across non-emulation repaint updates (October 12; CI pending)

The frontend now holds keyboard press edges across egui repaint cycles until the next actual emulated frame, consuming each short press exactly once after submitting the SNES controller state. The earlier same-update `key_pressed` check missed a tap if no emulator frame was due in that update. Held keys continue to work normally, while text focus, key rebinding, pause and unloaded sessions discard pending taps. This is a desktop input fix with synthetic egui regressions for no-frame repaint persistence, one-time consumption and suppression. Native hardware latency remains unmeasured.

## Short keyboard taps captured for gameplay (October 12; CI pending)

Keyboard SNES button mapping now accepts either the current held state or the key-press edge from the current egui input frame. This prevents a short press+release, entirely between two 60 Hz emulation updates, from disappearing. It does not latch buttons across multiple emulated frames or affect typing-focused controls, which remain gated by input source selection. An egui raw-input regression supplies key-down and key-up events in one frame and checks that the next frame no longer sees that press. Native Wayland latency is still unmeasured.

## Automatic input and text-field isolation (October 12; CI pending)

The native frontend's Auto keyboard+gamepad mode no longer discards all game input when a search or settings text field has focus. Keyboard-to-SNES controls are suppressed while typing or recording a new key binding, but physical gamepads remain enabled and their held buttons remain visible to the emulated controller. Keyboard-only mode continues to suppress in-game controls while editing text, and Gamepad-only mode is unchanged. A focused source-selection regression covers all three modes, keyboard focus, and rebinding. Native Wayland/gamepad acceptance remains open.

## BRR ENDX decode timing (October 12; CI pending)

S-DSP `ENDX` now latches when a voice begins decoding an end-marked BRR block, not only when the last sample of that block has finished playback. This matters to sound drivers that poll voice-end flags before choosing the next note or effect. The first keyed block raises its pending end event on the following DSP tick (KON still clears the old flag), while a later block can raise the event on the decode transition. The end/loop and PCM progression stay unchanged. Targeted tests cover one-shot and looping samples, ENDX acknowledgement, a later end-marked block and save-state continuation. Hardware reference: [fullsnes S-DSP ENDX behavior](https://patrickjohnston.org/ASM/ROM%20data/snestek.htm). This remains sample-stepped rather than a cycle-exact DSP pipeline; no commercial audio claim is made.

## IPL boot-pointer memory fidelity (October 12; CI pending)

The opt-in IPL transfer decoder now mirrors each accepted block/entry address into SPC direct-page $00/$01, as the hardware boot loader's `MOVW $00,YA` does. At each 256-byte transfer-index rollover it also increments the high byte of that live pointer. These bytes are observable by an uploaded sound driver after handoff; previously they remained stale even when the decoder had accepted the correct address internally. TEST RAM-write protection still gates these pointer writes. Tests cover initial transfer, index wrap, block replacement, entrypoint handoff, zero-marker fall-through and actual APU mailbox traffic. The legacy production sound bootstrap is unchanged. Commercial-ROM upload/audio acceptance is still unverified.

## New-cartridge sound-mode isolation (October 11; CI pending)

Loading any new cartridge now explicitly disarms the previous isolated IPL transfer decoder and host-clocked sound-CPU execution before emulator reset. A plain reset within the same diagnostic can still retain a deliberately armed decoder, but an ordinary library game can never inherit it from the previously loaded experimental game. A core regression checks both status flags and the restored production bootstrap after cartridge replacement; the frontend checks restoration of ordinary disk-backed SRAM access. This was identified by an independent failed CI test and fixed at the emulator mode boundary.

## Explicit, nonpersistent desktop SPC700 driver probe (October 11; CI pending)

The graphical frontend now accepts `--rom /path/to/game.sfc --experimental-sound-driver` as an explicit, single-startup-ROM probe. It arms the already available isolated IPL upload and host-clocked sound-driver execution immediately after loading that ROM, allowing generated PCM to reach the threaded speaker backend without enabling unverified sound in ordinary library play. Originally this isolated all disk save data including reads; the 2026-10-13 revision loads existing battery SRAM as a read-only copy while continuing to block on-disk save writes and persistent slot access. The opt-in flag is not stored in user configuration and is not inherited by later library-selected games. Regression tests verify opt-in-only CLI argument parsing, disabled durable persistence, untouched existing SRAM/state-slot files, and restoration of normal mode on ordinary ROM reload. This does not establish that a real commercial driver will upload, execute correctly, or sound right; an experimental game may stall.

## SPC700 standby bus poll fidelity (October 11; CI pending)

The independent reference records SLEEP (`EF`) and STOP (`FF`) as a three-cycle opcode followed by repeated two-cycle read/wait bus polls at the parked PC. Starbyte's idle step now performs the corresponding read/wait pair without decoding further instructions or moving the PC; the sound-CPU instruction counter remains unchanged during standby. The conformance harness gathers the two trailing idle polls and compares all seven recorded cycles. This corrects an actual bus trace discrepancy rather than simply waiving the failure, while maintaining the documented lack of automatic interrupt wakeup.

## Independent SPC700 single-step conformance fixtures (October 11; CI pending)

An offline integration regression now exercises 2128 CPU instructions against the MIT-licensed [SingleStepTests/spc700](https://github.com/SingleStepTests/spc700) v1 reference corpus. Twelve deterministic cases each were sampled from 20 opcodes `00`, `7A`, `9A`, `AF`, `C6`, `C7`, `D7`, `8F`, `4C`, `5C`, `E8`, `8D`, `CD`, `2F`, `F0`, `D0`, `3F`, `6F`, `CF` and `9E`, plus eight cases each from 61 additional opcode values in hex bank `01`–`3E` (excluding `2F`, already in the original group). Another eight cases each are included for 60 opcode values in hex bank `40`–`7F`, excluding existing `4C`, `5C`, `6F` and `7A` samples. Eight more cases each from 59 opcode values in hex bank `80`–`BF` (excluding `8D`, `8F`, `9A`, `9E` and `AF`) and eight cases from 56 additional opcode values in hex bank `C0`–`FF` (excluding already covered `C6`, `C7`, `CD`, `CF`, `D0`, `D7`, `E8`, `F0`) bring coverage to **all 256 SPC700 opcode values**. The harness compares starting registers/memory with expected final PC, A/X/Y, SP, PSW, RAM writes, cycle count, and specified bus read/write/wait addresses and values; unspecified bus fields remain wildcards rather than invented zeroes. The exact fixture and MIT copyright notice are committed under `crates/starbyte-core/tests/fixtures` and run in ordinary Linux/Windows workspace CI without network or private ROMs. This is a JSMoo-derived independent instruction-level oracle, **not** hardware-audio acceptance or evidence of authentic commercial sound-program execution.

## Terminal run reports for stalled IPL probes (October 11; CI pending)

A `starbyte run --report-json PATH` request now writes its final snapshot even if an emulated frame fails, including stalls on the **first requested frame**. The additive failure-only fields are `status: "failed"` and `failure.requested_frame`, `failure.completed_frames`, and `failure.error`. The original `frames` still means frames requested, and `frame_counter` records actual current emulated timing. CPU register state, IPL transfer counters, current SPC execution status and DSP registers reflect the failure point, while `audio_run` and `apu_upload_timeline` count only fully completed frames. The CLI still exits nonzero, and a report filesystem error does not replace the original emulation error. Normal successful JSON schema is unchanged. Synthetic tests cover a first-frame failure and the success-only absence of failure fields; a real commercial-ROM stall remains untested.

## Threaded, bounded desktop sound and reproducible audio dependency lock (October 11)

The native desktop sends synthesized stereo PCM over a bounded six-frame nonblocking channel to a dedicated audio thread. Default-device discovery, Rodio sample queueing and ten-second retry logic run outside the egui update loop, so a slow or unavailable speaker cannot synchronously block UI drawing. ROM changes, pause, state loads and emulation errors invalidate old queued generations; audio frames older than 150 ms are discarded instead of being played late. Unit tests cover generation invalidation, stale-buffer age, low-amplitude tone generation, sample conversion and retry deadlines without hardware. Cargo-generated `Cargo.lock` now includes Rodio 0.21.1, CPAL 0.16.0 and their dependencies with source checksums; CI is configured to reject future lockfile drift with `--locked`. Native sound-card playback and commercial-ROM audio are still unverified.

## Synthetic speaker probe and device recovery (October 11; verification pending)

The desktop binary now offers `--speaker-test`: it opens the default Rodio/CPAL device, plays a short original low-volume 440 Hz stereo sine tone, and exits without loading any ROM, IPL firmware, or UI. Stereo symmetry, bounded PCM amplitude, and generated sample length have pure unit regressions independent of hardware. If default audio initialization fails during actual play, the frontend now retries after a 10-second cooldown upon subsequent nonzero PCM rather than permanently disabling speakers until relaunch. Retry timing is unit tested and failures remain nonfatal. The probe has not yet been executed on target Linux hardware and cannot validate commercial sound-driver behavior. **Cargo.lock is now checked in with the exact Rodio/CPAL dependency graph generated by Cargo on the Linux CI runner.** Subsequent CI runs require `--locked` for compilation and testing; native Linux device playback remains unverified.

## Desktop audio output plumbing (October 11; CI and Linux speaker checks pending)

The native egui frontend now has a Rodio playback-only output backend for the core's 32 kHz interleaved signed stereo PCM. It is **lazy** (no audio device opened unless nonzero PCM is actually produced), **optional** (audio initialization failures log a warning rather than terminating the game), and **bounded** (stale queued frame buffers are skipped to avoid unbounded sound latency). Pausing, cartridge loads and state restores clear pending audio. PCM16-to-float stereo conversion has synthetic unit regressions; no copyrighted assets or ROM samples enter this repo. The dependency is restricted to the GUI binary; headless diagnostics and core do not require desktop speaker access. This does not switch on the experimental SPC700 driver in normal gameplay, so authentic Super Mario World sound remains unverified. Linux CI has compiled the audio backend and passed the Rust workspace tests at `00811a3`; latest-commit cross-platform CI remains to be confirmed. The final Rodio Cargo.lock resolution is committed, but target Linux speaker playback and real commercial game audio are not yet verified. Native Wayland and Linux sound-device availability are separate acceptance gates.

## Standby instruction accounting in isolated APU diagnostics (October 11; CI not yet confirmed)

The APU's direct SPC700 instruction API now distinguishes a dispatched `SLEEP` or `STOP` opcode from subsequent halted-idle bus polls. The opcode entering standby still increments the instruction count once; later synthetic idle cycles advance the SPC cycle counter but do **not** inflate `spc700_steps` or the opt-in `spc700_execution.instructions` diagnostic. A focused regression covers both halt modes, idle PC stability and clock accounting. This changes no production sound-driver scheduling or commercial-game audio behavior. Source is committed; latest-commit Linux/Windows tests and lint remain unconfirmed.

## Frame-level opt-in IPL progress timeline (October 11; CI not yet confirmed)

The headless CLI now retains first/last completed-frame milestones for accepted IPL bytes, bytes actually stored in SPC RAM, entrypoint handoff and genuine uploaded SPC700 instruction execution. The new `apu_upload_timeline` object appears in frame logs and successful final reports **only when the isolated IPL probe is enabled**. Each associated `frames_since_*` count helps reveal uploads or drivers that have ceased advancing without treating repeated polling as successful transfer or sound-CPU work. Null means that no matching progress has been observed in a completed frame. The counters do not expose ROM or sound-program contents, decide whether a game is hung, or alter emulator execution. Synthetic CLI regressions exercise accepted-but-not-stored transfers, handoff without driver execution, later idle frames and the opt-in-only reporting boundary. This feature has been committed to `main`, but no latest-commit Rust compilation, hosted CI pass or genuine commercial-ROM audio acceptance has been confirmed.

## Fresh isolated IPL upload timing (October 11; CI not yet confirmed)

Rearming an opt-in IPL upload on an existing APU instance now clears the **standalone SPC700-to-master-clock fractional remainder**, in addition to the already reset host-clocked credit and phase. This prevents a new diagnostic transfer and later sound-driver execution from inheriting fractional CPU time from an earlier independent probe. A synthetic SPC program is uploaded, handed off, executed for one NOP, and rearmed to assert cleared timing/entrypoint/transfer state. Ordinary production-mode gameplay is unaffected. The change has been committed without a new confirmed Linux/Windows CI result.

## SPC700 completion and audio-execution boundary (October 10)

The SPC700 interpreter now **recognizes all 256 opcode values**, including the previously missing memory INC/DEC, shifts and rotates, bit operations, compare/loop branches, word arithmetic, BRK/RETI, DIV edge behavior, decimal adjustment, and SLEEP/STOP standby states. The complete dispatch plus synthetic regressions passed hosted Linux/Windows tests and lint at [`bbcfde7e`](https://github.com/sguzman/starbyte/actions/runs/38066031908). This is **opcode coverage, not cycle-perfect accuracy**, and does not imply a commercial sound driver executes correctly.

A bounded **opt-in isolated** SPC700 program runner has also been added to the APU code. It requires a completed sound-program upload/entrypoint, executes at most the requested number of instructions, uses fractional SPC700-to-NTSC clock conversion, advances timers and synthesizes DSP samples into a local buffer. An original synthetic IPL fixture configures a BRR-playing DSP voice through uploaded SPC700 instructions. This path is **not switched on for commercial ROM emulation**; its own hosted regression result must be checked against the latest commit. The production APU still uses its bootstrap shortcut and placeholder clock, and **Super Mario World audio remains unverified/silent**. No Linux speaker backend is connected.

The **full synthetic CPU-to-audio path** now has an additional integration regression: an original 65816 ROM issues real `$2140-$2143` bus writes to upload three SPC blocks (BRR directory, waveform, sound-driver instructions), hands off execution and produces nonzero stereo samples through the partial DSP. Linux, Windows and lint passed in [run 420](https://github.com/sguzman/starbyte/actions/runs/38099827012). Another synthetic fixture verifies mailbox acknowledgements read back by the 65816 itself. This validates transport wiring; it does **not** verify copyrighted commercial sound-program timing or actual Linux speaker output.

The opt-in probe now separates **upload completed** from **sound program executed**: after handoff, `apu_io_activity.isolated_ipl_upload.spc700_execution` includes the SPC700 PC, actual instruction count, accumulated cycles, and halt state. In the ordinary upload probe, executed instruction count stays zero; no sound driver is automatically run. A regression also prevents pre-probe dummy cycles from inflating this counter. A complete synthetic 65816 upload was saved and restored **mid-transfer**, then resumed through the same multi-block handoff and DSP synthesis; generated audio and APU state matched the uninterrupted run. The most recent code milestone passed Linux tests, Windows tests, formatting, and Clippy in [run 437](https://github.com/sguzman/starbyte/actions/runs/38100405134). This still does not establish real Super Mario World audio or Linux speaker playback.

An explicit headless `run --apu-upload-probe --no-save-ram` diagnostic replaces the production bootstrap shim **only for that opt-in run** and exposes `apu_io_activity.isolated_ipl_upload` progress in frame/final JSON reports. It can cause real ROMs to stall and does not execute commercial audio automatically. The regular emulator and GUI keep the existing compatibility path by default; a separate [automation guide](automation.md) documents its behavior and limitations.

## DSP envelope and hardware noise (October 10)

The partial DSP now tracks the shared 32-rate envelope counter and has **timed ADSR attack, decay and sustain**, plus direct GAIN and all four timed GAIN modes (linear decrease, exponential decrease, linear increase, bent increase). Per-voice envelope phases and the shared clock survive save-state round trips. The **shared 15-bit noise LFSR** uses FLG's clock-rate selector and the per-voice NON bitmask; noise state also survives save states. Synthetic DSP regression tests and workspace lint passed on Linux and Windows in [run 440 for envelope groundwork](https://github.com/sguzman/starbyte/actions/runs/38101434488) and [run 442 for shared noise](https://github.com/sguzman/starbyte/actions/runs/38101509374). These implementations are not yet hardware bit-exact, and neither authentic Super Mario World sound-driver execution nor real Linux audio output is validated. Bit-exact echo/FIR timing, decoded-sample pipeline latency and other DSP fidelity gaps remain; sample-stepped Gaussian BRR interpolation has now been added.

## PMON, BRR filter limits and explicit WAV diagnostics (October 10)

The partial DSP now implements **PMON** pitch modulation for voices 1-7, driven by the prior voice's envelope-scaled output. The voice-zero PMON bit is ignored. Positive and negative modulation and save-state continuity passed Linux, Windows and lint in [run 467](https://github.com/sguzman/starbyte/actions/runs/38102466160). BRR filter saturation and wraparound at high amplitudes have also been corrected to retain signed 16-bit sample history; the overflow regression passed Linux, Windows and lint in [run 475](https://github.com/sguzman/starbyte/actions/runs/38102726110). Filters 2 and 3 now preserve the DSP's signed integer-shift operation order, rather than mathematically equivalent coefficients that differ by rounding; positive and negative history regressions passed Linux, Windows and lint in [run 481](https://github.com/sguzman/starbyte/actions/runs/38102863779).

The existing **opt-in** CLI sound-driver probe also accepts `--audio-wav /explicit/path/file.wav`. It writes a bounded, five-minute maximum **32 kHz stereo, signed 16-bit RIFF/WAVE** file using the generated DSP PCM, without requiring a Linux speaker or adding a new audio dependency. Normal gameplay, transport-only probes, and requests without an explicit path do not write any audio file. Unit tests verify signed samples and the WAV header; an integration fixture uploads original directory/sample/driver blocks from a synthetic 65816 ROM and verifies a WAV with nonzero PCM. The implementation and one-frame synthetic end-to-end test passed Ubuntu, Windows and lint in [run 473](https://github.com/sguzman/starbyte/actions/runs/38102654104). A further regression verifies deterministic two-frame PCM concatenation, passing Linux, Windows and lint in [run 476](https://github.com/sguzman/starbyte/actions/runs/38102753365). **This validates a synthetic audio export route, not audible or musically correct Super Mario World playback.**

The DSP also now latches **ENDX** on BRR end events, including looping BRR blocks, rather than repeatedly flagging only permanently stopped voices. Tests distinguish loop detection, software clear, reassertion on a later loop, and non-reassertion after a finished sample. The looping/one-shot ENDX regressions passed Linux, Windows and lint in [run 484](https://github.com/sguzman/starbyte/actions/runs/38102937860), while a driver-visible SPC700 $F2/$F3 read/clear regression passed in [run 487](https://github.com/sguzman/starbyte/actions/runs/38103066108).

## RAM-backed stereo echo and FIR bring-up (October 11)

The partial DSP now processes **eight-tap signed stereo FIR echo** using the same 64 KiB SPC audio RAM that the SPC700 reads and writes. ESA selects the echo base, EDL selects a circular stereo delay, EON selects voices routed to the echo send, and EVOL/EFB and FLG control echo output volume, feedback and echo RAM write inhibition. The DSP models 16-bit stereo PCM reads/writes, ring wraparound and save-state persistence for the eight-sample FIR history and position. This is **an emulated memory path**, not an unrelated private echo buffer. Changes to SPC audio RAM made by the echo engine are visible through normal SPC700 reads.

Core echo synthesis, FIR, feedback, write-inhibit and save-state tests passed Linux and Windows tests plus lint in [run 491](https://github.com/sguzman/starbyte/actions/runs/38158646107). Further regressions verify 64 KiB echo-address wrap, echo configuration through SPC700 $F2/$F3 and APU-level save-state replay; all Linux, Windows and lint jobs passed by [run 494](https://github.com/sguzman/starbyte/actions/runs/38158801060). The implementation is **not a cycle-exact S-DSP phase pipeline**. Gaussian interpolation is now present in a separate sample-step implementation, but real game sound drivers and a Linux audio output backend remain unverified. Default gameplay still retains the bootstrap compatibility path and does not automatically enable isolated driver execution.

## Four-tap BRR Gaussian interpolation (October 11)

The partial DSP now generates the 512 coefficients used by the S-DSP Gaussian filter from a permissively documented mathematical curve and applies **four-tap signed interpolation** to decoded BRR samples. Each voice retains the preceding block's three final samples so a BRR boundary or loop does not create an artificial hard discontinuity; this history is serialized for deterministic save-state playback. Fractional-pitch playback uses the low 12 bits of the decoded-sample phase, and the interpolator models the DSP's signed 16-bit intermediate wrap and final clamp. Voice NON/noise still overrides BRR interpolation as expected. Reference coefficient landmarks, phase mixing, signed overflow, PMON/echo compatibility and block-boundary state passed Linux tests, Windows tests, formatting and Clippy in [run 505](https://github.com/sguzman/starbyte/actions/runs/38159869993). An additional opposite-polarity two-block regression passed Linux, Windows and lint in [run 506](https://github.com/sguzman/starbyte/actions/runs/38159958373), confirming transitions blend across BRR boundaries rather than stepping directly to the next waveform.

An APU-level regression also verifies that the same Gaussian-filtered PCM is generated when master-clock time is delivered in one block or small CPU-sized chunks, including a midstream APU serialization/restore; it passed Linux, Windows and lint in [run 510](https://github.com/sguzman/starbyte/actions/runs/38160148667). Re-key-on history reset and deserialization of older save states lacking the Gaussian carryover field passed all three CI jobs in [run 511](https://github.com/sguzman/starbyte/actions/runs/38160188559). Voice envelope multiplication now uses the DSP's signed arithmetic shift and clears the low output bit before PMON, OUTX and stereo mixing; this and its negative low-amplitude regression passed in [run 513](https://github.com/sguzman/starbyte/actions/runs/38160315602).

The final main-volume, echo-send and feedback multipliers now use signed fixed-point arithmetic shifts rather than truncating integer division. Tests exercise negative low-amplitude PCM and the SPC700-visible OUTX/ENVX registers, including deterministic save-state restoration; this work passed Linux, Windows and lint in [runs 516 and 518](https://github.com/sguzman/starbyte/actions/runs/38161659689). The Gaussian table's complete 512-entry match against a known S-DSP reference has also been independently checked, with a whole-table checksum regression and startup prewarming added (latest CI verification applies separately).

**Boundary:** this is accurate *sample-step* filtering of the current synthetic BRR decoder, not a 32-phase cycle-exact S-DSP pipeline. Decoded BRR scheduling, internal register latency and commercial driver timing still need validation. It also does **not** enable or prove genuine Super Mario World sound or an operating-system speaker backend.

## Timer-driven uploaded SPC700 music fixture (October 11)

The opt-in host-clocked SPC700 runner now has an end-to-end **timer-gated sound-driver regression**. An original uploaded program writes target 64 to the sound CPU's timer-2 target register $FC, enables timer 2 through CONTROL $F1, sets voice/DSP registers, then repeatedly reads the read-to-clear counter at $FF. When the timer becomes nonzero it writes KON through $F2/$F3 to play an original looping BRR sample. The test asserts the driver and generated PCM remain silent before the first timer tick, produce audible-range synthesized samples after timer events, and resume with byte-for-byte PCM and complete APU-state equality after a midstream save-state restore.

This fixture passed Ubuntu and Windows tests plus formatting and Clippy in [run 529](https://github.com/sguzman/starbyte/actions/runs/38162270172). The separate scheduler permission/zero-time and 4096-instruction guard, global DAC mute with continuing echo-RAM/OUTX activity, and Gaussian coefficient prewarming passed hosted CI in [run 520 (prewarm)](https://github.com/sguzman/starbyte/actions/runs/38161764559), [run 523 (scheduler guard)](https://github.com/sguzman/starbyte/actions/runs/38161863187) and [run 525 (mute/echo)](https://github.com/sguzman/starbyte/actions/runs/38161987518). The generated coefficient table was independently compared against a 512-entry S-DSP reference, with all entries matching; regression fingerprints preserve the entire table, rather than checking four landmarks alone.

**Remaining boundary:** timer-driven synthetic driver execution still does not establish faithful real-game uploads, exact sound CPU/DSP interleaving, real Super Mario World music, or audible Linux speaker output. The production runtime retains the compatibility bootstrap and does not automatically run uploaded programs.

## Explicit host-clocked sound-driver probe (October 10)

An **additional opt-in** diagnostic now advances uploaded SPC700 instructions alongside 65816 master-clock time. It uses a fractional sound-CPU clock, carries signed cycle credit between CPU steps, and bounds the instructions executed within each host quantum. Timers and DSP samples are clocked by the existing frame path, **not double-clocked by the sound CPU scheduler**. The new headless switch `--apu-execute-driver` requires `--apu-upload-probe --no-save-ram`; ordinary playback, GUI sessions, and the transport-only upload probe never enable this scheduler implicitly. Tests exercise an original 65816 upload, driver register writes and stereo sample generation in this mode, and verify deterministic midstream save-state restoration. The host-clocked implementation passed Ubuntu, Windows and lint in [run 457](https://github.com/sguzman/starbyte/actions/runs/38101877874). Later additions supply `isolated_ipl_upload.audio_frame` counters for the last completed frame (stereo pairs, nonzero pairs, peak amplitude) without saving or playing audio; the CLI integration test and snapshot opt-in guard have separate follow-up CI.

This is an experimental compatibility diagnostic, **not proof of genuine Super Mario World sound, hardware-accurate SPC/DSP timing, or Linux audio-device integration**. Proprietary sound-driver probes have not yet established an accepted game-specific audio trace.

## Complete SPC700 opcode-cycle reference matrix (October 11; CI not yet confirmed)

The core now contains an explicit **256-entry baseline-cycle table** inside the opcode regression suite, derived from the Nintendo SPC700 Appendix C opcode tables as independently transcribed in [sony-spc700-python's hardware.json](https://github.com/gufranco/sony-spc700-python/blob/main/conformance/hardware.json). A test executes every one of the 256 opcodes under three differing A/X/Y/PSW/operand configurations, compares the observed bus-event count to the documented baseline, and permits exactly two additional cycles for any of the 28 conditional-branch opcodes. It also checks that each instruction's cumulative cycle counter agrees with the emitted trace.

The table explicitly overrides the manual's erroneous `MOVW dp,YA` (`$DA`) four-cycle entry with the **five cycles shown by actual recorded SPC700 bus execution**. The independent reference identifies the underlying hardware corpus as [SingleStepTests/ProcessorTests at `bb11756436da8fd16cce86aef63dc6725f48836f`](https://github.com/SingleStepTests/ProcessorTests/tree/bb11756436da8fd16cce86aef63dc6725f48836f/spc700/v1). This is a **cycle-count guard, not cycle-exact address/value equivalence** and not evidence of commercial-game audio. The suite is committed but has **not yet been run or confirmed green** on the latest commit.

## SPC700 memory-move bus-side effects (October 11; CI not yet confirmed)

Two more read/write sequencing errors in the standalone SPC700 interpreter are corrected against the independently recorded SingleStepTests bus order. Opcode `$AF` (`MOV (X)+,A`) performs a **wait**, not a read of the memory destination, before storing A and incrementing X. The similar opcode `$C6` (`MOV (X),A`) **does** read the destination before its write. Both still take four cycles. This difference is observable if X addresses timer output `$FF`: only `$C6` clears the output through its read; `$AF` must not. APU integration regressions cover both forms, timer output, direct-page address wrapping, and four-cycle traces.

Opcode `$FA` (`MOV dp,dp`) now reads the source **before fetching the destination offset**. The earlier implementation fetched both operands before reading the source. The opcode still takes five cycles and **never reads the destination**. The source-level regression checks every bus-event address; the APU regression exercises timer 2's read-to-clear `$FF` as source and transfers that value to regular audio RAM `$20`. The same ordering correction now applies to all six memory-to-memory ALU families (`OR/AND/EOR/CMP/ADC/SBC dp,dp`): the source read precedes the destination-operand fetch. Unit tests check the exact instruction bus sequence for all six families, and an APU integration regression reads timer output `$FF` as the source of `OR $20,$FF` while checking the operand-fetch order and stored result.

`TSET1` (`$0E`) and `TCLR1` (`$4E`) also perform two consecutive **real reads of their absolute target address**, then a write. The previous interpreter inserted an idle slot in place of the second read. The corrected bus trace keeps the first-read value for arithmetic and the write, but a read-to-clear timer output returns its value on the first read and zero on the second. An APU regression checks both reads and the final store to timer-output address `$FF`.

The SPC700's conditional poll/loop branches now also preserve the recorded order of internal cycles and displacement reads. `BBS/BBC` (bit tests) and `CBNE` (compare and branch) idle **before** fetching their branch displacement; `DBNZ Y` performs a dummy read of the displacement, then an idle, then reads the displacement again. The earlier model put those same cycles after the displacement or replaced a real read with another idle. Regressions pin all four kinds of bus events and retain the original taken/not-taken cycle counts. This aligns how sound-driver loops poll MMIO, without changing their flag or branch arithmetic.

References: [Anomie's SPC700 opcode/bus notes](https://github.com/gilligan/snesdev/blob/master/docs/spc700.txt), [Near's SPC700 cycle-breakdown discussion](https://forums.nesdev.org/viewtopic.php?start=75&t=16140), and the [independent cycle-by-cycle SPC700 conformance implementation](https://github.com/gufranco/sony-spc700-python). These source comparisons are not a substitute for running Starbyte's own CI or commercial-game audio probes.

## Indirect-Y SPC700 load/store bus order (October 11; CI not yet confirmed)

Hardware-recorded SPC700 bus traces distinguish indirect-Y **loads** from **stores** at the internal cycle. All six accumulator ALU `[dp]+Y` forms (`OR/AND/EOR/CMP/ADC/SBC`) and `MOV A,[dp]+Y` now idle **before** reading the wrapped direct-page low/high pointer. `MOV [dp]+Y,A` instead reads both pointer bytes **before** its idle and then reads/writes the effective destination. The earlier shared load path used the store's idle placement for all seven load opcodes. Both operations retain their previous six/seven total cycle counts; the 256-opcode timing matrix alone could not reveal the mismatch. An opcode-family trace regression asserts all reads/writes and the distinct wait slot. A separate APU test uses timer 2's read-to-clear `$FF` as the low pointer byte, wrapped `$00` as high byte, and checks both load/store MMIO effects and memory results. References: [pinned independent SPC700 cycle/bus conformance implementation](https://github.com/gufranco/sony-spc700-python), `spc700/core.py` `operand_address(a_idy/idy_a)`. These are committed regressions, **not a claim that a recent Rust CI run has passed**.

## Opt-in IPL RAM-write accounting (October 11; CI not yet confirmed)

The isolated IPL transport now exposes both `bytes_written` (accepted/acknowledged payload bytes) and `bytes_stored` (payload bytes actually written into SPC audio RAM). These can differ when SMP TEST bit 1 blocks ordinary RAM writes; writes to the `$F0-$FF` MMIO overlay still count as stored, as that window remains writable. Unit and APU integration regressions cover blocked storage, continued acknowledgements, MMIO exemption, reenabled ordinary RAM writes and serialized counter state. The APU-level TEST-reenable fixture also now clears the executing SPC700's direct-page flag before writing TEST through the ordinary SPC bus, matching the previously enforced PSW.P gate.

**Boundary:** the telemetry is read-only and appears only in explicitly enabled IPL CLI reports; the production upload shortcut, mailbox acknowledgements, and playback behavior do not change. These changes are committed but Rust compilation and CI remain unconfirmed. A high stored-byte count does not establish correct sound-driver execution or audible music.

## Hardware-recorded SPC700 execution vectors (October 11; CI not yet confirmed)

A pinned conformance slice now holds **26 actual processor-test recordings across 13 opcode families**: TCALL, BRK, indirect JMP, CALL, PCALL, RET, RET1, MOV (X)+,A, MOV A,(X)+, MOVW YA,dp, MOVW dp,YA, MOV [dp]+Y,A and MOV A,[dp]+Y. Each fixture supplies the recorded initial and final PC, A/X/Y, SP, PSW and sparse RAM plus its ordered read/write/wait bus cycles. The Rust regression reconstructs RAM, executes one instruction, checks all registers and the complete 64 KiB final image, and compares bus access kind, address and known recorded values for each cycle. Unknown values in the hardware recording are deliberately not treated as asserted reads.

The source is the [pinned SingleStepTests SPC700 v1 corpus](https://github.com/SingleStepTests/ProcessorTests/tree/bb11756436da8fd16cce86aef63dc6725f48836f/spc700/v1) at commit `bb11756436da8fd16cce86aef63dc6725f48836f`. The curated sample is in `crates/starbyte-core/tests/fixtures/spc700-bus-reference.json`; the runner is in `src/apu/spc700.rs`. This adds independent bus-order and register-result checks beyond the earlier 256-opcode cycle-count guard. **These new tests are committed but have not yet been verified with Rust compilation or hosted CI**, and 26 synthetic single-step records do not demonstrate commercial sound-driver compatibility.

## SPC700 word-instruction bus-cycle fidelity (October 11; CI not yet confirmed)

The standalone SPC700 interpreter now implements five-cycle `MOVW YA,dp` and `MOVW dp,YA` traces. The load inserts its internal wait between the low and high reads; the store performs a real **read of the low destination address** before the two writes. **Important reference conflict:** Nintendo-derived opcode tables and SNESdev's overview list `MOVW dp,YA` (`$DA`) at four cycles, but Anomie's SPC700 notes identify a five-cycle operation with a destination read, and a separate SingleStepTests bus-conformance project reports that hardware-recorded execution takes five. The source follows the observed bus behavior; do not remove its destination read based solely on the manual's four-cycle entry. `ADDW/SUBW YA,dp` retain five cycles but place their internal wait before the high read. `INCW/DECW dp` now interleave *read low, write low, read high, write high* instead of reading both bytes before either write. The existing CMPW read-only four-cycle path remains unchanged.

Unit regressions verify cycle counts and read/write event addresses and order, including direct-page high-byte wrapping. An APU integration test writes `MOVW $FF,YA` and confirms its low-byte read clears the SPC700 timer-2 output at `$FF` before the store, while the underlying MMIO RAM receives the data byte. These changes target actual sound-driver CPU timing and port side effects; production sound execution remains opt-in, and authentic SMW audio and hosted CI are unverified. References: [Anomie's SPC700 opcode and bus notes](https://github.com/gilligan/snesdev/blob/master/docs/spc700.txt), [the independent bus-conformance discrepancy report](https://github.com/gufranco/sony-spc700-python#the-manual-is-wrong-about-one-instruction), and [the conflicting manual-derived listing](https://snes.nesdev.org/wiki/SPC-700_instruction_set).

## SPC TEST RAM write protection and IPL semantics (October 11; CI not yet confirmed)

The SPC700 `TEST` register at `$F0` now preserves its power-on `$0A` value and implements **bit 1 RAM-write enable**. Clearing this bit inhibits ordinary audio-RAM writes, including S-DSP echo-buffer storage, without blocking MMIO writes at `$F0-$FF` or pausing DSP voices, FIR processing, clocks or PCM generation. Reenabling the bit restores RAM and echo writes at their current positions. As on hardware, writes to the TEST latch are ignored if the SPC700 PSW direct-page flag is set; underlying register-window RAM still receives bus writes. The interpreted instruction bus now explicitly carries the **executing** SPC700's PSW into TEST MMIO handling, instead of consulting the default placeholder that temporarily occupies the APU during CPU callback dispatch. An opcode-level regression covers SETP, CLRP, absolute TEST writes and resumed ordinary RAM writes. The TEST latch is serialized, defaults to writable on legacy snapshots, and resets to `$0A`.

Tests cover direct MMIO and ordinary RAM writes, interpreted SPC700 MOV operations, echo suppression with live PCM output, deterministic DSP continuation after save/restore, and the isolated IPL decoder. The IPL decoder now **acknowledges and counts accepted transfer bytes even if TEST blocks the destination RAM store**, rather than bypassing the hardware RAM gate. The `bytes_written` telemetry therefore means *accepted IPL bytes*, not proof that each byte was committed to audio RAM. The decoder still handles writes within `$F0-$FF` normally, using the IPL boot loader's clear P flag instead of a potentially stale interpreter PSW. Opt-in reports now include `smp_test.latch` and `smp_test.ram_writable` to distinguish inhibited RAM from a stalled sound driver; normal gameplay reports omit them. Reference: [SNESdev S-SMP TEST register](https://snes.nesdev.org/wiki/S-SMP) and [Anomie's SPC700 register notes](https://github.com/gilligan/snesdev/blob/master/docs/spc700.txt).

The undocumented TEST processor wait-state modes, RAM-read inhibition, and abnormal timer clock-control modes remain **unimplemented**. No unsafe register settings are emulated as speed changes. These are committed source and regressions, **not yet confirmed** Rust CI results or authentic commercial-game audio acceptance.

## Opt-in sound-CPU / timer / DSP scheduling order (October 11; CI not yet confirmed)

The experimental `--apu-execute-driver` emulation path now interleaves SPC700 execution, hardware timer advancement, and DSP sample generation at **six NTSC master-clock quanta**, matching the existing 65816 bus-cycle unit. Previously the emulator clocked the timers for the *whole* 65816 instruction, then executed its full SPC700 quota, and only then synthesized all due PCM. That ordering could let timer-polling sound drivers observe ticks that should not have happened yet and move DSP configuration ahead of samples within that CPU instruction. The smaller quanta apply **only when the host-clocked sound-driver probe has been explicitly enabled**; ordinary commercial gameplay keeps the prior single-quantum `step_master_cycles` and PCM path unchanged.

A synthetic uploaded SPC program configures timer 2, polls its read-to-clear output, and acknowledges the first positive tick on an APU output port. Its test compares a 2,000-master-clock emulator quantum against independently sequenced six-clock APU execution, including the full serialized APU state and PCM buffer. A mid-run APU save/restore at a six-master-clock boundary also reproduces the exact final PCM and timer/driver phase, while a separate regression verifies the default production path remains identical to the previous single-quantum algorithm. The host still completes the 65816 instruction's bus operations before APU substeps, so this does **not** provide bus-cycle interleaving of CPU writes with SPC700 MMIO, nor cycle-exact 32-phase DSP timing.

**Verification boundary:** source and regression tests are committed, but hosted Linux/Windows `cargo test`, formatting and lint results remain unconfirmed, as do authentic SMW music execution and Linux speaker output.

## PMON extreme-pitch interpolation limit (October 11; CI not yet confirmed)

The sample-stepped DSP now caps the **accumulated interpolation position** at `$7FFF` after pitch modulation, matching the limiter in Snes9x's S-DSP reference implementation. The unmodulated `VxPITCH` remains a 14-bit register value (`$0000-$3FFF`); a sufficiently positive previous-voice `OUTX` can almost double it. Before this correction, combining a high modulated pitch with a nearly full fractional sample could skip one additional BRR sample within a DSP tick. A direct voice accumulator boundary regression and a two-voice PMON integration regression cover the case and preserve ordinary one-sample stepping. The integration fixture now drives voice zero with deterministic full-scale **NON noise**, while voice one plays BRR data; the original ordinary BRR stimulus did not guarantee pitch saturation. The regression checks both a PMON-enabled saturation case and an identical PMON-disabled control, so it cannot accidentally pass with insufficient modulation. Reference: [Snes9x SPC_DSP.cpp](https://github.com/snes9xgit/snes9x/blob/master/apu/bapu/dsp/SPC_DSP.cpp), especially the interpolation-position cap following the pitch modulation step.

**Fidelity boundary:** the current partial engine maintains one-sample Gaussian/BRR indexing rather than the S-DSP's internal four-sample interpolation ring and 32-phase pipeline. This change enforces its high-pitch saturation within the existing approximation, not a declaration of cycle-exact or bit-exact sound. Hosted Rust CI completion and commercial-game audio still require verification.

## CPU-to-SPC mailbox write-event fidelity (October 11; CI not yet confirmed)

The 65816-to-SPC mailbox bridge now replays **only CPU port writes that actually occurred** since the last APU synchronization. Previously the system bus blindly resent the cached last-written values of all four ports after *every* CPU instruction, undoing the SPC700's `CONTROL` ($F1) input-latch clear (bits 4/5) even when the main CPU had not sent a new command. The new four-bit pending-write mask is recorded on normal bus MMIO writes, drained after each 65816 instruction, cleared at reset, and serialized with a default of zero for older save states. It forwards explicit repeat writes of identical bytes, which the sound CPU must distinguish from idle time. A system-bus regression covers clearing, idle sync, re-sending an identical byte, and pending-write restoration. An actual 65816 instruction sequence also verifies clearing survives NOP and a save-state round trip, but the subsequent matching write is delivered.

The CLI's `apu_io_activity` now preserves its `cpu_to_apu_ports` field as the **last 65816 write mirrors** and adds `apu_input_latch_ports` for what the SPC700 currently reads after a `CONTROL` clear. These may legitimately differ. No RAM payload or proprietary sound-program content is added to reports. This is a concrete fix toward authentic sound-driver command exchange, but emulator timing still does not interleave arbitrary DMA writes and SPC execution at individual bus-cycle granularity. Latest hosted CI and commercial ROM acceptance have not been verified.

## Envelope arithmetic and held key-off controls (October 11; CI not yet confirmed)

The partial DSP now shares a single **post-decrement exponential envelope step** across ADSR decay/sustain and exponential GAIN: subtract one first, then subtract the shifted *decremented* envelope. Using the original envelope before the shift had incorrectly dropped one extra envelope unit at exact $100 boundaries (for example, $400 should become $3FC, not $3FB). Direct regressions check zero, $100 and $400 for both envelope modes. References: [SNESdev envelope timing](https://snes.nesdev.org/wiki/DSP_envelopes) and [Snes9x's S-DSP implementation](https://github.com/snes9xgit/snes9x/blob/master/apu/bapu/dsp/SPC_DSP.cpp).

DSP `KOFF` bits are now **held controls**, rather than one-time releases that can be bypassed by writing `KON` afterward. While a voice's KOFF bit remains set, the next sample forces that voice into release; other voices remain unaffected. Clearing KOFF allows a subsequent KON to start that voice. New tests cover selective KOFF, Gaussian startup, mid-KOFF save-state restoration, and APU-level DSPADDR/DSPDATA writes with PCM resuming after clear. As with FLG reset, this is **sample-boundary gating**, not exact internal two-sample latch timing. See [SNESdev KON/KOFF behavior](https://snes.nesdev.org/wiki/S-DSP_registers).

These changes do not enable sound drivers automatically in production. Latest hosted CI is still unconfirmed; authentic Super Mario World music and live Linux audio device output are separate acceptance gates.

## DSP echo-delay register boundary timing (October 11; CI not yet confirmed)

The S-DSP echo ring now **latches `EDL` only at the beginning of an echo-buffer traversal**, rather than allowing a mid-ring register write to change the active buffer length immediately. The CPU-visible `EDL` register updates on write, but the previous latched delay remains effective until the ring wraps. The internal latched size is serialized for exact save-state continuation; old save states lacking this field fall back to the current register until a natural boundary. `EDL=0` still writes exactly one four-byte stereo echo word. Direct DSP regressions check both delay directions, zero-length semantics, the maximum 7680-sample/240 ms echo ring, masking of the writable high EDL bits, older save-state migration and restoration; an APU-level test verifies deferred changes through SPC700 DSPADDR/DSPDATA writes, NTSC master-clock accumulation and a sentinel placed in the next echo RAM page. This follows [Anomie's S-DSP EDL errata](https://gist.github.com/nyanpasu64/a0d916ce6924912a7116682bf778e9a0) and [SNESdev's echo-buffer description](https://snes.nesdev.org/wiki/S-DSP_registers).

**Verification:** the implementation and tests are committed. This models sample-boundary latching, not all 32 internal S-DSP pipeline phases; latest Linux/Windows CI completion and authentic game audio remain unverified.

## DSP echo special RAM protection (October 11; CI not yet confirmed)

The S-DSP echo writer now skips SPC RAM addresses `$00F8` and `$00F9`. Those two bytes remain ordinary writable RAM to the SPC700 bus, but are protected from DSP echo buffer writes even when the echo ring spans the first 256 bytes. The regression covers both direct DSP synthesis and APU-configured DSP voice/echo activity, including writes immediately beside the protected range. This behavior is documented by [Anomie's SPC700 register reference](https://github.com/gilligan/snesdev/blob/master/docs/spc700.txt); it does not establish wider echo or CPU/DSP cycle accuracy.

## Run-wide audio observability (October 11; CI not yet confirmed)

Headless `starbyte-cli run --report-json` now includes an additive `audio_run` object that counts 32 kHz stereo sample pairs across **all completed frames**, nonzero pairs, peak absolute sample magnitude, and the first one-based requested frame with nonzero PCM (or null when all sampled frames were silent). This complements, rather than replaces, the opt-in `apu_io_activity.isolated_ipl_upload.audio_frame` snapshot, which still describes only the final frame. The accumulator needs no WAV file or multi-frame PCM buffer and cannot turn silent commercial audio into working music. A pure unit regression exercises short audio followed by a silent final frame, signed -32768 peak, and always-silent runs. The same cumulative `audio_run` values now also appear in each flushed JSONL frame-log entry, including a terminal error record after a stalled real-ROM probe (counting **only completed frames**); a failure regression retains evidence of earlier PCM in both the terminal JSONL record and an explicitly requested final failure report. See [automation diagnostics](automation.md). Latest-commit hosted CI is not confirmed.

## IPL-to-MMIO replay and zero-marker fall-through (October 11; CI not yet confirmed)

An isolated IPL upload copies sound-program bytes through the SPC700 memory bus, **not just into a detached RAM array**. The APU now replays upload bytes in the $F0-$FF MMIO range through the corresponding hardware handlers as well as the RAM backing. This enables an IPL transfer directly targeting $F2/$F3 to select and write actual DSP registers, and preserves the documented overwrite of port-0 byte acknowledgements when an IPL store targets output mailbox $F4. New tests exercise these effects at the isolated APU boundary and using actual 65816 instructions over $2140-$2143 with save-state continuation.

For diagnosing stalled uploads without dumping copyrighted program bytes, the opt-in CLI's `apu_io_activity.isolated_ipl_upload` snapshot now includes the decoder's next `expected_index` and a persisted `ignored_tokens` count (distinct invalid port-0 tokens only, not unchanged polling). The new counters are additive fields, not a change to production-mode behavior. Regressions verify invalid-index counting and save-state continuity. A read-only `dsp_registers` snapshot is now exposed in the same opt-in report: master volumes, FLG, sample directory, KON/KOFF, echo controls and selected voice routing registers. No ROM or SPC RAM contents are included; this is diagnostic state, not proof of rendered music. An emulator-level fixture verifies that the snapshot reports the DSP register programmed by real 65816 IPL bus traffic, and that the API returns no value outside explicit upload probes.

A zero-valued *forward block marker* requires special treatment: if the old block expected index $FE or $FF, token $00 ends that block, and the IPL immediately consumes the still-present nonzero mode byte as byte index zero of the next block. The decoder now records that **implicit first data byte**, moves its expected index to one, and replays the MMIO side effect if its new destination is a hardware address. Tests verify the explicit and wrapped transitions, duplicate-zero suppression, automatic data transfer, and DSPADDR/DSPDATA interaction. Normal uploaders should avoid zero-valued markers, as documented by [Anomie's disassembly](https://github.com/gilligan/snesdev/blob/master/docs/spc700.txt) and [SnesLab's IPL protocol](https://www.sneslab.net/wiki/SPC700/IPL_ROM).

**Verification:** committed source and synthetic regressions; latest-commit hosted Rust tests, lint, and authentic ROM upload evidence remain unconfirmed. The opt-in isolation boundary and the silent production APU are unchanged.

## IPL upload protocol ordering (October 11; CI not yet confirmed)

The isolated, opt-in IPL transfer decoder now uses the SPC700 loader's **signed 8-bit CMP/BPL block-end comparison**, rather than treating every unexpected port-0 index as a block termination. A transfer retains its current byte and acknowledgement indices when it sees stale indices or commands more than 128 indices ahead; valid forward commands and index-wrap boundaries still work. Tests exercise the transfer decoder, the actual APU mailbox path, the exact 128-step signed comparison boundary, and a save-state restored after an ignored index. See [the SPC700 IPL protocol](https://snes.nesdev.org/wiki/Booting_the_SPC700) and [Anomie's documented IPL disassembly](https://github.com/gilligan/snesdev/blob/master/docs/spc700.txt).

**Verification boundary:** these changes are committed, with synthetic tests added, but hosted CI completion and authentic commercial-ROM acceptance are **not** yet confirmed. The production bootstrap shortcut and silent commercial audio behavior remain unchanged. The next acceptance gate remains a complete observed real-ROM upload, verified SPC700 instruction progress, expected DSP register changes, and nonzero audio in the existing opt-in diagnostic.

## DSP held soft-reset semantics (October 11; CI not yet confirmed)

The DSP's `FLG.7` soft reset now forces zero-envelope key-off **while the reset bit remains set**, rather than only discarding voice structures at the moment of the write. Repeated `KON` under held reset cannot produce voice PCM at the next DSP sample; clearing reset permits new notes. The change preserves decoder state on the register write and leaves the echo/FIR read and output paths active, consistent with [SNESdev FLG behavior](https://snes.nesdev.org/wiki/S-DSP_registers) and [Anomie's DSP documentation](https://gist.github.com/nyanpasu64/a0d916ce6924912a7116682bf778e9a0). Synthetic DSP and APU-level MMIO regressions check echo under reset, blocked KON and zero ENVX/OUTX, and deterministic restored PCM after clearing reset. This is sample-stepped behavior, not verification of internal 32-cycle phase interleaving or commercial sound-driver timing. Hosted CI for this revision has not been confirmed.

## SPC700 MMIO / RAM write coherency (October 11; CI not yet confirmed)

The isolated SPC700 bus now mirrors **all MMIO writes** into backing SPC audio RAM, in addition to each register's hardware side effect. This includes TEST/CONTROL ($F0/$F1), DSPADDR/DSPDATA ($F2/$F3), output mailboxes ($F4-$F7), write-only timer targets ($FA-$FC), and attempted writes to read-only timer outputs ($FD-$FF). As before, SPC700 MMIO reads return their own register behavior rather than those RAM bytes. Tests cover direct bus writes, executable SPC700 MOV instructions, timer target readback, independent CPU/SPC mailbox visibility, and memory behind the register overlay. Hardware references: [Anomie's SPC700 document](https://github.com/gilligan/snesdev/blob/master/docs/spc700.txt) and [Near's S-SMP bus discussion](https://forums.nesdev.org/viewtopic.php?start=30&t=16140).

**Outstanding fidelity:** TEST processor speed, RAM-read inhibit and abnormal timer clock modes are not modeled; its RAM-write protection is now implemented as described in the newer October 11 section. MMIO/DSP cycle-exact scheduling is unfinished. Documentation differs on the high-bit readback of DSPADDR $F2; the implementation preserves the address latch and write-inhibit behavior without declaring that ambiguity hardware-validated. The new bus regressions still require hosted Rust/Windows CI confirmation.

## Post-run-529 audio correctness work (October 11; CI not yet confirmed)

The subsequent audio work is **committed to `main` but does not yet have independently confirmed Ubuntu, Windows, formatting or Clippy results at the latest head**. Do not elevate these source-level regressions to the verified table until an exact-commit Actions run is inspected:

- The SPC700's stage-1 timer prescalers now free-run through CONTROL disable/enable transitions; only the downstream timer counters reset when enabled. A newly written target below the running stage-2 counter waits for its eight-bit wrap rather than firing early. Reads from write-only target registers $FA-$FC and TEST/CONTROL registers $F0/$F1 now return zero rather than exposing underlying SPC RAM. The timer implementation has new edge, target-rewrite, reference-loop, MMIO and APU save-state regressions; an additional SPC700 instruction fixture exercises write-only reads.
- DSPADDR $F2 now retains bit 7 for SPC700-visible reads. The DSPDATA $F3 write path ignores writes to high-bit-set (read-only) mirrors, while reads still select the low seven address bits. Both direct MMIO and executed-SPC700 regressions were added.
- Echo FIR coefficients now apply FIR0 to the oldest sample and FIR7 to the newest, with signed 16-bit wrap for the first seven products and saturation only on the final addition. Original echo tests were updated and new tap-order/overflow fixtures added.
- Voice mixing now applies signed left/right volume and 16-bit saturation after **each** voice's contribution to both the main and echo sums, rather than accumulating full precision until after all voices. A four-voice cancellation regression checks DAC output and SPC echo RAM.
- The opt-in host-clocked SPC scheduler now returns without executing instructions when zero host time is supplied, even if a previous bounded call left positive cycle credit. A capped-run regression protects this condition.
- These changes have been independently cross-checked as arithmetic models, not compiled or proven on SNES hardware. The emulator's 32-phase DSP interleaving, actual commercial-driver synchronization and real Linux speaker playback remain outstanding.

Hardware references: [S-SMP register/timer behavior](https://snes.nesdev.org/wiki/S-SMP), [S-DSP FIR/filter behavior](https://snes.nesdev.org/wiki/S-DSP_registers). All code remains within Starbyte's existing audio-emulation scope. No unrelated UI, reverse-direction behavior, Python tooling or copyrighted ROM files were added.

## What exists today

| Area | Implementation and evidence | Remaining uncertainty |
| --- | --- | --- |
| Rust workspace | Four crates: core, frontend, native egui, CLI; CI builds and tests all targets | No distributed Linux release validated |
| CPU and system | 65816 dispatch recognizes all 256 opcode values; DMA/HDMA, timer IRQs, master-clock-to-dot timing; $4016 serial and VBlank automatic $4218/$4219 joypad polling; newly implemented unsigned $4202-$4206/$4214-$4217 CPU hardware math, with synthetic regressions | In-level real-ROM traces confirm Right and Left movement, camera reversal, jumping and death recovery; precise collision semantics and bus timing remain unverified |
| PPU | Software Mode 0/1 tile backgrounds, sprites, CGRAM, priority, scroll, windows, newly implemented main/subscreen color math and brightness; SMW menus, welcome text and **2,900-frame overworld probe** | In-level framebuffer captures confirm correctly moving foreground, backgrounds, Mario, enemies and HUD across horizontal camera scroll (X=0→285); more complex PPU effects remain unverified |
| APU/audio | SPC700/APU bootstrap and CPU port traffic; independent 64 KiB SPC RAM, fractional 32 kHz DSP clock, BRR decoding and eight-voice mixing; isolated sound-driver pipeline now accepts IPL-style indexed, multi-block transfers into SPC RAM with byte acknowledgements, wraparound and explicit entrypoint handoff; synthetic tests pass BRR output from transferred RAM into the partial DSP; the legacy placeholder sound-CPU clock is now bulk-advanced; DSP $F2/$F3, mailboxes $F4-$F7, timers, CONTROL and optional user-supplied IPL overlay retain isolated regressions | Ordinary commercial-ROM audio remains silent because the production runtime still uses a bootstrap shortcut rather than executing uploaded SPC700 sound programs (an opt-in bounded isolated runner exists separately), rate-controlled ADSR/GAIN and NON/noise are implemented in the partial DSP, with sample-stepped Gaussian interpolation and RAM-backed echo/FIR available, but cycle-exact echo/FIR and hardware-level timing fidelity remain missing, and no speaker backend is connected |
| Saves | Core state serialization and save-RAM paths, CLI state handling; frontend in-memory quick slot F5/F8 | Disk-backed slots 1–3 and XDG SRAM autosave now exist; actual game save reliability and native Wayland UX remain unverified |
| Desktop | Library, covers, metadata, cheats, gamepads, Play View, F9/Escape navigation, responsive tiled-window layout, fullscreen and integer display scaling; frame scheduler now targets nominal NTSC ~60.0985 Hz instead of 60 Hz | Actual Wayland/Hyprland play, input focus, measured smoothness, resize and dialogs not firsthand tested |
| Automation | Versioned CLI JSON introspection, diagnostics, ROM inspection and run reports; read-only local Cheatarium SNES candidate search | No running MCP server; cheat lookup does not identify exact cartridge builds or activate cheats |
| Packaging | Cargo sources and test workflow | No release installer or app repository |

See [PPU coverage](ppu-coverage.md) for a register-by-register account. A **2,900-frame real-ROM run (2026-10-09)** validates a significant PPU fix: Starbyte now composites BG2 on the subscreen with main-screen BG1/BG3/OBJ, restoring the fully colored **Yoshi's Island overworld** in Super Mario World. All frames completed without CPU errors; actual captures show water, land, paths, hills and icons that were absent in the prior nearly-black map. Input already navigates menus and the initial welcome scene into game mode `$0E` (overworld). Right at frame 2100 sets a plausible map destination `(112,124)` from `(104,120)`, but the player's `$7E1F17/$7E1F19` coordinates instead fluctuate by ~66–67 pixels per frame, continuing after release; B does not enter a level. A follow-up **22,438-instruction trace** isolated the exact failure: the game writes `$D0` and `$04` to `$4202/$4203` and expects `$0340`, then divides by `$08` and expects `$0068`; previously the two result reads returned open bus `$4242`. The CPU-side multiplier/divider ports are implemented and unit-tested, and the subsequent **2,750-frame commercial-ROM retest confirms stable movement**: the overworld advances from `(104,120)` to `(152,136)`, Mario enters Yoshi's Island 2 (mode `$14` from frame 2424), and two B-triggered jump/land cycles occur in the level. A subsequent **3,575-frame scripted run confirms actual in-level horizontal movement, camera scrolling, moving jumps, an on-screen 200-point score event, death state, life decrement from 4 to 3, and return to the overworld**. The precise cause of the death and special mechanics remain unknown; the later Left pulse occurred *after* returning to the overworld. This is a materially functional playable segment but not a complete compatibility claim. The new DSP code can decode and mix real BRR samples when synthetic test code writes SPC RAM and DSP registers, but the commercial-ROM path **still produces silence** because the emulator does not yet execute uploaded SPC700 game sound programs; the standalone SPC bus supports $F2/$F3 register access, but it is not yet driven by a running sound CPU. No speaker backend is connected. Running, jumping, scrolling and death/recovery are confirmed, but full level completion, advanced collision correctness, saves and native Hyprland/Wayland input remain unverified. The precise ROM revision/hash remains unidentified. No ROM bytes or screenshots were committed to public GitHub; see [game-specific compatibility evidence](compatibility/super-mario-world.md).

## Controller encoding correction validated against real ROM

A completed 560-frame Super Mario World run delivered the scheduled Start pulse to the host and latched joypad word at frames 450–454 but did not enter file selection. Source comparison against the SNES controller layout identified an incorrect button encoding: Start was represented as `$0008` instead of hardware-correct `$1000`; all twelve bits were misplaced. The encoded word and serial shift direction have been corrected, with unconnected ports returning zero. Synthetic regressions are included, and **the subsequent 700-frame ROM test confirmed that Start opens the file-selection menu**. The initial welcome message, overworld navigation, first level entry and basic jumps are now confirmed by successive real-ROM probes. See [Super Mario World compatibility](compatibility/super-mario-world.md).

## Cartridge save persistence

Starbyte's desktop session now loads content-scoped SRAM from XDG data storage when a ROM is opened, and atomically persists modified SRAM on game switch, at approximately 30-second intervals while the GUI is active, and on app exit. Unchanged SRAM isn't rewritten. This is independent of disk save-state slots and of temporary F5/F8 quick saves. Saving and loading on real commercial titles remains unverified.

## Archive-backed user libraries

Starbyte discovers the existing `~/Games/Roms/SNES` collection when that directory is present and persists it as a normal removable `library.rom_dirs` configuration entry. ROMs within ZIP archives are indexed by their internal cartridge header; archives remain read-only, and only chosen members are materialized in `$XDG_CACHE_HOME/starbyte/extracted-roms/`. Library cache scans run in a worker thread and refresh on launch, even when a previous snapshot exists. Cache manifests are atomically replaced once per completed scan rather than rewritten for every ROM, damaged ZIPs are skipped without hiding healthy games, and corrupt/truncated extracted cache files are recreated atomically. ZIP members are size-bounded (64 MiB). The core's direct ZIP loader also bounds decompression and skips invalid-header ROM members to allow a later valid member in the same archive. This does not establish that the two previously rejected user ZIPs are fixed; their contents have not been available for inspection. Each completed scan also writes `manifests/rom-scan-report.json` with source names and reasons for skipped archives or members; the Settings panel provides an **Open Scan Report** action. This is implementation behavior, not a claim that every ZIP or every SNES title loads.

## Cover art

Retail box art is not ordinarily embedded in SNES ROM images. The library offers **Get Covers** to explicitly index Libretro's thumbnail catalog and download artwork for installed games to XDG cache. It leaves game ZIPs unchanged, reuses existing images instead of repeatedly downloading them, and shows a placeholder where a game lacks a match. The operation uses the network and may be lengthy for large collections; commercial game artwork is not bundled in Starbyte.

## Launch and background-work isolation

The library's **Play Selected** toolbar action and each installed grid card's **Play game** button now queue an explicit ROM launch. Resume/Pause applies only to a previously loaded game. ROM materialization has a separate worker queue from network artwork/metadata, so slow artwork downloads cannot block the start of a selected local game. A persistent bottom status strip displays launch/worker feedback. The launcher/isolation changes passed hosted CI at `9fe41ff5` (2026-10-08); native Wayland behavior remains unverified. This removes a startup scheduling problem; it does **not** prove that Super Mario World renders or plays correctly.

## Built-in visual smoke test

`cargo run -p starbyte-egui -- --demo` creates a copyright-free 65816 LoROM in the XDG cache and opens its alternating red/cyan checkerboard through the normal emulator frontend. This permits testing tiling, focus, fullscreen, screenshot export and scaling without a commercial cartridge. The Play View's Input overlay reports the effective keyboard/gamepad SNES button state for local input smoke tests; the generated checkerboard itself remains static. A core unit test checks exact screen pixels, but **the native Wayland window must still be tested by a user with that compositor**.

## How to use what's implemented

- Start the desktop with `cargo run -p starbyte-egui --`; pass `--rom /path/to/game.sfc` or `--rom-dir /path/to/roms`, or select **Open ROM...** in the library. Dragging a local ROM or single-game ZIP onto the desktop window also uses the same validated loader. Multi-ROM ZIPs require selection through the library. Opening a ROM switches to the central **Play View**.
- In Play View: **F9** toggles the game/library; **Escape** returns to library; **F5/F8** save/load one temporary in-memory slot. The toolbar has Pause, Fullscreen and a Disk Slots menu for three per-ROM persistent slots; F12 exports a screenshot to XDG data storage. Disk slots remain local and are never uploaded.
- For reproducible, local-only inspection: `cargo run -p starbyte-cli -- doctor --json`, `capabilities`, `inspect /path/to/game.sfc --json`, or `run /path/to/game.sfc --frames 1 --report-json /path/to/report.json`. See [automation](automation.md).
- New settings default to XDG config/cache locations. Prior checkout-local settings can be read on first run from the original directory; see [README](../README.md). Config writes use atomic temporary-file replacement so interruptions do not truncate a previously valid configuration.

## Desktop responsiveness instrumentation

The Session panel's 60-frame debug step now advances one frame per UI update with a Cancel control, rather than calling all 60 frames synchronously and freezing the interface for the entire batch. Play View has an optional Timing overlay; the Session panel also displays the exponential moving average, peak, and number of emulation frames exceeding the 16.7 ms budget. These are measurements of `FrontendSession::run_frame` only, not full wall-clock input latency or GPU presentation FPS. **Each individual emulation frame still runs on the egui UI thread**, so it can stall briefly; a new 1.5-second per-frame deadline and instruction-progress guard now pause playback and report the frame/PC rather than allowing an unbounded loop. This is a safety net, not improved rendering correctness or background emulation.

## Reproducible early-frame evidence

The headless `starbyte-cli run` command supports `--frame-log PATH` (one flushed `starbyte.frame_log.v1` record per attempted frame, including CPU registers, PPU-write and APU-port counters), **optional `--watch-wram 7E0100,7E0094` for side-effect-free canonical WRAM bytes**, and optional bounded local framebuffer images (`--frame-images-dir DIR`, `--frame-image-every N`, `--max-frame-images N`). On a recoverable frame error, the JSONL log retains earlier completed frames and a final error record, whereas the end-of-run report may not exist. Frame images contain only successfully completed framebuffers and do not prove the artwork is rendered correctly. Use `--no-save-ram` to avoid reading or writing SRAM sidecars during controlled diagnosis. Implementation and CLI integration tests exist; do not treat the tools themselves as evidence that Super Mario World is playable. See [automation](automation.md) for example commands.

## Early-frame compatibility failures

The commercial-ROM fixture recorder now uses the core's guarded frame runner for ordinary capture and an explicit 20,000-instruction-per-frame ceiling while collecting full instruction traces. Stalled probes return a frame/CPU-address error rather than running indefinitely. This bounds instruction count, not wall-clock time or trace size across many requested frames; it does not imply that an actual commercial title is playable.

A test of a commercial game that stops within a few frames needs the exact game's name and diagnostic log before its root cause can be determined. Starbyte now surfaces a recoverable emulation error in Play View, pauses playback, and records the error in Logs; this cannot protect against native process crashes or Rust panics. A terminal backtrace is required for those. A successful native checkerboard still does not demonstrate commercial playability.

## Cheatarium integration acceptance boundary

The current desktop uses a persistent, locally selected Cheatarium checkout: both GUI refresh and read-only CLI lookup consume `generated/v1/snes.json.gz`. The obsolete Libretro cheat downloader has been removed. At launch, Starbyte refreshes local cheat candidates for installed games asynchronously if the configured index exists. Old online cheat caches are suppressed in both newly generated and previously saved snapshots.

Supported individual SNES Game Genie codes replace matching cartridge-ROM bus reads without rewriting any ROM bytes; supported individual 7E/7F WRAM writes use a separate runtime path. The Game Details view reports matching ROM-read counts, distinguishing user activation from runtime interception. This does **not** verify the claimed cheat effect or exact game build, and no commercial ROM cheat effect has been verified. Unreviewed compound codes remain read-only. The independent Cheatarium SHA-256 release registry is currently empty, so exact revision matching must not be claimed. See [Cheatarium integration](cheatarium.md) for consumer details.

## Playability acceptance gates still open

1. Boot a user-provided, legally held Super Mario World ROM and capture a stable, recognizable title image, not just a nonblack frame.
2. **Real-ROM gameplay achieved:** menus and welcome, fully rendered overworld mode $0E, smooth overworld map movement, Yoshi's Island 2 entry, rightward walking/running, in-level camera scrolling, jumping, scoring event, death, life decrement and overworld return. Next investigate item/block interactions, further level routes and saves. No additional reverse-direction investigation is needed or prioritized.
3. Integrate genuine audio synthesis and an actual Linux sound output backend; confirm sustained low-stutter playback.
4. Run the desktop natively under Wayland/Hyprland. Exercise tiling, maximize/fullscreen, focus, key/controller mappings, reconnects and gamepad input.
5. Record game-by-game outcomes in a compatibility matrix with exact ROM identity, test steps, and evidence. Keep broken/unknown outcomes visible.
6. Package versioned Linux builds only after the above core usability has been established.

**Development direction (2026-10-09): Starbyte remains active.** The latest private 3,350-frame Super Mario World retest completed without emulation errors. Right advanced Mario to X=341; Left from frame 2775 reversed movement to X=59 by frame 3050, with the camera reversing from X=235 to X=0; a subsequent Right press resumed motion. These results are sufficient to **defer any further work on reverse movement**: there is no observed reversal bug to chase. Continue the existing emulator roadmap, focusing on actual game usability, authentic audio synthesis/output, native Linux/Wayland experience and gameplay correctness. Full level completion, detailed item/collision compatibility, authentic audio, saves and native performance are still unverified. Do not turn completed probes into an open-ended series of local user QA assignments; prioritize independent code changes and CI.

See the [product roadmap](roadmap.md) for prioritized outstanding work. Git commits and GitHub Actions retain the chronological development log; this page is intentionally a *current-state description*, not a running changelog.
