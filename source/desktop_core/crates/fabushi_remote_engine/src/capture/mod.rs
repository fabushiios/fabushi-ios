use crate::error::{EngineResult, RemoteEngineError};
use crate::is_headless_test_env;
use gpui_telegram_proto::remote_control_contracts::DisplayInfoDto;
use image::{DynamicImage, ImageFormat, Rgba, RgbaImage};
use parking_lot::RwLock;
use std::collections::HashMap;
use std::io::Cursor;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CaptureFormat {
    Jpeg { quality: u8 },
    Png,
    Bgra8,
}

impl Default for CaptureFormat {
    fn default() -> Self {
        Self::Jpeg { quality: 82 }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CapturedFrame {
    pub width: u32,
    pub height: u32,
    pub mime_type: String,
    pub data: Vec<u8>,
    pub captured_at_ms: i64,
    pub display_id: u32,
    pub seq: u64,
}

impl CapturedFrame {
    pub fn to_base64(&self) -> String {
        use base64::Engine as _;
        base64::engine::general_purpose::STANDARD.encode(&self.data)
    }

    pub fn to_data_url(&self) -> String {
        format!("data:{};base64,{}", self.mime_type, self.to_base64())
    }
}

pub trait ScreenCaptureBackend: Send + Sync {
    fn list_displays(&self) -> EngineResult<Vec<DisplayInfoDto>>;
    fn capture_display(&self, display_index: u32, format: CaptureFormat) -> EngineResult<CapturedFrame>;
}

pub(crate) fn encode_rgba_image(
    img: RgbaImage,
    format: CaptureFormat,
    display_id: u32,
    seq: u64,
) -> EngineResult<CapturedFrame> {
    let width = img.width();
    let height = img.height();
    let (mime_type, data) = match format {
        CaptureFormat::Bgra8 => {
            let mut bgra = img.into_raw();
            for px in bgra.chunks_exact_mut(4) {
                px.swap(0, 2);
            }
            ("application/octet-stream".to_string(), bgra)
        }
        CaptureFormat::Png => {
            let mut buf = Cursor::new(Vec::new());
            DynamicImage::ImageRgba8(img)
                .write_to(&mut buf, ImageFormat::Png)
                .map_err(|e| RemoteEngineError::Other(format!("PNG encode error: {e}")))?;
            ("image/png".to_string(), buf.into_inner())
        }
        CaptureFormat::Jpeg { quality } => {
            let mut buf = Cursor::new(Vec::new());
            let rgb = DynamicImage::ImageRgba8(img).to_rgb8();
            let mut encoder = image::codecs::jpeg::JpegEncoder::new_with_quality(
                &mut buf,
                quality.clamp(10, 100),
            );
            encoder
                .encode_image(&rgb)
                .map_err(|e| RemoteEngineError::Other(format!("JPEG encode error: {e}")))?;
            ("image/jpeg".to_string(), buf.into_inner())
        }
    };
    Ok(CapturedFrame {
        width,
        height,
        mime_type,
        data,
        captured_at_ms: chrono::Utc::now().timestamp_millis(),
        display_id,
        seq,
    })
}

#[derive(Debug, Default)]
pub struct HeadlessMockCaptureBackend {
    seq: AtomicU64,
}

impl ScreenCaptureBackend for HeadlessMockCaptureBackend {
    fn list_displays(&self) -> EngineResult<Vec<DisplayInfoDto>> {
        Ok(vec![
            DisplayInfoDto {
                display_id: 0,
                name: "Fabushi Headless Primary".to_string(),
                width: 1920,
                height: 1080,
                scale_factor_x100: 200,
                is_primary: true,
                is_virtual: false,
                refresh_hz: 60,
            },
            DisplayInfoDto {
                display_id: 1,
                name: "Fabushi Headless Secondary".to_string(),
                width: 2560,
                height: 1440,
                scale_factor_x100: 100,
                is_primary: false,
                is_virtual: true,
                refresh_hz: 120,
            },
        ])
    }

    fn capture_display(&self, display_index: u32, format: CaptureFormat) -> EngineResult<CapturedFrame> {
        let seq = self.seq.fetch_add(1, Ordering::Relaxed) + 1;
        let img = RgbaImage::from_pixel(64, 36, Rgba([30, 107, 255, 255]));
        encode_rgba_image(img, format, display_index, seq)
    }
}

#[cfg(target_os = "macos")]
mod macos_capture {
    use super::*;
    use core_foundation::base::TCFType;
    use core_foundation::data::CFData;
    use core_foundation::string::CFString;
    use core_foundation_sys::base::{kCFAllocatorDefault, CFRelease, CFTypeRef};
    use core_foundation_sys::data::{CFDataCreateMutable, CFDataRef, CFMutableDataRef};
    use core_foundation_sys::dictionary::CFDictionaryRef;
    use core_foundation_sys::string::CFStringRef;
    use core_graphics::display::CGDisplay;
    use foreign_types::ForeignType;
    use std::ffi::c_void;
    use std::ptr;

    #[link(name = "CoreGraphics", kind = "framework")]
    unsafe extern "C" {
        fn CGPreflightScreenCaptureAccess() -> bool;
    }

    #[link(name = "ImageIO", kind = "framework")]
    unsafe extern "C" {
        fn CGImageDestinationCreateWithData(
            data: CFMutableDataRef,
            type_identifier: CFStringRef,
            count: usize,
            options: CFDictionaryRef,
        ) -> *mut c_void;
        fn CGImageDestinationAddImage(
            destination: *mut c_void,
            image: *mut c_void,
            properties: CFDictionaryRef,
        );
        fn CGImageDestinationFinalize(destination: *mut c_void) -> u8;
    }

    pub struct MacOsCaptureBackend {
        seq: AtomicU64,
    }

    impl MacOsCaptureBackend {
        pub fn new() -> Self {
            Self {
                seq: AtomicU64::new(0),
            }
        }
    }

    impl ScreenCaptureBackend for MacOsCaptureBackend {
        fn list_displays(&self) -> EngineResult<Vec<DisplayInfoDto>> {
            let ids = CGDisplay::active_displays().map_err(|err| {
                RemoteEngineError::Other(format!("CGDisplay::active_displays failed: {err:?}"))
            })?;
            let main_id = CGDisplay::main().id;
            let mut out = Vec::with_capacity(ids.len());
            for (idx, id) in ids.into_iter().enumerate() {
                let d = CGDisplay::new(id);
                let bounds = d.bounds();
                let logical_w = bounds.size.width.max(1.0);
                let px_w = (d.pixels_wide() as u32).max(1);
                let px_h = (d.pixels_high() as u32).max(1);
                let scale_x100 = ((px_w as f64 / logical_w) * 100.0).round() as u32;
                let refresh_hz = d
                    .display_mode()
                    .map(|m| m.refresh_rate().round() as u32)
                    .filter(|hz| *hz > 0)
                    .unwrap_or(60);
                out.push(DisplayInfoDto {
                    display_id: idx as u32,
                    name: if id == main_id {
                        format!("Primary Display ({id})")
                    } else {
                        format!("Display {idx} ({id})")
                    },
                    width: px_w,
                    height: px_h,
                    scale_factor_x100: scale_x100.max(100),
                    is_primary: id == main_id,
                    is_virtual: false,
                    refresh_hz,
                });
            }
            Ok(out)
        }

        fn capture_display(
            &self,
            display_index: u32,
            format: CaptureFormat,
        ) -> EngineResult<CapturedFrame> {
            if !unsafe { CGPreflightScreenCaptureAccess() } {
                return Err(RemoteEngineError::PermissionDenied(
                    "Screen Recording permission required in System Settings > Privacy & Security"
                        .to_string(),
                ));
            }
            let seq = self.seq.fetch_add(1, Ordering::Relaxed) + 1;
            let ids = CGDisplay::active_displays().map_err(|err| {
                RemoteEngineError::Other(format!("CGDisplay::active_displays failed: {err:?}"))
            })?;
            let id = *ids.get(display_index as usize).ok_or_else(|| {
                RemoteEngineError::NotFound(format!("display index {display_index} not found"))
            })?;

            // Fast zero-subprocess CGDisplay capture path with ImageIO hardware encoding
            if matches!(format, CaptureFormat::Jpeg { .. } | CaptureFormat::Png) {
                if let Some(image) = CGDisplay::new(id).image() {
                    let width = image.width() as u32;
                    let height = image.height() as u32;
                    let data_ref = unsafe { CFDataCreateMutable(kCFAllocatorDefault, 0) };
                    if !data_ref.is_null() {
                        let (uti, mime) = match format {
                            CaptureFormat::Png => ("public.png", "image/png"),
                            _ => ("public.jpeg", "image/jpeg"),
                        };
                        let uti_cf = CFString::new(uti);
                        let dest = unsafe {
                            CGImageDestinationCreateWithData(
                                data_ref,
                                uti_cf.as_concrete_TypeRef(),
                                1,
                                ptr::null(),
                            )
                        };
                        if !dest.is_null() {
                            unsafe {
                                CGImageDestinationAddImage(
                                    dest,
                                    image.as_ptr() as *mut c_void,
                                    ptr::null(),
                                );
                            }
                            let finalized = unsafe { CGImageDestinationFinalize(dest) };
                            unsafe { CFRelease(dest as CFTypeRef) };
                            if finalized != 0 {
                                let cf_data =
                                    unsafe { CFData::wrap_under_create_rule(data_ref as CFDataRef) };
                                if !cf_data.is_empty() {
                                    return Ok(CapturedFrame {
                                        width,
                                        height,
                                        mime_type: mime.to_string(),
                                        data: cf_data.bytes().to_vec(),
                                        captured_at_ms: chrono::Utc::now().timestamp_millis(),
                                        display_id: display_index,
                                        seq,
                                    });
                                }
                            } else {
                                unsafe { CFRelease(data_ref as CFTypeRef) };
                            }
                        } else {
                            unsafe { CFRelease(data_ref as CFTypeRef) };
                        }
                    }
                }
            }

            // Native ScreenCaptureKit fallback via xcap (zero subprocess)
            capture_via_xcap(display_index, format, seq)
        }
    }
}

// iOS target_os adaptation: xcap is only compiled on macOS/Linux/Windows; iOS captures via ReplayKit.
#[cfg(any(target_os = "macos", target_os = "linux", target_os = "windows"))]
fn capture_via_xcap(
    display_index: u32,
    format: CaptureFormat,
    seq: u64,
) -> EngineResult<CapturedFrame> {
    let monitors = xcap::Monitor::all()
        .map_err(|e| RemoteEngineError::Other(format!("xcap monitor enumeration failed: {e}")))?;
    let monitor = monitors.get(display_index as usize).ok_or_else(|| {
        RemoteEngineError::NotFound(format!("display index {display_index} not found"))
    })?;
    let img = monitor
        .capture_image()
        .map_err(|e| RemoteEngineError::Other(format!("xcap capture_image failed: {e}")))?;
    encode_rgba_image(img, format, display_index, seq)
}

#[cfg(any(target_os = "linux", target_os = "windows"))]
struct XcapCaptureBackend {
    seq: AtomicU64,
}

#[cfg(any(target_os = "linux", target_os = "windows"))]
impl XcapCaptureBackend {
    fn new() -> Self {
        Self {
            seq: AtomicU64::new(0),
        }
    }
}

#[cfg(any(target_os = "linux", target_os = "windows"))]
impl ScreenCaptureBackend for XcapCaptureBackend {
    fn list_displays(&self) -> EngineResult<Vec<DisplayInfoDto>> {
        let monitors = xcap::Monitor::all()
            .map_err(|e| RemoteEngineError::Other(format!("xcap monitor enumeration failed: {e}")))?;
        let mut out = Vec::with_capacity(monitors.len());
        for (idx, m) in monitors.iter().enumerate() {
            let name = m.name().unwrap_or_else(|_| format!("Display {idx}"));
            let width = m.width().unwrap_or(1920);
            let height = m.height().unwrap_or(1080);
            let scale = (m.scale_factor().unwrap_or(1.0) * 100.0).round() as u32;
            let is_primary = m.is_primary().unwrap_or(idx == 0);
            let freq = m.frequency().unwrap_or(60.0).round() as u32;
            out.push(DisplayInfoDto {
                display_id: idx as u32,
                name,
                width,
                height,
                scale_factor_x100: scale.max(100),
                is_primary,
                is_virtual: false,
                refresh_hz: freq.max(30),
            });
        }
        Ok(out)
    }

    fn capture_display(&self, display_index: u32, format: CaptureFormat) -> EngineResult<CapturedFrame> {
        let seq = self.seq.fetch_add(1, Ordering::Relaxed) + 1;
        capture_via_xcap(display_index, format, seq)
    }
}

#[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
struct UnsupportedScreenCaptureBackend;

#[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
impl ScreenCaptureBackend for UnsupportedScreenCaptureBackend {
    fn list_displays(&self) -> EngineResult<Vec<DisplayInfoDto>> {
        Err(RemoteEngineError::Unavailable(
            "local desktop screen capture unavailable on iOS; use ReplayKit Broadcast Upload Extension".into(),
        ))
    }

    fn capture_display(
        &self,
        _display_index: u32,
        _format: CaptureFormat,
    ) -> EngineResult<CapturedFrame> {
        Err(RemoteEngineError::Unavailable(
            "local desktop screen capture unavailable on iOS; use ReplayKit Broadcast Upload Extension".into(),
        ))
    }
}

pub struct ScreenCaptureEngine {
    backend: Arc<dyn ScreenCaptureBackend>,
    cache: RwLock<HashMap<u32, CapturedFrame>>,
}

impl Default for ScreenCaptureEngine {
    fn default() -> Self {
        Self::new()
    }
}

impl ScreenCaptureEngine {
    pub fn new() -> Self {
        if is_headless_test_env() {
            return Self::with_backend(Arc::new(HeadlessMockCaptureBackend::default()));
        }
        #[cfg(target_os = "macos")]
        {
            Self::with_backend(Arc::new(macos_capture::MacOsCaptureBackend::new()))
        }
        #[cfg(any(target_os = "linux", target_os = "windows"))]
        {
            Self::with_backend(Arc::new(XcapCaptureBackend::new()))
        }
        #[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
        {
            Self::with_backend(Arc::new(UnsupportedScreenCaptureBackend))
        }
    }

    pub fn with_backend(backend: Arc<dyn ScreenCaptureBackend>) -> Self {
        Self {
            backend,
            cache: RwLock::new(HashMap::new()),
        }
    }

    pub fn headless() -> Self {
        Self::with_backend(Arc::new(HeadlessMockCaptureBackend::default()))
    }

    pub fn list_displays(&self) -> EngineResult<Vec<DisplayInfoDto>> {
        self.backend.list_displays()
    }

    pub fn capture_screen(
        &self,
        display_index: u32,
        format: CaptureFormat,
    ) -> EngineResult<CapturedFrame> {
        let frame = self.backend.capture_display(display_index, format)?;
        self.cache.write().insert(display_index, frame.clone());
        Ok(frame)
    }

    pub fn cached_latest_frame(&self, display_index: u32, max_age_ms: i64) -> Option<CapturedFrame> {
        let now = chrono::Utc::now().timestamp_millis();
        self.cache
            .read()
            .get(&display_index)
            .filter(|f| now.saturating_sub(f.captured_at_ms) <= max_age_ms)
            .cloned()
    }

    pub fn capture_or_cached(
        &self,
        display_index: u32,
        max_age_ms: i64,
        format: CaptureFormat,
    ) -> EngineResult<CapturedFrame> {
        if let Some(frame) = self.cached_latest_frame(display_index, max_age_ms) {
            return Ok(frame);
        }
        self.capture_screen(display_index, format)
    }

    pub fn invalidate_cache(&self) {
        self.cache.write().clear();
    }
}
