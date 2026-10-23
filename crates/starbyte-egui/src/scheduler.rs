//! Independent, single-owner emulation clock.
//!
//! This is the scheduler foundation for moving the live FrontendSession
//! out of eframe::App::update. It deliberately has no egui dependency:
//! visibility of a Wayland surface must not determine whether a game ticks.
//! Do not hand the machine itself to the UI; send commands and coalesced
//! completed frames instead. The GUI integration follows as a separate step.

use std::{
    collections::VecDeque,
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc::{self, Receiver, RecvTimeoutError, Sender, TryRecvError},
        Arc, Mutex,
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

/// Hardware NTSC frame period used by the existing player.
pub const NTSC_FRAME_INTERVAL: Duration = Duration::from_nanos(16_639_357);

/// The real emulated frame rate, measured on the worker thread rather than
/// inferred from eframe UI redraws or GPU presentation.
#[derive(Debug, Default, Clone, Copy)]
pub struct WorkerFrameStats {
    pub fps: Option<f32>,
    pub last_gap_ms: f32,
    pub hitches_40ms: u64,
}

#[derive(Default)]
struct WorkerRateWindow {
    completed: VecDeque<Instant>,
    last_gap_ms: f32,
    hitches_40ms: u64,
}

impl WorkerRateWindow {
    fn record(&mut self, now: Instant) {
        if let Some(previous) = self.completed.back() {
            let gap = now.saturating_duration_since(*previous);
            self.last_gap_ms = gap.as_secs_f32() * 1_000.0;
            if gap >= Duration::from_millis(40) {
                self.hitches_40ms = self.hitches_40ms.saturating_add(1);
            }
        }
        self.completed.push_back(now);
        self.prune(now);
    }

    fn prune(&mut self, now: Instant) {
        while self.completed.front().is_some_and(|first| {
            now.saturating_duration_since(*first) > Duration::from_secs(2)
        }) {
            self.completed.pop_front();
        }
    }

    fn stats(&mut self, now: Instant) -> WorkerFrameStats {
        self.prune(now);
        let fps = match (self.completed.front(), self.completed.back()) {
            (Some(first), Some(last)) if self.completed.len() > 1 => {
                let elapsed = last.saturating_duration_since(*first);
                (elapsed >= Duration::from_millis(250)).then(|| {
                    (self.completed.len() - 1) as f32 / elapsed.as_secs_f32()
                })
            }
            _ => None,
        };
        WorkerFrameStats {
            fps,
            last_gap_ms: self.last_gap_ms,
            hitches_40ms: self.hitches_40ms,
        }
    }
}

/// A window may lose Wayland frame callbacks before the GUI has a chance
/// to report blur. This opt-in heartbeat implements the default pause policy
/// even when the compositor stops invoking eframe::App::update entirely.
/// Crucially, when background pause is disabled, expiry never stops frames.
struct GuiHeartbeat {
    origin: Instant,
    enabled: AtomicBool,
    last_ms: AtomicU64,
}

impl GuiHeartbeat {
    const STALE_AFTER_MS: u64 = 500;

    fn new() -> Self {
        Self {
            origin: Instant::now(),
            enabled: AtomicBool::new(false),
            last_ms: AtomicU64::new(0),
        }
    }

    fn pulse(&self) {
        self.last_ms
            .store(self.origin.elapsed().as_millis() as u64, Ordering::Release);
        self.enabled.store(true, Ordering::Release);
    }

    fn is_recent(&self) -> bool {
        self.is_recent_at(self.origin.elapsed().as_millis() as u64)
    }

    fn is_recent_at(&self, now_ms: u64) -> bool {
        !self.enabled.load(Ordering::Acquire)
            || now_ms.saturating_sub(self.last_ms.load(Ordering::Acquire))
                <= Self::STALE_AFTER_MS
    }
}

/// Keep one fresh frame, never a backlog of framebuffers.
pub struct FrameMailbox<Frame> {
    latest: Mutex<Option<Frame>>,
    produced: AtomicU64,
    replaced: AtomicU64,
    fault: Mutex<Option<String>>,
    rate: Mutex<WorkerRateWindow>,
}

impl<Frame> Default for FrameMailbox<Frame> {
    fn default() -> Self {
        Self {
            latest: Mutex::new(None),
            produced: AtomicU64::new(0),
            replaced: AtomicU64::new(0),
            fault: Mutex::new(None),
            rate: Mutex::new(WorkerRateWindow::default()),
        }
    }
}

impl<Frame> FrameMailbox<Frame> {
    fn push(&self, frame: Frame) {
        let mut slot = self.latest.lock().unwrap_or_else(|poison| poison.into_inner());
        if slot.replace(frame).is_some() {
            self.replaced.fetch_add(1, Ordering::Relaxed);
        }
        self.produced.fetch_add(1, Ordering::Relaxed);
        self.rate
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .record(Instant::now());
    }

    /// Remove the newest completed frame without blocking on emulation.
    pub fn take(&self) -> Option<Frame> {
        self.latest
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .take()
    }

    pub fn produced_count(&self) -> u64 {
        self.produced.load(Ordering::Relaxed)
    }

    pub fn replaced_count(&self) -> u64 {
        self.replaced.load(Ordering::Relaxed)
    }

    pub fn worker_frame_stats(&self) -> WorkerFrameStats {
        self.rate
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .stats(Instant::now())
    }

    fn suspend_timing_window(&self) {
        let mut rate = self.rate.lock().unwrap_or_else(|poison| poison.into_inner());
        rate.completed.clear();
        rate.last_gap_ms = 0.0;
    }

    fn record_error(&self, error: String) {
        let mut slot = self.fault.lock().unwrap_or_else(|poison| poison.into_inner());
        *slot = Some(error);
    }

    pub fn take_error(&self) -> Option<String> {
        self.fault
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .take()
    }
}

enum Command<Machine> {
    Run(bool),
    Focus(bool),
    PauseWhenUnfocused(bool),
    Speed(u16),
    Invoke(
        Box<dyn FnOnce(&mut Machine) -> Result<(), String> + Send>,
        Sender<Result<(), String>>,
    ),
    Post(Box<dyn FnOnce(&mut Machine) + Send>),
    Shutdown,
}

#[derive(Clone, Copy)]
struct ClockPolicy {
    playing: bool,
    focused: bool,
    pause_when_unfocused: bool,
    speed_percent: u16,
}

impl Default for ClockPolicy {
    fn default() -> Self {
        Self {
            playing: false,
            focused: true,
            pause_when_unfocused: true,
            speed_percent: 100,
        }
    }
}

impl ClockPolicy {
    fn should_run(self) -> bool {
        self.playing && (self.focused || !self.pause_when_unfocused)
    }

    fn interval(self, base: Duration) -> Duration {
        let speed = u64::from(self.speed_percent.clamp(1, 1_600));
        let nanos = (base.as_nanos() * 100 / u128::from(speed)).max(1);
        Duration::from_nanos(u64::try_from(nanos).unwrap_or(u64::MAX))
    }
}

fn apply_command<Machine>(
    command: Command<Machine>,
    machine: &mut Machine,
    policy: &mut ClockPolicy,
) -> bool {
    match command {
        Command::Run(running) => policy.playing = running,
        Command::Focus(focused) => policy.focused = focused,
        Command::PauseWhenUnfocused(enabled) => policy.pause_when_unfocused = enabled,
        Command::Speed(speed) => policy.speed_percent = speed.clamp(1, 1_600),
        Command::Invoke(action, response) => {
            let _ = response.send(action(machine));
        }
        Command::Post(action) => action(machine),
        Command::Shutdown => return false,
    }
    true
}

/// Owns a machine in a dedicated thread, not inside the egui update loop.
///
/// UI actions that mutate a game must use `invoke`; the caller can never
/// acquire a simultaneous mutable borrow of the underlying machine.
pub struct EmulatorScheduler<Machine: Send + 'static, Frame: Send + 'static> {
    commands: Sender<Command<Machine>>,
    frames: Arc<FrameMailbox<Frame>>,
    gui_heartbeat: Arc<GuiHeartbeat>,
    join: Option<JoinHandle<Machine>>,
}

impl<Machine: Send + 'static, Frame: Send + 'static> EmulatorScheduler<Machine, Frame> {
    /// Start the clock with an owned machine and frame producer. Initially
    /// paused until Run(true), so games are never advanced before their UI
    /// has finished loading them and configuring input/audio.
    pub fn spawn(
        machine: Machine,
        base_interval: Duration,
        step: impl FnMut(&mut Machine) -> Result<Frame, String> + Send + 'static,
    ) -> std::io::Result<Self> {
        Self::spawn_with_pause_hook(machine, base_interval, step, |_| {})
    }

    /// Called exactly once when the clock transitions from running to
    /// suspended (manual pause, lost focus, or frame failure). This lets
    /// the session owner clear queued game PCM before the video stops.
    pub fn spawn_with_pause_hook(
        machine: Machine,
        base_interval: Duration,
        mut step: impl FnMut(&mut Machine) -> Result<Frame, String> + Send + 'static,
        mut on_pause: impl FnMut(&mut Machine) + Send + 'static,
    ) -> std::io::Result<Self> {
        let (sender, receiver) = mpsc::channel();
        let frames = Arc::new(FrameMailbox::default());
        let publication = Arc::clone(&frames);
        let gui_heartbeat = Arc::new(GuiHeartbeat::new());
        let worker_heartbeat = Arc::clone(&gui_heartbeat);
        let join = thread::Builder::new()
            .name("starbyte-emulation".to_owned())
            .spawn(move || {
                run_owned_clock(
                    machine,
                    receiver,
                    publication,
                    worker_heartbeat,
                    base_interval,
                    &mut step,
                    &mut on_pause,
                )
            })?;
        Ok(Self {
            commands: sender,
            frames,
            gui_heartbeat,
            join: Some(join),
        })
    }

    pub fn set_playing(&self, playing: bool) {
        let _ = self.commands.send(Command::Run(playing));
    }

    pub fn set_focused(&self, focused: bool) {
        let _ = self.commands.send(Command::Focus(focused));
    }

    pub fn set_pause_when_unfocused(&self, enabled: bool) {
        let _ = self.commands.send(Command::PauseWhenUnfocused(enabled));
    }

    /// Opt in to compositor-stall detection and acknowledge a real UI paint.
    /// The preview should call this from every update, including when unfocused.
    pub fn gui_heartbeat(&self) {
        self.gui_heartbeat.pulse();
    }

    pub fn set_speed_percent(&self, speed: u16) {
        let _ = self.commands.send(Command::Speed(speed));
    }

    /// Synchronous and serialized with frame execution. Suitable for
    /// cartridge changes, state slots, SRAM flushing and cheat updates.
    pub fn invoke(
        &self,
        action: impl FnOnce(&mut Machine) -> Result<(), String> + Send + 'static,
    ) -> Result<(), String> {
        let (reply, result) = mpsc::channel();
        self.commands
            .send(Command::Invoke(Box::new(action), reply))
            .map_err(|_| "emulation worker is not running".to_owned())?;
        result
            .recv()
            .map_err(|_| "emulation worker stopped before acknowledging command".to_owned())?
    }

    /// One-way state/input updates must never block a menu or gamepad read.
    /// The worker applies each update before its next frame.
    pub fn post(
        &self,
        action: impl FnOnce(&mut Machine) + Send + 'static,
    ) -> Result<(), String> {
        self.commands
            .send(Command::Post(Box::new(action)))
            .map_err(|_| "emulation worker is not running".to_owned())
    }

    /// Return a value from the thread that exclusively owns the emulator.
    /// Use sparingly for user-requested save/load operations, not every paint.
    pub fn transact<Value: Send + 'static>(
        &self,
        action: impl FnOnce(&mut Machine) -> Result<Value, String> + Send + 'static,
    ) -> Result<Value, String> {
        let (reply, result) = mpsc::channel();
        self.invoke(move |machine| {
            let outcome = action(machine);
            reply
                .send(outcome)
                .map_err(|_| "emulation transaction receiver closed".to_owned())
        })?;
        result
            .recv()
            .map_err(|_| "emulation transaction result not delivered".to_owned())?
    }

    pub fn take_error(&self) -> Option<String> {
        self.frames.take_error()
    }

    pub fn take_latest_frame(&self) -> Option<Frame> {
        self.frames.take()
    }

    pub fn completed_frames(&self) -> u64 {
        self.frames.produced_count()
    }

    pub fn skipped_video_frames(&self) -> u64 {
        self.frames.replaced_count()
    }

    pub fn frame_stats(&self) -> WorkerFrameStats {
        self.frames.worker_frame_stats()
    }

    /// Shut down and return the owned machine so the UI can flush its SRAM
    /// and dispose resources in their normal order.
    pub fn shutdown(mut self) -> thread::Result<Machine> {
        self.shutdown_inner()
    }

    fn shutdown_inner(&mut self) -> thread::Result<Machine> {
        let _ = self.commands.send(Command::Shutdown);
        self.join
            .take()
            .expect("scheduler shutdown requested twice")
            .join()
    }
}

