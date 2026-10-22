//! Independent, single-owner emulation clock.
//!
//! This is the scheduler foundation for moving the live FrontendSession
//! out of eframe::App::update. It deliberately has no egui dependency:
//! visibility of a Wayland surface must not determine whether a game ticks.
//! Do not hand the machine itself to the UI; send commands and coalesced
//! completed frames instead. The GUI integration follows as a separate step.

use std::{
    sync::{
        atomic::{AtomicU64, Ordering},
        mpsc::{self, Receiver, RecvTimeoutError, Sender, TryRecvError},
        Arc, Mutex,
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

/// Hardware NTSC frame period used by the existing player.
pub const NTSC_FRAME_INTERVAL: Duration = Duration::from_nanos(16_639_357);

/// Keep one fresh frame, never a backlog of framebuffers.
pub struct FrameMailbox<Frame> {
    latest: Mutex<Option<Frame>>,
    produced: AtomicU64,
    replaced: AtomicU64,
}

impl<Frame> Default for FrameMailbox<Frame> {
    fn default() -> Self {
        Self {
            latest: Mutex::new(None),
            produced: AtomicU64::new(0),
            replaced: AtomicU64::new(0),
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
    join: Option<JoinHandle<Machine>>,
}

impl<Machine: Send + 'static, Frame: Send + 'static> EmulatorScheduler<Machine, Frame> {
    /// Start the clock with an owned machine and frame producer. Initially
    /// paused until Run(true), so games are never advanced before their UI
    /// has finished loading them and configuring input/audio.
    pub fn spawn(
        machine: Machine,
        base_interval: Duration,
        mut step: impl FnMut(&mut Machine) -> Result<Frame, String> + Send + 'static,
    ) -> std::io::Result<Self> {
        let (sender, receiver) = mpsc::channel();
        let frames = Arc::new(FrameMailbox::default());
        let publication = Arc::clone(&frames);
        let join = thread::Builder::new()
            .name("starbyte-emulation".to_owned())
            .spawn(move || run_owned_clock(machine, receiver, publication, base_interval, &mut step))?;
        Ok(Self {
            commands: sender,
            frames,
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

    pub fn take_latest_frame(&self) -> Option<Frame> {
        self.frames.take()
    }

    pub fn completed_frames(&self) -> u64 {
        self.frames.produced_count()
    }

    pub fn skipped_video_frames(&self) -> u64 {
        self.frames.replaced_count()
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
    base_interval: Duration,
    step: &mut impl FnMut(&mut Machine) -> Result<Frame, String>,
) -> Machine {
    let mut policy = ClockPolicy::default();
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
        if !policy.should_run() {
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
        if !before.should_run() || before.speed_percent != policy.speed_percent {
            deadline = Instant::now();
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
            Err(_) => {
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
