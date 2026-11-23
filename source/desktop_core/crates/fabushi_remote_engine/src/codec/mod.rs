use crate::capture::CapturedFrame;
use crate::error::EngineResult;
use crate::is_headless_test_env;
use gpui_telegram_proto::remote_control_contracts::{
    ColorSubsampling, RemoteStreamSettingsDto, StreamCodec, StreamQuality,
};
use serde::{Deserialize, Serialize};

const EMBEDDED_CODEC_PROFILES_JSON: &str = include_str!("codec_profiles.json");

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CodecProfileRule {
    pub id: String,
    pub os: String,
    pub gpu_vendor: String,
    pub preferred_codecs: Vec<StreamCodec>,
    pub supports_444: bool,
    pub supports_hdr: bool,
    pub max_fps: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CodecProfileConfig {
    pub version: u32,
    pub rules: Vec<CodecProfileRule>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EncodedFramePacket {
    pub codec: StreamCodec,
    pub width: u32,
    pub height: u32,
    pub is_keyframe: bool,
    pub timestamp_ms: i64,
    pub payload: Vec<u8>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct NetworkMetricsSample {
    pub rtt_ms: u32,
    pub packet_loss_pct: f32,
    pub decode_time_ms: u32,
    pub queue_depth: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AbrDecision {
    pub target_fps: u32,
    pub quality: StreamQuality,
    pub bitrate_kbps: u32,
    pub color_mode: ColorSubsampling,
    pub downgraded: bool,
}

/// Adaptive Bitrate & FPS controller responding to RTT, packet loss, and decode latency.
#[derive(Debug, Clone)]
pub struct AbrController {
    base: RemoteStreamSettingsDto,
    current: AbrDecision,
    stable_good_samples: u32,
}

impl AbrController {
    pub fn new(initial: RemoteStreamSettingsDto) -> Self {
        let initial_fps = initial.target_fps.clamp(15, 144);
        let initial_bitrate = Self::bitrate_for_quality(initial.quality);
        let decision = AbrDecision {
            target_fps: initial_fps,
            quality: initial.quality,
            bitrate_kbps: initial_bitrate,
            color_mode: initial.color_mode,
            downgraded: false,
        };
        Self {
            base: initial,
            current: decision,
            stable_good_samples: 0,
        }
    }

    fn bitrate_for_quality(q: StreamQuality) -> u32 {
        match q {
            StreamQuality::Smooth => 2_500,
            StreamQuality::Hd | StreamQuality::Auto => 6_000,
            StreamQuality::Ultra4K => 16_000,
            StreamQuality::Original => 25_000,
        }
    }

    pub fn current_decision(&self) -> AbrDecision {
        self.current
    }

    pub fn observe(&mut self, sample: NetworkMetricsSample) -> AbrDecision {
        let severe = sample.rtt_ms >= 150
            || sample.packet_loss_pct >= 3.0
            || sample.decode_time_ms >= 35
            || sample.queue_depth >= 6;
        let moderate = sample.rtt_ms >= 80
            || sample.packet_loss_pct >= 1.0
            || sample.decode_time_ms >= 20
            || sample.queue_depth >= 3;

        if severe {
            self.stable_good_samples = 0;
            self.current = AbrDecision {
                target_fps: self.base.target_fps.min(30),
                quality: StreamQuality::Smooth,
                bitrate_kbps: 1_500,
                color_mode: ColorSubsampling::Yuv420,
                downgraded: true,
            };
        } else if moderate {
            self.stable_good_samples = 0;
            self.current = AbrDecision {
                target_fps: self.base.target_fps.min(60),
                quality: StreamQuality::Hd,
                bitrate_kbps: 4_500,
                color_mode: ColorSubsampling::Yuv420,
                downgraded: true,
            };
        } else {
            self.stable_good_samples = self.stable_good_samples.saturating_add(1);
            if !self.current.downgraded || self.stable_good_samples >= 3 {
                self.current = AbrDecision {
                    target_fps: self.base.target_fps.clamp(15, 144),
                    quality: self.base.quality,
                    bitrate_kbps: Self::bitrate_for_quality(self.base.quality),
                    color_mode: self.base.color_mode,
                    downgraded: false,
                };
            }
        }
        self.current
    }
}

#[cfg(target_os = "macos")]
mod macos_videotoolbox {
    #[link(name = "VideoToolbox", kind = "framework")]
    unsafe extern "C" {
        fn VTIsHardwareDecodeSupported(codec_type: u32) -> u8;
    }

    const K_CM_VIDEO_CODEC_TYPE_H264: u32 = u32::from_be_bytes(*b"avc1");
    const K_CM_VIDEO_CODEC_TYPE_HEVC: u32 = u32::from_be_bytes(*b"hvc1");

    pub fn supports_hardware_hevc() -> bool {
        unsafe { VTIsHardwareDecodeSupported(K_CM_VIDEO_CODEC_TYPE_HEVC) != 0 }
    }

    pub fn supports_hardware_h264() -> bool {
        unsafe { VTIsHardwareDecodeSupported(K_CM_VIDEO_CODEC_TYPE_H264) != 0 }
    }
}

pub struct VideoCodecEngine {
    config: CodecProfileConfig,
}

impl Default for VideoCodecEngine {
    fn default() -> Self {
        Self::new()
    }
}

impl VideoCodecEngine {
    pub fn new() -> Self {
        let config = serde_json::from_str(EMBEDDED_CODEC_PROFILES_JSON).unwrap_or(CodecProfileConfig {
            version: 1,
            rules: Vec::new(),
        });
        Self { config }
    }

    pub fn embedded_profiles_json() -> &'static str {
        EMBEDDED_CODEC_PROFILES_JSON
    }

    pub fn profiles(&self) -> &[CodecProfileRule] {
        &self.config.rules
    }

    pub fn videotoolbox_hardware_ready(&self) -> (bool, bool) {
        if is_headless_test_env() {
            return (false, false);
        }
        #[cfg(target_os = "macos")]
        {
            (
                macos_videotoolbox::supports_hardware_h264(),
                macos_videotoolbox::supports_hardware_hevc(),
            )
        }
        #[cfg(not(target_os = "macos"))]
        {
            (false, false)
        }
    }

    pub fn negotiate_codec(
        &self,
        requested: StreamCodec,
        gpu_vendor: Option<&str>,
        os: &str,
        hdr: bool,
        color: ColorSubsampling,
    ) -> StreamCodec {
        let vendor = gpu_vendor.unwrap_or("unknown").to_ascii_lowercase();
        let os_norm = os.to_ascii_lowercase();
        let matched = self
            .config
            .rules
            .iter()
            .find(|r| r.os == os_norm && r.gpu_vendor == vendor)
            .or_else(|| self.config.rules.iter().find(|r| r.os == os_norm));

        let Some(rule) = matched else {
            return if requested == StreamCodec::Auto {
                StreamCodec::Vp8
            } else {
                requested
            };
        };

        if requested != StreamCodec::Auto && rule.preferred_codecs.contains(&requested) {
            if hdr && !rule.supports_hdr && requested == StreamCodec::H265 {
                return StreamCodec::H264;
            }
            if color == ColorSubsampling::TrueColor444 && !rule.supports_444 {
                return StreamCodec::H264;
            }
            return requested;
        }

        if hdr && !rule.supports_hdr {
            return StreamCodec::H264;
        }
        if color == ColorSubsampling::TrueColor444 && !rule.supports_444 {
            return StreamCodec::H264;
        }

        rule.preferred_codecs
            .first()
            .copied()
            .unwrap_or(StreamCodec::JpegFallback)
    }

    pub fn encode_frame(
        &self,
        frame: &CapturedFrame,
        settings: &RemoteStreamSettingsDto,
    ) -> EngineResult<EncodedFramePacket> {
        let mut codec = self.negotiate_codec(
            settings.codec,
            None,
            std::env::consts::OS,
            settings.hdr,
            settings.color_mode,
        );
        if matches!(codec, StreamCodec::H264 | StreamCodec::H265) && !self.videotoolbox_hardware_ready().0 {
            // In headless or non-VideoToolbox runtime, fall back safely to JPEG/VP8 packet framing.
            codec = if frame.mime_type == "image/jpeg" {
                StreamCodec::JpegFallback
            } else {
                StreamCodec::Vp8
            };
        }
        Ok(EncodedFramePacket {
            codec,
            width: frame.width,
            height: frame.height,
            is_keyframe: frame.seq <= 1 || frame.seq % 60 == 0,
            timestamp_ms: frame.captured_at_ms,
            payload: frame.data.clone(),
        })
    }
}
