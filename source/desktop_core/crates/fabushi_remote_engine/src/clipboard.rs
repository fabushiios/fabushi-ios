use crate::error::EngineResult;
use crate::is_headless_test_env;
use gpui_telegram_proto::remote_control_contracts::ClipboardPayloadDto;
use parking_lot::Mutex;
use sha2::{Digest, Sha256};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

pub trait ClipboardBackend: Send + Sync {
    fn read_text(&self) -> EngineResult<Option<String>>;
    fn write_text(&self, text: &str) -> EngineResult<()>;
    fn read_image_png_b64(&self) -> EngineResult<Option<String>>;
    fn write_image_png_b64(&self, b64: &str) -> EngineResult<()>;
}

#[derive(Debug, Default)]
pub struct InMemoryClipboardBackend {
    text: Mutex<Option<String>>,
    image_b64: Mutex<Option<String>>,
}

impl ClipboardBackend for InMemoryClipboardBackend {
    fn read_text(&self) -> EngineResult<Option<String>> {
        Ok(self.text.lock().clone())
    }

    fn write_text(&self, text: &str) -> EngineResult<()> {
        *self.text.lock() = Some(text.to_string());
        *self.image_b64.lock() = None;
        Ok(())
    }

    fn read_image_png_b64(&self) -> EngineResult<Option<String>> {
        Ok(self.image_b64.lock().clone())
    }

    fn write_image_png_b64(&self, b64: &str) -> EngineResult<()> {
        *self.image_b64.lock() = Some(b64.to_string());
        *self.text.lock() = None;
        Ok(())
    }
}

#[cfg(target_os = "macos")]
#[allow(unexpected_cfgs)]
mod macos_clipboard {
    use super::*;
    use objc::runtime::Object;
    use objc::{class, msg_send, sel, sel_impl};

    pub struct MacOsPasteboardBackend;

    unsafe fn nsstring(s: &str) -> *mut Object {
        let cls = class!(NSString);
        let obj: *mut Object = msg_send![cls, alloc];
        let bytes = s.as_ptr() as *const std::ffi::c_void;
        let len = s.len();
        // NSUTF8StringEncoding = 4
        let ns: *mut Object = msg_send![obj, initWithBytes:bytes length:len encoding:4usize];
        let _: *mut Object = msg_send![ns, autorelease];
        ns
    }

    impl ClipboardBackend for MacOsPasteboardBackend {
        fn read_text(&self) -> EngineResult<Option<String>> {
            unsafe {
                let pb: *mut Object = msg_send![class!(NSPasteboard), generalPasteboard];
                if pb.is_null() {
                    return Ok(None);
                }
                let ptype = nsstring("public.utf8-plain-text");
                let ns_str: *mut Object = msg_send![pb, stringForType:ptype];
                if ns_str.is_null() {
                    return Ok(None);
                }
                let c_ptr: *const libc::c_char = msg_send![ns_str, UTF8String];
                if c_ptr.is_null() {
                    return Ok(None);
                }
                let s = std::ffi::CStr::from_ptr(c_ptr).to_string_lossy().into_owned();
                Ok(Some(s))
            }
        }

        fn write_text(&self, text: &str) -> EngineResult<()> {
            unsafe {
                let pb: *mut Object = msg_send![class!(NSPasteboard), generalPasteboard];
                if pb.is_null() {
                    return Ok(());
                }
                let _: i64 = msg_send![pb, clearContents];
                let ns = nsstring(text);
                let ptype = nsstring("public.utf8-plain-text");
                let _: bool = msg_send![pb, setString:ns forType:ptype];
            }
            Ok(())
        }

        fn read_image_png_b64(&self) -> EngineResult<Option<String>> {
            unsafe {
                let pb: *mut Object = msg_send![class!(NSPasteboard), generalPasteboard];
                if pb.is_null() {
                    return Ok(None);
                }
                let ptype = nsstring("public.png");
                let ns_data: *mut Object = msg_send![pb, dataForType:ptype];
                if ns_data.is_null() {
                    return Ok(None);
                }
                let len: usize = msg_send![ns_data, length];
                let ptr: *const u8 = msg_send![ns_data, bytes];
                if ptr.is_null() || len == 0 {
                    return Ok(None);
                }
                let slice = std::slice::from_raw_parts(ptr, len);
                use base64::Engine as _;
                Ok(Some(base64::engine::general_purpose::STANDARD.encode(slice)))
            }
        }

        fn write_image_png_b64(&self, b64: &str) -> EngineResult<()> {
            use base64::Engine as _;
            let bytes = base64::engine::general_purpose::STANDARD
                .decode(b64.trim())
                .map_err(|e| crate::error::RemoteEngineError::InvalidArgument(e.to_string()))?;
            unsafe {
                let pb: *mut Object = msg_send![class!(NSPasteboard), generalPasteboard];
                if pb.is_null() {
                    return Ok(());
                }
                let _: i64 = msg_send![pb, clearContents];
                let ns_data: *mut Object = msg_send![
                    class!(NSData),
                    dataWithBytes:bytes.as_ptr() as *const std::ffi::c_void
                    length:bytes.len()
                ];
                let ptype = nsstring("public.png");
                let _: bool = msg_send![pb, setData:ns_data forType:ptype];
            }
            Ok(())
        }
    }
}

