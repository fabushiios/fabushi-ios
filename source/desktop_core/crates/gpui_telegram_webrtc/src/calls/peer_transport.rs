//! Real WebRTC RTCPeerConnection Transport & Dual Live/MockLoopback Modes
//!
//! Provides WebRtcPeerTransport wrapping RTCPeerConnection for live P2P media
//! streaming with fallback to an in-memory MockLoopback transport for deterministic CI testing.

use crate::ice::TurnToken;
use crate::media::ai_media_tap::AiMediaTap;
use crate::media::opus_codec::{
    CodecError, OpusDepacketizer, OpusPacketizer, RtpPacket, OPUS_PAYLOAD_TYPE, VP8_PAYLOAD_TYPE,
};
use crate::signaling::SignalingFrame;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::Duration;
use thiserror::Error;
use tokio::sync::{mpsc, Mutex, Notify};
use webrtc::api::interceptor_registry::register_default_interceptors;
use webrtc::api::media_engine::{MediaEngine, MIME_TYPE_OPUS, MIME_TYPE_VP8};
use webrtc::api::setting_engine::SettingEngine;
use webrtc::api::APIBuilder;
use webrtc::data_channel::data_channel_init::RTCDataChannelInit;
use webrtc::data_channel::data_channel_message::DataChannelMessage;
use webrtc::data_channel::data_channel_state::RTCDataChannelState;
use webrtc::data_channel::RTCDataChannel;
use webrtc::ice_transport::ice_candidate::RTCIceCandidateInit;
use webrtc::ice_transport::ice_server::RTCIceServer;
use webrtc::interceptor::registry::Registry;
use webrtc::media::Sample;
use webrtc::peer_connection::configuration::RTCConfiguration;
use webrtc::peer_connection::peer_connection_state::RTCPeerConnectionState;
use webrtc::peer_connection::sdp::session_description::RTCSessionDescription;
use webrtc::peer_connection::RTCPeerConnection;
use webrtc::rtp::codecs::vp8::Vp8Packet;
use webrtc::rtp::packetizer::Depacketizer as _;
use webrtc::rtp_transceiver::rtp_codec::{
    RTCRtpCodecCapability, RTCRtpCodecParameters, RTPCodecType,
};
use webrtc::track::track_local::track_local_static_rtp::TrackLocalStaticRTP;
use webrtc::track::track_local::track_local_static_sample::TrackLocalStaticSample;
use webrtc::track::track_local::{TrackLocal, TrackLocalWriter};

