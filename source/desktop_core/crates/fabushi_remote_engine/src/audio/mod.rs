use crate::error::EngineResult;
use crate::is_headless_test_env;
use parking_lot::Mutex;
use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

pub const AUDIO_SAMPLE_RATE_HZ: u32 = 48_000;
pub const AUDIO_CHANNELS: u16 = 2;
pub const AUDIO_FRAME_DURATION_MS: u32 = 20;
pub const AUDIO_SAMPLES_PER_20MS_FRAME: usize =
    (AUDIO_SAMPLE_RATE_HZ as usize * AUDIO_CHANNELS as usize * AUDIO_FRAME_DURATION_MS as usize)
        / 1000;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AudioPcmFrame {
    pub sample_rate_hz: u32,
    pub channels: u16,
    pub timestamp_ms: i64,
    pub samples: Vec<i16>,
}

impl AudioPcmFrame {
    pub fn to_le_bytes(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(self.samples.len() * 2);
        for s in &self.samples {
            out.extend_from_slice(&s.to_le_bytes());
        }
        out
    }

    pub fn from_le_bytes(
        sample_rate_hz: u32,
        channels: u16,
        timestamp_ms: i64,
        bytes: &[u8],
    ) -> Self {
        let samples = bytes
            .chunks_exact(2)
            .map(|c| i16::from_le_bytes([c[0], c[1]]))
            .collect();
        Self {
            sample_rate_hz,
            channels,
            timestamp_ms,
            samples,
        }
    }
}

#[cfg(target_os = "macos")]
mod macos_coreaudio {
    use std::ffi::c_void;
    use std::mem::size_of;

    #[repr(C)]
    #[derive(Clone, Copy)]
    struct AudioObjectPropertyAddress {
        m_selector: u32,
        m_scope: u32,
        m_element: u32,
    }

    #[link(name = "CoreAudio", kind = "framework")]
    unsafe extern "C" {
        fn AudioObjectGetPropertyData(
            in_object_id: u32,
            in_address: *const AudioObjectPropertyAddress,
            in_qualifier_data_size: u32,
            in_qualifier_data: *const c_void,
            io_data_size: *mut u32,
            out_data: *mut c_void,
        ) -> i32;
        fn AudioObjectSetPropertyData(
            in_object_id: u32,
            in_address: *const AudioObjectPropertyAddress,
            in_qualifier_data_size: u32,
            in_qualifier_data: *const c_void,
            in_data_size: u32,
            in_data: *const c_void,
        ) -> i32;
    }

    const K_AUDIO_OBJECT_SYSTEM_OBJECT: u32 = 1;
    const K_AUDIO_HARDWARE_PROPERTY_DEFAULT_OUTPUT_DEVICE: u32 = u32::from_be_bytes(*b"dOut");
    const K_AUDIO_OBJECT_PROPERTY_SCOPE_GLOBAL: u32 = u32::from_be_bytes(*b"glob");
    const K_AUDIO_OBJECT_PROPERTY_SCOPE_OUTPUT: u32 = u32::from_be_bytes(*b"outp");
    const K_AUDIO_DEVICE_PROPERTY_MUTE: u32 = u32::from_be_bytes(*b"mute");

    fn default_output_device() -> Option<u32> {
        let addr = AudioObjectPropertyAddress {
            m_selector: K_AUDIO_HARDWARE_PROPERTY_DEFAULT_OUTPUT_DEVICE,
            m_scope: K_AUDIO_OBJECT_PROPERTY_SCOPE_GLOBAL,
            m_element: 0,
        };
        let mut device_id: u32 = 0;
        let mut size = size_of::<u32>() as u32;
        let status = unsafe {
            AudioObjectGetPropertyData(
                K_AUDIO_OBJECT_SYSTEM_OBJECT,
                &addr,
                0,
                std::ptr::null(),
                &mut size,
                (&mut device_id as *mut u32).cast::<c_void>(),
            )
        };
        if status == 0 && device_id != 0 {
            Some(device_id)
        } else {
            None
        }
    }