impl<Machine: Send + 'static, Frame: Send + 'static> Drop for EmulatorScheduler<Machine, Frame> {
    fn drop(&mut self) {
        if self.join.is_some() {
            let _ = self.shutdown_inner();
        }
    }
}

fn run_owned_clock<Machine, Frame>(
    mut machine: Machine,
    commands: Receiver<Command<Machine>>,
    frames: Arc<FrameMailbox<Frame>>,
    gui_heartbeat: Arc<GuiHeartbeat>,
    base_interval: Duration,
    step: &mut impl FnMut(&mut Machine) -> Result<Frame, String>,
    on_pause: &mut impl FnMut(&mut Machine),
) -> Machine {
    let mut policy = ClockPolicy::default();
    let mut was_running = false;
    let mut active_speed = policy.speed_percent;
    let mut deadline = Instant::now();
    loop {
        let before = policy;
        loop {
            match commands.try_recv() {
                Ok(command) => {
                    if !apply_command(command, &mut machine, &mut policy) {
                        return machine;
                    }
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => return machine,
            }
        }
        let running = policy.playing
            && (!policy.pause_when_unfocused
                || (policy.focused && gui_heartbeat.is_recent()));
        if was_running && !running {
            on_pause(&mut machine);
            // A deliberate pause is not a gameplay hitch, and restarting
            // must not count the whole background interval as a frame gap.
            frames.suspend_timing_window();
        }
        was_running = running;
        if !running {
            deadline = Instant::now();
            match commands.recv_timeout(Duration::from_millis(50)) {
                Ok(command) => {
                    if !apply_command(command, &mut machine, &mut policy) {
                        return machine;
                    }
                }
                Err(RecvTimeoutError::Disconnected) => return machine,
                Err(RecvTimeoutError::Timeout) => {}
            }
            continue;
        }

        let interval = policy.interval(base_interval);
        if !before.should_run()
            || !was_running
            || active_speed != policy.speed_percent
        {
            // A command may arrive while waiting in recv_timeout. Comparing
            // to the policy captured at the top of *this* iteration is not
            // enough: it can already contain the changed speed. Track the
            // speed from the last truly executed scheduling interval.
            deadline = Instant::now();
            active_speed = policy.speed_percent;
        }
        let now = Instant::now();
        if now < deadline {
            match commands.recv_timeout(deadline.duration_since(now)) {
                Ok(command) => {
                    if !apply_command(command, &mut machine, &mut policy) {
                        return machine;
                    }
                }
                Err(RecvTimeoutError::Disconnected) => return machine,
                Err(RecvTimeoutError::Timeout) => {}
            }
            continue;
        }

        match step(&mut machine) {
            Ok(frame) => frames.push(frame),
            Err(error) => {
                frames.record_error(error);
                policy.playing = false;
                continue;
            }
        }
        // Never run an unbounded burst after host sleep/CPU overload.
        // Skip missed deadlines instead of fast-forwarding the game.
        deadline += interval;
        if Instant::now().saturating_duration_since(deadline) > interval {
            deadline = Instant::now() + interval;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn focus_policy_and_speed_remain_independent() {
        let mut policy = ClockPolicy::default();
        policy.playing = true;
        assert!(policy.should_run());
        policy.focused = false;
        assert!(!policy.should_run());
        policy.pause_when_unfocused = false;
        assert!(policy.should_run());
        assert_eq!(policy.interval(NTSC_FRAME_INTERVAL), NTSC_FRAME_INTERVAL);
        policy.speed_percent = 200;
        assert_eq!(policy.interval(Duration::from_millis(20)), Duration::from_millis(10));
        policy.playing = false;
        assert!(!policy.should_run());
    }

    #[test]
    fn compositor_stall_pauses_only_when_pause_policy_is_enabled() {
        let beat = GuiHeartbeat::new();
        assert!(beat.is_recent());
        beat.pulse();
        let last = beat.last_ms.load(Ordering::Acquire);
        assert!(beat.is_recent_at(last + GuiHeartbeat::STALE_AFTER_MS));
        assert!(!beat.is_recent_at(last + GuiHeartbeat::STALE_AFTER_MS + 1));

        let mut policy = ClockPolicy {
            playing: true,
            focused: true,
            ..Default::default()
        };
        assert!(!(!policy.pause_when_unfocused
            || (policy.focused
                && beat.is_recent_at(last + GuiHeartbeat::STALE_AFTER_MS + 1))));
        policy.pause_when_unfocused = false;
        assert!(!policy.pause_when_unfocused
            || (policy.focused
                && beat.is_recent_at(last + GuiHeartbeat::STALE_AFTER_MS + 1)));
    }

    #[test]
    fn mailbox_drops_obsolete_video_without_dropping_emulation() {
        let box_of_frames = FrameMailbox::default();
        box_of_frames.push(1);
        box_of_frames.push(2);
        box_of_frames.push(3);
        assert_eq!(box_of_frames.take(), Some(3));
        assert_eq!(box_of_frames.take(), None);
        assert_eq!(box_of_frames.produced_count(), 3);
        assert_eq!(box_of_frames.replaced_count(), 2);
    }

    #[test]
    fn worker_advances_without_any_ui_repaint() {
        let worker = EmulatorScheduler::spawn(
            0_u64,
            Duration::from_millis(2),
            |machine| {
                *machine += 1;
                Ok(*machine)
            },
        )
        .unwrap();
        worker.set_playing(true);
        std::thread::sleep(Duration::from_millis(45));
        assert!(worker.completed_frames() >= 2);
        let latest = worker.take_latest_frame().unwrap();
        assert!(latest >= 2);
        let final_machine = worker.shutdown().unwrap();
        assert!(final_machine >= latest);
    }

    #[test]
    fn pause_callback_runs_only_on_actual_suspend_transitions() {
        struct Counts { ticks: u32, pauses: u32 }
        let worker = EmulatorScheduler::spawn_with_pause_hook(
            Counts { ticks: 0, pauses: 0 },
            Duration::from_millis(2),
            |machine| {
                machine.ticks += 1;
                Ok(machine.ticks)
            },
            |machine| machine.pauses += 1,
        )
        .unwrap();
        worker.set_playing(true);
        std::thread::sleep(Duration::from_millis(20));
        worker.set_focused(false);
        std::thread::sleep(Duration::from_millis(20));
        worker.set_focused(false);
        std::thread::sleep(Duration::from_millis(20));
        let counts = worker.shutdown().unwrap();
        assert!(counts.ticks >= 1);
        assert_eq!(counts.pauses, 1);
    }

    #[test]
    fn deliberate_pause_clears_fps_sample_window() {
        let frames = FrameMailbox::default();
        frames.push(1);
        frames.push(2);
        frames.suspend_timing_window();
        let rate = frames.worker_frame_stats();
        assert_eq!(rate.fps, None);
        assert_eq!(rate.last_gap_ms, 0.0);
    }

    #[test]
    fn worker_transactions_and_async_input_are_ordered() {
        let worker = EmulatorScheduler::spawn(
            0_i32,
            Duration::from_millis(2),
            |machine| Ok(*machine),
        )
        .unwrap();
        worker.post(|machine| *machine = 21).unwrap();
        worker.post(|machine| *machine *= 2).unwrap();
        assert_eq!(worker.transact(|machine| Ok(*machine)).unwrap(), 42);
        assert_eq!(worker.shutdown().unwrap(), 42);
    }

    #[test]
    fn worker_reports_failure_without_losing_owned_machine() {
        let worker = EmulatorScheduler::spawn(
            0_u8,
            Duration::from_millis(2),
            |_machine| -> Result<u8, String> { Err("synthetic frame fault".to_owned()) },
        )
        .unwrap();
        worker.set_playing(true);
        let started = Instant::now();
        while worker.take_error().is_none() && started.elapsed() < Duration::from_secs(1) {
            std::thread::sleep(Duration::from_millis(5));
        }
        // No frame means the worker stopped cleanly on its error, without a
        // compositor-dependent repaint or a lost session.
        assert_eq!(worker.completed_frames(), 0);
        assert_eq!(worker.shutdown().unwrap(), 0);
    }

    #[test]
    fn worker_commands_are_serialized_with_frames() {
        let worker = EmulatorScheduler::spawn(
            0_u64,
            Duration::from_millis(5),
            |machine| {
                *machine += 1;
                Ok(*machine)
            },
        )
        .unwrap();
        worker.invoke(|machine| {
            *machine = 40;
            Ok(())
        }).unwrap();
        worker.set_playing(true);
        std::thread::sleep(Duration::from_millis(15));
        worker.set_playing(false);
        worker.invoke(|machine| {
            if *machine < 40 {
                return Err("counter went backwards".to_owned());
            }
            Ok(())
        }).unwrap();
        assert!(worker.shutdown().unwrap() >= 40);
    }
}