const LOOPBACK_CHANNEL_CAPACITY: usize = 1024;
const LIVE_FRAME_MAGIC: &[u8; 4] = b"GVF1";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TransportMode {
    Live,
    MockLoopback,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PeerConnectionState {
    New,
    Connecting,
    Connected,
    Disconnected,
    Failed,
    Closed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PeerTransportConfig {
    pub mode: TransportMode,
    pub ice_servers: Vec<TurnToken>,
    pub enable_audio: bool,
    pub enable_video: bool,
    pub audio_sample_rate: u32,
    pub audio_channels: u16,
    pub video_width: u32,
    pub video_height: u32,
    pub video_fps: u32,
}

impl Default for PeerTransportConfig {
    fn default() -> Self {
        Self {
            mode: TransportMode::MockLoopback,
            ice_servers: Vec::new(),
            enable_audio: true,
            enable_video: true,
            audio_sample_rate: 48_000,
            audio_channels: 1,
            video_width: 640,
            video_height: 360,
            video_fps: 30,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VideoFrameData {
    pub width: u32,
    pub height: u32,
    pub timestamp_ms: u64,
    pub data: Vec<u8>,
}

#[derive(Debug, Error)]
pub enum WebRtcTransportError {
    #[error("WebRTC internal error: {0}")]
    Internal(String),
    #[error("Invalid transport state: {0}")]
    InvalidState(String),
    #[error("Codec error: {0}")]
    Codec(#[from] CodecError),
    #[error("Channel closed")]
    ChannelClosed,
}

pub type AudioFrameCallback = Arc<dyn Fn(&[i16], u32, u16) + Send + Sync>;
pub type VideoFrameCallback = Arc<dyn Fn(&VideoFrameData) + Send + Sync>;
pub type IceCandidateCallback = Arc<dyn Fn(SignalingFrame) + Send + Sync>;
pub type StateChangeCallback = Arc<dyn Fn(PeerConnectionState) + Send + Sync>;
pub type DataChannelMessageCallback = Arc<dyn Fn(&str, &[u8], bool) + Send + Sync>;
pub type DataChannelOpenCallback = Arc<dyn Fn(&str) + Send + Sync>;

/// Reliability profile for an `RTCDataChannel`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DataChannelReliability {
    /// Ordered and reliable delivery (control RPC, terminal, clipboard, file chunks).
    ReliableOrdered,
    /// Unordered, zero-retransmit delivery (low-latency mouse deltas and raw frame packets).
    UnreliableUnordered,
}

/// Payload delivered over a `MockLoopback` or `RTCDataChannel` pipe.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DataChannelPayload {
    pub label: String,
    pub is_string: bool,
    pub data: Vec<u8>,
}

/// Standard DataChannel label for reliable control/input/terminal/clipboard JSON frames.
pub const CONTROL_DATA_CHANNEL_LABEL: &str = "control";
/// Standard DataChannel label for unreliable low-latency pointer deltas.
pub const INPUT_FAST_DATA_CHANNEL_LABEL: &str = "input-fast";
/// Standard DataChannel label for low-latency remote desktop video frame chunks.
pub const MEDIA_FRAME_DATA_CHANNEL_LABEL: &str = "media-frame";

fn encode_live_video_payload(frame: &VideoFrameData) -> Vec<u8> {
    let mut out = Vec::with_capacity(24 + frame.data.len());
    out.extend_from_slice(LIVE_FRAME_MAGIC);
    out.extend_from_slice(&frame.width.to_le_bytes());
    out.extend_from_slice(&frame.height.to_le_bytes());
    out.extend_from_slice(&frame.timestamp_ms.to_le_bytes());
    out.extend_from_slice(&(frame.data.len() as u32).to_le_bytes());
    out.extend_from_slice(&frame.data);
    out
}

fn decode_live_video_payload(bytes: &[u8]) -> Option<VideoFrameData> {
    if bytes.len() < 24 || &bytes[0..4] != LIVE_FRAME_MAGIC {
        return None;
    }
    let width = u32::from_le_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]);
    let height = u32::from_le_bytes([bytes[8], bytes[9], bytes[10], bytes[11]]);
    let timestamp_ms = u64::from_le_bytes([
        bytes[12], bytes[13], bytes[14], bytes[15], bytes[16], bytes[17], bytes[18], bytes[19],
    ]);
    let data_len = u32::from_le_bytes([bytes[20], bytes[21], bytes[22], bytes[23]]) as usize;
    if bytes.len() < 24 + data_len {
        return None;
    }
    Some(VideoFrameData {
        width,
        height,
        timestamp_ms,
        data: bytes[24..24 + data_len].to_vec(),
    })
}

pub struct WebRtcPeerTransport {
    config: PeerTransportConfig,
    mode: TransportMode,
    state: Arc<parking_lot::RwLock<PeerConnectionState>>,

    // Media & Codec (decoupled interior locks so TX and RX never deadlock)
    packetizer: Arc<parking_lot::Mutex<OpusPacketizer>>,
    depacketizer: Arc<parking_lot::Mutex<OpusDepacketizer>>,
    ai_media_tap: Arc<parking_lot::RwLock<Option<Arc<dyn AiMediaTap>>>>,

    // Callbacks (shared with live RTCPeerConnection async handlers)
    on_audio: Arc<parking_lot::RwLock<Option<AudioFrameCallback>>>,
    on_video: Arc<parking_lot::RwLock<Option<VideoFrameCallback>>>,
    on_ice_candidate: Arc<parking_lot::RwLock<Option<IceCandidateCallback>>>,
    on_state_change: Arc<parking_lot::RwLock<Option<StateChangeCallback>>>,
    on_data_channel_message: Arc<parking_lot::RwLock<Option<DataChannelMessageCallback>>>,
    on_data_channel_open: Arc<parking_lot::RwLock<Option<DataChannelOpenCallback>>>,

    // MockLoopback decoupled senders & receivers
    loopback_audio_tx: Arc<parking_lot::RwLock<Option<mpsc::Sender<RtpPacket>>>>,
    loopback_video_tx: Arc<parking_lot::RwLock<Option<mpsc::Sender<VideoFrameData>>>>,
    loopback_dc_tx: Arc<parking_lot::RwLock<Option<mpsc::Sender<DataChannelPayload>>>>,
    loopback_audio_rx: Arc<Mutex<Option<mpsc::Receiver<RtpPacket>>>>,
    loopback_video_rx: Arc<Mutex<Option<mpsc::Receiver<VideoFrameData>>>>,
    loopback_dc_rx: Arc<Mutex<Option<mpsc::Receiver<DataChannelPayload>>>>,
    loopback_rx_handle: Arc<parking_lot::Mutex<Option<tokio::task::AbortHandle>>>,
    loopback_open_channels: Arc<parking_lot::RwLock<HashSet<String>>>,

    // Live WebRTC instance, local media tracks, and DataChannels
    peer_connection: Option<Arc<RTCPeerConnection>>,
    live_audio_track: Option<Arc<TrackLocalStaticRTP>>,
    live_video_track: Option<Arc<TrackLocalStaticSample>>,
    data_channels: Arc<parking_lot::RwLock<HashMap<String, Arc<RTCDataChannel>>>>,
    data_channel_reliability: Arc<parking_lot::RwLock<HashMap<String, DataChannelReliability>>>,
    dc_open_notify: Arc<Notify>,
}

impl WebRtcPeerTransport {
    pub async fn new(config: PeerTransportConfig) -> Result<Self, WebRtcTransportError> {
        let channels = config.audio_channels;
        let mode = config.mode;
        let mut transport = Self {
            config: config.clone(),
            mode,
            state: Arc::new(parking_lot::RwLock::new(PeerConnectionState::New)),
            packetizer: Arc::new(parking_lot::Mutex::new(OpusPacketizer::new(
                channels, 0x12345678,
            ))),
            depacketizer: Arc::new(parking_lot::Mutex::new(OpusDepacketizer::new(channels))),
            ai_media_tap: Arc::new(parking_lot::RwLock::new(None)),
            on_audio: Arc::new(parking_lot::RwLock::new(None)),
            on_video: Arc::new(parking_lot::RwLock::new(None)),
            on_ice_candidate: Arc::new(parking_lot::RwLock::new(None)),
            on_state_change: Arc::new(parking_lot::RwLock::new(None)),
            on_data_channel_message: Arc::new(parking_lot::RwLock::new(None)),
            on_data_channel_open: Arc::new(parking_lot::RwLock::new(None)),
            loopback_audio_tx: Arc::new(parking_lot::RwLock::new(None)),
            loopback_video_tx: Arc::new(parking_lot::RwLock::new(None)),
            loopback_dc_tx: Arc::new(parking_lot::RwLock::new(None)),
            loopback_audio_rx: Arc::new(Mutex::new(None)),
            loopback_video_rx: Arc::new(Mutex::new(None)),
            loopback_dc_rx: Arc::new(Mutex::new(None)),
            loopback_rx_handle: Arc::new(parking_lot::Mutex::new(None)),
            loopback_open_channels: Arc::new(parking_lot::RwLock::new(HashSet::new())),
            peer_connection: None,
            live_audio_track: None,
            live_video_track: None,
            data_channels: Arc::new(parking_lot::RwLock::new(HashMap::new())),
            data_channel_reliability: Arc::new(parking_lot::RwLock::new(HashMap::new())),
            dc_open_notify: Arc::new(Notify::new()),
        };

        if mode == TransportMode::Live {
            transport.init_live_webrtc().await?;
        }

        Ok(transport)
    }

    async fn init_live_webrtc(&mut self) -> Result<(), WebRtcTransportError> {
        static INIT_RUSTLS: std::sync::Once = std::sync::Once::new();
        INIT_RUSTLS.call_once(|| {
            if rustls::crypto::CryptoProvider::get_default().is_none() {
                let _ = rustls::crypto::ring::default_provider().install_default();
            }
        });

        let mut media_engine = MediaEngine::default();
        let audio_channels = self.config.audio_channels.clamp(1, 2);

        // Register Opus Audio codec (PT 111, clock rate 48000, configured channels)
        media_engine
            .register_codec(
                RTCRtpCodecParameters {
                    capability: RTCRtpCodecCapability {
                        mime_type: MIME_TYPE_OPUS.to_string(),
                        clock_rate: 48_000,
                        channels: audio_channels,
                        sdp_fmtp_line: "minptime=10;useinbandfec=1".to_string(),
                        rtcp_feedback: vec![],
                    },
                    payload_type: OPUS_PAYLOAD_TYPE,
                    ..Default::default()
                },
                RTPCodecType::Audio,
            )
            .map_err(|e| WebRtcTransportError::Internal(e.to_string()))?;

        // Register VP8 Video codec (PT 96, clock rate 90000)
        media_engine
            .register_codec(
                RTCRtpCodecParameters {
                    capability: RTCRtpCodecCapability {
                        mime_type: MIME_TYPE_VP8.to_string(),
                        clock_rate: 90_000,
                        channels: 0,
                        sdp_fmtp_line: "".to_string(),
                        rtcp_feedback: vec![],
                    },
                    payload_type: VP8_PAYLOAD_TYPE,
                    ..Default::default()
                },
                RTPCodecType::Video,
            )
            .map_err(|e| WebRtcTransportError::Internal(e.to_string()))?;

        let mut registry = Registry::new();
        registry = register_default_interceptors(registry, &mut media_engine)
            .map_err(|e| WebRtcTransportError::Internal(e.to_string()))?;

        let mut setting_engine = SettingEngine::default();
        setting_engine.set_receive_mtu(8192);

        let api = APIBuilder::new()
            .with_media_engine(media_engine)
            .with_interceptor_registry(registry)
            .with_setting_engine(setting_engine)
            .build();

        let ice_servers: Vec<RTCIceServer> = self
            .config
            .ice_servers
            .iter()
            .map(|t| RTCIceServer {
                urls: t.uris.clone(),
                username: t.username.clone(),
                credential: t.credential.clone(),
                ..Default::default()
            })
            .collect();

        let rtc_config = RTCConfiguration {
            ice_servers,
            ..Default::default()
        };

        let pc = Arc::new(
            api.new_peer_connection(rtc_config)
                .await
                .map_err(|e| WebRtcTransportError::Internal(e.to_string()))?,
        );

        // Attach local Opus audio track (PT 111)
        if self.config.enable_audio {
            let audio_track = Arc::new(TrackLocalStaticRTP::new(
                RTCRtpCodecCapability {
                    mime_type: MIME_TYPE_OPUS.to_string(),
                    clock_rate: 48_000,
                    channels: audio_channels,
                    sdp_fmtp_line: "minptime=10;useinbandfec=1".to_string(),
                    rtcp_feedback: vec![],
                },
                "audio".to_string(),
                "gpui-telegram-stream".to_string(),
            ));
            pc.add_track(Arc::clone(&audio_track) as Arc<dyn TrackLocal + Send + Sync>)
                .await
                .map_err(|e| WebRtcTransportError::Internal(e.to_string()))?;
            self.live_audio_track = Some(audio_track);
        }

        // Attach local VP8 video track (PT 96)
        if self.config.enable_video {
            let video_track = Arc::new(TrackLocalStaticSample::new(
                RTCRtpCodecCapability {
                    mime_type: MIME_TYPE_VP8.to_string(),
                    clock_rate: 90_000,
                    channels: 0,
                    sdp_fmtp_line: "".to_string(),
                    rtcp_feedback: vec![],
                },
                "video".to_string(),
                "gpui-telegram-stream".to_string(),
            ));
            pc.add_track(Arc::clone(&video_track) as Arc<dyn TrackLocal + Send + Sync>)
                .await
                .map_err(|e| WebRtcTransportError::Internal(e.to_string()))?;
            self.live_video_track = Some(video_track);
        }

        // Wire ICE candidate discovery callback
        let ice_cb_arc = Arc::clone(&self.on_ice_candidate);
        pc.on_ice_candidate(Box::new(move |candidate| {
            let ice_cb_arc = Arc::clone(&ice_cb_arc);
            Box::pin(async move {
                if let Some(cand) = candidate {
                    if let Ok(init) = cand.to_json() {
                        if let Some(cb) = ice_cb_arc.read().clone() {
                            cb(SignalingFrame::IceCandidate {
                                call_id: String::new(),
                                candidate: init.candidate,
                                sdp_mid: init.sdp_mid,
                                sdp_mline_index: init.sdp_mline_index,
                            });
                        }
                    }
                }
            })
        }));

        // Wire PeerConnection state change callback
        let state_arc = Arc::clone(&self.state);
        let state_cb_arc = Arc::clone(&self.on_state_change);
        pc.on_peer_connection_state_change(Box::new(move |s: RTCPeerConnectionState| {
            let mapped = match s {
                RTCPeerConnectionState::New => PeerConnectionState::New,
                RTCPeerConnectionState::Connecting => PeerConnectionState::Connecting,
                RTCPeerConnectionState::Connected => PeerConnectionState::Connected,
                RTCPeerConnectionState::Disconnected => PeerConnectionState::Disconnected,
                RTCPeerConnectionState::Failed => PeerConnectionState::Failed,
                RTCPeerConnectionState::Closed => PeerConnectionState::Closed,
                RTCPeerConnectionState::Unspecified => PeerConnectionState::New,
            };
            *state_arc.write() = mapped;
            if let Some(cb) = state_cb_arc.read().clone() {
                cb(mapped);
            }
            Box::pin(async {})
        }));

        // Wire incoming remote RTP track reader
        let depacketizer = Arc::clone(&self.depacketizer);
        let ai_tap = Arc::clone(&self.ai_media_tap);
        let on_audio = Arc::clone(&self.on_audio);
        let on_video = Arc::clone(&self.on_video);
        let sample_rate = self.config.audio_sample_rate;
        let channels = self.config.audio_channels;
        pc.on_track(Box::new(move |track, _receiver, _transceiver| {
            let depacketizer = Arc::clone(&depacketizer);
            let ai_tap = Arc::clone(&ai_tap);
            let on_audio = Arc::clone(&on_audio);
            let on_video = Arc::clone(&on_video);
            Box::pin(async move {
                let kind = track.kind();
                tokio::spawn(async move {
                    if kind == RTPCodecType::Audio {
                        while let Ok((rtp_pkt, _attr)) = track.read_rtp().await {
                            let pkt = RtpPacket {
                                version: rtp_pkt.header.version,
                                padding: rtp_pkt.header.padding,
                                extension: rtp_pkt.header.extension,
                                csrc_count: rtp_pkt.header.csrc.len() as u8,
                                marker: rtp_pkt.header.marker,
                                payload_type: rtp_pkt.header.payload_type,
                                sequence_number: rtp_pkt.header.sequence_number,
                                timestamp: rtp_pkt.header.timestamp,
                                ssrc: rtp_pkt.header.ssrc,
                                payload: rtp_pkt.payload.to_vec(),
                            };
                            let decoded = {
                                let mut dep = depacketizer.lock();
                                dep.depacketize(&pkt).ok()
                            };
                            if let Some(samples) = decoded {
                                if let Some(tap) = ai_tap.read().clone() {
                                    tap.on_incoming_pcm_frame(&samples, sample_rate, channels);
                                }
                                if let Some(cb) = on_audio.read().clone() {
                                    cb(&samples, sample_rate, channels);
                                }
                            }
                        }
                    } else if kind == RTPCodecType::Video {
                        const MAX_LIVE_VIDEO_FRAME_BYTES: usize = 4 * 1024 * 1024;
                        let mut vp8_dep = Vp8Packet::default();
                        let mut frame_buf: Vec<u8> = Vec::new();
                        let mut drop_until_start = false;
                        while let Ok((rtp_pkt, _attr)) = track.read_rtp().await {
                            if let Ok(chunk) = vp8_dep.depacketize(&rtp_pkt.payload) {
                                if vp8_dep.s == 1 {
                                    frame_buf.clear();
                                    drop_until_start = false;
                                }
                                if drop_until_start {
                                    continue;
                                }
                                if frame_buf.len().saturating_add(chunk.len()) > MAX_LIVE_VIDEO_FRAME_BYTES {
                                    frame_buf.clear();
                                    drop_until_start = true;
                                    continue;
                                }
                                frame_buf.extend_from_slice(&chunk);
                                if rtp_pkt.header.marker {
                                    if let Some(frame) = decode_live_video_payload(&frame_buf) {
                                        if let Some(cb) = on_video.read().clone() {
                                            cb(&frame);
                                        }
                                    }
                                    frame_buf.clear();
                                }
                            }
                        }
                    }
                });
            })
        }));

        // Wire incoming remote RTCDataChannel events (reliable control + unreliable input/frame channels)
        let dc_map = Arc::clone(&self.data_channels);
        let dc_rel = Arc::clone(&self.data_channel_reliability);
        let on_dc_msg = Arc::clone(&self.on_data_channel_message);
        let on_dc_open = Arc::clone(&self.on_data_channel_open);
        let dc_notify = Arc::clone(&self.dc_open_notify);
        pc.on_data_channel(Box::new(move |dc: Arc<RTCDataChannel>| {
            let dc_map = Arc::clone(&dc_map);
            let dc_rel = Arc::clone(&dc_rel);
            let on_dc_msg = Arc::clone(&on_dc_msg);
            let on_dc_open = Arc::clone(&on_dc_open);
            let dc_notify = Arc::clone(&dc_notify);
            Box::pin(async move {
                let rel = if dc.ordered() {
                    DataChannelReliability::ReliableOrdered
                } else {
                    DataChannelReliability::UnreliableUnordered
                };
                Self::wire_live_data_channel(
                    dc,
                    rel,
                    dc_map,
                    dc_rel,
                    on_dc_msg,
                    on_dc_open,
                    dc_notify,
                );
            })
        }));

        self.peer_connection = Some(pc);
        self.set_state(PeerConnectionState::New);

        Ok(())
    }

    fn wire_live_data_channel(
        dc: Arc<RTCDataChannel>,
        reliability: DataChannelReliability,
        channels_map: Arc<parking_lot::RwLock<HashMap<String, Arc<RTCDataChannel>>>>,
        reliability_map: Arc<parking_lot::RwLock<HashMap<String, DataChannelReliability>>>,
        on_msg: Arc<parking_lot::RwLock<Option<DataChannelMessageCallback>>>,
        on_open: Arc<parking_lot::RwLock<Option<DataChannelOpenCallback>>>,
        open_notify: Arc<Notify>,
    ) {
        let label = dc.label().to_string();
        channels_map.write().insert(label.clone(), Arc::clone(&dc));
        reliability_map.write().insert(label.clone(), reliability);

        let open_label = label.clone();
        let on_open_cb = Arc::clone(&on_open);
        let open_notify_cb = Arc::clone(&open_notify);
        dc.on_open(Box::new(move || {
            let lbl = open_label.clone();
            let cb_arc = Arc::clone(&on_open_cb);
            let notify = Arc::clone(&open_notify_cb);
            Box::pin(async move {
                notify.notify_waiters();
                if let Some(cb) = cb_arc.read().clone() {
                    cb(&lbl);
                }
            })
        }));

        let msg_label = label.clone();
        let on_msg_cb = Arc::clone(&on_msg);
        dc.on_message(Box::new(move |msg: DataChannelMessage| {
            let lbl = msg_label.clone();
            let cb_arc = Arc::clone(&on_msg_cb);
            Box::pin(async move {
                if let Some(cb) = cb_arc.read().clone() {
                    cb(&lbl, &msg.data, msg.is_string);
                }
            })
        }));

        let close_label = label;
        let channels_close = Arc::clone(&channels_map);
        dc.on_close(Box::new(move || {
            let lbl = close_label.clone();
            let map = Arc::clone(&channels_close);
            Box::pin(async move {
                map.write().remove(&lbl);
            })
        }));
    }

    fn build_loopback_half(
        cfg: PeerTransportConfig,
        ssrc: u32,
        audio_tx: mpsc::Sender<RtpPacket>,
        video_tx: mpsc::Sender<VideoFrameData>,
        dc_tx: mpsc::Sender<DataChannelPayload>,
        audio_rx: mpsc::Receiver<RtpPacket>,
        video_rx: mpsc::Receiver<VideoFrameData>,
        dc_rx: mpsc::Receiver<DataChannelPayload>,
    ) -> Self {
        let channels = cfg.audio_channels;
        Self {
            config: cfg,
            mode: TransportMode::MockLoopback,
            state: Arc::new(parking_lot::RwLock::new(PeerConnectionState::Connected)),
            packetizer: Arc::new(parking_lot::Mutex::new(OpusPacketizer::new(channels, ssrc))),
            depacketizer: Arc::new(parking_lot::Mutex::new(OpusDepacketizer::new(channels))),
            ai_media_tap: Arc::new(parking_lot::RwLock::new(None)),
            on_audio: Arc::new(parking_lot::RwLock::new(None)),
            on_video: Arc::new(parking_lot::RwLock::new(None)),
            on_ice_candidate: Arc::new(parking_lot::RwLock::new(None)),
            on_state_change: Arc::new(parking_lot::RwLock::new(None)),
            on_data_channel_message: Arc::new(parking_lot::RwLock::new(None)),
            on_data_channel_open: Arc::new(parking_lot::RwLock::new(None)),
            loopback_audio_tx: Arc::new(parking_lot::RwLock::new(Some(audio_tx))),
            loopback_video_tx: Arc::new(parking_lot::RwLock::new(Some(video_tx))),
            loopback_dc_tx: Arc::new(parking_lot::RwLock::new(Some(dc_tx))),
            loopback_audio_rx: Arc::new(Mutex::new(Some(audio_rx))),
            loopback_video_rx: Arc::new(Mutex::new(Some(video_rx))),
            loopback_dc_rx: Arc::new(Mutex::new(Some(dc_rx))),
            loopback_rx_handle: Arc::new(parking_lot::Mutex::new(None)),
            loopback_open_channels: Arc::new(parking_lot::RwLock::new(HashSet::new())),
            peer_connection: None,
            live_audio_track: None,
            live_video_track: None,
            data_channels: Arc::new(parking_lot::RwLock::new(HashMap::new())),
            data_channel_reliability: Arc::new(parking_lot::RwLock::new(HashMap::new())),
            dc_open_notify: Arc::new(Notify::new()),
        }
    }

    /// Creates a connected pair of transports in MockLoopback mode for headless CI testing
    pub fn create_loopback_pair(config: PeerTransportConfig) -> (Self, Self) {
        let mut cfg = config;
        cfg.mode = TransportMode::MockLoopback;

        let (a_audio_tx, a_audio_rx) = mpsc::channel::<RtpPacket>(LOOPBACK_CHANNEL_CAPACITY);
        let (a_video_tx, a_video_rx) = mpsc::channel::<VideoFrameData>(LOOPBACK_CHANNEL_CAPACITY);
        let (a_dc_tx, a_dc_rx) = mpsc::channel::<DataChannelPayload>(LOOPBACK_CHANNEL_CAPACITY);

        let (b_audio_tx, b_audio_rx) = mpsc::channel::<RtpPacket>(LOOPBACK_CHANNEL_CAPACITY);
        let (b_video_tx, b_video_rx) = mpsc::channel::<VideoFrameData>(LOOPBACK_CHANNEL_CAPACITY);
        let (b_dc_tx, b_dc_rx) = mpsc::channel::<DataChannelPayload>(LOOPBACK_CHANNEL_CAPACITY);

        let transport_a = Self::build_loopback_half(
            cfg.clone(),
            0x11111111,
            b_audio_tx,
            b_video_tx,
            b_dc_tx,
            a_audio_rx,
            a_video_rx,
            a_dc_rx,
        );
        let transport_b = Self::build_loopback_half(
            cfg,
            0x22222222,
            a_audio_tx,
            a_video_tx,
            a_dc_tx,
            b_audio_rx,
            b_video_rx,
            b_dc_rx,
        );

        (transport_a, transport_b)
    }

    /// Takes ownership of the loopback receiver channels for independent async pump tasks
    pub async fn take_loopback_receivers(
        &self,
    ) -> (
        Option<mpsc::Receiver<RtpPacket>>,
        Option<mpsc::Receiver<VideoFrameData>>,
    ) {
        let audio_rx = self.loopback_audio_rx.lock().await.take();
        let video_rx = self.loopback_video_rx.lock().await.take();
        (audio_rx, video_rx)
    }

    /// Spawns a background loopback receiver task on this transport (safe with concurrent sends)
    pub async fn spawn_loopback_receiver(this: Arc<Self>) -> Option<tokio::task::JoinHandle<()>> {
        let (audio_rx_opt, video_rx_opt) = this.take_loopback_receivers().await;
        let dc_rx_opt = this.loopback_dc_rx.lock().await.take();
        if audio_rx_opt.is_none() && video_rx_opt.is_none() && dc_rx_opt.is_none() {
            return None;
        }
        let depacketizer = Arc::clone(&this.depacketizer);
        let ai_tap = Arc::clone(&this.ai_media_tap);
        let on_audio = Arc::clone(&this.on_audio);
        let on_video = Arc::clone(&this.on_video);
        let on_dc = Arc::clone(&this.on_data_channel_message);
        let sample_rate = this.config.audio_sample_rate;
        let channels = this.config.audio_channels;

        let handle = tokio::spawn(async move {
            let mut audio_rx = audio_rx_opt;
            let mut video_rx = video_rx_opt;
            let mut dc_rx = dc_rx_opt;
            loop {
                tokio::select! {
                    Some(rtp) = async {
                        match audio_rx.as_mut() {
                            Some(rx) => rx.recv().await,
                            None => std::future::pending().await,
                        }
                    } => {
                        let samples = {
                            let mut dep = depacketizer.lock();
                            dep.depacketize(&rtp).ok()
                        };
                        if let Some(samples) = samples {
                            if let Some(tap) = ai_tap.read().clone() {
                                tap.on_incoming_pcm_frame(&samples, sample_rate, channels);
                            }
                            if let Some(cb) = on_audio.read().clone() {
                                cb(&samples, sample_rate, channels);
                            }
                        }
                    }
                    Some(frame) = async {
                        match video_rx.as_mut() {
                            Some(rx) => rx.recv().await,
                            None => std::future::pending().await,
                        }
                    } => {
                        if let Some(cb) = on_video.read().clone() {
                            cb(&frame);
                        }
                    }
                    Some(dc_msg) = async {
                        match dc_rx.as_mut() {
                            Some(rx) => rx.recv().await,
                            None => std::future::pending().await,
                        }
                    } => {
                        if let Some(cb) = on_dc.read().clone() {
                            cb(&dc_msg.label, &dc_msg.data, dc_msg.is_string);
                        }
                    }
                    else => break,
                }
            }
        });
        *this.loopback_rx_handle.lock() = Some(handle.abort_handle());
        Some(handle)
    }

    /// Receives next audio frame from MockLoopback channel, depacketizes it, and dispatches callbacks.
    /// Uses decoupled interior locks so concurrent `send_audio_pcm` / `send_video_frame` / `recv_loopback_video` never deadlock.
    pub async fn recv_loopback_audio(&self) -> Result<Vec<i16>, WebRtcTransportError> {
        let packet = {
            let mut rx_guard = self.loopback_audio_rx.lock().await;
            let rx = rx_guard
                .as_mut()
                .ok_or(WebRtcTransportError::ChannelClosed)?;
            rx.recv().await
        };
        if let Some(packet) = packet {
            let samples = {
                let mut dep = self.depacketizer.lock();
                dep.depacketize(&packet)?
            };
            if let Some(tap) = self.ai_media_tap.read().clone() {
                tap.on_incoming_pcm_frame(
                    &samples,
                    self.config.audio_sample_rate,
                    self.config.audio_channels,
                );
            }
            if let Some(cb) = self.on_audio.read().clone() {
                cb(
                    &samples,
                    self.config.audio_sample_rate,
                    self.config.audio_channels,
                );
            }
            return Ok(samples);
        }
        Err(WebRtcTransportError::ChannelClosed)
    }

    /// Receives next video frame from MockLoopback channel and dispatches callback.
    /// Uses decoupled interior locks so concurrent `send_audio_pcm` / `send_video_frame` / `recv_loopback_audio` never deadlock.
    pub async fn recv_loopback_video(&self) -> Result<VideoFrameData, WebRtcTransportError> {
        let frame = {
            let mut rx_guard = self.loopback_video_rx.lock().await;
            let rx = rx_guard
                .as_mut()
                .ok_or(WebRtcTransportError::ChannelClosed)?;
            rx.recv().await
        };
        if let Some(frame) = frame {
            if let Some(cb) = self.on_video.read().clone() {
                cb(&frame);
            }
            return Ok(frame);
        }
        Err(WebRtcTransportError::ChannelClosed)
    }

    /// Spawns the loopback receiver task to dispatch incoming packets to registered callbacks
    pub fn start_loopback_receiver(
        this: Arc<Mutex<Self>>,
        mut audio_rx: mpsc::Receiver<RtpPacket>,
        mut video_rx: mpsc::Receiver<VideoFrameData>,
    ) -> tokio::task::JoinHandle<()> {
        tokio::spawn(async move {
            loop {
                tokio::select! {
                    Some(rtp) = audio_rx.recv() => {
                        let (depacketizer, ai_tap, on_audio, sample_rate, channels) = {
                            let guard = this.lock().await;
                            (
                                Arc::clone(&guard.depacketizer),
                                Arc::clone(&guard.ai_media_tap),
                                Arc::clone(&guard.on_audio),
                                guard.config.audio_sample_rate,
                                guard.config.audio_channels,
                            )
                        };
                        let samples = {
                            let mut dep = depacketizer.lock();
                            dep.depacketize(&rtp).ok()
                        };
                        if let Some(samples) = samples {
                            if let Some(tap) = ai_tap.read().clone() {
                                tap.on_incoming_pcm_frame(&samples, sample_rate, channels);
                            }
                            if let Some(cb) = on_audio.read().clone() {
                                cb(&samples, sample_rate, channels);
                            }
                        }
                    }
                    Some(frame) = video_rx.recv() => {
                        let on_video = {
                            let guard = this.lock().await;
                            Arc::clone(&guard.on_video)
                        };
                        if let Some(cb) = on_video.read().clone() {
                            cb(&frame);
                        }
                    }
                    else => break,
                }
            }
        })
    }

    /// Receives next DataChannel message from MockLoopback channel and dispatches callback.
    pub async fn recv_loopback_data_channel(
        &self,
    ) -> Result<DataChannelPayload, WebRtcTransportError> {
        let payload = {
            let mut rx_guard = self.loopback_dc_rx.lock().await;
            let rx = rx_guard
                .as_mut()
                .ok_or(WebRtcTransportError::ChannelClosed)?;
            rx.recv().await
        };
        if let Some(msg) = payload {
            if let Some(cb) = self.on_data_channel_message.read().clone() {
                cb(&msg.label, &msg.data, msg.is_string);
            }
            return Ok(msg);
        }
        Err(WebRtcTransportError::ChannelClosed)
    }

    pub fn set_ai_media_tap(&self, tap: Arc<dyn AiMediaTap>) {
        *self.ai_media_tap.write() = Some(tap);
    }

    pub fn set_audio_frame_callback(&self, cb: AudioFrameCallback) {
        *self.on_audio.write() = Some(cb);
    }

    pub fn set_video_frame_callback(&self, cb: VideoFrameCallback) {
        *self.on_video.write() = Some(cb);
    }

    pub fn set_ice_candidate_callback(&self, cb: IceCandidateCallback) {
        *self.on_ice_candidate.write() = Some(cb);
    }

    pub fn set_state_change_callback(&self, cb: StateChangeCallback) {
        *self.on_state_change.write() = Some(cb);
    }

    pub fn set_data_channel_message_callback(&self, cb: DataChannelMessageCallback) {
        *self.on_data_channel_message.write() = Some(cb);
    }

    pub fn set_data_channel_open_callback(&self, cb: DataChannelOpenCallback) {
        *self.on_data_channel_open.write() = Some(cb);
    }

    /// Creates a named `RTCDataChannel` with the requested reliability semantics.
    /// In `Live` mode, call before `create_offer()` so SDP negotiation includes SCTP `m=application`.
    pub async fn create_data_channel(
        &self,
        label: &str,
        reliability: DataChannelReliability,
    ) -> Result<(), WebRtcTransportError> {
        if label.trim().is_empty() {
            return Err(WebRtcTransportError::InvalidState(
                "DataChannel label must not be empty".to_string(),
            ));
        }

        if self.mode == TransportMode::Live {
            let pc = self.peer_connection.as_ref().ok_or_else(|| {
                WebRtcTransportError::InvalidState("PeerConnection is closed".to_string())
            })?;
            let init = match reliability {
                DataChannelReliability::ReliableOrdered => RTCDataChannelInit {
                    ordered: Some(true),
                    ..Default::default()
                },
                DataChannelReliability::UnreliableUnordered => RTCDataChannelInit {
                    ordered: Some(false),
                    max_retransmits: Some(0),
                    ..Default::default()
                },
            };
            let dc = pc
                .create_data_channel(label, Some(init))
                .await
                .map_err(|e| WebRtcTransportError::Internal(e.to_string()))?;
            Self::wire_live_data_channel(
                dc,
                reliability,
                Arc::clone(&self.data_channels),
                Arc::clone(&self.data_channel_reliability),
                Arc::clone(&self.on_data_channel_message),
                Arc::clone(&self.on_data_channel_open),
                Arc::clone(&self.dc_open_notify),
            );
            return Ok(());
        }

        self.loopback_open_channels
            .write()
            .insert(label.to_string());
        self.data_channel_reliability
            .write()
            .insert(label.to_string(), reliability);
        self.dc_open_notify.notify_waiters();
        if let Some(cb) = self.on_data_channel_open.read().clone() {
            cb(label);
        }
        Ok(())
    }

    /// Opens the 3 standard Fabushi remote-control channels:
    /// `"control"` (reliable), `"input-fast"` (unreliable), and `"media-frame"` (unreliable).
    pub async fn open_remote_control_channels(&self) -> Result<(), WebRtcTransportError> {
        self.create_data_channel(
            CONTROL_DATA_CHANNEL_LABEL,
            DataChannelReliability::ReliableOrdered,
        )
        .await?;
        self.create_data_channel(
            INPUT_FAST_DATA_CHANNEL_LABEL,
            DataChannelReliability::UnreliableUnordered,
        )
        .await?;
        self.create_data_channel(
            MEDIA_FRAME_DATA_CHANNEL_LABEL,
            DataChannelReliability::UnreliableUnordered,
        )
        .await?;
        Ok(())
    }

    pub fn has_data_channel(&self, label: &str) -> bool {
        if self.mode == TransportMode::Live {
            self.data_channels.read().contains_key(label)
        } else {
            self.loopback_open_channels.read().contains(label)
        }
    }

    pub fn data_channel_reliability(&self, label: &str) -> Option<DataChannelReliability> {
        self.data_channel_reliability.read().get(label).copied()
    }

    pub fn is_data_channel_open(&self, label: &str) -> bool {
        if self.mode == TransportMode::Live {
            self.data_channels
                .read()
                .get(label)
                .is_some_and(|dc| dc.ready_state() == RTCDataChannelState::Open)
        } else {
            self.loopback_open_channels.read().contains(label)
        }
    }

    /// Waits up to `timeout` for the named DataChannel to reach `Open` state.
    pub async fn wait_for_data_channel_open(
        &self,
        label: &str,
        timeout: Duration,
    ) -> Result<(), WebRtcTransportError> {
        if self.is_data_channel_open(label) {
            return Ok(());
        }
        let deadline = tokio::time::Instant::now() + timeout;
        loop {
            if self.is_data_channel_open(label) {
                return Ok(());
            }
            let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
            if remaining.is_zero() {
                return Err(WebRtcTransportError::InvalidState(format!(
                    "Timed out waiting for DataChannel '{label}' to open"
                )));
            }
            let _ = tokio::time::timeout(
                remaining.min(Duration::from_millis(50)),
                self.dc_open_notify.notified(),
            )
            .await;
        }
    }

    /// Sends raw binary bytes over a named DataChannel (`Live` SCTP or `MockLoopback`).
    pub async fn send_data_channel(
        &self,
        label: &str,
        data: &[u8],
    ) -> Result<(), WebRtcTransportError> {
        if self.mode == TransportMode::Live {
            let dc = self
                .data_channels
                .read()
                .get(label)
                .cloned()
                .ok_or_else(|| {
                    WebRtcTransportError::InvalidState(format!(
                        "DataChannel '{label}' does not exist"
                    ))
                })?;
            if dc.ready_state() != RTCDataChannelState::Open {
                return Err(WebRtcTransportError::InvalidState(format!(
                    "DataChannel '{label}' is not open (state={:?})",
                    dc.ready_state()
                )));
            }
            let payload = data.to_vec().into();
            dc.send(&payload)
                .await
                .map_err(|e| WebRtcTransportError::Internal(e.to_string()))?;
            return Ok(());
        }

        let tx_opt = self.loopback_dc_tx.read().clone();
        if let Some(tx) = tx_opt {
            tx.send(DataChannelPayload {
                label: label.to_string(),
                is_string: false,
                data: data.to_vec(),
            })
            .await
            .map_err(|_| WebRtcTransportError::ChannelClosed)?;
            return Ok(());
        }
        Err(WebRtcTransportError::ChannelClosed)
    }

    /// Sends a UTF-8 text message over a named DataChannel (`Live` SCTP or `MockLoopback`).
    pub async fn send_data_channel_text(
        &self,
        label: &str,
        text: &str,
    ) -> Result<(), WebRtcTransportError> {
        if self.mode == TransportMode::Live {
            let dc = self
                .data_channels
                .read()
                .get(label)
                .cloned()
                .ok_or_else(|| {
                    WebRtcTransportError::InvalidState(format!(
                        "DataChannel '{label}' does not exist"
                    ))
                })?;
            if dc.ready_state() != RTCDataChannelState::Open {
                return Err(WebRtcTransportError::InvalidState(format!(
                    "DataChannel '{label}' is not open (state={:?})",
                    dc.ready_state()
                )));
            }
            dc.send_text(text)
                .await
                .map_err(|e| WebRtcTransportError::Internal(e.to_string()))?;
            return Ok(());
        }

        let tx_opt = self.loopback_dc_tx.read().clone();
        if let Some(tx) = tx_opt {
            tx.send(DataChannelPayload {
                label: label.to_string(),
                is_string: true,
                data: text.as_bytes().to_vec(),
            })
            .await
            .map_err(|_| WebRtcTransportError::ChannelClosed)?;
            return Ok(());
        }
        Err(WebRtcTransportError::ChannelClosed)
    }

    pub fn set_loopback_audio_tx(&self, tx: mpsc::Sender<RtpPacket>) {
        *self.loopback_audio_tx.write() = Some(tx);
    }

    pub fn set_loopback_video_tx(&self, tx: mpsc::Sender<VideoFrameData>) {
        *self.loopback_video_tx.write() = Some(tx);
    }

    fn set_state(&self, new_state: PeerConnectionState) {
        *self.state.write() = new_state;
        if let Some(cb) = self.on_state_change.read().clone() {
            cb(new_state);
        }
    }

    pub async fn create_offer(&mut self) -> Result<String, WebRtcTransportError> {
        if self.mode == TransportMode::Live {
            if let Some(ref pc) = self.peer_connection {
                let offer = pc
                    .create_offer(None)
                    .await
                    .map_err(|e| WebRtcTransportError::Internal(e.to_string()))?;
                pc.set_local_description(offer.clone())
                    .await
                    .map_err(|e| WebRtcTransportError::Internal(e.to_string()))?;
                self.set_state(PeerConnectionState::Connecting);
                return Ok(offer.sdp);
            }
        }

        // MockLoopback offer synthesis
        self.set_state(PeerConnectionState::Connecting);
        let sdp = "v=0\r\no=- 1000 2 IN IP4 127.0.0.1\r\ns=gpui-webrtc-mock\r\nt=0 0\r\n\
             m=audio 9 UDP/TLS/RTP/SAVPF 111\r\nc=IN IP4 0.0.0.0\r\na=rtpmap:111 opus/48000/2\r\n\
             m=video 9 UDP/TLS/RTP/SAVPF 96\r\na=rtpmap:96 VP8/90000\r\n"
            .to_string();
        Ok(sdp)
    }

    pub async fn create_answer(&mut self) -> Result<String, WebRtcTransportError> {
        if self.mode == TransportMode::Live {
            if let Some(ref pc) = self.peer_connection {
                let answer = pc
                    .create_answer(None)
                    .await
                    .map_err(|e| WebRtcTransportError::Internal(e.to_string()))?;
                pc.set_local_description(answer.clone())
                    .await
                    .map_err(|e| WebRtcTransportError::Internal(e.to_string()))?;
                self.set_state(PeerConnectionState::Connecting);
                return Ok(answer.sdp);
            }
        }

        // MockLoopback answer synthesis
        self.set_state(PeerConnectionState::Connected);
        let sdp = "v=0\r\no=- 1000 2 IN IP4 127.0.0.1\r\ns=gpui-webrtc-mock\r\nt=0 0\r\n\
             m=audio 9 UDP/TLS/RTP/SAVPF 111\r\nc=IN IP4 0.0.0.0\r\na=rtpmap:111 opus/48000/2\r\n\
             m=video 9 UDP/TLS/RTP/SAVPF 96\r\na=rtpmap:96 VP8/90000\r\n"
            .to_string();
        Ok(sdp)
    }

    pub async fn set_remote_description(
        &mut self,
        sdp: &str,
        is_offer: bool,
    ) -> Result<(), WebRtcTransportError> {
        if self.mode == TransportMode::Live {
            if let Some(ref pc) = self.peer_connection {
                let desc = if is_offer {
                    RTCSessionDescription::offer(sdp.to_string())
                        .map_err(|e| WebRtcTransportError::Internal(e.to_string()))?
                } else {
                    RTCSessionDescription::answer(sdp.to_string())
                        .map_err(|e| WebRtcTransportError::Internal(e.to_string()))?
                };
                pc.set_remote_description(desc)
                    .await
                    .map_err(|e| WebRtcTransportError::Internal(e.to_string()))?;
                return Ok(());
            }
        }

        // MockLoopback state transition
        self.set_state(PeerConnectionState::Connected);
        Ok(())
    }

    pub async fn add_ice_candidate(
        &self,
        candidate: &str,
        sdp_mid: Option<String>,
        sdp_mline_index: Option<u16>,
    ) -> Result<(), WebRtcTransportError> {
        if self.mode == TransportMode::Live {
            if candidate.trim().is_empty() {
                return Ok(());
            }
            if let Some(ref pc) = self.peer_connection {
                let init = RTCIceCandidateInit {
                    candidate: candidate.to_string(),
                    sdp_mid,
                    sdp_mline_index,
                    username_fragment: None,
                };
                pc.add_ice_candidate(init)
                    .await
                    .map_err(|e| WebRtcTransportError::Internal(e.to_string()))?;
            }
        }
        Ok(())
    }

    /// Sends raw PCM audio samples (encodes with Opus and delivers to live peer or loopback)
    pub async fn send_audio_pcm(&self, samples: &[i16]) -> Result<(), WebRtcTransportError> {
        // AI Media Tap outgoing monitoring
        if let Some(tap) = self.ai_media_tap.read().clone() {
            tap.on_outgoing_pcm_frame(
                samples,
                self.config.audio_sample_rate,
                self.config.audio_channels,
            );
        }

        let packets = {
            let mut pack = self.packetizer.lock();
            pack.push_pcm(samples)
        };

        if self.mode == TransportMode::Live {
            if let Some(ref track) = self.live_audio_track {
                for packet in packets {
                    let rtp_pkt = webrtc::rtp::packet::Packet {
                        header: webrtc::rtp::header::Header {
                            version: packet.version,
                            padding: packet.padding,
                            extension: packet.extension,
                            marker: packet.marker,
                            payload_type: packet.payload_type,
                            sequence_number: packet.sequence_number,
                            timestamp: packet.timestamp,
                            ssrc: packet.ssrc,
                            ..Default::default()
                        },
                        payload: packet.payload.into(),
                    };
                    track
                        .write_rtp(&rtp_pkt)
                        .await
                        .map_err(|e| WebRtcTransportError::Internal(e.to_string()))?;
                }
            }
            return Ok(());
        }

        let tx_opt = self.loopback_audio_tx.read().clone();
        if let Some(tx) = tx_opt {
            for packet in packets {
                tx.send(packet)
                    .await
                    .map_err(|_| WebRtcTransportError::ChannelClosed)?;
            }
        }

        Ok(())
    }

    /// Sends a video frame (to live peer VP8 track or MockLoopback channel)
    pub async fn send_video_frame(
        &self,
        frame: &VideoFrameData,
    ) -> Result<(), WebRtcTransportError> {
        if self.mode == TransportMode::Live {
            if let Some(ref track) = self.live_video_track {
                let encoded = encode_live_video_payload(frame);
                let fps = self.config.video_fps.max(1) as u64;
                let sample = Sample {
                    data: encoded.into(),
                    duration: Duration::from_millis(1000 / fps),
                    ..Default::default()
                };
                track
                    .write_sample(&sample)
                    .await
                    .map_err(|e| WebRtcTransportError::Internal(e.to_string()))?;
            }
            return Ok(());
        }

        let tx_opt = self.loopback_video_tx.read().clone();
        if let Some(tx) = tx_opt {
            tx.send(frame.clone())
                .await
                .map_err(|_| WebRtcTransportError::ChannelClosed)?;
        }
        Ok(())
    }

    /// Delivers an incoming RTP packet directly into depacketizer and callbacks (for testing)
    pub fn deliver_incoming_rtp(&self, packet: &RtpPacket) -> Result<(), WebRtcTransportError> {
        let samples = {
            let mut dep = self.depacketizer.lock();
            dep.depacketize(packet)?
        };
        if let Some(tap) = self.ai_media_tap.read().clone() {
            tap.on_incoming_pcm_frame(
                &samples,
                self.config.audio_sample_rate,
                self.config.audio_channels,
            );
        }
        if let Some(cb) = self.on_audio.read().clone() {
            cb(
                &samples,
                self.config.audio_sample_rate,
                self.config.audio_channels,
            );
        }
        Ok(())
    }

    /// Delivers an incoming video frame directly into callbacks (for testing)
    pub fn deliver_incoming_video(&self, frame: &VideoFrameData) {
        if let Some(cb) = self.on_video.read().clone() {
            cb(frame);
        }
    }

    pub fn state(&self) -> PeerConnectionState {
        *self.state.read()
    }

    pub fn mode(&self) -> TransportMode {
        self.mode
    }

    pub fn config(&self) -> &PeerTransportConfig {
        &self.config
    }

    pub async fn close(&mut self) -> Result<(), WebRtcTransportError> {
        let dcs: Vec<Arc<RTCDataChannel>> = {
            let mut guard = self.data_channels.write();
            guard.drain().map(|(_, dc)| dc).collect()
        };
        for dc in dcs {
            let _ = dc.close().await;
        }
        self.loopback_open_channels.write().clear();
        if let Some(pc) = self.peer_connection.take() {
            let _ = pc.close().await;
        }
        if let Some(handle) = self.loopback_rx_handle.lock().take() {
            handle.abort();
        }
        self.set_state(PeerConnectionState::Closed);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

    #[tokio::test]
    async fn test_mock_loopback_data_channels_reliable_and_unreliable() {
        let (alice, bob) = WebRtcPeerTransport::create_loopback_pair(PeerTransportConfig::default());
        alice.open_remote_control_channels().await.unwrap();
        bob.open_remote_control_channels().await.unwrap();

        assert!(alice.is_data_channel_open(CONTROL_DATA_CHANNEL_LABEL));
        assert_eq!(
            alice.data_channel_reliability(CONTROL_DATA_CHANNEL_LABEL),
            Some(DataChannelReliability::ReliableOrdered)
        );
        assert_eq!(
            alice.data_channel_reliability(INPUT_FAST_DATA_CHANNEL_LABEL),
            Some(DataChannelReliability::UnreliableUnordered)
        );
        assert_eq!(
            alice.data_channel_reliability(MEDIA_FRAME_DATA_CHANNEL_LABEL),
            Some(DataChannelReliability::UnreliableUnordered)
        );

        let received = Arc::new(parking_lot::Mutex::new(Vec::<(String, Vec<u8>, bool)>::new()));
        let rec_clone = Arc::clone(&received);
        bob.set_data_channel_message_callback(Arc::new(move |label, data, is_string| {
            rec_clone.lock().push((label.to_string(), data.to_vec(), is_string));
        }));

        alice
            .send_data_channel_text(CONTROL_DATA_CHANNEL_LABEL, r#"{"op":"ping"}"#)
            .await
            .unwrap();
        alice
            .send_data_channel(INPUT_FAST_DATA_CHANNEL_LABEL, &[1, 2, 3, 4])
            .await
            .unwrap();

        let first = bob.recv_loopback_data_channel().await.unwrap();
        assert_eq!(first.label, CONTROL_DATA_CHANNEL_LABEL);
        assert!(first.is_string);
        assert_eq!(first.data, br#"{"op":"ping"}"#);

        let second = bob.recv_loopback_data_channel().await.unwrap();
        assert_eq!(second.label, INPUT_FAST_DATA_CHANNEL_LABEL);
        assert!(!second.is_string);
        assert_eq!(second.data, vec![1, 2, 3, 4]);

        assert_eq!(received.lock().len(), 2);
    }

    #[tokio::test]
    async fn test_live_webrtc_reliable_and_unreliable_data_channels() {
        let cfg = PeerTransportConfig {
            mode: TransportMode::Live,
            enable_audio: false,
            enable_video: false,
            ..Default::default()
        };
        let mut alice = WebRtcPeerTransport::new(cfg.clone()).await.unwrap();
        let mut bob = WebRtcPeerTransport::new(cfg).await.unwrap();

        alice.open_remote_control_channels().await.unwrap();

        let (a_ice_tx, mut a_ice_rx) = mpsc::unbounded_channel::<SignalingFrame>();
        let (b_ice_tx, mut b_ice_rx) = mpsc::unbounded_channel::<SignalingFrame>();
        alice.set_ice_candidate_callback(Arc::new(move |f| {
            let _ = a_ice_tx.send(f);
        }));
        bob.set_ice_candidate_callback(Arc::new(move |f| {
            let _ = b_ice_tx.send(f);
        }));

        let got_control = Arc::new(AtomicBool::new(false));
        let got_input = Arc::new(AtomicUsize::new(0));
        let got_ctrl_clone = Arc::clone(&got_control);
        let got_input_clone = Arc::clone(&got_input);
        bob.set_data_channel_message_callback(Arc::new(move |label, data, is_string| {
            if label == CONTROL_DATA_CHANNEL_LABEL && is_string && data == br#"{"cmd":"lock"}"# {
                got_ctrl_clone.store(true, Ordering::SeqCst);
            } else if label == INPUT_FAST_DATA_CHANNEL_LABEL && !is_string && data == [0xAA, 0xBB] {
                got_input_clone.fetch_add(1, Ordering::SeqCst);
            }
        }));

        let offer = alice.create_offer().await.unwrap();
        assert!(offer.contains("m=application"), "Offer SDP must negotiate DataChannels");
        bob.set_remote_description(&offer, true).await.unwrap();
        let answer = bob.create_answer().await.unwrap();
        alice.set_remote_description(&answer, false).await.unwrap();

        let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
        while tokio::time::Instant::now() < deadline {
            while let Ok(SignalingFrame::IceCandidate {
                candidate,
                sdp_mid,
                sdp_mline_index,
                ..
            }) = a_ice_rx.try_recv()
            {
                let _ = bob.add_ice_candidate(&candidate, sdp_mid, sdp_mline_index).await;
            }
            while let Ok(SignalingFrame::IceCandidate {
                candidate,
                sdp_mid,
                sdp_mline_index,
                ..
            }) = b_ice_rx.try_recv()
            {
                let _ = alice.add_ice_candidate(&candidate, sdp_mid, sdp_mline_index).await;
            }
            if alice.is_data_channel_open(CONTROL_DATA_CHANNEL_LABEL)
                && alice.is_data_channel_open(INPUT_FAST_DATA_CHANNEL_LABEL)
                && bob.is_data_channel_open(CONTROL_DATA_CHANNEL_LABEL)
                && bob.is_data_channel_open(INPUT_FAST_DATA_CHANNEL_LABEL)
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }

        alice
            .wait_for_data_channel_open(CONTROL_DATA_CHANNEL_LABEL, Duration::from_secs(3))
            .await
            .unwrap();
        bob.wait_for_data_channel_open(INPUT_FAST_DATA_CHANNEL_LABEL, Duration::from_secs(3))
            .await
            .unwrap();

        assert_eq!(
            bob.data_channel_reliability(CONTROL_DATA_CHANNEL_LABEL),
            Some(DataChannelReliability::ReliableOrdered)
        );
        assert_eq!(
            bob.data_channel_reliability(INPUT_FAST_DATA_CHANNEL_LABEL),
            Some(DataChannelReliability::UnreliableUnordered)
        );

        alice
            .send_data_channel_text(CONTROL_DATA_CHANNEL_LABEL, r#"{"cmd":"lock"}"#)
            .await
            .unwrap();
        alice
            .send_data_channel(INPUT_FAST_DATA_CHANNEL_LABEL, &[0xAA, 0xBB])
            .await
            .unwrap();

        let msg_deadline = tokio::time::Instant::now() + Duration::from_secs(5);
        while tokio::time::Instant::now() < msg_deadline {
            if got_control.load(Ordering::SeqCst) && got_input.load(Ordering::SeqCst) >= 1 {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }

        assert!(got_control.load(Ordering::SeqCst), "reliable control channel failed");
        assert!(got_input.load(Ordering::SeqCst) >= 1, "unreliable input-fast channel failed");

        alice.close().await.unwrap();
        bob.close().await.unwrap();
    }
}