fn digest_payload(payload: &ClipboardPayloadDto) -> [u8; 32] {
    let mut h = Sha256::new();
    h.update(payload.mime_type.as_bytes());
    h.update(b"\0");
    if let Some(t) = payload.text.as_deref() {
        h.update(t.as_bytes());
    }
    h.update(b"\0");
    if let Some(img) = payload.image_b64.as_deref() {
        h.update(img.as_bytes());
    }
    h.finalize().into()
}

pub struct ClipboardSyncEngine {
    origin_id: String,
    seq: AtomicU64,
    last_remote_origin_seq: Mutex<(String, u64)>,
    last_applied_digest: Mutex<Option<[u8; 32]>>,
    backend: Arc<dyn ClipboardBackend>,
}

impl ClipboardSyncEngine {
    pub fn new(origin_id: impl Into<String>) -> Self {
        if is_headless_test_env() {
            return Self::with_backend(origin_id, Arc::new(InMemoryClipboardBackend::default()));
        }
        #[cfg(target_os = "macos")]
        {
            return Self::with_backend(origin_id, Arc::new(macos_clipboard::MacOsPasteboardBackend));
        }
        #[cfg(not(target_os = "macos"))]
        {
            Self::with_backend(origin_id, Arc::new(InMemoryClipboardBackend::default()))
        }
    }

    pub fn with_backend(origin_id: impl Into<String>, backend: Arc<dyn ClipboardBackend>) -> Self {
        Self {
            origin_id: origin_id.into(),
            seq: AtomicU64::new(0),
            last_remote_origin_seq: Mutex::new((String::new(), 0)),
            last_applied_digest: Mutex::new(None),
            backend,
        }
    }

    pub fn headless(origin_id: impl Into<String>) -> Self {
        Self::with_backend(origin_id, Arc::new(InMemoryClipboardBackend::default()))
    }

    pub fn origin_id(&self) -> &str {
        &self.origin_id
    }

    pub fn get_local(&self) -> EngineResult<ClipboardPayloadDto> {
        let seq = self.seq.fetch_add(1, Ordering::SeqCst) + 1;
        if let Some(text) = self.backend.read_text()? {
            return Ok(ClipboardPayloadDto {
                mime_type: "text/plain".to_string(),
                text: Some(text),
                image_b64: None,
                origin_id: self.origin_id.clone(),
                seq,
            });
        }
        if let Some(image_b64) = self.backend.read_image_png_b64()? {
            return Ok(ClipboardPayloadDto {
                mime_type: "image/png".to_string(),
                text: None,
                image_b64: Some(image_b64),
                origin_id: self.origin_id.clone(),
                seq,
            });
        }
        Ok(ClipboardPayloadDto {
            mime_type: "text/plain".to_string(),
            text: Some(String::new()),
            image_b64: None,
            origin_id: self.origin_id.clone(),
            seq,
        })
    }

    /// Sets local clipboard directly from a local caller (CLI / host UI) without remote loop guard.
    pub fn set_local_text(&self, text: &str) -> EngineResult<ClipboardPayloadDto> {
        self.backend.write_text(text)?;
        let seq = self.seq.fetch_add(1, Ordering::SeqCst) + 1;
        let payload = ClipboardPayloadDto {
            mime_type: "text/plain".to_string(),
            text: Some(text.to_string()),
            image_b64: None,
            origin_id: self.origin_id.clone(),
            seq,
        };
        *self.last_applied_digest.lock() = Some(digest_payload(&payload));
        Ok(payload)
    }

    pub fn set_local_image_png_b64(&self, b64: &str) -> EngineResult<ClipboardPayloadDto> {
        self.backend.write_image_png_b64(b64)?;
        let seq = self.seq.fetch_add(1, Ordering::SeqCst) + 1;
        let payload = ClipboardPayloadDto {
            mime_type: "image/png".to_string(),
            text: None,
            image_b64: Some(b64.to_string()),
            origin_id: self.origin_id.clone(),
            seq,
        };
        *self.last_applied_digest.lock() = Some(digest_payload(&payload));
        Ok(payload)
    }

    /// Applies a remote clipboard payload, returning `false` when suppressed by loop prevention.
    pub fn apply_remote(&self, payload: &ClipboardPayloadDto) -> EngineResult<bool> {
        if payload.origin_id == self.origin_id {
            return Ok(false);
        }
        {
            let mut last = self.last_remote_origin_seq.lock();
            if last.0 == payload.origin_id && payload.seq <= last.1 {
                return Ok(false);
            }
            *last = (payload.origin_id.clone(), payload.seq);
        }
        let digest = digest_payload(payload);
        {
            let mut prev_digest = self.last_applied_digest.lock();
            if *prev_digest == Some(digest) {
                return Ok(false);
            }
            *prev_digest = Some(digest);
        }
        if let Some(text) = payload.text.as_deref() {
            self.backend.write_text(text)?;
            return Ok(true);
        }
        if let Some(image_b64) = payload.image_b64.as_deref() {
            self.backend.write_image_png_b64(image_b64)?;
            return Ok(true);
        }
        Ok(false)
    }
}
