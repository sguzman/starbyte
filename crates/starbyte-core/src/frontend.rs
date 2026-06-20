//! Host/frontend integration traits.

use crate::apu::AudioFrame;

/// A frontend that can consume video frames.
pub trait VideoSink {
    /// Present an RGBA8 framebuffer.
    fn present_frame(&mut self, width: u32, height: u32, rgba: &[u8]);
}

/// A frontend that can consume audio samples.
pub trait AudioSink {
    /// Push an audio frame to the host.
    fn push_audio_frame(&mut self, frame: &AudioFrame);
}

/// A frontend that can provide controller state.
pub trait InputSource {
    /// Poll current button state for a controller id.
    fn poll_controller(&self, id: usize) -> crate::input::ControllerState;
}

/// Host timing abstraction shared by CLI and GUI frontends.
pub trait HostClock {
    /// Nanoseconds since an arbitrary host-defined epoch.
    fn now_nanos(&self) -> u128;

    /// Request that the host sleep until the given epoch.
    fn sleep_until(&self, target_nanos: u128);
}