    pub fn get_output_muted() -> Option<bool> {
        let dev = default_output_device()?;
        let addr = AudioObjectPropertyAddress {
            m_selector: K_AUDIO_DEVICE_PROPERTY_MUTE,
            m_scope: K_AUDIO_OBJECT_PROPERTY_SCOPE_OUTPUT,
            m_element: 0,
        };
        let mut muted: u32 = 0;
        let mut size = size_of::<u32>() as u32;
        let status = unsafe {
            AudioObjectGetPropertyData(
                dev,
                &addr,
                0,
                std::ptr::null(),
                &mut size,
                (&mut muted as *mut u32).cast::<c_void>(),
            )
        };
        if status == 0 {
            Some(muted != 0)
        } else {
            None
        }
    }

    pub fn set_output_muted(muted: bool) {
        let Some(dev) = default_output_device() else {
            return;
        };
        let addr = AudioObjectPropertyAddress {
            m_selector: K_AUDIO_DEVICE_PROPERTY_MUTE,
            m_scope: K_AUDIO_OBJECT_PROPERTY_SCOPE_OUTPUT,
            m_element: 0,
        };
        let val: u32 = u32::from(muted);
        unsafe {
            let _ = AudioObjectSetPropertyData(
                dev,
                &addr,
                0,
                std::ptr::null(),
                size_of::<u32>() as u32,
                (&val as *const u32).cast::<c_void>(),
            );
        }
    }
}

/// RAII guard restoring remote speaker mute state on disconnect/drop.
pub struct RemoteSpeakerMuteGuard {
    muted_flag: Arc<AtomicBool>,
    previous_muted: bool,
    apply_os_mute: bool,
}

impl Drop for RemoteSpeakerMuteGuard {
    fn drop(&mut self) {
        self.muted_flag.store(self.previous_muted, Ordering::SeqCst);
        if self.apply_os_mute && !is_headless_test_env() {
            #[cfg(target_os = "macos")]
            macos_coreaudio::set_output_muted(self.previous_muted);
        }
    }
}

pub struct AudioPipeline {
    remote_speaker_muted: Arc<AtomicBool>,
    mic_passthrough: AtomicBool,
    capture_queue: Mutex<VecDeque<AudioPcmFrame>>,
}

impl Default for AudioPipeline {
    fn default() -> Self {
        Self::new()
    }
}

impl AudioPipeline {
    pub fn new() -> Self {
        Self {
            remote_speaker_muted: Arc::new(AtomicBool::new(false)),
            mic_passthrough: AtomicBool::new(false),
            capture_queue: Mutex::new(VecDeque::with_capacity(64)),
        }
    }

    pub fn mute_remote_speaker(&self) -> EngineResult<RemoteSpeakerMuteGuard> {
        let apply_os_mute = !is_headless_test_env();
        let prev = if apply_os_mute {
            #[cfg(target_os = "macos")]
            let prev_os = macos_coreaudio::get_output_muted()
                .unwrap_or_else(|| self.remote_speaker_muted.load(Ordering::SeqCst));
            #[cfg(not(target_os = "macos"))]
            let prev_os = self.remote_speaker_muted.load(Ordering::SeqCst);
            #[cfg(target_os = "macos")]
            macos_coreaudio::set_output_muted(true);
            prev_os
        } else {
            self.remote_speaker_muted.load(Ordering::SeqCst)
        };
        self.remote_speaker_muted.store(true, Ordering::SeqCst);
        Ok(RemoteSpeakerMuteGuard {
            muted_flag: Arc::clone(&self.remote_speaker_muted),
            previous_muted: prev,
            apply_os_mute,
        })
    }

    pub fn set_mic_passthrough(&self, enabled: bool) -> EngineResult<()> {
        self.mic_passthrough.store(enabled, Ordering::SeqCst);
        Ok(())
    }

    pub fn is_mic_passthrough_enabled(&self) -> bool {
        self.mic_passthrough.load(Ordering::SeqCst)
    }

    pub fn is_remote_speaker_muted(&self) -> bool {
        self.remote_speaker_muted.load(Ordering::SeqCst)
    }

    pub fn push_capture_frame(&self, frame: AudioPcmFrame) {
        let mut q = self.capture_queue.lock();
        if q.len() >= 128 {
            q.pop_front();
        }
        q.push_back(frame);
    }

    pub fn pop_capture_frame(&self) -> Option<AudioPcmFrame> {
        self.capture_queue.lock().pop_front()
    }
}
