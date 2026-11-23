use std::ffi::{c_char, CStr};
use std::io::Read as _;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::{Path, PathBuf};

const MAX_TRACE_PAYLOAD_BYTES: usize = 64 * 1024 * 1024;

use serde_json::{json, Value};

use crate::ffi::{extract_panic_message, string_to_c_ptr};

pub fn default_app_data_dir() -> PathBuf {
    if let Some(path) = std::env::var_os("FABUSHI_APP_DATA") {
        return PathBuf::from(path);
    }
    #[cfg(any(target_os = "macos", target_os = "ios"))]
    if let Some(home) = std::env::var_os("HOME") {
        return PathBuf::from(home).join("Library/Application Support/com.ombhrum.fabushi");
    }
    #[cfg(target_os = "windows")]
    if let Some(data) = std::env::var_os("APPDATA") {
        return PathBuf::from(data).join("Fabushi");
    }
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".fabushi")
}

/// Legacy compatibility export for cloud agent trace conversion.
/// Wrapped in `catch_unwind` and strictly fail-closed (never returns fabricated empty rows).
///
/// # Safety
/// If `len > 0`, `bytes` must point to at least `len` readable bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mahayana_cloud_agent_trace_rows(
    bytes: *const u8,
    len: usize,
) -> *mut c_char {
    let result = catch_unwind(AssertUnwindSafe(|| {
        if bytes.is_null() || len == 0 {
            return json!({
                "ok": false,
                "rows": Value::Null,
                "error": "invalid_arguments: cloud agent trace input is empty or null"
            })
            .to_string();
        }
        if len > MAX_TRACE_PAYLOAD_BYTES {
            return json!({
                "ok": false,
                "rows": Value::Null,
                "error": format!("invalid_arguments: trace payload size {len} exceeds maximum limit {MAX_TRACE_PAYLOAD_BYTES}")
            })
            .to_string();
        }
        let slice = unsafe { std::slice::from_raw_parts(bytes, len) };
        match std::str::from_utf8(slice) {
            Ok(text) => {
                let mut rows: Vec<Value> = Vec::new();
                for line in text.lines().map(str::trim).filter(|l| !l.is_empty()) {
                    match serde_json::from_str::<Value>(line) {
                        Ok(val) if val.is_object() => rows.push(val),
                        _ => {
                            return json!({
                                "ok": false,
                                "rows": Value::Null,
                                "error": "capability_unavailable: binary or non-JSONL cloud agent trace format is not supported on iOS"
                            })
                            .to_string();
                        }
                    }
                }
                if rows.is_empty() {
                    json!({
                        "ok": false,
                        "rows": Value::Null,
                        "error": "invalid_arguments: no trace rows present in payload"
                    })
                    .to_string()
                } else {
                    json!({
                        "ok": true,
                        "rows": rows,
                        "error": Value::Null
                    })
                    .to_string()
                }
            }
            Err(_) => json!({
                "ok": false,
                "rows": Value::Null,
                "error": "capability_unavailable: legacy protobuf cloud agent trace decoding is unavailable on iOS"
            })
            .to_string(),
        }
    }));

    match result {
        Ok(payload) => string_to_c_ptr(payload),
        Err(panic) => {
            let msg = extract_panic_message(panic);
            string_to_c_ptr(
                json!({
                    "ok": false,
                    "rows": Value::Null,
                    "error": format!("host_panic: {msg}")
                })
                .to_string(),
            )
        }
    }
}

fn parse_delimited_line(line: &str, delim: char) -> Vec<String> {
    let mut cells = Vec::new();
    let mut current = String::new();
    let mut in_quotes = false;
    let mut chars = line.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '"' {
            if in_quotes && chars.peek() == Some(&'"') {
                current.push('"');
                chars.next();
            } else {
                in_quotes = !in_quotes;
            }
        } else if c == delim && !in_quotes {
            cells.push(std::mem::take(&mut current));
        } else {
            current.push(c);
        }
    }
    cells.push(current);
    cells
}

/// Legacy compatibility export for bounded spreadsheet parsing.
/// Wrapped in `catch_unwind`; genuinely parses bounded UTF-8 CSV/TSV files and fails closed
/// on missing files, size overflows, or unsupported binary workbook formats (never fakes empty sheets).
///
/// # Safety
/// If non-null, `path` must point to a valid NUL-terminated C string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mahayana_spreadsheet_parse_file(
    path: *const c_char,
    max_bytes: usize,
    max_rows: usize,
) -> *mut c_char {
    let result = catch_unwind(AssertUnwindSafe(|| {
        if path.is_null() {
            return json!({
                "ok": false,
                "result": Value::Null,
                "error": "invalid_arguments: spreadsheet path pointer is null"
            })
            .to_string();
        }
        let path_str = match unsafe { CStr::from_ptr(path) }.to_str() {
            Ok(s) if !s.trim().is_empty() => s.trim(),
            _ => {
                return json!({
                    "ok": false,
                    "result": Value::Null,
                    "error": "invalid_arguments: spreadsheet path is empty or invalid UTF-8"
                })
                .to_string();
            }
        };

        let file_path = Path::new(path_str);
        let ext = file_path
            .extension()
            .and_then(|e| e.to_str())
            .map(|e| e.to_ascii_lowercase())
            .unwrap_or_default();
        if ext != "csv" && ext != "tsv" {
            return json!({
                "ok": false,
                "result": Value::Null,
                "error": format!("capability_unavailable: spreadsheet format '.{ext}' is not supported by mobile host parser (CSV/TSV only)")
            })
            .to_string();
        }

        let meta = match std::fs::metadata(file_path) {
            Ok(m) if m.is_file() => m,
            Ok(_) => {
                return json!({
                    "ok": false,
                    "result": Value::Null,
                    "error": "invalid_arguments: spreadsheet path is not a regular file"
                })
                .to_string();
            }
            Err(e) => {
                return json!({
                    "ok": false,
                    "result": Value::Null,
                    "error": format!("not_found: unable to read spreadsheet file: {e}")
                })
                .to_string();
            }
        };

        let byte_limit = if max_bytes == 0 { 10 * 1024 * 1024 } else { max_bytes };
        if meta.len() > byte_limit as u64 {
            return json!({
                "ok": false,
                "result": Value::Null,
                "error": format!("invalid_arguments: file size {} exceeds limit {}", meta.len(), byte_limit)
            })
            .to_string();
        }

        let file = match std::fs::File::open(file_path) {
            Ok(f) => f,
            Err(e) => {
                return json!({
                    "ok": false,
                    "result": Value::Null,
                    "error": format!("not_found: unable to open spreadsheet file: {e}")
                })
                .to_string();
            }
        };
        let mut raw_bytes = Vec::new();
        if let Err(e) = file
            .take((byte_limit as u64).saturating_add(1))
            .read_to_end(&mut raw_bytes)
        {
            return json!({
                "ok": false,
                "result": Value::Null,
                "error": format!("storage_error: failed to read spreadsheet file: {e}")
            })
            .to_string();
        }
        if raw_bytes.len() > byte_limit {
            return json!({
                "ok": false,
                "result": Value::Null,
                "error": format!("invalid_arguments: file size exceeds limit {byte_limit}")
            })
            .to_string();
        }
        let content = match String::from_utf8(raw_bytes) {
            Ok(s) => s,
            Err(e) => {
                return json!({
                    "ok": false,
                    "result": Value::Null,
                    "error": format!("invalid_utf8: failed to decode UTF-8 spreadsheet file: {e}")
                })
                .to_string();
            }
        };

        let delim = if ext == "tsv" { '\t' } else { ',' };
        let row_limit = if max_rows == 0 { 1000 } else { max_rows };
        let mut parsed_rows: Vec<Vec<String>> = Vec::new();
        let mut total_rows: usize = 0;
        let mut truncated = false;
        for line in content.lines() {
            total_rows += 1;
            if parsed_rows.len() >= row_limit {
                truncated = true;
            } else {
                parsed_rows.push(parse_delimited_line(line, delim));
            }
        }

        let sheet_name = file_path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("Sheet1");
        let column_count = parsed_rows.iter().map(Vec::len).max().unwrap_or(0);

        json!({
            "ok": true,
            "result": {
                "sheets": [{
                    "name": sheet_name,
                    "rows": parsed_rows,
                    "rowCount": parsed_rows.len(),
                    "totalRows": total_rows,
                    "columnCount": column_count,
                    "truncated": truncated
                }]
            },
            "error": Value::Null
        })
        .to_string()
    }));

    match result {
        Ok(payload) => string_to_c_ptr(payload),
        Err(panic) => {
            let msg = extract_panic_message(panic);
            string_to_c_ptr(
                json!({
                    "ok": false,
                    "result": Value::Null,
                    "error": format!("host_panic: {msg}")
                })
                .to_string(),
            )
        }
    }
}
