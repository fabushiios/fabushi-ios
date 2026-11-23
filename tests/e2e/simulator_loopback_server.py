#!/usr/bin/env python3
"""Stateful HTTP/1.1 Loopback Protocol Server for Fabushi iOS v1.6.8 Simulator E2E.

Standard-library only Python 3 implementation mirroring `tests/e2e/rust_ffi/src/loopback_server.rs`
plus typed `/__e2e__/*` control endpoints for Swift 6 opaque-box XCUITests.

Designed to execute exclusively in GitHub Actions CI (`ubuntu-latest` self-test and `macos-15`
iOS Simulator runs).
"""

from __future__ import annotations

import argparse
import base64
import binascii
import datetime
import hashlib
import http.client
import http.server
import json
import os
import pathlib
import signal
import socket
import subprocess
import sys
import tempfile
import threading
import time
import urllib.error
import urllib.parse
import urllib.request
import uuid
from collections import deque
import struct
from typing import Any

MAX_HTTP_BODY_BYTES = 32 * 1024 * 1024
MAX_HTTP_TRAILER_LINES = 64
MAX_UPLOAD_TOTAL_SIZE = 4 * 1024 * 1024 * 1024
MAX_UPLOAD_PART_SIZE = 16 * 1024 * 1024
MAX_RECORDED_BODY_PREVIEW = 16 * 1024
DEFAULT_SERVER_VERSION = "1.6.8"
DEVICE_SYNC_CODE_DOMAIN_V1 = b"DEVICE_SYNC_CODE_DOMAIN_V1"
WS_MAGIC_GUID = "258EAFA5-E914-47DA-95CA-C5AB0DC85B11"
HTTP_SOCKET_TIMEOUT_SECS = 5.0
WS_POLL_TIMEOUT_SECS = 0.05
WS_FRAME_BODY_DEADLINE_SECS = 3.0
MAX_WS_FRAME_PAYLOAD_BYTES = 4 * 1024 * 1024
MAX_WS_MESSAGE_BYTES = 16 * 1024 * 1024

CHAT_TYPE_STR_TO_INT: dict[str, int] = {
    "private": 0,
    "dm": 0,
    "direct": 0,
    "group": 1,
    "supergroup": 2,
    "forum": 2,
    "channel": 3,
    "bot": 4,
    "saved": 5,
    "saved_messages": 5,
}

ALLOWED_FALLBACK_NAMESPACES: frozenset[str] = frozenset(
    {
        "chats",
        "messages",
        "topics",
        "folders",
        "contacts",
        "peers",
        "users",
        "blocked",
        "settings",
        "fabushi",
        "agent",
        "multi-agent",
        "multi_agent",
        "coding",
        "tasks",
        "magpie",
        "mcp",
        "computer",
        "computers",
        "remote",
        "tailnet",
        "miniapps",
        "webmcp",
        "stickers",
        "gifs",
        "stories",
        "calls",
        "webrtc",
        "talk",
        "search",
        "e2ee",
        "wallet",
        "actions",
        "devices",
        "push",
        "updates",
        "proxy",
    }
)

PUBLIC_EXACT_ROUTES: frozenset[str] = frozenset(
    {
        "/health",
        "/healthz",
        "/api/v1/health",
        "/api/v1/status",
        "/api/v1/auth/2fa/verify",
        "/api/v1/auth/two-factor/verify",
        "/api/v1/auth/pair/init",
        "/api/v1/auth/pair/verify",
        "/api/v1/auth/login",
        "/api/v1/auth/register",
        "/api/v1/auth/kdf-params",
        "/api/v1/auth/kdf_params",
        "/api/v1/auth/passkey/challenge",
        "/api/v1/auth/passkeys/challenge",
        "/api/v1/auth/passkey/verify",
        "/api/v1/auth/passkeys/verify",
        "/api/v1/auth/passkey/login",
        "/api/v1/auth/passkey/authenticate",
    }
)

PUBLIC_SEGMENT_PREFIXES: tuple[str, ...] = (
    "/api/v1/auth/pair/poll",
    "/api/v1/auth/email",
    "/api/v1/auth/phone",
    "/api/v1/auth/alipay",
    "/article",
    "/iv",
    "/.well-known",
)

PROTECTED_EXACT_ROUTES: frozenset[str] = frozenset(
    {
        "/api/v1/auth/me",
        "/api/v1/auth/logout",
        "/api/v1/auth/pair/accept",
        "/api/v1/auth/upgrade-split-key",
        "/api/v1/auth/upgrade_split_key",
        "/api/v1/auth/change-password",
        "/api/v1/auth/change_password",
    }
)

PROTECTED_SEGMENT_PREFIXES: tuple[str, ...] = (
    "/api/v1/auth/sessions",
    "/api/v1/auth/2fa",
    "/api/v1/auth/two-factor",
    "/api/v1/auth/passkey",
    "/api/v1/auth/passkeys",
    "/api/v1/chats",
    "/api/v1/messages",
    "/api/v1/sync",
    "/api/v1/user-vault",
    "/api/v1/vault",
    "/api/v1/folders",
    "/api/v1/contacts",
    "/api/v1/settings",
    "/api/v1/agent",
    "/api/v1/tasks",
    "/api/v1/mcp",
    "/api/v1/computer",
    "/api/v1/computers",
    "/api/v1/remote",
    "/api/v1/tailnet",
    "/api/v1/miniapps",
    "/api/v1/webmcp",
    "/api/v1/talk",
    "/api/v1/webrtc",
    "/api/v1/search",
    "/api/v1/wallet",
    "/api/v1/e2ee",
    "/api/v1/push",
    "/api/v1/stories",
    "/api/v1/media",
    "/api/v1/ws",
)


class BadRequestError(Exception):
    """Raised when incoming HTTP framing is malformed."""


class PayloadTooLargeError(Exception):
    """Raised when incoming HTTP body exceeds MAX_HTTP_BODY_BYTES."""


class ApiParamError(Exception):
    """Raised on invalid API path/query/JSON numeric or structural parameters -> HTTP 400."""


class _WsProtocolError(Exception):
    def __init__(self, code: int, reason: str) -> None:
        super().__init__(reason)
        self.code = code
        self.reason = reason


class _WsClosedError(Exception):
    """Raised when a WebSocket peer closes or times out mid-frame."""


def _is_strict_int(val: Any) -> bool:
    return isinstance(val, int) and not isinstance(val, bool)


def _is_ascii_uint_str(val: str) -> bool:
    return bool(val) and val.isascii() and val.isdigit()


def _is_ascii_int_str(val: str) -> bool:
    if not val or not val.isascii():
        return False
    if val[0] == "-":
        return len(val) > 1 and val[1:].isdigit()
    return val.isdigit()


def _parse_strict_int(
    val: Any,
    *,
    field: str = "value",
    min_val: int | None = None,
    max_val: int | None = None,
) -> int:
    if isinstance(val, bool):
        raise ValueError(f"boolean not allowed for integer {field}")
    parsed: int | None = None
    if isinstance(val, int):
        parsed = val
    elif isinstance(val, str):
        s = val.strip()
        if s and (
            (s[0] in ("-", "+") and len(s) > 1 and s[1:].isascii() and s[1:].isdigit())
            or (s.isascii() and s.isdigit())
        ):
            parsed = int(s)
    if parsed is None:
        raise ValueError(f"invalid integer for {field}: {val!r}")
    if min_val is not None and parsed < min_val:
        raise ValueError(f"{field} must be >= {min_val}")
    if max_val is not None and parsed > max_val:
        raise ValueError(f"{field} must be <= {max_val}")
    return parsed


def parse_int_or_400(
    val: Any,
    *,
    default: int | None = None,
    field: str = "value",
    min_val: int | None = None,
    max_val: int | None = None,
    allow_none: bool = False,
) -> int:
    if val is None:
        if allow_none and default is not None:
            return default
        raise ApiParamError(f"missing or null integer for {field}")
    try:
        return _parse_strict_int(val, field=field, min_val=min_val, max_val=max_val)
    except ValueError as exc:
        raise ApiParamError(str(exc)) from exc


def _opt_list(body: dict[str, Any], key: str) -> list[Any]:
    if key not in body or body[key] is None:
        return []
    val = body[key]
    if not isinstance(val, list):
        raise ValueError(f"{key} must be a list")
    return val


def _normalize_chat_type(raw: Any, *, default: int) -> int:
    if raw is None:
        return default
    if isinstance(raw, bool):
        raise ApiParamError("boolean not allowed for chat_type")
    if isinstance(raw, int):
        if 0 <= raw <= 5:
            return raw
        raise ApiParamError(f"chat_type integer out of range 0..=5: {raw}")
    if isinstance(raw, str):
        s = raw.strip().lower()
        if not s:
            return default
        if s in CHAT_TYPE_STR_TO_INT:
            return CHAT_TYPE_STR_TO_INT[s]
        if s.isascii() and s.isdigit():
            n = int(s)
            if 0 <= n <= 5:
                return n
    raise ApiParamError(f"invalid chat_type: {raw!r}")


def now_ms() -> int:
    return int(time.time() * 1000)


def ms_to_rfc3339(ts_ms: int) -> str:
    dt = datetime.datetime.fromtimestamp(ts_ms / 1000.0, tz=datetime.timezone.utc)
    return dt.strftime("%Y-%m-%dT%H:%M:%S.%f")[:-3] + "Z"


def hex_sha256(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def compute_device_sync_code(request_id: str, ephemeral_pub_b64: str) -> str:
    h = hashlib.sha256()
    h.update(DEVICE_SYNC_CODE_DOMAIN_V1)
    h.update(request_id.encode("utf-8"))
    h.update(ephemeral_pub_b64.encode("utf-8"))
    digest = h.digest()
    val = int.from_bytes(digest[:4], "big") % 1_000_000
    return f"{val:06d}"


def compute_websocket_accept(sec_websocket_key: str) -> str:
    material = (sec_websocket_key.strip() + WS_MAGIC_GUID).encode("ascii")
    return base64.b64encode(hashlib.sha1(material).digest()).decode("ascii")


def _truncate_utf8_bytes(text: str, max_bytes: int) -> bytes:
    raw = text.encode("utf-8")
    if len(raw) <= max_bytes:
        return raw
    out = bytearray()
    for ch in text:
        ch_b = ch.encode("utf-8")
        if len(out) + len(ch_b) > max_bytes:
            break
        out.extend(ch_b)
    return bytes(out)


def _write_ws_frame(sock: socket.socket, opcode: int, payload: bytes) -> None:
    b0 = 0x80 | (opcode & 0x0F)
    n = len(payload)
    if n <= 125:
        header = struct.pack("!BB", b0, n)
    elif n <= 65535:
        header = struct.pack("!BBH", b0, 126, n)
    else:
        header = struct.pack("!BBQ", b0, 127, n)
    sock.sendall(header + payload)


def _write_ws_close(sock: socket.socket, code: int, reason: str = "") -> None:
    reason_bytes = _truncate_utf8_bytes(reason, 123)
    _write_ws_frame(sock, 0x8, struct.pack("!H", code) + reason_bytes)


class _WsConnReader:
    """RFC 6455 incremental frame reader supporting buffered initial bytes and stop_event."""

    def __init__(self, initial: bytes = b"") -> None:
        self.buf = bytearray(initial)

    def _ensure_bytes(
        self, sock: socket.socket, needed: int, stop_event: threading.Event
    ) -> None:
        deadline = time.monotonic() + WS_FRAME_BODY_DEADLINE_SECS
        while len(self.buf) < needed:
            if stop_event.is_set():
                raise _WsClosedError("server_stopping")
            rem = deadline - time.monotonic()
            if rem <= 0:
                raise _WsClosedError("frame_body_timeout")
            sock.settimeout(min(rem, 0.1))
            try:
                chunk = sock.recv(4096)
            except (TimeoutError, socket.timeout):
                continue
            if chunk == b"":
                raise _WsClosedError("eof")
            self.buf.extend(chunk)

    def read_frame(
        self, sock: socket.socket, stop_event: threading.Event
    ) -> tuple[bool, int, bytes] | None:
        if not self.buf:
            if stop_event.is_set():
                raise _WsClosedError("server_stopping")
            sock.settimeout(WS_POLL_TIMEOUT_SECS)
            try:
                chunk = sock.recv(4096)
            except (TimeoutError, socket.timeout):
                return None
            if chunk == b"":
                raise _WsClosedError("eof")
            self.buf.extend(chunk)

        self._ensure_bytes(sock, 2, stop_event)
        b0, b1 = self.buf[0], self.buf[1]
        fin = (b0 & 0x80) != 0
        if (b0 & 0x70) != 0:
            raise _WsProtocolError(1002, "non_zero_rsv")
        opcode = b0 & 0x0F
        if (b1 & 0x80) == 0:
            raise _WsProtocolError(1002, "unmasked_client_frame")

        len7 = b1 & 0x7F
        if len7 <= 125:
            offset, payload_len = 2, len7
        elif len7 == 126:
            self._ensure_bytes(sock, 4, stop_event)
            offset = 4
            payload_len = struct.unpack("!H", self.buf[2:4])[0]
        else:
            self._ensure_bytes(sock, 10, stop_event)
            offset = 10
            payload_len = struct.unpack("!Q", self.buf[2:10])[0]
            if payload_len & (1 << 63):
                raise _WsProtocolError(1002, "invalid_64bit_length_msb")

        if (opcode & 0x08) != 0 and (not fin or payload_len > 125):
            raise _WsProtocolError(1002, "invalid_control_frame")
        if payload_len > MAX_WS_FRAME_PAYLOAD_BYTES:
            raise _WsProtocolError(1009, "frame_too_large")

        total = offset + 4 + payload_len
        self._ensure_bytes(sock, total, stop_event)
        mask = bytes(self.buf[offset : offset + 4])
        payload = bytearray(self.buf[offset + 4 : total])
        del self.buf[:total]
        for i in range(len(payload)):
            payload[i] ^= mask[i & 3]
        return fin, opcode, bytes(payload)


def matches_segment_prefix(path: str, prefix: str) -> bool:
    return path == prefix or path.startswith(prefix + "/")


def normalize_request_target(raw_target: str) -> tuple[str, str]:
    lower = raw_target.lower()
    if lower.startswith("http://") or lower.startswith("https://"):
        after_scheme = raw_target[8:] if lower.startswith("https://") else raw_target[7:]
        slash_pos = after_scheme.find("/")
        query_pos = after_scheme.find("?")
        if slash_pos != -1 and (query_pos == -1 or slash_pos < query_pos):
            normalized = after_scheme[slash_pos:]
        elif query_pos != -1:
            normalized = "/" + after_scheme[query_pos:]
        else:
            normalized = "/"
    else:
        normalized = raw_target
    if "?" in normalized:
        p, q = normalized.split("?", 1)
        return p, q
    return normalized, ""


def extract_query_param(query: str, key: str) -> str | None:
    if not query:
        return None
    for pair in query.split("&"):
        if "=" not in pair:
            continue
        k, v = pair.split("=", 1)
        if k == key:
            decoded = urllib.parse.unquote(v).strip()
            if decoded:
                return decoded
    return None


def extract_bearer_token(headers: dict[str, str], query: str) -> str | None:
    auth = headers.get("authorization", "").strip()
    if len(auth) > 7 and auth[:7].lower() == "bearer ":
        tok = auth[7:].strip()
        if tok:
            return tok
    return extract_query_param(query, "token")


def is_protected_route(path: str) -> bool:
    if path in PUBLIC_EXACT_ROUTES:
        return False
    for prefix in PUBLIC_SEGMENT_PREFIXES:
        if matches_segment_prefix(path, prefix):
            return False
    if path in PROTECTED_EXACT_ROUTES:
        return True
    for prefix in PROTECTED_SEGMENT_PREFIXES:
        if matches_segment_prefix(path, prefix):
            return True
    return False


def resolve_media_get_id(path: str) -> str | None:
    for prefix in (
        "/api/v1/media/blob/",
        "/api/v1/media/stream/",
        "/api/v1/media/download/",
    ):
        if path.startswith(prefix):
            rest = path[len(prefix) :]
            return rest if rest else None
    if path.startswith("/api/v1/media/"):
        rest = path[len("/api/v1/media/") :]
        if (
            rest
            and "/" not in rest
            and not rest.startswith("upload")
            and rest not in ("edit", "instant-view", "instant_view")
        ):
            return rest
    return None


def parse_byte_range(range_header: str, total_len: int) -> tuple[bool, int, int]:
    """Return (satisfiable, start, end) for an HTTP Range: bytes=... header."""
    raw = range_header.strip()
    if not raw.lower().startswith("bytes=") or total_len <= 0:
        return False, 0, 0
    spec = raw[6:].split(",", 1)[0].strip()
    if "-" not in spec:
        return False, 0, 0
    left, right = spec.split("-", 1)
    left = left.strip()
    right = right.strip()
    if not left and right:
        if not _is_ascii_uint_str(right):
            return False, 0, 0
        suffix = int(right)
        if suffix <= 0:
            return False, 0, 0
        start = max(0, total_len - suffix)
        return True, start, total_len - 1
    if not _is_ascii_uint_str(left):
        return False, 0, 0
    if right and not _is_ascii_uint_str(right):
        return False, 0, 0
    start = int(left)
    end = total_len - 1 if not right else int(right)
    if start < 0 or end < start or start >= total_len:
        return False, 0, 0
    return True, start, min(end, total_len - 1)


def atomic_write_text(dest: pathlib.Path, text: str) -> None:
    dest.parent.mkdir(parents=True, exist_ok=True)
    fd, tmp_name = tempfile.mkstemp(
        dir=str(dest.parent), prefix=f".{dest.name}.", suffix=".tmp"
    )
    try:
        with os.fdopen(fd, "wb") as handle:
            handle.write(text.encode("utf-8"))
            handle.flush()
            os.fsync(handle.fileno())
        os.replace(tmp_name, dest)
    except BaseException:
        try:
            os.unlink(tmp_name)
        except OSError:
            pass
        raise


class LoopbackState:
    """Thread-safe stateful backend matching Rust `loopback_server.rs` + Simulator UI fixtures."""

    def __init__(self, version: str = DEFAULT_SERVER_VERSION) -> None:
        self.lock = threading.RLock()
        self.version = version
        self.reset()

    def reset(self) -> None:
        with self.lock:
            self.offline: bool = False
            self.drop_next_ws: bool = False
            self.ws_push_queue: deque[str] = deque()
            self.ws_received_frames: list[str] = []
            self.recorded: list[dict[str, Any]] = []
            self.faults: list[dict[str, Any]] = []
            self.delays: dict[str, int] = {}
            self.media_blobs: dict[str, bytes] = {}
            self.journal_floors: dict[int, int] = {}
            self.alipay_attempts: dict[str, dict[str, Any]] = {}
            self.alipay_state_to_attempt: dict[str, str] = {}
            self.consumed_alipay_codes: set[str] = set()
            self.passkeys: dict[str, dict[str, Any]] = {}
            self.passkey_challenges: dict[str, dict[str, Any]] = {}
            self.phone_challenges: dict[str, dict[str, Any]] = {}
            self.computer_leases: dict[str, dict[str, Any]] = {}
            self.users_by_id: dict[int, dict[str, Any]] = {}
            self.username_to_id: dict[str, int] = {}
            self.sessions_by_token: dict[str, dict[str, Any]] = {}
            self.session_id_to_token: dict[str, str] = {}
            self.revoked_tokens: set[str] = set()
            self.two_factor_challenges: dict[str, dict[str, Any]] = {}
            self.qr_pairs: dict[str, dict[str, Any]] = {}
            self.chats: dict[int, dict[str, Any]] = {}
            self.chats_by_nonce: dict[str, int] = {}
            self.chat_invites: dict[str, dict[str, Any]] = {}
            self.chat_messages: dict[int, list[dict[str, Any]]] = {}
            self.chat_pts: dict[int, int] = {}
            self.chat_nonce_map: dict[tuple[int, int], int] = {}
            self.vault_items: dict[tuple[int, str, str], dict[str, Any]] = {}
            self.device_syncs: dict[str, dict[str, Any]] = {}
            self.uploads: dict[str, dict[str, Any]] = {}
            self.remote_nodes: dict[str, dict[str, Any]] = {}
            self.miniapps: dict[str, dict[str, Any]] = {}
            self.tasks: dict[str, dict[str, Any]] = {}
            self.mcp_servers: dict[str, dict[str, Any]] = {
                "fabushi-official": {
                    "server_id": "fabushi-official",
                    "name": "Fabushi Official MCP",
                    "protocol_version": "2025-11-25",
                    "status": "connected",
                }
            }
            self.next_user_id: int = 1000
            self.next_chat_id: int = 10
            self.next_msg_id: int = 1
            self.next_seq: int = 1

    def alloc_seq(self) -> int:
        seq = self.next_seq
        self.next_seq += 1
        return seq

    def ensure_user_by_id(self, uid: int) -> dict[str, Any]:
        if uid not in self.users_by_id:
            uname = f"user_{uid}"
            self.users_by_id[uid] = {
                "user_id": uid,
                "username": uname,
                "first_name": uname,
                "auth_version": 1,
                "key_version": 1,
                "password": None,
                "auth_salt_b64": base64.b64encode(bytes([0x42] * 16)).decode("ascii"),
                "auth_verifier_b64": None,
                "kdf_params": {"m_cost_kib": 65536, "t_cost": 3, "p_cost": 4},
                "vault_envelope": None,
                "two_factor_password": None,
                "two_factor_hint": None,
                "two_factor_recovery_email": None,
            }
            self.username_to_id.setdefault(uname, uid)
        return self.users_by_id[uid]

    def default_user_id(self) -> int:
        if self.users_by_id:
            return min(self.users_by_id.keys())
        self.ensure_user_by_id(1001)
        return 1001

    def mint_session(self, uid: int, device_name: str, ts: int) -> dict[str, Any]:
        seq = self.alloc_seq()
        sid = f"sess_{uid}_{seq}"
        tok = f"tok_{uid}_{seq}_{uuid.uuid4().hex[:12]}"
        sess = {
            "session_id": sid,
            "user_id": uid,
            "token": tok,
            "device_name": device_name,
            "created_at_ms": ts,
            "expires_at_ms": ts + 86_400_000,
            "revoked": False,
        }
        self.sessions_by_token[tok] = sess
        self.session_id_to_token[sid] = tok
        return sess

    def is_token_rejected(self, tok: str) -> bool:
        if tok in self.revoked_tokens:
            return True
        sess = self.sessions_by_token.get(tok)
        if sess is not None:
            return bool(sess.get("revoked"))
        if tok.startswith("tok_"):
            return True
        if self.revoked_tokens:
            return True
        return False

    def lookup_delay_ms(self, path: str) -> int:
        with self.lock:
            for prefix, ms in self.delays.items():
                if path.startswith(prefix):
                    return ms
            return 0

    def pop_matching_fault(self, method: str, path: str) -> dict[str, Any] | None:
        with self.lock:
            for fault in self.faults:
                f_method = fault.get("method")
                if f_method and f_method.upper() != method.upper():
                    continue
                if not path.startswith(fault["path_prefix"]):
                    continue
                rem = fault.get("remaining_hits")
                if rem is not None and rem <= 0:
                    continue
                if rem is not None:
                    fault["remaining_hits"] = rem - 1
                return dict(fault)
            return None

    def record_request(
        self, method: str, path: str, query: str, headers: dict[str, str], body: bytes
    ) -> None:
        with self.lock:
            preview = body[:MAX_RECORDED_BODY_PREVIEW].decode("utf-8", errors="replace")
            self.recorded.append(
                {
                    "method": method,
                    "path": path,
                    "query": query,
                    "body_size": len(body),
                    "body_preview": preview,
                    "timestamp_ms": now_ms(),
                }
            )


def _str_nonempty_trim(val: Any) -> str | None:
    if isinstance(val, str):
        t = val.strip()
        return t if t else None
    return None


def _str_nonblank_raw(val: Any) -> str | None:
    if isinstance(val, str) and val.strip():
        return val
    return None


def _format_stored_message(msg: dict[str, Any]) -> dict[str, Any]:
    out: dict[str, Any] = {
        "message_id": msg["message_id"],
        "id": msg["message_id"],
        "chat_id": msg["chat_id"],
        "pts": msg["pts"],
        "text": msg["text"],
        "content": msg["text"],
        "client_nonce": msg.get("client_nonce"),
        "sender_id": msg["sender_id"],
        "timestamp_ms": msg["timestamp_ms"],
        "delivery_status": "sent",
        "status": "sent",
    }
    if msg.get("message_type") is not None:
        out["message_type"] = msg["message_type"]
    if msg.get("media_file_id") is not None:
        out["media_file_id"] = msg["media_file_id"]
    if msg.get("instant_view_url") is not None:
        out["instant_view_url"] = msg["instant_view_url"]
    return out


def _poll_alipay_attempt(
    state: LoopbackState, attempt_id: str, ts: int
) -> tuple[int, dict[str, Any]]:
    with state.lock:
        rec = state.alipay_attempts.get(attempt_id)
        if rec is None:
            return 404, {"error": "oauth_attempt_not_found"}
        if rec["status"] == "cancelled":
            return 409, {"error": "oauth_cancelled", "status": "cancelled"}
        if rec["status"] in ("confirmed", "consumed"):
            return (
                200,
                {
                    "attempt_id": rec["attempt_id"],
                    "state": rec["state"],
                    "status": "confirmed",
                    "session": rec.get("issued_session"),
                    "expires_at_ms": rec["expires_at_ms"],
                },
            )
        if rec["status"] == "expired" or ts > rec["expires_at_ms"]:
            rec["status"] = "expired"
            return 400, {"error": "oauth_expired", "status": "expired"}
        return (
            200,
            {
                "attempt_id": rec["attempt_id"],
                "state": rec["state"],
                "status": rec["status"],
                "session": rec.get("issued_session"),
                "poll_after_ms": 1000,
                "expires_at_ms": rec["expires_at_ms"],
            },
        )


def _cancel_alipay_attempt(
    state: LoopbackState, attempt_id: str | None, state_tok: str | None
) -> tuple[int, dict[str, Any]]:
    with state.lock:
        resolved_id = attempt_id
        if not resolved_id and state_tok:
            resolved_id = state.alipay_state_to_attempt.get(state_tok)
        if not resolved_id or resolved_id not in state.alipay_attempts:
            return 404, {"error": "oauth_attempt_not_found"}
        rec = state.alipay_attempts[resolved_id]
        rec["status"] = "cancelled"
        rec["issued_session"] = None
        return (
            200,
            {
                "ok": True,
                "attempt_id": rec["attempt_id"],
                "state": rec["state"],
                "status": "cancelled",
                "cancelled": True,
            },
        )


def handle_e2e_control(
    state: LoopbackState,
    method: str,
    path: str,
    query: str,
    body_bytes: bytes,
) -> tuple[int, dict[str, Any]]:
    """Typed `/__e2e__/*` control plane (immune to offline/faults/delays, excluded from recorded)."""
    ts = now_ms()
    body: dict[str, Any] = {}
    if body_bytes.strip():
        try:
            parsed = json.loads(body_bytes.decode("utf-8"))
            if not isinstance(parsed, dict):
                return 400, {
                    "ok": False,
                    "error": "invalid_payload",
                    "message": "request body must be a JSON object",
                }
            body = parsed
        except (UnicodeDecodeError, json.JSONDecodeError) as exc:
            return 400, {
                "ok": False,
                "error": "invalid_json",
                "message": str(exc),
            }

    if method == "POST" and path == "/__e2e__/reset":
        state.reset()
        return 200, {"ok": True, "reset": True, "ts_ms": ts}

    if method == "POST" and path == "/__e2e__/seed":
        with state.lock:
            # Stage & pre-validate EVERYTHING before mutating ANY state field
            staged_users: list[dict[str, Any]] = []
            staged_chats: list[dict[str, Any]] = []
            staged_messages: list[dict[str, Any]] = []
            staged_media: list[tuple[str, bytes]] = []
            staged_vault: list[tuple[int | None, str, str, dict[str, Any]]] = []
            staged_floors: dict[int, int] = {}
            staged_nodes: list[dict[str, Any]] = []
            staged_miniapps: list[dict[str, Any]] = []
            staged_ws_push: list[str] = []
            staged_alipay_confirm: dict[str, Any] | None = None
            staged_alipay_expire: str | None = None

            try:
                for u_spec in _opt_list(body, "users"):
                    if not isinstance(u_spec, dict):
                        raise ValueError("users[] entries must be objects")
                    uname = _str_nonempty_trim(u_spec.get("username"))
                    if not uname:
                        raise ValueError("users[].username must be non-empty")
                    first_name = _str_nonempty_trim(u_spec.get("first_name")) or uname
                    auth_verifier_b64 = _str_nonempty_trim(u_spec.get("auth_verifier_b64"))
                    password = (
                        None
                        if auth_verifier_b64 is not None
                        else _str_nonblank_raw(u_spec.get("password"))
                    )
                    raw_av = u_spec.get("auth_version")
                    if auth_verifier_b64 is not None:
                        auth_version = (
                            _parse_strict_int(raw_av, field="users[].auth_version", min_val=1)
                            if raw_av is not None
                            else 2
                        )
                    else:
                        if raw_av is not None:
                            _parse_strict_int(raw_av, field="users[].auth_version", min_val=1)
                        auth_version = 1
                    raw_kv = u_spec.get("key_version")
                    key_version = (
                        _parse_strict_int(raw_kv, field="users[].key_version", min_val=1)
                        if raw_kv is not None
                        else 1
                    )
                    auth_salt_b64 = _str_nonempty_trim(
                        u_spec.get("auth_salt_b64")
                    ) or base64.b64encode(bytes([0x42] * 16)).decode("ascii")
                    mint_session = u_spec.get("mint_session", True)
                    if mint_session is None:
                        mint_session = True
                    if not isinstance(mint_session, bool):
                        raise ValueError("users[].mint_session must be boolean")
                    kdf_params = u_spec.get("kdf_params")
                    if kdf_params is None:
                        kdf_params = {"m_cost_kib": 65536, "t_cost": 3, "p_cost": 4}
                    elif not isinstance(kdf_params, dict):
                        raise ValueError("users[].kdf_params must be an object")
                    staged_users.append(
                        {
                            "username": uname,
                            "first_name": first_name,
                            "auth_version": auth_version,
                            "key_version": key_version,
                            "password": password,
                            "auth_salt_b64": auth_salt_b64,
                            "auth_verifier_b64": auth_verifier_b64,
                            "kdf_params": kdf_params,
                            "vault_envelope": u_spec.get("vault_envelope"),
                            "two_factor_password": _str_nonblank_raw(
                                u_spec.get("two_factor_password")
                            ),
                            "two_factor_hint": _str_nonempty_trim(
                                u_spec.get("two_factor_hint")
                            ),
                            "two_factor_recovery_email": _str_nonempty_trim(
                                u_spec.get("two_factor_recovery_email")
                            ),
                            "mint_session": mint_session,
                            "device_name": _str_nonempty_trim(u_spec.get("device_name"))
                            or "iOS Simulator",
                        }
                    )

                for c_spec in _opt_list(body, "chats"):
                    if not isinstance(c_spec, dict):
                        raise ValueError("chats[] entries must be objects")
                    title = _str_nonempty_trim(c_spec.get("title"))
                    if not title:
                        raise ValueError("chats[].title must be non-empty")
                    cid_opt = (
                        _parse_strict_int(c_spec["chat_id"], field="chats[].chat_id")
                        if c_spec.get("chat_id") is not None
                        else None
                    )
                    chat_type = _normalize_chat_type(c_spec.get("chat_type"), default=1)
                    raw_mc = c_spec.get("member_count")
                    member_count = (
                        _parse_strict_int(raw_mc, field="chats[].member_count", min_val=0)
                        if raw_mc is not None
                        else 2
                    )
                    pinned = c_spec.get("pinned")
                    if pinned is not None and not isinstance(pinned, bool):
                        raise ValueError("chats[].pinned must be boolean")
                    unread_count = (
                        _parse_strict_int(
                            c_spec["unread_count"], field="chats[].unread_count", min_val=0
                        )
                        if c_spec.get("unread_count") is not None
                        else None
                    )
                    is_unread_marked = c_spec.get("is_unread_marked")
                    if is_unread_marked is not None and not isinstance(is_unread_marked, bool):
                        raise ValueError("chats[].is_unread_marked must be boolean")
                    bot_started = c_spec.get("bot_started")
                    if bot_started is not None and not isinstance(bot_started, bool):
                        raise ValueError("chats[].bot_started must be boolean")
                    staged_chats.append(
                        {
                            "chat_id": cid_opt,
                            "title": title,
                            "chat_type": chat_type,
                            "member_count": member_count,
                            "pinned": pinned,
                            "unread_count": unread_count,
                            "is_unread_marked": is_unread_marked,
                            "bot_started": bot_started,
                        }
                    )

                for m_spec in _opt_list(body, "messages"):
                    if not isinstance(m_spec, dict):
                        raise ValueError("messages[] entries must be objects")
                    cid = (
                        _parse_strict_int(m_spec["chat_id"], field="messages[].chat_id")
                        if m_spec.get("chat_id") is not None
                        else 1
                    )
                    text = m_spec.get("text")
                    if text is None:
                        text = m_spec.get("content", "")
                    if not isinstance(text, str):
                        raise ValueError("messages[].text must be a string")
                    client_nonce = (
                        _parse_strict_int(
                            m_spec["client_nonce"], field="messages[].client_nonce"
                        )
                        if m_spec.get("client_nonce") is not None
                        else None
                    )
                    sender_id_opt = (
                        _parse_strict_int(m_spec["sender_id"], field="messages[].sender_id")
                        if m_spec.get("sender_id") is not None
                        else None
                    )
                    message_type = _str_nonempty_trim(m_spec.get("message_type"))
                    media_file_id = _str_nonempty_trim(m_spec.get("media_file_id"))
                    instant_view_url = _str_nonempty_trim(m_spec.get("instant_view_url"))
                    staged_messages.append(
                        {
                            "chat_id": cid,
                            "text": text,
                            "client_nonce": client_nonce,
                            "sender_id": sender_id_opt,
                            "message_type": message_type,
                            "media_file_id": media_file_id,
                            "instant_view_url": instant_view_url,
                        }
                    )

                for b_spec in _opt_list(body, "media_blobs"):
                    if not isinstance(b_spec, dict):
                        raise ValueError("media_blobs[] entries must be objects")
                    fid = _str_nonempty_trim(b_spec.get("file_id"))
                    data_b64 = b_spec.get("data_b64")
                    if not fid or not isinstance(data_b64, str):
                        raise ValueError(
                            "media_blobs[] requires file_id and base64 data_b64"
                        )
                    raw = base64.b64decode(data_b64.encode("ascii"))
                    staged_media.append((fid, raw))

                for v_spec in _opt_list(body, "vault_items"):
                    if not isinstance(v_spec, dict):
                        raise ValueError("vault_items[] entries must be objects")
                    uid_opt = (
                        _parse_strict_int(v_spec["user_id"], field="vault_items[].user_id")
                        if v_spec.get("user_id") is not None
                        else None
                    )
                    cat = _str_nonempty_trim(v_spec.get("category")) or "general"
                    key = _str_nonempty_trim(v_spec.get("item_key"))
                    if not key:
                        raise ValueError("vault_items[].item_key must be non-empty")
                    staged_vault.append((uid_opt, cat, key, dict(v_spec)))

                if "journal_floors" in body and body["journal_floors"] is not None:
                    jf = body["journal_floors"]
                    if not isinstance(jf, dict):
                        raise ValueError("journal_floors must be an object")
                    for k, v in jf.items():
                        fk = _parse_strict_int(k, field="journal_floors key")
                        fv = _parse_strict_int(v, field="journal_floors value", min_val=0)
                        staged_floors[fk] = fv

                for n_spec in _opt_list(body, "remote_nodes"):
                    if not isinstance(n_spec, dict):
                        raise ValueError("remote_nodes[] entries must be objects")
                    did = _str_nonempty_trim(n_spec.get("device_id"))
                    if not did:
                        raise ValueError("remote_nodes[].device_id must be non-empty")
                    online_val = n_spec.get("online", True)
                    if not isinstance(online_val, bool):
                        raise ValueError("remote_nodes[].online must be boolean")
                    is_cur_val = n_spec.get("is_current_device", False)
                    if not isinstance(is_cur_val, bool):
                        raise ValueError("remote_nodes[].is_current_device must be boolean")
                    staged_nodes.append(
                        {
                            "device_id": did,
                            "name": _str_nonempty_trim(n_spec.get("name")) or did,
                            "online": online_val,
                            "is_current_device": is_cur_val,
                            "screen_capture_permission": str(
                                n_spec.get("screen_capture_permission", "granted_active")
                            ),
                            "accessibility_drag_permission": str(
                                n_spec.get("accessibility_drag_permission", "granted_active")
                            ),
                        }
                    )

                for app_spec in _opt_list(body, "miniapps"):
                    if not isinstance(app_spec, dict):
                        raise ValueError("miniapps[] entries must be objects")
                    aid = _str_nonempty_trim(app_spec.get("app_id"))
                    if not aid:
                        raise ValueError("miniapps[].app_id must be non-empty")
                    tos_val = app_spec.get("tos_accepted", False)
                    if not isinstance(tos_val, bool):
                        raise ValueError("miniapps[].tos_accepted must be boolean")
                    wmcp_val = app_spec.get("webmcp_enabled", True)
                    if not isinstance(wmcp_val, bool):
                        raise ValueError("miniapps[].webmcp_enabled must be boolean")
                    staged_miniapps.append(
                        {
                            "app_id": aid,
                            "title": _str_nonempty_trim(app_spec.get("title")) or aid,
                            "tos_accepted": tos_val,
                            "webmcp_enabled": wmcp_val,
                        }
                    )

                for frame_item in _opt_list(body, "ws_push_frames"):
                    if not isinstance(frame_item, str):
                        raise ValueError("ws_push_frames[] entries must be strings")
                    staged_ws_push.append(frame_item)

                if "alipay_confirm" in body and body["alipay_confirm"] is not None:
                    ac = body["alipay_confirm"]
                    if not isinstance(ac, dict):
                        raise ValueError("alipay_confirm must be an object")
                    aid = _str_nonempty_trim(ac.get("attempt_id"))
                    if not aid:
                        raise ValueError("alipay_confirm.attempt_id must be non-empty")
                    ac_uid_opt = (
                        _parse_strict_int(ac["user_id"], field="alipay_confirm.user_id")
                        if ac.get("user_id") is not None
                        else None
                    )
                    staged_alipay_confirm = {
                        "attempt_id": aid,
                        "user_id": ac_uid_opt,
                        "session_id": _str_nonempty_trim(ac.get("session_id")),
                        "token": _str_nonempty_trim(ac.get("token")),
                    }

                if "alipay_expire" in body and body["alipay_expire"] is not None:
                    exp_id = _str_nonempty_trim(body["alipay_expire"])
                    if not exp_id or exp_id not in state.alipay_attempts:
                        raise ValueError("alipay_expire attempt_id not found")
                    staged_alipay_expire = exp_id
            except (ValueError, TypeError, ApiParamError, binascii.Error) as exc:
                return 400, {
                    "ok": False,
                    "error": "invalid_seed_payload",
                    "message": str(exc),
                }

            # All validation succeeded — commit atomically
            seeded_users: list[dict[str, Any]] = []
            seeded_chats: list[dict[str, Any]] = []
            seeded_messages: list[dict[str, Any]] = []
            seeded_media_ids: list[str] = []
            seeded_vault_count = 0
            seeded_node_ids: list[str] = []
            seeded_miniapp_ids: list[str] = []

            for su in staged_users:
                uname = su["username"]
                uid = state.username_to_id.get(uname)
                if uid is None:
                    state.next_user_id += 1
                    uid = state.next_user_id
                user = {
                    "user_id": uid,
                    "username": uname,
                    "first_name": su["first_name"],
                    "auth_version": su["auth_version"],
                    "key_version": su["key_version"],
                    "password": su["password"],
                    "auth_salt_b64": su["auth_salt_b64"],
                    "auth_verifier_b64": su["auth_verifier_b64"],
                    "kdf_params": su["kdf_params"],
                    "vault_envelope": su["vault_envelope"],
                    "two_factor_password": su["two_factor_password"],
                    "two_factor_hint": su["two_factor_hint"],
                    "two_factor_recovery_email": su["two_factor_recovery_email"],
                }
                state.username_to_id[uname] = uid
                state.users_by_id[uid] = user
                sid = None
                tok = None
                if su["mint_session"]:
                    sess = state.mint_session(uid, su["device_name"], ts)
                    sid = sess["session_id"]
                    tok = sess["token"]
                seeded_users.append(
                    {
                        "user_id": uid,
                        "username": uname,
                        "session_id": sid,
                        "token": tok,
                    }
                )

            for sc in staged_chats:
                if sc["chat_id"] is not None:
                    cid = sc["chat_id"]
                    if cid >= state.next_chat_id:
                        state.next_chat_id = cid + 1
                else:
                    state.next_chat_id += 1
                    cid = state.next_chat_id
                chat_obj: dict[str, Any] = {
                    "id": cid,
                    "chat_id": cid,
                    "title": sc["title"],
                    "chat_type": sc["chat_type"],
                    "member_count": sc["member_count"],
                    "is_group": sc["chat_type"] in (1, 2, 3),
                }
                if sc["pinned"] is not None:
                    chat_obj["pinned"] = sc["pinned"]
                if sc["unread_count"] is not None:
                    chat_obj["unread_count"] = sc["unread_count"]
                if sc["is_unread_marked"] is not None:
                    chat_obj["is_unread_marked"] = sc["is_unread_marked"]
                if sc["bot_started"] is not None:
                    chat_obj["bot_started"] = sc["bot_started"]
                state.chats[cid] = chat_obj
                state.chat_messages.setdefault(cid, [])
                state.chat_pts.setdefault(cid, 0)
                seeded_chats.append({"chat_id": cid, "title": sc["title"]})

            for sm in staged_messages:
                cid = sm["chat_id"]
                mid = state.next_msg_id
                state.next_msg_id += 1
                pts = state.chat_pts.get(cid, 0) + 1
                state.chat_pts[cid] = pts
                sender_id = (
                    sm["sender_id"]
                    if sm["sender_id"] is not None
                    else state.default_user_id()
                )
                msg: dict[str, Any] = {
                    "message_id": mid,
                    "chat_id": cid,
                    "pts": pts,
                    "text": sm["text"],
                    "client_nonce": sm["client_nonce"],
                    "sender_id": sender_id,
                    "timestamp_ms": ts,
                }
                if sm["message_type"] is not None:
                    msg["message_type"] = sm["message_type"]
                if sm["media_file_id"] is not None:
                    msg["media_file_id"] = sm["media_file_id"]
                if sm["instant_view_url"] is not None:
                    msg["instant_view_url"] = sm["instant_view_url"]
                state.chat_messages.setdefault(cid, []).append(msg)
                seeded_messages.append({"message_id": mid, "chat_id": cid, "pts": pts})

            for fid, raw in staged_media:
                state.media_blobs[fid] = raw
                seeded_media_ids.append(fid)

            for uid_opt, cat, key, v_copy in staged_vault:
                uid = uid_opt if uid_opt is not None else state.default_user_id()
                state.vault_items[(uid, cat, key)] = v_copy
                seeded_vault_count += 1

            for fk, fv in staged_floors.items():
                state.journal_floors[fk] = fv

            for node in staged_nodes:
                state.remote_nodes[node["device_id"]] = node
                seeded_node_ids.append(node["device_id"])

            for app_obj in staged_miniapps:
                state.miniapps[app_obj["app_id"]] = app_obj
                seeded_miniapp_ids.append(app_obj["app_id"])

            if staged_ws_push:
                state.ws_push_queue.extend(staged_ws_push)

            if staged_alipay_confirm is not None:
                aid = staged_alipay_confirm["attempt_id"]
                rec = state.alipay_attempts.get(aid)
                if rec is None:
                    st_tok = f"fbs_{aid}"
                    rec = {
                        "attempt_id": aid,
                        "state": st_tok,
                        "status": "pending",
                        "issued_session": None,
                        "expires_at_ms": ts + 600_000,
                    }
                    state.alipay_attempts[aid] = rec
                    state.alipay_state_to_attempt[st_tok] = aid
                uid = (
                    staged_alipay_confirm["user_id"]
                    if staged_alipay_confirm["user_id"] is not None
                    else state.default_user_id()
                )
                state.ensure_user_by_id(uid)
                custom_tok = staged_alipay_confirm["token"]
                custom_sid = staged_alipay_confirm["session_id"]
                if custom_tok is not None:
                    seq = state.alloc_seq()
                    sid = custom_sid or f"sess_{uid}_{seq}"
                    sess = {
                        "session_id": sid,
                        "user_id": uid,
                        "token": custom_tok,
                        "device_name": "iOS Alipay Seeded",
                        "created_at_ms": ts,
                        "expires_at_ms": ts + 86_400_000,
                        "revoked": False,
                    }
                    state.revoked_tokens.discard(custom_tok)
                    state.sessions_by_token[custom_tok] = sess
                    state.session_id_to_token[sid] = custom_tok
                else:
                    sess = state.mint_session(uid, "iOS Alipay Seeded", ts)
                    if custom_sid is not None:
                        old_sid = sess["session_id"]
                        state.session_id_to_token.pop(old_sid, None)
                        sess["session_id"] = custom_sid
                        state.session_id_to_token[custom_sid] = sess["token"]
                rec["status"] = "confirmed"
                rec["issued_session"] = {
                    "user_id": uid,
                    "session_id": sess["session_id"],
                    "token": sess["token"],
                    "expires_at": ms_to_rfc3339(sess["expires_at_ms"]),
                    "expires_at_ms": sess["expires_at_ms"],
                }

            if staged_alipay_expire is not None:
                state.alipay_attempts[staged_alipay_expire]["status"] = "expired"
                state.alipay_attempts[staged_alipay_expire]["expires_at_ms"] = ts - 1000

            return (
                200,
                {
                    "ok": True,
                    "seeded": {
                        "users": seeded_users,
                        "chats": seeded_chats,
                        "messages": seeded_messages,
                        "media_blob_ids": seeded_media_ids,
                        "vault_item_count": seeded_vault_count,
                        "remote_node_ids": seeded_node_ids,
                        "miniapp_ids": seeded_miniapp_ids,
                    },
                },
            )

    if method == "DELETE" and path == "/__e2e__/fault":
        with state.lock:
            state.faults.clear()
            state.delays.clear()
            state.offline = False
            state.drop_next_ws = False
            return (
                200,
                {
                    "ok": True,
                    "offline": False,
                    "active_fault_count": 0,
                    "active_delay_count": 0,
                },
            )

    if method == "POST" and path == "/__e2e__/fault":
        # Atomic pre-validation strictly BEFORE acquiring lock or mutating ANY state
        try:
            clear_flag: bool = False
            if "clear" in body and body["clear"] is not None:
                if not isinstance(body["clear"], bool):
                    raise ValueError("clear must be boolean")
                clear_flag = body["clear"]

            offline_opt: bool | None = None
            if "offline" in body and body["offline"] is not None:
                if not isinstance(body["offline"], bool):
                    raise ValueError("offline must be boolean")
                offline_opt = body["offline"]

            drop_next_ws_opt: bool | None = None
            if "drop_next_ws" in body and body["drop_next_ws"] is not None:
                if not isinstance(body["drop_next_ws"], bool):
                    raise ValueError("drop_next_ws must be boolean")
                drop_next_ws_opt = body["drop_next_ws"]

            raw_fault_entries: list[Any] = []
            if "http_fault" in body and body["http_fault"] is not None:
                raw_fault_entries.append(body["http_fault"])
            if "http_faults" in body and body["http_faults"] is not None:
                if not isinstance(body["http_faults"], list):
                    raise ValueError("http_faults must be a list")
                raw_fault_entries.extend(body["http_faults"])

            staged_faults: list[dict[str, Any]] = []
            for f in raw_fault_entries:
                if not isinstance(f, dict):
                    raise ValueError("http_fault entry must be an object")
                prefix = _str_nonempty_trim(f.get("path_prefix"))
                if not prefix:
                    raise ValueError("http_fault requires non-empty path_prefix")
                status = f.get("status")
                if not _is_strict_int(status) or status < 200 or status > 599 or status in (204, 304):
                    raise ValueError(
                        "http_fault requires valid integer status in 200..599 excluding 204/304"
                    )
                f_method: str | None = None
                if "method" in f and f["method"] is not None:
                    f_method_trim = _str_nonempty_trim(f["method"])
                    if not f_method_trim:
                        raise ValueError("http_fault.method must be a non-empty string")
                    f_method = f_method_trim.upper()
                retry_after_ms = f.get("retry_after_ms")
                if retry_after_ms is not None and (
                    not _is_strict_int(retry_after_ms) or retry_after_ms < 0
                ):
                    raise ValueError("http_fault.retry_after_ms must be a non-negative integer")
                remaining_hits = f.get("remaining_hits")
                if remaining_hits is not None and (
                    not _is_strict_int(remaining_hits) or remaining_hits < 0
                ):
                    raise ValueError("http_fault.remaining_hits must be a non-negative integer")
                f_body = f.get("body")
                if f_body is None:
                    f_body = {"error": f"injected_http_{status}"}
                elif not isinstance(f_body, dict):
                    raise ValueError("http_fault.body must be an object")
                staged_faults.append(
                    {
                        "path_prefix": prefix,
                        "method": f_method,
                        "status": status,
                        "body": f_body,
                        "retry_after_ms": retry_after_ms,
                        "remaining_hits": remaining_hits,
                    }
                )

            staged_delay: tuple[str, int] | None = None
            if "delay" in body and body["delay"] is not None:
                d = body["delay"]
                if not isinstance(d, dict):
                    raise ValueError("delay must be an object")
                d_prefix = _str_nonempty_trim(d.get("path_prefix"))
                delay_ms = d.get("delay_ms")
                if not d_prefix or not _is_strict_int(delay_ms) or delay_ms < 0:
                    raise ValueError(
                        "delay requires non-empty path_prefix and non-negative integer delay_ms"
                    )
                staged_delay = (d_prefix, delay_ms)
        except (ValueError, TypeError) as exc:
            return 400, {
                "ok": False,
                "error": "invalid_fault_payload",
                "message": str(exc),
            }

        with state.lock:
            if clear_flag:
                state.faults.clear()
                state.delays.clear()
                state.offline = False
                state.drop_next_ws = False
            if offline_opt is not None:
                state.offline = offline_opt
            if drop_next_ws_opt is not None:
                state.drop_next_ws = drop_next_ws_opt
            if staged_faults:
                state.faults.extend(staged_faults)
            if staged_delay is not None:
                state.delays[staged_delay[0]] = staged_delay[1]

            active_faults = sum(
                1
                for f in state.faults
                if f.get("remaining_hits") is None or f["remaining_hits"] > 0
            )
            return (
                200,
                {
                    "ok": True,
                    "offline": state.offline,
                    "active_fault_count": active_faults,
                    "active_delay_count": len(state.delays),
                },
            )

    if method == "GET" and path == "/__e2e__/state":
        path_prefix = extract_query_param(query, "path_prefix")
        include_bodies_raw = (extract_query_param(query, "include_bodies") or "true").lower()
        include_bodies = include_bodies_raw not in ("false", "0", "no")
        with state.lock:
            matched: list[dict[str, Any]] = []
            for rec in state.recorded:
                if path_prefix and not rec["path"].startswith(path_prefix):
                    continue
                if include_bodies:
                    matched.append(dict(rec))
                else:
                    matched.append(
                        {
                            "method": rec["method"],
                            "path": rec["path"],
                            "query": rec["query"],
                            "body_size": rec["body_size"],
                            "body_preview": "",
                            "timestamp_ms": rec["timestamp_ms"],
                        }
                    )
            active_sessions = [
                s["session_id"]
                for s in state.sessions_by_token.values()
                if not s.get("revoked")
            ]
            total_messages = sum(len(v) for v in state.chat_messages.values())
            active_faults = sum(
                1
                for f in state.faults
                if f.get("remaining_hits") is None or f["remaining_hits"] > 0
            )
            return (
                200,
                {
                    "ok": True,
                    "version": state.version,
                    "offline": state.offline,
                    "active_fault_count": active_faults,
                    "recorded_request_count": len(state.recorded),
                    "matched_request_count": len(matched),
                    "recorded_requests": matched,
                    "counts": {
                        "users": len(state.users_by_id),
                        "active_sessions": len(active_sessions),
                        "revoked_tokens": len(state.revoked_tokens),
                        "chats": len(state.chats),
                        "messages": total_messages,
                        "media_blobs": len(state.media_blobs),
                        "vault_items": len(state.vault_items),
                        "computer_leases": len(state.computer_leases),
                        "remote_nodes": len(state.remote_nodes),
                        "miniapps": len(state.miniapps),
                        "tasks": len(state.tasks),
                    },
                    "chat_ids": sorted(state.chats.keys()),
                    "active_session_ids": sorted(active_sessions),
                    "ws_received_frames": list(state.ws_received_frames),
                },
            )

    return (
        404,
        {
            "error": "not_found",
            "message": f"unrecognized e2e control route: {method} {path}",
            "path": path,
            "method": method,
        },
    )


def route_api_request(
    state: LoopbackState,
    method: str,
    path: str,
    query: str,
    headers: dict[str, str],
    body: dict[str, Any],
) -> tuple[int, dict[str, Any]]:
    ts = now_ms()
    bearer_token = extract_bearer_token(headers, query)

    caller_session: dict[str, Any] | None = None
    with state.lock:
        if bearer_token is not None:
            if is_protected_route(path) and state.is_token_rejected(bearer_token):
                return 401, {"error": "unauthorized", "message": "session_revoked"}
            sess = state.sessions_by_token.get(bearer_token)
            if sess is not None and not sess.get("revoked"):
                caller_session = dict(sess)

    # Health, status & OAuth well-known discovery
    if method in ("GET", "HEAD") and path in ("/health", "/healthz", "/api/v1/health"):
        return 200, {"ok": True, "version": state.version, "ts_ms": ts}

    if method in ("GET", "HEAD") and path == "/api/v1/status":
        return (
            200,
            {
                "ok": True,
                "server": "gpui_telegram_server",
                "status": "healthy",
                "version": state.version,
                "ts_ms": ts,
            },
        )

    if method == "GET" and path in (
        "/.well-known/oauth-authorization-server",
        "/.well-known/oauth-protected-resource",
    ):
        return (
            200,
            {
                "issuer": "http://127.0.0.1",
                "authorization_endpoint": "/api/v1/mcp/oauth/authorize",
                "token_endpoint": "/api/v1/mcp/oauth/token",
                "registration_endpoint": "/api/v1/mcp/oauth/register",
                "mcp_protocol_version": "2025-11-25",
                "supported_mcp_versions": ["2025-11-25"],
                "code_challenge_methods_supported": ["S256"],
                "scopes_supported": [
                    "mcp:read",
                    "mcp:write",
                    "mcp:tools",
                    "mcp:resources",
                    "mcp:prompts",
                    "mcp:tasks",
                    "mcp:sampling",
                    "mcp:roots",
                    "mcp:notifications",
                    "mcp:elicitation",
                    "mcp:computer",
                    "mcp:agent",
                    "mcp:vault",
                    "mcp:audit",
                ],
            },
        )

    # Auth KDF params
    if method in ("GET", "POST") and path in (
        "/api/v1/auth/kdf-params",
        "/api/v1/auth/kdf_params",
    ):
        ident = (
            extract_query_param(query, "identifier")
            or extract_query_param(query, "username")
            or _str_nonempty_trim(body.get("identifier"))
            or _str_nonempty_trim(body.get("username"))
        )
        with state.lock:
            if ident and ident in state.username_to_id:
                uid = state.username_to_id[ident]
                user = state.users_by_id.get(uid)
                if user is not None:
                    effective_auth_version = (
                        user["auth_version"]
                        if user["auth_verifier_b64"] is not None
                        else 1
                    )
                    return (
                        200,
                        {
                            "auth_version": effective_auth_version,
                            "key_version": user["key_version"],
                            "kdf_alg": (
                                "split_key_v2"
                                if effective_auth_version >= 2
                                else "legacy_v1"
                            ),
                            "auth_salt_b64": user["auth_salt_b64"],
                            "kdf_params": user["kdf_params"],
                        },
                    )
        return (
            200,
            {
                "auth_version": 2,
                "key_version": 1,
                "kdf_alg": "split_key_v2",
                "auth_salt_b64": base64.b64encode(bytes([0x42] * 16)).decode("ascii"),
                "kdf_params": {"m_cost_kib": 65536, "t_cost": 3, "p_cost": 4},
            },
        )

    # Auth register
    if method == "POST" and path == "/api/v1/auth/register":
        with state.lock:
            uname = _str_nonempty_trim(body.get("username"))
            if not uname:
                return 400, {"error": "invalid_username", "message": "username is required"}
            if uname in state.username_to_id:
                return 409, {"error": "user_already_exists"}
            auth_verifier_b64 = _str_nonempty_trim(body.get("auth_verifier_b64"))
            password = (
                None
                if auth_verifier_b64 is not None
                else _str_nonblank_raw(body.get("password"))
            )
            if auth_verifier_b64 is None and password is None:
                return (
                    400,
                    {
                        "error": "invalid_credentials",
                        "message": "password or auth_verifier_b64 is required",
                    },
                )
            raw_av = body.get("auth_version")
            if auth_verifier_b64 is not None:
                auth_version = (
                    parse_int_or_400(raw_av, field="auth_version", min_val=1)
                    if raw_av is not None
                    else 2
                )
            else:
                if raw_av is not None:
                    parse_int_or_400(raw_av, field="auth_version", min_val=1)
                auth_version = 1
            vault_envelope = body.get("vault_envelope")
            default_kv = (
                vault_envelope.get("key_version", 1)
                if isinstance(vault_envelope, dict)
                else 1
            )
            if "key_version" in body:
                key_version = parse_int_or_400(body["key_version"], field="key_version", min_val=1)
            else:
                key_version = parse_int_or_400(default_kv, field="vault_envelope.key_version", min_val=1)
            state.next_user_id += 1
            uid = state.next_user_id
            first_name = _str_nonempty_trim(body.get("first_name")) or uname
            auth_salt_b64 = _str_nonempty_trim(
                body.get("auth_salt_b64")
            ) or base64.b64encode(bytes([0x42] * 16)).decode("ascii")
            kdf_params = body.get(
                "kdf_params", {"m_cost_kib": 65536, "t_cost": 3, "p_cost": 4}
            )
            device_name = _str_nonempty_trim(body.get("device_name")) or "iOS E2E Primary"
            state.users_by_id[uid] = {
                "user_id": uid,
                "username": uname,
                "first_name": first_name,
                "auth_version": auth_version,
                "key_version": key_version,
                "password": password,
                "auth_salt_b64": auth_salt_b64,
                "auth_verifier_b64": auth_verifier_b64,
                "kdf_params": kdf_params,
                "vault_envelope": vault_envelope,
                "two_factor_password": None,
                "two_factor_hint": None,
                "two_factor_recovery_email": None,
            }
            state.username_to_id[uname] = uid
            sess = state.mint_session(uid, device_name, ts)
            return (
                200,
                {
                    "user_id": uid,
                    "username": uname,
                    "first_name": first_name,
                    "session_id": sess["session_id"],
                    "token": sess["token"],
                    "created_at": ms_to_rfc3339(sess["created_at_ms"]),
                    "expires_at": ms_to_rfc3339(sess["expires_at_ms"]),
                    "expires_at_ms": sess["expires_at_ms"],
                    "auth_version": auth_version,
                    "key_version": key_version,
                    "vault_unlocked": True,
                    "requires_2fa": False,
                },
            )

    # Auth login
    if method == "POST" and path == "/api/v1/auth/login":
        with state.lock:
            uname_opt = _str_nonempty_trim(body.get("username")) or _str_nonempty_trim(
                body.get("identifier")
            )
            device_name = _str_nonempty_trim(body.get("device_name")) or "iOS Device"
            provided_v = _str_nonempty_trim(body.get("auth_verifier_b64"))
            provided_pw = _str_nonblank_raw(body.get("password"))

            if uname_opt and uname_opt in state.username_to_id:
                uid = state.username_to_id[uname_opt]
                user = state.users_by_id[uid]
                if user["auth_verifier_b64"] is not None:
                    if provided_v is not None:
                        if provided_v != user["auth_verifier_b64"]:
                            return 401, {"error": "invalid_credentials"}
                    elif user["password"] is not None:
                        if provided_pw != user["password"]:
                            return 401, {"error": "invalid_credentials"}
                    else:
                        return 401, {"error": "invalid_credentials"}
                elif user["password"] is not None:
                    if provided_pw != user["password"]:
                        return 401, {"error": "invalid_credentials"}
                else:
                    return 401, {"error": "invalid_credentials"}

                if user["two_factor_password"] is not None:
                    seq = state.alloc_seq()
                    challenge_token = f"tfa_{uid}_{seq}"
                    state.two_factor_challenges[challenge_token] = {
                        "user_id": uid,
                        "device_name": device_name,
                        "expires_at_ms": ts + 300_000,
                    }
                    return (
                        200,
                        {
                            "requires_2fa": True,
                            "challenge_token": challenge_token,
                            "hint": user["two_factor_hint"],
                            "user_id": uid,
                        },
                    )

                sess = state.mint_session(uid, device_name, ts)
                return (
                    200,
                    {
                        "user_id": uid,
                        "username": user["username"],
                        "first_name": user["first_name"],
                        "session_id": sess["session_id"],
                        "token": sess["token"],
                        "created_at": ms_to_rfc3339(sess["created_at_ms"]),
                        "expires_at": ms_to_rfc3339(sess["expires_at_ms"]),
                        "expires_at_ms": sess["expires_at_ms"],
                        "auth_version": user["auth_version"],
                        "key_version": user["key_version"],
                        "vault_envelope": user["vault_envelope"],
                        "vault_unlocked": True,
                        "requires_2fa": False,
                    },
                )

            if provided_v is None and provided_pw is None:
                return (
                    401,
                    {"error": "invalid_credentials", "message": "missing credentials"},
                )
            raw_av = body.get("auth_version")
            if provided_v is not None:
                auth_version = (
                    parse_int_or_400(raw_av, field="auth_version", min_val=1)
                    if raw_av is not None
                    else 2
                )
            else:
                if raw_av is not None:
                    parse_int_or_400(raw_av, field="auth_version", min_val=1)
                auth_version = 1
            state.next_user_id += 1
            uid = state.next_user_id
            uname = uname_opt or f"user_{uid}"
            password = None if provided_v is not None else provided_pw
            state.users_by_id[uid] = {
                "user_id": uid,
                "username": uname,
                "first_name": uname,
                "auth_version": auth_version,
                "key_version": 1,
                "password": password,
                "auth_salt_b64": _str_nonempty_trim(
                    body.get("auth_salt_b64")
                )
                or base64.b64encode(bytes([0x42] * 16)).decode("ascii"),
                "auth_verifier_b64": provided_v,
                "kdf_params": body.get(
                    "kdf_params", {"m_cost_kib": 65536, "t_cost": 3, "p_cost": 4}
                ),
                "vault_envelope": body.get("vault_envelope"),
                "two_factor_password": None,
                "two_factor_hint": None,
                "two_factor_recovery_email": None,
            }
            state.username_to_id[uname] = uid
            sess = state.mint_session(uid, device_name, ts)
            return (
                200,
                {
                    "user_id": uid,
                    "username": uname,
                    "first_name": uname,
                    "session_id": sess["session_id"],
                    "token": sess["token"],
                    "created_at": ms_to_rfc3339(sess["created_at_ms"]),
                    "expires_at": ms_to_rfc3339(sess["expires_at_ms"]),
                    "expires_at_ms": sess["expires_at_ms"],
                    "auth_version": auth_version,
                    "key_version": 1,
                    "vault_unlocked": True,
                    "requires_2fa": False,
                },
            )

    # Auth logout
    if method == "POST" and path == "/api/v1/auth/logout":
        with state.lock:
            revoked_sid = None
            if bearer_token is not None:
                sess = state.sessions_by_token.get(bearer_token)
                if sess is not None:
                    sess["revoked"] = True
                    revoked_sid = sess["session_id"]
                state.revoked_tokens.add(bearer_token)
            gen_num = state.alloc_seq()
            return (
                200,
                {
                    "ok": True,
                    "logged_out": True,
                    "revoked": True,
                    "session_id": revoked_sid,
                    "account_generation": gen_num,
                },
            )

    # Split-Key V2 upgrade & atomic password rotation
    if method == "POST" and path in (
        "/api/v1/auth/upgrade-split-key",
        "/api/v1/auth/upgrade_split_key",
    ):
        with state.lock:
            uid = caller_session["user_id"] if caller_session else state.default_user_id()
            user = state.ensure_user_by_id(uid)
            has_cur_pw = "current_password" in body and body["current_password"] is not None
            has_cur_v = (
                "current_auth_verifier_b64" in body
                and body["current_auth_verifier_b64"] is not None
            )
            cur_pw = _str_nonblank_raw(body.get("current_password"))
            cur_v = _str_nonempty_trim(body.get("current_auth_verifier_b64"))
            if user["password"] is not None and has_cur_pw and cur_pw != user["password"]:
                return 401, {"error": "invalid_credentials"}
            if (
                user["auth_verifier_b64"] is not None
                and has_cur_v
                and cur_v != user["auth_verifier_b64"]
            ):
                return 401, {"error": "invalid_credentials"}
            new_salt = _str_nonempty_trim(
                body.get("new_auth_salt_b64") or body.get("auth_salt_b64")
            )
            new_v = _str_nonempty_trim(
                body.get("new_auth_verifier_b64") or body.get("auth_verifier_b64")
            )
            raw_av = body.get("auth_version")
            parsed_av = (
                parse_int_or_400(raw_av, field="auth_version", min_val=1)
                if raw_av is not None
                else None
            )
            parsed_kv = (
                parse_int_or_400(body["key_version"], field="key_version", min_val=1)
                if "key_version" in body and body["key_version"] is not None
                else None
            )
            if new_salt:
                user["auth_salt_b64"] = new_salt
            if new_v:
                user["auth_verifier_b64"] = new_v
                user["password"] = None
            new_env = body.get("rewrapped_vault_envelope") or body.get("vault_envelope")
            if new_env is not None:
                user["vault_envelope"] = new_env
            if user["auth_verifier_b64"] is not None:
                user["auth_version"] = parsed_av if parsed_av is not None else 2
            else:
                user["auth_version"] = 1
            if parsed_kv is not None:
                user["key_version"] = parsed_kv
            gen_num = state.alloc_seq()
            return (
                200,
                {
                    "ok": True,
                    "upgraded": True,
                    "auth_version": user["auth_version"],
                    "key_version": user["key_version"],
                    "vault_unlocked": True,
                    "account_generation": gen_num,
                },
            )

    if method == "POST" and path in (
        "/api/v1/auth/change-password",
        "/api/v1/auth/change_password",
    ):
        with state.lock:
            uid = caller_session["user_id"] if caller_session else state.default_user_id()
            user = state.ensure_user_by_id(uid)
            cur_v = _str_nonempty_trim(body.get("current_auth_verifier_b64"))
            cur_pw = _str_nonblank_raw(body.get("current_password"))
            if user["auth_verifier_b64"] is not None:
                if cur_v is not None:
                    if cur_v != user["auth_verifier_b64"]:
                        return 401, {"error": "invalid_credentials"}
                elif user["password"] is not None:
                    if cur_pw != user["password"]:
                        return 401, {"error": "invalid_credentials"}
                else:
                    return 401, {"error": "invalid_credentials"}
            elif user["password"] is not None:
                if cur_pw != user["password"]:
                    return 401, {"error": "invalid_credentials"}

            new_salt = _str_nonempty_trim(
                body.get("new_auth_salt_b64") or body.get("auth_salt_b64")
            )
            new_v = _str_nonempty_trim(
                body.get("new_auth_verifier_b64") or body.get("auth_verifier_b64")
            )
            new_pw = _str_nonblank_raw(body.get("new_password") or body.get("password"))
            if new_v is None and new_pw is None:
                return 400, {"error": "invalid_params", "message": "missing new credential"}
            raw_av = body.get("auth_version")
            parsed_av = (
                parse_int_or_400(raw_av, field="auth_version", min_val=1)
                if raw_av is not None
                else None
            )
            next_kv = (
                parse_int_or_400(body["key_version"], field="key_version", min_val=1)
                if "key_version" in body and body["key_version"] is not None
                else user["key_version"] + 1
            )
            if new_salt:
                user["auth_salt_b64"] = new_salt
            if new_v is not None:
                user["auth_verifier_b64"] = new_v
                user["password"] = None
                user["auth_version"] = parsed_av if parsed_av is not None else 2
            else:
                user["password"] = new_pw
                user["auth_verifier_b64"] = None
                user["auth_version"] = 1
            new_env = body.get("rewrapped_vault_envelope") or body.get("vault_envelope")
            if new_env is not None:
                user["vault_envelope"] = new_env
            user["key_version"] = next_kv

            old_tokens = []
            for sess in state.sessions_by_token.values():
                if sess["user_id"] == uid and not sess["revoked"]:
                    sess["revoked"] = True
                    old_tokens.append(sess["token"])
            for tok in old_tokens:
                state.revoked_tokens.add(tok)
            dev_name = caller_session["device_name"] if caller_session else "iOS Device"
            fresh = state.mint_session(uid, dev_name, ts)
            gen_num = state.alloc_seq()
            return (
                200,
                {
                    "ok": True,
                    "changed": True,
                    "upgraded": True,
                    "auth_version": user["auth_version"],
                    "key_version": user["key_version"],
                    "vault_unlocked": True,
                    "account_generation": gen_num,
                    "revoked_sessions_count": len(old_tokens),
                    "session_id": fresh["session_id"],
                    "token": fresh["token"],
                    "expires_at": ms_to_rfc3339(fresh["expires_at_ms"]),
                    "expires_at_ms": fresh["expires_at_ms"],
                },
            )

    # Auth me & device sessions
    if method == "GET" and path == "/api/v1/auth/me":
        with state.lock:
            uid = caller_session["user_id"] if caller_session else state.default_user_id()
            user = state.ensure_user_by_id(uid)
            sid = caller_session["session_id"] if caller_session else "sess_default"
            dev = caller_session["device_name"] if caller_session else "iOS Device"
            created_ms = caller_session["created_at_ms"] if caller_session else ts
            expires_ms = (
                caller_session["expires_at_ms"] if caller_session else ts + 86_400_000
            )
            return (
                200,
                {
                    "user_id": uid,
                    "username": user["username"],
                    "first_name": user["first_name"],
                    "session_id": sid,
                    "device_name": dev,
                    "account_generation": state.next_seq,
                    "created_at": ms_to_rfc3339(created_ms),
                    "expires_at": ms_to_rfc3339(expires_ms),
                    "expires_at_ms": expires_ms,
                    "auth_version": user["auth_version"],
                    "key_version": user["key_version"],
                    "two_factor_enabled": user["two_factor_password"] is not None,
                },
            )

    if method == "GET" and path == "/api/v1/auth/sessions":
        with state.lock:
            uid = caller_session["user_id"] if caller_session else state.default_user_id()
            cur_sid = caller_session["session_id"] if caller_session else None
            sessions = [
                {
                    "session_id": s["session_id"],
                    "device_name": s["device_name"],
                    "created_at_ms": s["created_at_ms"],
                    "expires_at_ms": s["expires_at_ms"],
                    "is_current": s["session_id"] == cur_sid,
                }
                for s in state.sessions_by_token.values()
                if s["user_id"] == uid and not s["revoked"]
            ]
            sessions.sort(key=lambda item: item["session_id"])
            return 200, {"user_id": uid, "sessions": sessions}

    if (
        (method in ("DELETE", "POST") and path == "/api/v1/auth/sessions/others")
        or (method == "POST" and path == "/api/v1/auth/sessions/revoke-others")
    ):
        with state.lock:
            uid = caller_session["user_id"] if caller_session else state.default_user_id()
            cur_sid = caller_session["session_id"] if caller_session else None
            count = 0
            for s in state.sessions_by_token.values():
                if s["user_id"] == uid and not s["revoked"] and s["session_id"] != cur_sid:
                    s["revoked"] = True
                    state.revoked_tokens.add(s["token"])
                    count += 1
            return 200, {"ok": True, "revoked_count": count}

    if (
        method == "DELETE" and path.startswith("/api/v1/auth/sessions/")
    ) or (
        method == "POST"
        and path.startswith("/api/v1/auth/sessions/")
        and path.endswith("/revoke")
    ):
        sid = path[len("/api/v1/auth/sessions/") :]
        if sid.endswith("/revoke"):
            sid = sid[: -len("/revoke")]
        with state.lock:
            tok = state.session_id_to_token.get(sid)
            if not tok or tok not in state.sessions_by_token:
                return 404, {"error": "session_not_found"}
            state.sessions_by_token[tok]["revoked"] = True
            state.revoked_tokens.add(tok)
            gen_num = state.alloc_seq()
            return 200, {"revoked": True, "session_id": sid, "account_generation": gen_num}

    # Cloud 2FA
    if method == "GET" and path in (
        "/api/v1/auth/2fa",
        "/api/v1/auth/two-factor",
        "/api/v1/auth/two-factor/status",
    ):
        with state.lock:
            uid = caller_session["user_id"] if caller_session else state.default_user_id()
            user = state.ensure_user_by_id(uid)
            enabled = user["two_factor_password"] is not None
            return (
                200,
                {
                    "enabled": enabled,
                    "has_password": enabled,
                    "hint": user["two_factor_hint"],
                    "recovery_email": user["two_factor_recovery_email"],
                    "has_recovery_email": user["two_factor_recovery_email"] is not None,
                },
            )

    if method in ("POST", "PUT") and path in (
        "/api/v1/auth/2fa",
        "/api/v1/auth/two-factor",
        "/api/v1/auth/2fa/set",
        "/api/v1/auth/two-factor/set",
    ):
        raw_pw = body.get("new_password") if "new_password" in body else body.get("password")
        new_pw = raw_pw.strip() if isinstance(raw_pw, str) else ""
        if not new_pw or len(new_pw.encode("utf-8")) > 1024:
            return 400, {"error": "invalid_password"}
        with state.lock:
            uid = caller_session["user_id"] if caller_session else state.default_user_id()
            user = state.ensure_user_by_id(uid)
            if user["two_factor_password"] is not None:
                if body.get("current_password") != user["two_factor_password"]:
                    return 401, {"error": "invalid_current_password"}
            user["two_factor_password"] = new_pw
            user["two_factor_hint"] = _str_nonempty_trim(body.get("hint"))
            user["two_factor_recovery_email"] = _str_nonempty_trim(
                body.get("recovery_email")
            )
            return (
                200,
                {
                    "enabled": True,
                    "has_password": True,
                    "hint": user["two_factor_hint"],
                    "recovery_email": user["two_factor_recovery_email"],
                    "has_recovery_email": user["two_factor_recovery_email"] is not None,
                },
            )

    if (
        method == "DELETE" and path in ("/api/v1/auth/2fa", "/api/v1/auth/two-factor")
    ) or (
        method == "POST"
        and path in ("/api/v1/auth/2fa/disable", "/api/v1/auth/two-factor/disable")
    ):
        with state.lock:
            uid = caller_session["user_id"] if caller_session else state.default_user_id()
            user = state.ensure_user_by_id(uid)
            cur = body.get("current_password") or body.get("password")
            if user["two_factor_password"] is not None and cur != user["two_factor_password"]:
                return 401, {"error": "invalid_current_password"}
            user["two_factor_password"] = None
            user["two_factor_hint"] = None
            return 200, {"enabled": False, "has_password": False}

    if method == "POST" and path in (
        "/api/v1/auth/2fa/verify",
        "/api/v1/auth/two-factor/verify",
    ):
        chal = _str_nonempty_trim(body.get("challenge_token"))
        pw = body.get("cloud_password") if "cloud_password" in body else body.get("password")
        if not isinstance(pw, str) or not pw.strip():
            return 401, {"error": "invalid_cloud_password"}
        with state.lock:
            rec = state.two_factor_challenges.get(chal or "")
            if rec is None:
                return 401, {"error": "invalid_challenge"}
            if ts > rec["expires_at_ms"]:
                return 401, {"error": "challenge_expired"}
            user = state.ensure_user_by_id(rec["user_id"])
            expected_pw = user.get("two_factor_password")
            if expected_pw is None or expected_pw != pw:
                return 401, {"error": "invalid_cloud_password"}
            state.two_factor_challenges.pop(chal, None)
            dev = _str_nonempty_trim(body.get("device_name")) or rec["device_name"]
            sess = state.mint_session(rec["user_id"], dev, ts)
            return (
                200,
                {
                    "verified": True,
                    "user_id": rec["user_id"],
                    "session_id": sess["session_id"],
                    "token": sess["token"],
                    "expires_at": ms_to_rfc3339(sess["expires_at_ms"]),
                    "expires_at_ms": sess["expires_at_ms"],
                },
            )

    # QR Device Pairing
    if method == "POST" and path == "/api/v1/auth/pair/init":
        with state.lock:
            seq = state.alloc_seq()
            tok = f"pair_{seq}_{uuid.uuid4().hex[:8]}"
            exp = ts + 300_000
            qr_url = f"tg://login?token={tok}"
            state.qr_pairs[tok] = {
                "pair_token": tok,
                "device_name": _str_nonempty_trim(body.get("device_name")) or "Paired iOS",
                "status": "pending",
                "requires_2fa": False,
                "accepted_by_user_id": None,
                "issued_session": None,
                "expires_at_ms": exp,
            }
            return (
                200,
                {
                    "pair_token": tok,
                    "qr_url": qr_url,
                    "qr_payload": qr_url,
                    "expires_at_ms": exp,
                },
            )

    if method == "POST" and path == "/api/v1/auth/pair/accept":
        tok = _str_nonempty_trim(body.get("pair_token")) or ""
        with state.lock:
            rec = state.qr_pairs.get(tok)
            if rec is None or rec["status"] != "pending" or ts > rec["expires_at_ms"]:
                return 400, {"error": "invalid_or_expired_pair_token"}
            uid = caller_session["user_id"] if caller_session else state.default_user_id()
            user = state.ensure_user_by_id(uid)
            rec["accepted_by_user_id"] = uid
            rec["status"] = "confirmed"
            rec["requires_2fa"] = user["two_factor_password"] is not None
            if not rec["requires_2fa"]:
                sess = state.mint_session(uid, rec["device_name"], ts)
                rec["issued_session"] = {
                    "user_id": uid,
                    "session_id": sess["session_id"],
                    "token": sess["token"],
                    "expires_at": ms_to_rfc3339(sess["expires_at_ms"]),
                    "expires_at_ms": sess["expires_at_ms"],
                }
            return 200, {"status": "confirmed", "requires_2fa": rec["requires_2fa"]}

    if method in ("GET", "POST") and (
        path == "/api/v1/auth/pair/poll" or path.startswith("/api/v1/auth/pair/poll/")
    ):
        if path.startswith("/api/v1/auth/pair/poll/"):
            tok = path[len("/api/v1/auth/pair/poll/") :]
        else:
            tok = (
                _str_nonempty_trim(body.get("pair_token"))
                or extract_query_param(query, "pair_token")
                or ""
            )
        with state.lock:
            rec = state.qr_pairs.get(tok)
            if rec is None:
                return 404, {"error": "pair_token_not_found"}
            return (
                200,
                {
                    "pair_token": rec["pair_token"],
                    "status": rec["status"],
                    "requires_2fa": rec["requires_2fa"],
                    "session": rec["issued_session"],
                    "expires_at_ms": rec["expires_at_ms"],
                },
            )

    if method == "POST" and path == "/api/v1/auth/pair/verify":
        tok = _str_nonempty_trim(body.get("pair_token")) or ""
        pw = _str_nonblank_raw(body.get("cloud_password"))
        with state.lock:
            rec = state.qr_pairs.get(tok)
            if rec is None:
                return 404, {"error": "pair_token_not_found"}
            if rec["status"] == "pending":
                return 400, {"error": "pair_not_confirmed"}
            uid = rec["accepted_by_user_id"] or state.default_user_id()
            if rec["requires_2fa"]:
                if not pw:
                    return 401, {"error": "two_factor_required"}
                user = state.ensure_user_by_id(uid)
                if user.get("two_factor_password") is None or user["two_factor_password"] != pw:
                    return 401, {"error": "invalid_cloud_password"}
            sess_json = rec["issued_session"]
            if sess_json is None:
                sess = state.mint_session(uid, rec["device_name"], ts)
                sess_json = {
                    "user_id": uid,
                    "session_id": sess["session_id"],
                    "token": sess["token"],
                    "expires_at": ms_to_rfc3339(sess["expires_at_ms"]),
                    "expires_at_ms": sess["expires_at_ms"],
                }
            return (
                200,
                {
                    "verified": True,
                    "user_id": sess_json["user_id"],
                    "session_id": sess_json["session_id"],
                    "token": sess_json["token"],
                    "expires_at": sess_json["expires_at"],
                    "expires_at_ms": sess_json["expires_at_ms"],
                    "session": sess_json,
                },
            )

    # Email & Phone OTP
    if method == "POST" and path in (
        "/api/v1/auth/email/send-code",
        "/api/v1/auth/email/send_code",
    ):
        return (
            200,
            {
                "ok": True,
                "sent": True,
                "expires_at_ms": ts + 600_000,
                "resend_after_ms": ts + 60_000,
            },
        )

    if method == "POST" and path in (
        "/api/v1/auth/email/verify-code",
        "/api/v1/auth/email/verify",
        "/api/v1/auth/email/verify_code",
        "/api/v1/auth/email/register",
    ):
        with state.lock:
            state.next_user_id += 1
            uid = state.next_user_id
            state.ensure_user_by_id(uid)
            sess = state.mint_session(uid, "iOS Email Auth", ts)
            return (
                200,
                {
                    "ok": True,
                    "authenticated": True,
                    "user_id": uid,
                    "session_id": sess["session_id"],
                    "token": sess["token"],
                    "expires_at": ms_to_rfc3339(sess["expires_at_ms"]),
                    "expires_at_ms": sess["expires_at_ms"],
                },
            )

    if method == "POST" and path in (
        "/api/v1/auth/phone/send-code",
        "/api/v1/auth/phone/send_code",
    ):
        with state.lock:
            seq = state.alloc_seq()
            pch = f"pch_{ts}_{seq}"
            exp = ts + 300_000
            state.phone_challenges[pch] = {
                "expected_code": "123456",
                "attempts": 0,
                "expires_at_ms": exp,
            }
            return (
                200,
                {
                    "ok": True,
                    "sent": True,
                    "phone_code_hash": pch,
                    "debug_code": "123456",
                    "expires_at_ms": exp,
                    "resend_after_ms": ts + 60_000,
                },
            )

    if method == "POST" and path in (
        "/api/v1/auth/phone/verify",
        "/api/v1/auth/phone/verify-code",
        "/api/v1/auth/phone/verify_code",
        "/api/v1/auth/phone/login",
    ):
        code = _str_nonempty_trim(body.get("code"))
        if not code or len(code) != 6 or not _is_ascii_uint_str(code):
            return 400, {"error": "invalid_code"}
        pch = _str_nonempty_trim(body.get("phone_code_hash"))
        with state.lock:
            if pch is not None:
                rec = state.phone_challenges.get(pch)
                if rec is None:
                    return 400, {"error": "invalid_phone_code_hash"}
                if rec["attempts"] >= 5:
                    return 400, {"error": "too_many_attempts"}
                if ts > rec["expires_at_ms"]:
                    return 400, {"error": "code_expired"}
                if code != rec["expected_code"]:
                    rec["attempts"] += 1
                    if rec["attempts"] >= 5:
                        return 400, {"error": "too_many_attempts"}
                    return 400, {"error": "invalid_code"}
                state.phone_challenges.pop(pch, None)
            state.next_user_id += 1
            uid = state.next_user_id
            state.ensure_user_by_id(uid)
            dev = _str_nonempty_trim(body.get("device_name")) or "iOS Phone Auth"
            if body.get("require_2fa_account") is True:
                seq = state.alloc_seq()
                chal = f"tfa_{uid}_{seq}"
                state.two_factor_challenges[chal] = {
                    "user_id": uid,
                    "device_name": dev,
                    "expires_at_ms": ts + 300_000,
                }
                return 200, {"requires_2fa": True, "challenge_token": chal, "user_id": uid}
            sess = state.mint_session(uid, dev, ts)
            return (
                200,
                {
                    "ok": True,
                    "authenticated": True,
                    "user_id": uid,
                    "session_id": sess["session_id"],
                    "token": sess["token"],
                    "expires_at": ms_to_rfc3339(sess["expires_at_ms"]),
                    "expires_at_ms": sess["expires_at_ms"],
                },
            )

    # Passkeys
    if method == "POST" and path in (
        "/api/v1/auth/passkey/register",
        "/api/v1/auth/passkeys",
        "/api/v1/auth/passkey/register/complete",
    ):
        cred_id = _str_nonempty_trim(body.get("credential_id"))
        pub_key = _str_nonempty_trim(
            body.get("public_key") or body.get("public_key_b64")
        )
        if not cred_id or not pub_key:
            return (
                400,
                {
                    "error": "invalid_passkey",
                    "message": "credential_id and public_key are required",
                },
            )
        dev = _str_nonempty_trim(body.get("device_name")) or "iOS Passkey"
        with state.lock:
            uid = caller_session["user_id"] if caller_session else state.default_user_id()
            state.ensure_user_by_id(uid)
            state.passkeys[cred_id] = {
                "credential_id": cred_id,
                "user_id": uid,
                "public_key": pub_key,
                "device_name": dev,
                "sign_count": 0,
                "created_at_ms": ts,
            }
            return (
                200,
                {
                    "ok": True,
                    "credential_id": cred_id,
                    "user_id": uid,
                    "device_name": dev,
                    "sign_count": 0,
                    "created_at_ms": ts,
                },
            )

    if method == "GET" and path in (
        "/api/v1/auth/passkey/list",
        "/api/v1/auth/passkeys",
        "/api/v1/auth/passkey",
    ):
        with state.lock:
            uid = caller_session["user_id"] if caller_session else state.default_user_id()
            items = sorted(
                (dict(p) for p in state.passkeys.values() if p["user_id"] == uid),
                key=lambda item: item["credential_id"],
            )
            return 200, {"passkeys": items, "total": len(items)}

    if method == "DELETE" and (
        path.startswith("/api/v1/auth/passkey/")
        or path.startswith("/api/v1/auth/passkeys/")
    ):
        cred_id = (
            path[len("/api/v1/auth/passkeys/") :]
            if path.startswith("/api/v1/auth/passkeys/")
            else path[len("/api/v1/auth/passkey/") :]
        )
        with state.lock:
            if cred_id in state.passkeys:
                state.passkeys.pop(cred_id)
                return 200, {"ok": True, "deleted": True, "credential_id": cred_id}
            return 404, {"error": "unknown_credential"}

    if method in ("POST", "GET") and path in (
        "/api/v1/auth/passkey/challenge",
        "/api/v1/auth/passkeys/challenge",
    ):
        with state.lock:
            uid: int | None = None
            if "user_id" in body and body["user_id"] is not None:
                uid = parse_int_or_400(body["user_id"], field="user_id")
            if uid is None:
                uname = _str_nonempty_trim(body.get("username")) or extract_query_param(
                    query, "username"
                )
                if uname:
                    uid = state.username_to_id.get(uname)
            if uid is None and caller_session:
                uid = caller_session["user_id"]
            seq = state.alloc_seq()
            chal = f"pkchal_{seq}_{uuid.uuid4().hex}"
            exp = ts + 300_000
            state.passkey_challenges[chal] = {"user_id": uid, "expires_at_ms": exp}
            return (
                200,
                {
                    "challenge": chal,
                    "rp_id": "gpui.telegram.local",
                    "timeout_ms": 300_000,
                    "user_id": uid,
                    "expires_at_ms": exp,
                },
            )

    if method == "POST" and path in (
        "/api/v1/auth/passkey/verify",
        "/api/v1/auth/passkeys/verify",
        "/api/v1/auth/passkey/login",
        "/api/v1/auth/passkey/authenticate",
    ):
        chal = _str_nonempty_trim(body.get("challenge"))
        cred_id = _str_nonempty_trim(body.get("credential_id"))
        if not chal:
            return 400, {"error": "invalid_challenge"}
        with state.lock:
            chal_rec = state.passkey_challenges.pop(chal, None)
            if chal_rec is None or ts > chal_rec["expires_at_ms"]:
                return 400, {"error": "invalid_challenge"}
            if not cred_id or cred_id not in state.passkeys:
                return 401, {"error": "unknown_credential"}
            pk = state.passkeys[cred_id]
            if chal_rec["user_id"] is not None and pk["user_id"] != chal_rec["user_id"]:
                return 401, {"error": "unknown_credential"}
            pk["sign_count"] += 1
            sess = state.mint_session(pk["user_id"], pk["device_name"], ts)
            return (
                200,
                {
                    "ok": True,
                    "verified": True,
                    "authenticated": True,
                    "user_id": pk["user_id"],
                    "session_id": sess["session_id"],
                    "token": sess["token"],
                    "expires_at": ms_to_rfc3339(sess["expires_at_ms"]),
                    "expires_at_ms": sess["expires_at_ms"],
                    "sign_count": pk["sign_count"],
                },
            )

    # Alipay OAuth & QR
    if method == "POST" and path == "/api/v1/auth/alipay/start":
        with state.lock:
            seq = state.alloc_seq()
            aid = f"alipay_{seq}"
            st = f"fbs_{uuid.uuid4().hex}"
            auth_url = f"https://openauth.alipay.com/oauth2/publicAppAuthorize.htm?state={st}"
            exp = ts + 600_000
            state.alipay_state_to_attempt[st] = aid
            state.alipay_attempts[aid] = {
                "attempt_id": aid,
                "state": st,
                "status": "pending",
                "issued_session": None,
                "expires_at_ms": exp,
            }
            return (
                200,
                {
                    "attempt_id": aid,
                    "state": st,
                    "auth_url": auth_url,
                    "qr_payload": auth_url,
                    "poll_after_ms": 1000,
                    "expires_in_ms": 600_000,
                    "expires_at_ms": exp,
                },
            )

    if method == "GET" and path.startswith("/api/v1/auth/alipay/poll/"):
        return _poll_alipay_attempt(state, path[len("/api/v1/auth/alipay/poll/") :], ts)

    if method in ("GET", "POST") and path == "/api/v1/auth/alipay/poll":
        aid = (
            _str_nonempty_trim(body.get("attempt_id"))
            or extract_query_param(query, "attempt_id")
            or ""
        )
        return _poll_alipay_attempt(state, aid, ts)

    if method == "POST" and path == "/api/v1/auth/alipay/cancel":
        return _cancel_alipay_attempt(
            state,
            _str_nonempty_trim(body.get("attempt_id")),
            _str_nonempty_trim(body.get("state")),
        )

    if method == "POST" and path.startswith("/api/v1/auth/alipay/cancel/"):
        return _cancel_alipay_attempt(
            state, _str_nonempty_trim(path[len("/api/v1/auth/alipay/cancel/") :]), None
        )

    if method == "DELETE" and path.startswith("/api/v1/auth/alipay/"):
        return _cancel_alipay_attempt(
            state, _str_nonempty_trim(path[len("/api/v1/auth/alipay/") :]), None
        )

    if (
        method in ("POST", "GET") and path == "/api/v1/auth/alipay/callback"
    ) or (method == "POST" and path == "/api/v1/auth/alipay/exchange"):
        st = _str_nonempty_trim(body.get("state")) or extract_query_param(query, "state")
        if not st:
            return 400, {"error": "oauth_state_mismatch", "message": "missing oauth state"}
        auth_code = (
            _str_nonempty_trim(body.get("auth_code") or body.get("code"))
            or extract_query_param(query, "auth_code")
            or extract_query_param(query, "code")
        )
        if not auth_code:
            return 400, {"error": "invalid_params", "message": "missing auth_code"}
        exp_aid = _str_nonempty_trim(
            body.get("attempt_id") or body.get("expected_attempt_id")
        ) or extract_query_param(query, "attempt_id")
        with state.lock:
            aid = state.alipay_state_to_attempt.get(st)
            if not aid:
                return 400, {"error": "oauth_state_mismatch", "message": "unknown oauth state"}
            if exp_aid and exp_aid != aid:
                return 400, {"error": "oauth_state_mismatch", "message": "attempt_id mismatch"}
            rec = state.alipay_attempts.get(aid)
            if rec is None:
                return 404, {"error": "oauth_attempt_not_found"}
            if rec["status"] == "cancelled":
                return 409, {"error": "oauth_cancelled", "status": "cancelled"}
            if rec["status"] != "pending" or ts > rec["expires_at_ms"]:
                return 400, {"error": "oauth_expired", "message": "oauth state unavailable"}
            if auth_code in state.consumed_alipay_codes:
                return 400, {"error": "oauth_expired", "message": "auth_code already consumed"}
            state.consumed_alipay_codes.add(auth_code)
            state.next_user_id += 1
            uid = state.next_user_id
            state.ensure_user_by_id(uid)
            sess = state.mint_session(uid, "iOS Alipay", ts)
            sess_json = {
                "user_id": uid,
                "session_id": sess["session_id"],
                "token": sess["token"],
                "expires_at": ms_to_rfc3339(sess["expires_at_ms"]),
                "expires_at_ms": sess["expires_at_ms"],
            }
            rec["status"] = "consumed"
            rec["issued_session"] = sess_json
            return (
                200,
                {
                    "authenticated": True,
                    "attempt_id": aid,
                    "user_id": uid,
                    "session_id": sess["session_id"],
                    "token": sess["token"],
                    "expires_at": ms_to_rfc3339(sess["expires_at_ms"]),
                    "expires_at_ms": sess["expires_at_ms"],
                    "session": sess_json,
                },
            )

    # Chats, invites & topics
    if method == "GET" and path == "/api/v1/chats":
        with state.lock:
            chats = [dict(state.chats[cid]) for cid in sorted(state.chats.keys())]
            return 200, {"chats": chats, "total": len(chats)}

    if method == "POST" and path in ("/api/v1/chats", "/api/v1/chats/create"):
        with state.lock:
            uid = caller_session["user_id"] if caller_session else state.default_user_id()
            raw_nonce = body.get("client_nonce")
            nonce_key: str | None = None
            if isinstance(raw_nonce, str) and raw_nonce.strip():
                nonce_key = raw_nonce.strip()
            elif isinstance(raw_nonce, int) and not isinstance(raw_nonce, bool):
                nonce_key = str(raw_nonce)
            if nonce_key and nonce_key in state.chats_by_nonce:
                existing = state.chats[state.chats_by_nonce[nonce_key]]
                resp = dict(existing)
                resp["deduplicated"] = True
                return 200, resp

            chat_type = _normalize_chat_type(body.get("chat_type"), default=0)
            if isinstance(body.get("member_ids"), list):
                unique_members: set[str] = {str(uid)}
                for m in body["member_ids"]:
                    unique_members.add(str(m))
                member_count = len(unique_members)
            elif isinstance(body.get("recipients"), list):
                unique_members = {str(uid)}
                for m in body["recipients"]:
                    val = m.get("user_id", m) if isinstance(m, dict) else m
                    unique_members.add(str(val))
                member_count = len(unique_members)
            else:
                member_count = 2
            state.next_chat_id += 1
            cid = state.next_chat_id
            title = _str_nonempty_trim(body.get("title")) or f"Chat {cid}"
            chat = {
                "id": cid,
                "chat_id": cid,
                "title": title,
                "chat_type": chat_type,
                "member_count": member_count,
                "creator_id": uid,
                "is_group": chat_type in (1, 2, 3),
            }
            state.chats[cid] = chat
            state.chat_messages.setdefault(cid, [])
            state.chat_pts.setdefault(cid, 0)
            if nonce_key:
                state.chats_by_nonce[nonce_key] = cid
            resp = dict(chat)
            resp["deduplicated"] = False
            return 200, resp

    if method == "POST" and path == "/api/v1/chats/join":
        code = _str_nonempty_trim(body.get("code")) or ""
        with state.lock:
            inv = state.chat_invites.get(code)
            if inv is None:
                return 404, {"error": "invalid_invite"}
            if inv["revoked"] or (
                inv["max_uses"] is not None and inv["uses"] >= inv["max_uses"]
            ):
                return 400, {"error": "invalid_invite"}
            inv["uses"] += 1
            chat_obj = state.chats.get(inv["chat_id"])
            if chat_obj is not None:
                chat_obj["member_count"] = int(chat_obj.get("member_count", 0)) + 1
            return 200, {"chat_id": inv["chat_id"], "joined": True}

    if path.startswith("/api/v1/chats/") and path.endswith("/invites"):
        cid_str = path[len("/api/v1/chats/") : -len("/invites")]
        if not _is_ascii_int_str(cid_str):
            return 404, {"error": "not_found"}
        cid = int(cid_str)
        with state.lock:
            if method == "POST":
                raw_max = (
                    body["max_uses"]
                    if "max_uses" in body and body["max_uses"] is not None
                    else body.get("usage_limit")
                )
                max_uses_int = (
                    parse_int_or_400(raw_max, field="max_uses", min_val=0)
                    if raw_max is not None
                    else None
                )
                seq = state.alloc_seq()
                inv_id = f"inv_{cid}_{seq}"
                code = f"join_{cid}_{seq}"
                inv = {
                    "invite_id": inv_id,
                    "chat_id": cid,
                    "code": code,
                    "url": f"https://fabushi.app/+{code}",
                    "max_uses": max_uses_int,
                    "usage_limit": max_uses_int,
                    "uses": 0,
                    "revoked": False,
                }
                state.chat_invites[code] = inv
                state.chat_invites[inv_id] = inv
                return 200, dict(inv)
            if method == "GET":
                seen: set[str] = set()
                out = []
                for inv in state.chat_invites.values():
                    if inv["chat_id"] == cid and inv["invite_id"] not in seen:
                        seen.add(inv["invite_id"])
                        out.append(dict(inv))
                return 200, {"invites": out}

    if path.startswith("/api/v1/chats/") and "/invites/" in path:
        rest = path[len("/api/v1/chats/") :]
        _, inv_part = rest.split("/invites/", 1)
        inv_id = inv_part[:-len("/revoke")] if inv_part.endswith("/revoke") else inv_part
        if method in ("DELETE", "POST"):
            with state.lock:
                inv = state.chat_invites.get(inv_id)
                if inv is None:
                    return 404, {"error": "invite_not_found"}
                inv["revoked"] = True
                return 200, {"revoked": True, "invite_id": inv["invite_id"]}
        if method == "GET":
            with state.lock:
                inv = state.chat_invites.get(inv_id)
                if inv is None:
                    return 404, {"error": "invite_not_found"}
                return 200, dict(inv)

    if method == "GET" and path.startswith("/api/v1/chats/") and path.endswith("/topics"):
        cid_str = path[len("/api/v1/chats/") : -len("/topics")]
        cid = parse_int_or_400(cid_str, field="chat_id")
        return 200, {"chat_id": cid, "topics": []}

    # Messages send/history/edit/delete/reaction/read
    if method == "POST" and (
        path == "/api/v1/messages/send"
        or (path.startswith("/api/v1/chats/") and path.endswith("/messages"))
    ):
        if path.startswith("/api/v1/chats/"):
            cid = parse_int_or_400(
                path[len("/api/v1/chats/") : -len("/messages")], field="chat_id"
            )
        else:
            cid = (
                parse_int_or_400(body["chat_id"], field="chat_id")
                if "chat_id" in body
                else 1
            )
        text = body.get("text")
        if text is None:
            text = body.get("content", "")
        text_str = str(text)
        nonce = body.get("client_nonce")
        nonce_int = int(nonce) if _is_strict_int(nonce) else None
        with state.lock:
            if nonce_int is not None and (cid, nonce_int) in state.chat_nonce_map:
                existing_mid = state.chat_nonce_map[(cid, nonce_int)]
                for m in state.chat_messages.get(cid, []):
                    if m["message_id"] == existing_mid:
                        return 200, _format_stored_message(m)
            mid = state.next_msg_id
            state.next_msg_id += 1
            pts = state.chat_pts.get(cid, 0) + 1
            state.chat_pts[cid] = pts
            uid = caller_session["user_id"] if caller_session else state.default_user_id()
            msg = {
                "message_id": mid,
                "chat_id": cid,
                "pts": pts,
                "text": text_str,
                "client_nonce": nonce_int,
                "sender_id": uid,
                "timestamp_ms": ts,
            }
            state.chat_messages.setdefault(cid, []).append(msg)
            if nonce_int is not None:
                state.chat_nonce_map[(cid, nonce_int)] = mid
            return 200, _format_stored_message(msg)

    if method == "GET" and (
        path == "/api/v1/messages/history"
        or (path.startswith("/api/v1/chats/") and path.endswith("/messages"))
    ):
        if path.startswith("/api/v1/chats/"):
            cid = parse_int_or_400(
                path[len("/api/v1/chats/") : -len("/messages")], field="chat_id"
            )
        else:
            cid_s = extract_query_param(query, "chat_id")
            cid = int(cid_s) if cid_s and _is_ascii_int_str(cid_s) else 1
        with state.lock:
            msgs = [_format_stored_message(m) for m in state.chat_messages.get(cid, [])]
            return (
                200,
                {
                    "chat_id": cid,
                    "messages": msgs,
                    "has_more": False,
                    "current_pts": state.chat_pts.get(cid, 0),
                },
            )

    if method == "GET" and path.startswith("/api/v1/chats/") and "/" not in path[len("/api/v1/chats/") :]:
        cid_str = path[len("/api/v1/chats/") :]
        if _is_ascii_int_str(cid_str):
            cid = int(cid_str)
            with state.lock:
                chat = state.chats.get(cid)
                if chat is None:
                    return 404, {"error": "chat_not_found"}
                return 200, dict(chat)

    if method == "POST" and path == "/api/v1/messages/edit":
        cid = parse_int_or_400(body["chat_id"], field="chat_id") if "chat_id" in body else 1
        mid = (
            parse_int_or_400(body["message_id"], field="message_id")
            if "message_id" in body
            else 0
        )
        new_text = str(body.get("text", ""))
        with state.lock:
            pts = state.chat_pts.get(cid, 0) + 1
            state.chat_pts[cid] = pts
            for m in state.chat_messages.get(cid, []):
                if m["message_id"] == mid:
                    m["text"] = new_text
                    break
            return (
                200,
                {
                    "ok": True,
                    "success": True,
                    "message_id": mid,
                    "text": new_text,
                    "pts": pts,
                    "edited_at_ms": ts,
                    "status": "edited",
                },
            )

    if method == "POST" and path == "/api/v1/messages/delete":
        cid = parse_int_or_400(body["chat_id"], field="chat_id") if "chat_id" in body else 1
        revoke = bool(body.get("revoke", False))
        if "message_ids" in body:
            raw_ids = body["message_ids"]
            if not isinstance(raw_ids, list):
                raise ApiParamError("message_ids must be a list")
            ids = {parse_int_or_400(x, field="message_ids[]") for x in raw_ids}
        else:
            ids = set()
        with state.lock:
            pts = state.chat_pts.get(cid, 0) + 1
            state.chat_pts[cid] = pts
            msgs = state.chat_messages.get(cid, [])
            before = len(msgs)
            state.chat_messages[cid] = [m for m in msgs if m["message_id"] not in ids]
            removed = before - len(state.chat_messages[cid])
            return (
                200,
                {
                    "ok": True,
                    "success": True,
                    "pts": pts,
                    "deleted_count": removed if removed > 0 else len(ids),
                    "revoke": revoke,
                },
            )

    if method == "POST" and path == "/api/v1/messages/reaction":
        cid = parse_int_or_400(body["chat_id"], field="chat_id") if "chat_id" in body else 1
        with state.lock:
            pts = state.chat_pts.get(cid, 0) + 1
            state.chat_pts[cid] = pts
            return 200, {"ok": True, "success": True, "pts": pts, "status": "reaction_updated"}

    if method == "POST" and path == "/api/v1/messages/read":
        cid = parse_int_or_400(body["chat_id"], field="chat_id") if "chat_id" in body else 1
        with state.lock:
            pts = state.chat_pts.get(cid, 0) + 1
            state.chat_pts[cid] = pts
            return 200, {"ok": True, "success": True, "pts": pts, "status": "history_read"}

    # PTS Sync
    if method in ("GET", "POST") and path in (
        "/api/v1/sync/get_difference",
        "/api/v1/sync/difference",
    ):
        cid_q = extract_query_param(query, "chat_id")
        pts_q = extract_query_param(query, "pts")
        lim_q = extract_query_param(query, "limit")
        if "chat_id" in body:
            cid = parse_int_or_400(body["chat_id"], field="chat_id")
        else:
            cid = int(cid_q) if cid_q and _is_ascii_int_str(cid_q) else 1
        if "pts" in body:
            req_pts = parse_int_or_400(body["pts"], field="pts")
        else:
            req_pts = int(pts_q) if pts_q and _is_ascii_uint_str(pts_q) else 0
        if "limit" in body:
            limit = max(1, parse_int_or_400(body["limit"], field="limit"))
        else:
            limit = max(1, int(lim_q) if lim_q and _is_ascii_uint_str(lim_q) else 100)
        with state.lock:
            floor = state.journal_floors.get(cid, 1)
            cur_pts = max(state.chat_pts.get(cid, 0), max(0, floor - 1))
            if req_pts + 1 < floor:
                return (
                    200,
                    {
                        "chat_id": cid,
                        "pts": req_pts,
                        "new_pts": floor,
                        "current_pts": max(cur_pts, floor),
                        "has_more": True,
                        "snapshot_required": True,
                        "messages": [],
                        "other_updates": [],
                        "updates": [],
                    },
                )
            matching = sorted(
                (m for m in state.chat_messages.get(cid, []) if m["pts"] > req_pts),
                key=lambda item: item["pts"],
            )
            has_more = len(matching) > limit
            matching = matching[:limit]
            new_pts = matching[-1]["pts"] if matching else req_pts
            records = [_format_stored_message(m) for m in matching]
            return (
                200,
                {
                    "chat_id": cid,
                    "pts": req_pts,
                    "new_pts": new_pts,
                    "current_pts": max(cur_pts, new_pts),
                    "has_more": has_more,
                    "snapshot_required": False,
                    "messages": records,
                    "other_updates": [],
                    "updates": records,
                },
            )

    if method == "GET" and path in ("/api/v1/sync/state", "/api/v1/sync/pts"):
        cid_q = extract_query_param(query, "chat_id")
        cid = int(cid_q) if cid_q and _is_ascii_int_str(cid_q) else 1
        with state.lock:
            cur_pts = state.chat_pts.get(cid, 0)
            return 200, {"chat_id": cid, "pts": cur_pts, "current_pts": cur_pts, "seq": state.next_seq}

    # Media JSON upload control & Instant View
    if method == "POST" and path == "/api/v1/media/upload/init":
        if "total_size" in body:
            total_size = parse_int_or_400(body["total_size"], field="total_size")
        elif "size_bytes" in body:
            total_size = parse_int_or_400(body["size_bytes"], field="size_bytes")
        else:
            total_size = 0
        if "part_size" in body:
            part_size = parse_int_or_400(body["part_size"], field="part_size")
        elif "chunk_size" in body:
            part_size = parse_int_or_400(body["chunk_size"], field="chunk_size")
        else:
            part_size = 1024 * 1024
        if total_size < 0 or total_size > MAX_UPLOAD_TOTAL_SIZE:
            return 400, {"error": "upload_size_exceeded"}
        if part_size <= 0 or part_size > MAX_UPLOAD_PART_SIZE:
            return 400, {"error": "invalid_part_size"}
        with state.lock:
            seq = state.alloc_seq()
            upl_id = f"upl_{seq}"
            filename = _str_nonempty_trim(body.get("filename")) or "upload.bin"
            exp_sha = _str_nonempty_trim(body.get("expected_sha256"))
            state.uploads[upl_id] = {
                "upload_id": upl_id,
                "filename": filename,
                "total_size": total_size,
                "part_size": part_size,
                "expected_sha256": exp_sha,
                "parts": {},
            }
            return (
                200,
                {
                    "upload_id": upl_id,
                    "filename": filename,
                    "part_size": part_size,
                    "chunk_size": part_size,
                    "total_size": total_size,
                    "size_bytes": total_size,
                },
            )

    if method == "GET" and path.startswith("/api/v1/media/upload/"):
        rest = path[len("/api/v1/media/upload/") :]
        upl_id = rest[:-len("/status")] if rest.endswith("/status") else rest
        with state.lock:
            sess = state.uploads.get(upl_id)
            if sess is None:
                return 404, {"error": "upload_not_found"}
            received = sorted(sess["parts"].keys())
            bytes_received = sum(len(b) for b in sess["parts"].values())
            return (
                200,
                {
                    "upload_id": sess["upload_id"],
                    "filename": sess["filename"],
                    "total_size": sess["total_size"],
                    "size_bytes": sess["total_size"],
                    "part_size": sess["part_size"],
                    "chunk_size": sess["part_size"],
                    "bytes_received": bytes_received,
                    "received_parts": received,
                    "status": "in_progress",
                },
            )

    if method == "POST" and path.startswith("/api/v1/media/upload/") and path.endswith("/complete"):
        upl_id = path[len("/api/v1/media/upload/") : -len("/complete")]
        with state.lock:
            sess = state.uploads.get(upl_id)
            if sess is None:
                return 404, {"error": "upload_not_found"}
            expected_parts = (
                0
                if sess["total_size"] == 0
                else (sess["total_size"] + sess["part_size"] - 1) // sess["part_size"]
            )
            if len(sess["parts"]) < expected_parts:
                return 400, {"error": "missing_upload_parts"}
            full = b"".join(sess["parts"][idx] for idx in sorted(sess["parts"].keys()))
            digest = hex_sha256(full)
            if sess["expected_sha256"] and sess["expected_sha256"] != digest:
                return 400, {"error": "checksum_mismatch"}
            fid = f"cas_{digest[:16]}"
            state.media_blobs[fid] = full
            state.media_blobs[digest] = full
            state.uploads.pop(upl_id, None)
            return (
                200,
                {
                    "upload_id": sess["upload_id"],
                    "filename": sess["filename"],
                    "file_id": fid,
                    "media_id": fid,
                    "sha256": digest,
                    "size": len(full),
                    "status": "completed",
                },
            )

    if method == "POST" and path == "/api/v1/media/edit":
        mid = _str_nonempty_trim(body.get("media_id") or body.get("file_id")) or "edited_media"
        return 200, {"ok": True, "edited": True, "media_id": mid, "file_id": mid, "timestamp_ms": ts}

    if method in ("GET", "POST") and path in (
        "/api/v1/media/instant-view",
        "/api/v1/media/instant_view",
    ):
        url = (
            _str_nonempty_trim(body.get("url"))
            or extract_query_param(query, "url")
            or "/article/v168"
        )
        return (
            200,
            {
                "ok": True,
                "url": url,
                "title": f"Fabushi v{state.version}",
                "blocks": [
                    {"type": "heading", "text": f"Fabushi v{state.version}"},
                    {"type": "paragraph", "text": "Instant View article content"},
                ],
            },
        )

    # Vault & X25519 Device Sync
    if path in ("/api/v1/user-vault/envelope", "/api/v1/vault/envelope"):
        with state.lock:
            uid = caller_session["user_id"] if caller_session else state.default_user_id()
            user = state.ensure_user_by_id(uid)
            if method == "GET":
                env = user.get("vault_envelope")
                out: dict[str, Any] = {"envelope": env}
                if isinstance(env, dict):
                    for k, v in env.items():
                        if k != "envelope":
                            out[k] = v
                return 200, out
            if method in ("PUT", "POST"):
                user["vault_envelope"] = dict(body)
                return (
                    200,
                    {
                        "ok": True,
                        "saved": True,
                        "kdf_alg": str(body.get("kdf_alg", "split_key_v2")),
                    },
                )

    if path in ("/api/v1/user-vault/items", "/api/v1/vault/items"):
        with state.lock:
            uid = caller_session["user_id"] if caller_session else state.default_user_id()
            if method == "GET":
                cat_filter = extract_query_param(query, "category")
                items = [
                    dict(v)
                    for (u, c, _), v in state.vault_items.items()
                    if u == uid and (cat_filter is None or cat_filter == c)
                ]
                return 200, {"items": items}
            if method in ("POST", "PUT"):
                cat = _str_nonempty_trim(body.get("category")) or "general"
                key = _str_nonempty_trim(body.get("item_key")) or "default"
                state.vault_items[(uid, cat, key)] = dict(body)
                return 200, {"ok": True, "category": cat, "item_key": key}

    if path.startswith("/api/v1/user-vault/items/") or path.startswith("/api/v1/vault/items/"):
        rest = (
            path[len("/api/v1/user-vault/items/") :]
            if path.startswith("/api/v1/user-vault/items/")
            else path[len("/api/v1/vault/items/") :]
        )
        with state.lock:
            uid = caller_session["user_id"] if caller_session else state.default_user_id()
            if method == "GET":
                if "/" in rest:
                    raw_cat, raw_key = rest.split("/", 1)
                    cat = urllib.parse.unquote(raw_cat)
                    key = urllib.parse.unquote(raw_key)
                    item = state.vault_items.get((uid, cat, key))
                    if item is None:
                        return 404, {"error": "vault_item_not_found"}
                    return 200, dict(item)
                cat = urllib.parse.unquote(rest)
                items = [
                    dict(v)
                    for (u, c, _), v in state.vault_items.items()
                    if u == uid and c == cat
                ]
                return 200, {"category": cat, "items": items}
            raw_cat, raw_key = rest.split("/", 1) if "/" in rest else ("general", rest)
            cat = urllib.parse.unquote(raw_cat)
            key = urllib.parse.unquote(raw_key)
            if method in ("PUT", "POST"):
                stored = dict(body)
                stored.setdefault("category", cat)
                stored.setdefault("item_key", key)
                state.vault_items[(uid, cat, key)] = stored
                return 200, {"ok": True, "category": cat, "item_key": key}
            if method == "DELETE":
                state.vault_items.pop((uid, cat, key), None)
                return 200, {"deleted": True, "category": cat, "item_key": key}

    if method == "POST" and path in (
        "/api/v1/user-vault/device-sync/request",
        "/api/v1/vault/device-sync/request",
    ):
        with state.lock:
            seq = state.alloc_seq()
            req_id = f"dsync_{seq}"
            dev_name = _str_nonempty_trim(body.get("device_name")) or "Secondary Device"
            pub_b64 = str(body.get("ephemeral_x25519_pub_b64", ""))
            code = compute_device_sync_code(req_id, pub_b64)
            exp = ts + 600_000
            rec = {
                "request_id": req_id,
                "device_name": dev_name,
                "ephemeral_x25519_pub_b64": pub_b64,
                "verification_code": code,
                "status": "pending",
                "wrapped_key_b64": None,
                "expires_at_ms": exp,
            }
            state.device_syncs[req_id] = rec
            return 200, dict(rec)

    if method == "GET" and path in (
        "/api/v1/user-vault/device-sync/pending",
        "/api/v1/vault/device-sync/pending",
    ):
        with state.lock:
            pending = [
                dict(r)
                for r in state.device_syncs.values()
                if r["status"] == "pending" and ts <= r["expires_at_ms"]
            ]
            return 200, {"requests": pending}

    if method == "POST" and (
        path in ("/api/v1/user-vault/device-sync/approve", "/api/v1/vault/device-sync/approve")
        or (
            (
                path.startswith("/api/v1/user-vault/device-sync/")
                or path.startswith("/api/v1/vault/device-sync/")
            )
            and path.endswith("/approve")
        )
    ):
        req_id = _str_nonempty_trim(body.get("request_id"))
        if not req_id:
            stripped = (
                path[len("/api/v1/user-vault/device-sync/") :]
                if path.startswith("/api/v1/user-vault/device-sync/")
                else path[len("/api/v1/vault/device-sync/") :]
            )
            req_id = stripped[:-len("/approve")]
        with state.lock:
            rec = state.device_syncs.get(req_id)
            if rec is None:
                return 404, {"error": "device_sync_not_found"}
            if rec["status"] == "denied":
                return 400, {"error": "device_sync_denied"}
            if ts > rec["expires_at_ms"]:
                return 400, {"error": "device_sync_expired"}
            rec["status"] = "approved"
            rec["wrapped_key_b64"] = body.get("wrapped_key_b64")
            return (
                200,
                {
                    "request_id": rec["request_id"],
                    "status": "approved",
                    "wrapped_key_b64": rec["wrapped_key_b64"],
                },
            )

    if method == "POST" and (
        path in ("/api/v1/user-vault/device-sync/deny", "/api/v1/vault/device-sync/deny")
        or (
            (
                path.startswith("/api/v1/user-vault/device-sync/")
                or path.startswith("/api/v1/vault/device-sync/")
            )
            and path.endswith("/deny")
        )
    ):
        req_id = _str_nonempty_trim(body.get("request_id"))
        if not req_id:
            stripped = (
                path[len("/api/v1/user-vault/device-sync/") :]
                if path.startswith("/api/v1/user-vault/device-sync/")
                else path[len("/api/v1/vault/device-sync/") :]
            )
            req_id = stripped[:-len("/deny")]
        with state.lock:
            rec = state.device_syncs.get(req_id)
            if rec is None:
                return 404, {"error": "device_sync_not_found"}
            rec["status"] = "denied"
            return 200, {"request_id": rec["request_id"], "status": "denied"}

    if method in ("GET", "POST") and (
        path in ("/api/v1/user-vault/device-sync/status", "/api/v1/vault/device-sync/status")
        or path.startswith("/api/v1/user-vault/device-sync/status/")
        or path.startswith("/api/v1/vault/device-sync/status/")
        or (
            (
                path.startswith("/api/v1/user-vault/device-sync/")
                or path.startswith("/api/v1/vault/device-sync/")
            )
            and path.rsplit("/", 1)[-1]
            not in ("pending", "request", "approve", "deny", "status")
        )
    ):
        req_id = (
            _str_nonempty_trim(body.get("request_id"))
            or extract_query_param(query, "request_id")
            or path.rsplit("/", 1)[-1]
        )
        with state.lock:
            rec = state.device_syncs.get(req_id)
            if rec is None:
                return 404, {"error": "device_sync_not_found"}
            return 200, dict(rec)

    if matches_segment_prefix(path, "/api/v1/user-vault/obscura") or matches_segment_prefix(
        path, "/api/v1/vault/obscura"
    ):
        return 200, {"ok": True, "obscura": True, "path": path, "method": method, "timestamp_ms": ts}

    # Agent runs & turns
    if method == "POST" and path in ("/api/v1/agent/runs", "/api/v1/agent/turn"):
        with state.lock:
            seq = state.alloc_seq()
            uid = caller_session["user_id"] if caller_session else state.default_user_id()
            sid = _str_nonempty_trim(body.get("session_id")) or (
                caller_session["session_id"] if caller_session else f"agent_sess_{seq}"
            )
            prompt = str(body.get("prompt", ""))
            if path.endswith("/runs"):
                return (
                    200,
                    {
                        "run_id": f"run_{seq}",
                        "session_id": sid,
                        "user_id": uid,
                        "status": "completed",
                        "prompt": prompt,
                        "response": f"Completed agent run for: {prompt}",
                        "outputs": [],
                    },
                )
            return (
                200,
                {
                    "turn_id": f"turn_{seq}",
                    "session_id": sid,
                    "caller_id": uid,
                    "status": "executed",
                    "prompt": prompt,
                    "response": f"Executed turn for: {prompt}",
                    "outputs": [],
                },
            )

    # Computers / Remote nodes (v1.6.8 ScreenCaptureKit & Accessibility permission metadata)
    if method == "GET" and path in ("/api/v1/computers", "/api/v1/remote/nodes"):
        with state.lock:
            nodes = [dict(state.remote_nodes[k]) for k in sorted(state.remote_nodes.keys())]
            return 200, {"nodes": nodes, "computers": nodes, "total": len(nodes)}

    if method == "POST" and path in (
        "/api/v1/computer/lease",
        "/api/v1/computer/lease/acquire",
    ):
        did = _str_nonempty_trim(body.get("device_id") or body.get("node_id")) or "default_device"
        with state.lock:
            seq = state.alloc_seq()
            lease = state.computer_leases.get(did)
            if lease is None:
                lease = {"lease_id": f"lease_{seq}", "generation": 0, "override_epoch": 0}
                state.computer_leases[did] = lease
            lease["generation"] += 1
            lease["lease_id"] = f"lease_{seq}"
            return (
                200,
                {
                    "lease_id": lease["lease_id"],
                    "target_device_id": did,
                    "device_id": did,
                    "generation": lease["generation"],
                    "override_epoch": lease["override_epoch"],
                    "state": "active",
                    "is_valid": True,
                },
            )

    if method == "POST" and path == "/api/v1/computer/lease/emergency_takeover":
        did = _str_nonempty_trim(body.get("device_id") or body.get("node_id")) or "default_device"
        with state.lock:
            seq = state.alloc_seq()
            lease = state.computer_leases.get(did)
            if lease is None:
                lease = {"lease_id": f"lease_{seq}", "generation": 0, "override_epoch": 0}
                state.computer_leases[did] = lease
            lease["generation"] += 1
            lease["override_epoch"] += 1
            lease["lease_id"] = f"lease_takeover_{seq}"
            return (
                200,
                {
                    "status": "emergency_takeover_executed",
                    "action": "preempt_and_fence",
                    "target_device_id": did,
                    "new_generation": lease["generation"],
                    "generation": lease["generation"],
                    "override_epoch": lease["override_epoch"],
                    "latency_sub_10ms": True,
                    "elapsed_ms": 1,
                },
            )

    if method == "POST" and path == "/api/v1/computer/action":
        did = _str_nonempty_trim(body.get("device_id") or body.get("node_id")) or "default_device"
        req_gen_raw = body.get("generation")
        req_gen: int | None = None
        if "generation" in body and req_gen_raw is not None:
            if not _is_strict_int(req_gen_raw):
                raise ApiParamError("generation must be an integer")
            req_gen = req_gen_raw
        with state.lock:
            node = state.remote_nodes.get(did)
            op = str(body.get("op", ""))
            if node is not None and op == "capture_screen":
                perm = node.get("screen_capture_permission", "granted_active")
                if perm != "granted_active":
                    return (
                        403,
                        {
                            "error": "fallback_png_permission_denied",
                            "permission": perm,
                            "sc_stream_error_code": -3801,
                            "target_device_id": did,
                        },
                    )
            lease = state.computer_leases.get(did)
            if lease is not None:
                if req_gen is not None and req_gen < lease["generation"]:
                    return (
                        403,
                        {
                            "error": "stale_generation",
                            "current_generation": lease["generation"],
                        },
                    )
                return (
                    200,
                    {
                        "status": "action_executed",
                        "generation": lease["generation"],
                        "lease_id": lease["lease_id"],
                        "target_device_id": did,
                    },
                )
            return (
                200,
                {
                    "status": "action_executed",
                    "generation": req_gen if req_gen is not None else 1,
                    "lease_id": "lease_default",
                    "target_device_id": did,
                },
            )

    # MiniApps catalog & stateful ToS persistence
    if method == "GET" and path == "/api/v1/miniapps":
        with state.lock:
            apps = [dict(state.miniapps[k]) for k in sorted(state.miniapps.keys())]
            return 200, {"miniapps": apps, "total": len(apps)}

    if (
        method == "GET"
        and path.startswith("/api/v1/miniapps/")
        and "/" not in path[len("/api/v1/miniapps/") :]
    ):
        aid = urllib.parse.unquote(path[len("/api/v1/miniapps/") :]).strip()
        with state.lock:
            app = state.miniapps.get(aid)
            if app is None:
                return 404, {"error": "miniapp_not_found"}
            return 200, dict(app)

    if method in ("POST", "PUT") and (
        path in ("/api/v1/miniapps/tos", "/api/v1/miniapps/accept-tos", "/api/v1/miniapps/accept_tos")
        or (
            path.startswith("/api/v1/miniapps/")
            and (
                path.endswith("/tos")
                or path.endswith("/accept-tos")
                or path.endswith("/accept_tos")
            )
        )
    ):
        aid = None
        if path.startswith("/api/v1/miniapps/") and path not in (
            "/api/v1/miniapps/tos",
            "/api/v1/miniapps/accept-tos",
            "/api/v1/miniapps/accept_tos",
        ):
            rest = path[len("/api/v1/miniapps/") :]
            aid = urllib.parse.unquote(rest.rsplit("/", 1)[0]).strip() or None
        if not aid:
            aid = _str_nonempty_trim(body.get("app_id") or body.get("bot_id"))
        if not aid:
            return 400, {"error": "invalid_params", "message": "app_id is required"}
        if "tos_accepted" in body and body["tos_accepted"] is not None:
            if not isinstance(body["tos_accepted"], bool):
                return 400, {"error": "invalid_params", "message": "tos_accepted must be boolean"}
            accepted = body["tos_accepted"]
        elif "accepted" in body and body["accepted"] is not None:
            if not isinstance(body["accepted"], bool):
                return 400, {"error": "invalid_params", "message": "accepted must be boolean"}
            accepted = body["accepted"]
        else:
            accepted = True
        with state.lock:
            app = state.miniapps.get(aid)
            if app is None:
                app = {
                    "app_id": aid,
                    "title": _str_nonempty_trim(body.get("title")) or aid,
                    "tos_accepted": accepted,
                    "webmcp_enabled": True,
                }
                state.miniapps[aid] = app
            else:
                app["tos_accepted"] = accepted
            resp = dict(app)
            resp["ok"] = True
            return 200, resp

    # Search, WebRTC, Tasks, MCP, Talk, Magpie, Updates
    if method == "GET" and path == "/api/v1/search/messages":
        q = extract_query_param(query, "q") or ""
        cid_q = extract_query_param(query, "chat_id")
        cid_filter = int(cid_q) if cid_q and _is_ascii_int_str(cid_q) else None
        q_lower = q.lower()
        with state.lock:
            results = []
            for cid, msgs in state.chat_messages.items():
                if cid_filter is not None and cid != cid_filter:
                    continue
                for m in msgs:
                    if not q_lower or q_lower in m["text"].lower():
                        results.append(_format_stored_message(m))
            return 200, {"query": q, "results": results, "total": len(results)}

    if method == "POST" and path == "/api/v1/webrtc/call/invite":
        with state.lock:
            seq = state.alloc_seq()
            uid = caller_session["user_id"] if caller_session else state.default_user_id()
            callee = body.get("callee_id", body.get("peer_id", 2001))
            return (
                200,
                {
                    "call_id": f"call_{seq}",
                    "caller_id": uid,
                    "callee_id": callee,
                    "status": "invited",
                },
            )

    if method == "POST" and path == "/api/v1/webrtc/call/signal":
        return 200, {"ok": True, "status": "relayed", "timestamp_ms": ts}

    if method == "GET" and path == "/api/v1/webrtc/turn_credentials":
        return (
            200,
            {
                "urls": ["stun:stun.l.google.com:19302"],
                "username": "ephemeral_fabushi_user",
                "credential": "ephemeral_fabushi_credential",
                "ice_servers": [{"urls": ["stun:stun.l.google.com:19302"]}],
                "ttl_seconds": 43200,
            },
        )

    if path == "/api/v1/tasks":
        with state.lock:
            if method == "GET":
                return (
                    200,
                    {
                        "columns": [
                            "inbox",
                            "active",
                            "review",
                            "completed",
                            "failed",
                            "cancelled",
                        ],
                        "tasks": [dict(t) for t in state.tasks.values()],
                    },
                )
            if method == "POST":
                seq = state.alloc_seq()
                tid = _str_nonempty_trim(body.get("task_id")) or f"task_{seq}"
                task = {
                    "task_id": tid,
                    "state": _str_nonempty_trim(body.get("initial_state")) or "inbox",
                    "approved": False,
                }
                state.tasks[tid] = task
                return 200, dict(task)

    if method == "POST" and path.startswith("/api/v1/tasks/") and path.endswith("/approve"):
        tid = path[len("/api/v1/tasks/") : -len("/approve")]
        with state.lock:
            task = state.tasks.setdefault(tid, {"task_id": tid, "state": "review"})
            if "approved" in body and isinstance(body["approved"], bool):
                approved = body["approved"]
            else:
                dec = str(body.get("decision", "approve")).strip().lower()
                approved = dec not in (
                    "deny",
                    "denied",
                    "reject",
                    "rejected",
                    "false",
                    "cancel",
                    "cancelled",
                )
            task["approved"] = approved
            task["state"] = "active" if approved else "cancelled"
            resp = dict(task)
            resp["decision"] = "approved" if approved else "rejected"
            resp["current_state"] = task["state"]
            resp["transition_success"] = True
            resp["gate_id"] = f"gate_{tid}"
            return 200, resp

    if method == "GET" and path == "/api/v1/mcp/servers":
        with state.lock:
            servers = [dict(s) for s in state.mcp_servers.values()]
            return 200, {"servers": servers, "total": len(servers)}

    if method == "GET" and path == "/api/v1/mcp/tools":
        return (
            200,
            {
                "tools": [
                    {"name": "messages.search", "read_only": True},
                    {"name": "chats.list", "read_only": True},
                ]
            },
        )

    if method == "POST" and path == "/api/v1/mcp/call":
        tool_name = _str_nonempty_trim(body.get("tool_name"))
        if not tool_name:
            return 400, {"error": "missing_tool_name"}
        return 200, {"ok": True, "tool_name": tool_name, "result": {"status": "ok"}}

    if method == "POST" and path == "/api/v1/mcp/permission/approve":
        req_id = _str_nonempty_trim(body.get("request_id"))
        if not req_id:
            return 400, {"error": "missing_request_id"}
        return 200, {"ok": True, "request_id": req_id, "allow": bool(body.get("allow", True))}

    if method == "POST" and path == "/api/v1/talk/session":
        return 200, {"ok": True, "session_id": f"talk_{ts}", "state": "listening"}

    if method == "POST" and path == "/api/v1/talk/barge_in":
        return 200, {"ok": True, "status": "barge_in_handled", "latency_ms": 18}

    if method == "POST" and path == "/api/v1/talk/audio":
        return 200, {"ok": True, "status": "queued"}

    if method == "GET" and path in ("/api/v1/magpie/v1", "/api/v1/magpie/v1/models"):
        return (
            200,
            {
                "object": "list",
                "data": [{"id": "gpt-4.1-mini", "object": "model", "owned_by": "magpie"}],
            },
        )

    if method == "GET" and path == "/api/v1/updates/check":
        return (
            200,
            {
                "latest_version": state.version,
                "store_url": "https://apps.apple.com/app/fabushi",
                "binary_self_update_allowed": False,
            },
        )

    # Exact 37 segment-bounded fallback namespaces (excluding auth, user-vault, vault, sync, media, __e2e__)
    if path.startswith("/api/v1/"):
        rest = path[len("/api/v1/") :]
        ns = rest.split("/", 1)[0]
        if ns in ALLOWED_FALLBACK_NAMESPACES:
            return 200, {"ok": True, "path": path, "method": method, "timestamp_ms": ts}

    # Fail-closed 404 on every unrecognized / sibling route
    return (
        404,
        {
            "error": "not_found",
            "message": f"unrecognized loopback route: {method} {path}",
            "path": path,
            "method": method,
        },
    )


class LoopbackHTTPServer(http.server.ThreadingHTTPServer):
    allow_reuse_address = True
    daemon_threads = True

    def __init__(
        self,
        server_address: tuple[str, int],
        handler_cls: type[http.server.BaseHTTPRequestHandler],
        state: LoopbackState,
        quiet: bool = False,
        stop_event: threading.Event | None = None,
    ) -> None:
        self.state = state
        self.quiet = quiet
        self.stop_event: threading.Event = (
            stop_event if stop_event is not None else threading.Event()
        )
        super().__init__(server_address, handler_cls)


class LoopbackHandler(http.server.BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"
    timeout = HTTP_SOCKET_TIMEOUT_SECS

    def handle_expect_100(self) -> bool:
        """Defer 100 Continue until _read_request_body() validates Content-Length/TE/offline."""
        return True

    def handle_one_request(self) -> None:
        try:
            super().handle_one_request()
        except (BrokenPipeError, ConnectionResetError, TimeoutError, socket.timeout, OSError):
            self.close_connection = True

    def log_message(self, fmt: str, *args: Any) -> None:
        srv = self.server
        if isinstance(srv, LoopbackHTTPServer) and srv.quiet:
            return
        sys.stderr.write(f"[loopback] {self.address_string()} - {fmt % args}\n")
        sys.stderr.flush()

    def _send_bytes(
        self,
        status: int,
        content_type: str,
        payload: bytes,
        extra_headers: list[tuple[str, str]] | None = None,
        is_head: bool = False,
    ) -> None:
        self.close_connection = True
        self.send_response(status)
        self.send_header("Content-Type", content_type)
        self.send_header("Content-Length", str(len(payload)))
        self.send_header("Connection", "close")
        if extra_headers:
            for k, v in extra_headers:
                self.send_header(k, v)
        self.end_headers()
        if not is_head and payload:
            self.wfile.write(payload)

    def _send_json(
        self,
        status: int,
        obj: dict[str, Any],
        extra_headers: list[tuple[str, str]] | None = None,
        is_head: bool = False,
    ) -> None:
        data = json.dumps(obj, separators=(",", ":")).encode("utf-8")
        self._send_bytes(status, "application/json", data, extra_headers, is_head=is_head)

    def _send_fault(self, fault: dict[str, Any], *, is_head: bool = False) -> None:
        extra: list[tuple[str, str]] = []
        retry_ms = fault.get("retry_after_ms")
        if _is_strict_int(retry_ms) and retry_ms > 0:
            sec = (retry_ms + 999) // 1000
            extra.append(("Retry-After", str(sec)))
        body_obj = fault.get("body")
        if not isinstance(body_obj, dict):
            body_obj = {"error": f"injected_http_{fault['status']}"}
        self._send_json(fault["status"], body_obj, extra_headers=extra, is_head=is_head)

    def _parse_content_length(self) -> int | None:
        cl_values = self.headers.get_all("Content-Length") or []
        if not cl_values:
            return None
        parsed_vals: list[int] = []
        for raw in cl_values:
            for part in raw.split(","):
                tok = part.strip()
                if not tok or not _is_ascii_uint_str(tok):
                    raise BadRequestError("invalid_content_length")
                parsed_vals.append(int(tok))
        if not parsed_vals:
            raise BadRequestError("invalid_content_length")
        first = parsed_vals[0]
        if any(v != first for v in parsed_vals[1:]):
            raise BadRequestError("conflicting_content_length")
        if first > MAX_HTTP_BODY_BYTES:
            raise PayloadTooLargeError("body_too_large")
        return first

    def _read_exact(self, length: int, *, err_msg: str = "truncated_body") -> bytes:
        buf = bytearray()
        remaining = length
        while remaining > 0:
            try:
                chunk = self.rfile.read(remaining)
            except (TimeoutError, socket.timeout) as exc:
                raise BadRequestError(f"{err_msg}_timeout") from exc
            if not chunk:
                raise BadRequestError(err_msg)
            buf.extend(chunk)
            remaining -= len(chunk)
        return bytes(buf)

    def _read_request_body(self) -> bytes:
        te_values = self.headers.get_all("Transfer-Encoding") or []
        is_chunked = any("chunked" in v.lower() for v in te_values)
        cl = self._parse_content_length()
        if is_chunked and cl is not None:
            raise BadRequestError("conflicting_te_and_cl")

        expect = self.headers.get("Expect", "")
        if (
            self.request_version >= "HTTP/1.1"
            and "100-continue" in expect.lower()
            and (is_chunked or (cl is not None and cl > 0))
        ):
            self.wfile.write(b"HTTP/1.1 100 Continue\r\n\r\n")
            self.wfile.flush()

        if is_chunked:
            out = bytearray()
            while True:
                try:
                    line = self.rfile.readline(65536)
                except (TimeoutError, socket.timeout) as exc:
                    raise BadRequestError("truncated_chunk_size") from exc
                if not line or not line.endswith(b"\n"):
                    raise BadRequestError("truncated_chunk_size")
                size_part = line.split(b";", 1)[0].strip()
                if not size_part or not all(
                    (48 <= b <= 57) or (65 <= b <= 70) or (97 <= b <= 102)
                    for b in size_part
                ):
                    raise BadRequestError("invalid_chunk_size")
                chunk_size = int(size_part.decode("ascii"), 16)
                if chunk_size == 0:
                    trailer_count = 0
                    while True:
                        try:
                            tline = self.rfile.readline(65536)
                        except (TimeoutError, socket.timeout) as exc:
                            raise BadRequestError("truncated_chunk_trailers") from exc
                        if not tline:
                            raise BadRequestError("truncated_chunk_trailers")
                        if not tline.endswith(b"\n"):
                            raise BadRequestError("chunk_trailer_too_long")
                        if tline in (b"\r\n", b"\n"):
                            break
                        trailer_count += 1
                        if trailer_count > MAX_HTTP_TRAILER_LINES:
                            raise BadRequestError("too_many_trailers")
                    break
                if len(out) + chunk_size > MAX_HTTP_BODY_BYTES:
                    raise PayloadTooLargeError("chunked_body_too_large")
                chunk = self._read_exact(chunk_size, err_msg="truncated_chunk_data")
                out.extend(chunk)
                try:
                    crlf = self.rfile.readline(4)
                except (TimeoutError, socket.timeout) as exc:
                    raise BadRequestError("missing_chunk_crlf") from exc
                if crlf not in (b"\r\n", b"\n"):
                    raise BadRequestError("missing_chunk_crlf")
            return bytes(out)

        if cl is None or cl == 0:
            return b""
        return self._read_exact(cl, err_msg="truncated_body")

    def _extract_media_bytes(self, headers: dict[str, str], body_bytes: bytes) -> bytes:
        ct = headers.get("content-type", "").lower()
        if ct.startswith("application/json"):
            try:
                parsed = json.loads(body_bytes.decode("utf-8"))
                if isinstance(parsed, dict) and isinstance(parsed.get("data_b64"), str):
                    return base64.b64decode(parsed["data_b64"].encode("ascii"))
            except (UnicodeDecodeError, json.JSONDecodeError, ValueError):
                pass
        return body_bytes

    def _handle_websocket(
        self, state: LoopbackState, ws_key: str, ws_token: str | None
    ) -> None:
        assert isinstance(self.server, LoopbackHTTPServer)
        try:
            accept = compute_websocket_accept(ws_key)
        except UnicodeEncodeError:
            self._send_json(400, {"error": "invalid_websocket_key"})
            return

        self.close_connection = True
        sock = self.connection
        # Drain any already-buffered bytes from self.rfile before taking raw socket ownership
        initial_ws = bytearray()
        max_initial_ws = MAX_WS_FRAME_PAYLOAD_BYTES + 65536
        sock.settimeout(0.0)
        try:
            while len(initial_ws) < max_initial_ws:
                chunk = self.rfile.read1(min(65536, max_initial_ws - len(initial_ws)))
                if not chunk:
                    break
                initial_ws.extend(chunk)
        except (BlockingIOError, TimeoutError, socket.timeout, OSError):
            pass
        reader = _WsConnReader(bytes(initial_ws))

        with state.lock:
            should_drop = state.drop_next_ws
            if should_drop:
                state.drop_next_ws = False
        if should_drop:
            try:
                sock.shutdown(socket.SHUT_RDWR)
            except OSError:
                pass
            return

        try:
            sock.settimeout(HTTP_SOCKET_TIMEOUT_SECS)
            handshake_resp = (
                "HTTP/1.1 101 Switching Protocols\r\n"
                "Upgrade: websocket\r\n"
                "Connection: Upgrade\r\n"
                f"Sec-WebSocket-Accept: {accept}\r\n\r\n"
            ).encode("ascii")
            sock.sendall(handshake_resp)

            frag_opcode: int | None = None
            frag_buf = bytearray()
            stop_event = self.server.stop_event

            def _dispatch_ws_data(msg_opcode: int, msg_bytes: bytes) -> None:
                if msg_opcode == 0x1:
                    try:
                        text = msg_bytes.decode("utf-8")
                    except UnicodeDecodeError as exc:
                        raise _WsProtocolError(1007, "invalid_utf8") from exc
                    with state.lock:
                        state.ws_received_frames.append(text)
                    ack = json.dumps(
                        {"type": "ws_ack", "echo": text, "timestamp_ms": now_ms()},
                        separators=(",", ":"),
                    ).encode("utf-8")
                    sock.settimeout(HTTP_SOCKET_TIMEOUT_SECS)
                    _write_ws_frame(sock, 0x1, ack)
                else:
                    sock.settimeout(HTTP_SOCKET_TIMEOUT_SECS)
                    _write_ws_frame(sock, 0x2, msg_bytes)

            while not stop_event.is_set():
                pending_pushes: list[str] = []
                with state.lock:
                    is_revoked = ws_token is not None and state.is_token_rejected(ws_token)
                    is_offline = state.offline
                    if not is_revoked and not is_offline:
                        while state.ws_push_queue:
                            pending_pushes.append(state.ws_push_queue.popleft())
                if is_revoked:
                    sock.settimeout(HTTP_SOCKET_TIMEOUT_SECS)
                    _write_ws_close(sock, 4001, "session_revoked")
                    break
                if is_offline:
                    sock.settimeout(HTTP_SOCKET_TIMEOUT_SECS)
                    _write_ws_close(sock, 1001, "offline")
                    break
                for msg in pending_pushes:
                    sock.settimeout(HTTP_SOCKET_TIMEOUT_SECS)
                    _write_ws_frame(sock, 0x1, msg.encode("utf-8"))

                try:
                    frame = reader.read_frame(sock, stop_event)
                except _WsProtocolError as exc:
                    sock.settimeout(HTTP_SOCKET_TIMEOUT_SECS)
                    _write_ws_close(sock, exc.code, exc.reason)
                    break
                except (_WsClosedError, OSError):
                    break

                if frame is None:
                    continue
                fin, opcode, payload = frame
                if opcode == 0x9:
                    sock.settimeout(HTTP_SOCKET_TIMEOUT_SECS)
                    _write_ws_frame(sock, 0xA, payload)
                elif opcode == 0xA:
                    continue
                elif opcode == 0x8:
                    sock.settimeout(HTTP_SOCKET_TIMEOUT_SECS)
                    _write_ws_close(sock, 1000, "")
                    break
                elif opcode in (0x1, 0x2):
                    if frag_opcode is not None:
                        sock.settimeout(HTTP_SOCKET_TIMEOUT_SECS)
                        _write_ws_close(sock, 1002, "interleaved_data_frame")
                        break
                    if fin:
                        try:
                            _dispatch_ws_data(opcode, payload)
                        except _WsProtocolError as exc:
                            sock.settimeout(HTTP_SOCKET_TIMEOUT_SECS)
                            _write_ws_close(sock, exc.code, exc.reason)
                            break
                    else:
                        if len(payload) > MAX_WS_MESSAGE_BYTES:
                            sock.settimeout(HTTP_SOCKET_TIMEOUT_SECS)
                            _write_ws_close(sock, 1009, "message_too_large")
                            break
                        frag_opcode = opcode
                        frag_buf = bytearray(payload)
                elif opcode == 0x0:
                    if frag_opcode is None:
                        sock.settimeout(HTTP_SOCKET_TIMEOUT_SECS)
                        _write_ws_close(sock, 1002, "unexpected_continuation")
                        break
                    if len(frag_buf) + len(payload) > MAX_WS_MESSAGE_BYTES:
                        sock.settimeout(HTTP_SOCKET_TIMEOUT_SECS)
                        _write_ws_close(sock, 1009, "message_too_large")
                        break
                    frag_buf.extend(payload)
                    if fin:
                        try:
                            _dispatch_ws_data(frag_opcode, bytes(frag_buf))
                        except _WsProtocolError as exc:
                            sock.settimeout(HTTP_SOCKET_TIMEOUT_SECS)
                            _write_ws_close(sock, exc.code, exc.reason)
                            break
                        frag_opcode = None
                        frag_buf.clear()
                else:
                    sock.settimeout(HTTP_SOCKET_TIMEOUT_SECS)
                    _write_ws_close(sock, 1002, "unsupported_opcode")
                    break
        except OSError:
            pass
        finally:
            try:
                sock.shutdown(socket.SHUT_RDWR)
            except OSError:
                pass

    def _dispatch(self) -> None:
        is_head = self.command.upper() == "HEAD"
        try:
            self._dispatch_inner()
        except ApiParamError as exc:
            self._send_json(
                400, {"error": "invalid_params", "message": str(exc)}, is_head=is_head
            )
        except PayloadTooLargeError as exc:
            self._send_json(
                413, {"error": "payload_too_large", "message": str(exc)}, is_head=is_head
            )
        except BadRequestError as exc:
            self._send_json(
                400, {"error": "bad_request", "message": str(exc)}, is_head=is_head
            )
        except (BrokenPipeError, ConnectionResetError, TimeoutError, socket.timeout):
            self.close_connection = True
        except Exception as exc:  # noqa: BLE001
            self._send_json(
                500,
                {"error": "internal_error", "message": f"{type(exc).__name__}: {exc}"},
                is_head=is_head,
            )

    def _dispatch_inner(self) -> None:
        assert isinstance(self.server, LoopbackHTTPServer)
        state = self.server.state
        method = self.command.upper()
        is_head = method == "HEAD"
        path, query = normalize_request_target(self.path)
        headers = {k.lower(): v for k, v in self.headers.items()}

        # Control plane `/__e2e__/*` is immune to offline, faults, and delays, and excluded from recorded
        if matches_segment_prefix(path, "/__e2e__"):
            body_bytes = self._read_request_body()
            status, resp = handle_e2e_control(state, method, path, query, body_bytes)
            self._send_json(status, resp, is_head=is_head)
            return

        # Offline simulation: hard transport disconnect without HTTP response
        with state.lock:
            is_offline = state.offline
        if is_offline:
            self.close_connection = True
            try:
                self.connection.shutdown(socket.SHUT_RDWR)
            except OSError:
                pass
            return

        body_bytes = self._read_request_body()
        state.record_request(method, path, query, headers, body_bytes)

        # Delay injection slept strictly OUTSIDE lock
        delay_ms = state.lookup_delay_ms(path)
        if delay_ms > 0:
            time.sleep(delay_ms / 1000.0)

        # Deterministic fault injection
        fault = state.pop_matching_fault(method, path)

        # HTTP CONNECT proxy tunneling (subject to offline/delays/faults)
        if method == "CONNECT":
            if fault is not None:
                self._send_fault(fault, is_head=False)
                return
            self.close_connection = True
            self.wfile.write(b"HTTP/1.1 200 Connection Established\r\n\r\n")
            self.wfile.flush()
            return

        # RFC 6455 WebSocket Upgrade Gate (strict path gate, pre-101 fault & Bearer revocation)
        upgrade_hdr = headers.get("upgrade", "").strip().lower()
        ws_key = headers.get("sec-websocket-key", "").strip()
        if upgrade_hdr == "websocket" and ws_key:
            if path not in ("/api/v1/ws", "/ws"):
                self._send_json(
                    404,
                    {
                        "error": "not_found",
                        "message": f"unrecognized websocket path: {path}",
                        "path": path,
                    },
                    is_head=is_head,
                )
                return
            if fault is not None:
                self._send_fault(fault, is_head=is_head)
                return
            ws_token = extract_bearer_token(headers, query)
            if ws_token is not None:
                with state.lock:
                    rejected = state.is_token_rejected(ws_token)
                if rejected:
                    self._send_json(
                        401,
                        {"error": "unauthorized", "message": "session_revoked"},
                        is_head=is_head,
                    )
                    return
            self._handle_websocket(state, ws_key, ws_token)
            return

        if fault is not None:
            self._send_fault(fault, is_head=is_head)
            return

        # Instant View HTML article routes (segment-bounded)
        if method in ("GET", "HEAD") and (
            matches_segment_prefix(path, "/article") or matches_segment_prefix(path, "/iv")
        ):
            slug = "v168"
            if path.startswith("/article/") and len(path) > len("/article/"):
                slug = path[len("/article/") :]
            elif path.startswith("/iv/") and len(path) > len("/iv/"):
                slug = path[len("/iv/") :]
            html = (
                f"<!doctype html><html><head><meta charset=\"utf-8\">"
                f"<title>Fabushi v{state.version} Instant View</title></head>"
                f"<body><article><h1>Fabushi v{state.version}</h1>"
                f"<p>Instant View content for {slug}.</p></article></body></html>"
            ).encode("utf-8")
            self._send_bytes(200, "text/html; charset=utf-8", html, is_head=is_head)
            return

        # Enforce Bearer revocation on ALL early binary/streaming /api/v1/media/* handlers
        if matches_segment_prefix(path, "/api/v1/media"):
            tok = extract_bearer_token(headers, query)
            if tok is not None:
                with state.lock:
                    rejected = state.is_token_rejected(tok)
                if rejected:
                    self._send_json(
                        401,
                        {"error": "unauthorized", "message": "session_revoked"},
                        is_head=is_head,
                    )
                    return

        # Binary media upload
        if method == "POST" and path == "/api/v1/media/upload":
            blob = self._extract_media_bytes(headers, body_bytes)
            digest = hex_sha256(blob)
            media_id = f"cas_{digest[:16]}"
            with state.lock:
                state.media_blobs[media_id] = blob
                state.media_blobs[digest] = blob
            self._send_json(
                200,
                {
                    "media_id": media_id,
                    "file_id": media_id,
                    "sha256": digest,
                    "size": len(blob),
                },
            )
            return

        # Resumable media upload part
        if (
            method in ("PUT", "POST")
            and path.startswith("/api/v1/media/upload/")
            and "/part/" in path
        ):
            rest = path[len("/api/v1/media/upload/") :]
            upl_id, part_s = rest.split("/part/", 1)
            if not _is_ascii_uint_str(part_s):
                self._send_json(400, {"error": "invalid_part_index"})
                return
            part_index = int(part_s)
            part_bytes = self._extract_media_bytes(headers, body_bytes)
            with state.lock:
                sess = state.uploads.get(upl_id)
                if sess is None:
                    self._send_json(404, {"error": "upload_not_found"})
                    return
                sess["parts"][part_index] = part_bytes
                received = sorted(sess["parts"].keys())
            self._send_json(
                200,
                {
                    "upload_id": upl_id,
                    "part_index": part_index,
                    "bytes_received": len(part_bytes),
                    "status": "accepted",
                    "received_parts": received,
                },
            )
            return

        # Unified GET/HEAD media retrieval (200 / 206 / 416 / 404)
        if method in ("GET", "HEAD"):
            file_id = resolve_media_get_id(path)
            if file_id is not None:
                with state.lock:
                    blob = state.media_blobs.get(file_id)
                if blob is None:
                    self._send_json(404, {"error": "media_not_found"}, is_head=is_head)
                    return
                if "range" in headers:
                    ok, start, end = parse_byte_range(headers["range"], len(blob))
                    if ok:
                        cr = f"bytes {start}-{end}/{len(blob)}"
                        self._send_bytes(
                            206,
                            "application/octet-stream",
                            blob[start : end + 1],
                            extra_headers=[
                                ("Accept-Ranges", "bytes"),
                                ("Content-Range", cr),
                            ],
                            is_head=is_head,
                        )
                        return
                    self._send_json(
                        416,
                        {"error": "range_not_satisfiable"},
                        extra_headers=[("Content-Range", f"bytes */{len(blob)}")],
                        is_head=is_head,
                    )
                    return
                self._send_bytes(
                    200,
                    "application/octet-stream",
                    blob,
                    extra_headers=[("Accept-Ranges", "bytes")],
                    is_head=is_head,
                )
                return

        # JSON API routes
        req_json: dict[str, Any] = {}
        if body_bytes.strip():
            try:
                parsed = json.loads(body_bytes.decode("utf-8"))
                if isinstance(parsed, dict):
                    req_json = parsed
            except (UnicodeDecodeError, json.JSONDecodeError):
                req_json = {}

        status, resp_obj = route_api_request(state, method, path, query, headers, req_json)
        self._send_json(status, resp_obj, is_head=is_head)

    def do_GET(self) -> None:
        self._dispatch()

    def do_HEAD(self) -> None:
        self._dispatch()

    def do_POST(self) -> None:
        self._dispatch()

    def do_PUT(self) -> None:
        self._dispatch()

    def do_PATCH(self) -> None:
        self._dispatch()

    def do_DELETE(self) -> None:
        self._dispatch()

    def do_OPTIONS(self) -> None:
        self._dispatch()

    def do_CONNECT(self) -> None:
        self._dispatch()


# ---------------------------------------------------------------------------
# Deterministic --self-test suite (11 checks, zero external deps, zero repo files)
# ---------------------------------------------------------------------------


def _http_call(
    base_url: str,
    method: str,
    path: str,
    body: dict[str, Any] | bytes | None = None,
    headers: dict[str, str] | None = None,
    timeout: float = 5.0,
) -> tuple[int, dict[str, str], bytes]:
    hdrs = dict(headers or {})
    data: bytes | None = None
    if isinstance(body, dict):
        data = json.dumps(body).encode("utf-8")
        hdrs.setdefault("Content-Type", "application/json")
    elif isinstance(body, bytes):
        data = body
    req = urllib.request.Request(f"{base_url}{path}", data=data, headers=hdrs, method=method)
    try:
        with urllib.request.urlopen(req, timeout=timeout) as resp:
            return resp.status, {k.lower(): v for k, v in resp.headers.items()}, resp.read()
    except urllib.error.HTTPError as err:
        return err.code, {k.lower(): v for k, v in err.headers.items()}, err.read()


def _json_call(
    base_url: str,
    method: str,
    path: str,
    body: dict[str, Any] | None = None,
    bearer: str | None = None,
) -> tuple[int, dict[str, Any]]:
    hdrs: dict[str, str] = {}
    if bearer:
        hdrs["Authorization"] = f"Bearer {bearer}"
    status, _, raw = _http_call(base_url, method, path, body=body, headers=hdrs)
    return status, json.loads(raw.decode("utf-8"))


def _raw_tcp_request(host: str, port: int, payload: bytes) -> tuple[int, str, bytes]:
    with socket.create_connection((host, port), timeout=5.0) as sock:
        sock.settimeout(5.0)
        sock.sendall(payload)
        chunks: list[bytes] = []
        while True:
            buf = sock.recv(65536)
            if not buf:
                break
            chunks.append(buf)
    raw = b"".join(chunks)
    sep = raw.find(b"\r\n\r\n")
    assert sep != -1, f"missing HTTP response headers: {raw!r}"
    head = raw[:sep].decode("utf-8", errors="replace")
    body = raw[sep + 4 :]
    status = int(head.splitlines()[0].split()[1])
    return status, head, body


def _start_ephemeral_server(version: str = DEFAULT_SERVER_VERSION) -> tuple[LoopbackHTTPServer, threading.Thread, str, int]:
    state = LoopbackState(version=version)
    srv = LoopbackHTTPServer(("127.0.0.1", 0), LoopbackHandler, state=state, quiet=True)
    port = srv.server_address[1]
    thread = threading.Thread(target=srv.serve_forever, kwargs={"poll_interval": 0.02}, daemon=True)
    thread.start()
    return srv, thread, f"http://127.0.0.1:{port}", port


def _stop_ephemeral_server(srv: LoopbackHTTPServer, thread: threading.Thread) -> None:
    srv.stop_event.set()
    srv.shutdown()
    thread.join(timeout=5.0)
    srv.server_close()


def run_self_test(report_path: str = "") -> int:
    checks: list[dict[str, Any]] = []

    def run_check(name: str, fn: Any) -> None:
        t0 = time.monotonic()
        try:
            detail = fn() or "ok"
            checks.append(
                {
                    "name": name,
                    "ok": True,
                    "duration_ms": int((time.monotonic() - t0) * 1000),
                    "detail": str(detail),
                }
            )
        except BaseException as exc:
            checks.append(
                {
                    "name": name,
                    "ok": False,
                    "duration_ms": int((time.monotonic() - t0) * 1000),
                    "detail": f"{type(exc).__name__}: {exc}",
                }
            )

    def check_1_cli_atomic_and_sigterm() -> str:
        with tempfile.TemporaryDirectory(prefix="fabushi_loopback_selftest_") as tmp:
            tmp_dir = pathlib.Path(tmp)
            port_file = tmp_dir / "loopback.port"
            url_file = tmp_dir / "loopback.url"
            proc = subprocess.Popen(
                [
                    sys.executable,
                    str(pathlib.Path(__file__).resolve()),
                    "--host",
                    "127.0.0.1",
                    "--port",
                    "0",
                    "--port-file",
                    str(port_file),
                    "--url-file",
                    str(url_file),
                    "--quiet",
                ],
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
            )
            try:
                deadline = time.monotonic() + 5.0
                while time.monotonic() < deadline:
                    if port_file.exists() and url_file.exists():
                        if port_file.stat().st_size > 0 and url_file.stat().st_size > 0:
                            break
                    if proc.poll() is not None:
                        raise AssertionError(
                            f"subprocess exited early with code {proc.returncode}"
                        )
                    time.sleep(0.05)
                assert port_file.exists() and url_file.exists(), "port/url files not created"
                tmp_leftovers = [p.name for p in tmp_dir.iterdir() if p.name.endswith(".tmp")]
                assert not tmp_leftovers, f"atomic temp files left behind: {tmp_leftovers}"
                port_s = port_file.read_text(encoding="utf-8").strip()
                url_s = url_file.read_text(encoding="utf-8").strip()
                assert port_s.isdigit() and int(port_s) > 0
                assert url_s == f"http://127.0.0.1:{port_s}", f"mismatch {url_s} vs {port_s}"
                st_h, h_json = _json_call(url_s, "GET", "/healthz")
                assert st_h == 200 and h_json["ok"] is True
                st_s, s_json = _json_call(url_s, "GET", "/api/v1/status")
                assert st_s == 200 and s_json["status"] == "healthy"
            finally:
                proc.send_signal(signal.SIGTERM)
                _, err_bytes = proc.communicate(timeout=5.0)
                rc = proc.returncode
            assert rc == 0, f"expected clean SIGTERM exit 0, got {rc} (stderr={err_bytes.decode('utf-8', errors='replace')!r})"
            return f"bound={url_s}, sigterm_rc=0"

    def check_2_auth_split_key_v2_rotation_logout() -> str:
        srv, th, base, _ = _start_ephemeral_server()
        try:
            st_empty_login, _ = _json_call(base, "POST", "/api/v1/auth/login", {})
            assert st_empty_login == 401
            st_empty_reg, _ = _json_call(base, "POST", "/api/v1/auth/register", {})
            assert st_empty_reg == 400
            salt1 = base64.b64encode(bytes([0x11] * 16)).decode("ascii")
            ver1 = base64.b64encode(bytes([0x22] * 32)).decode("ascii")
            ver2 = base64.b64encode(bytes([0x33] * 32)).decode("ascii")
            st_reg, reg = _json_call(
                base,
                "POST",
                "/api/v1/auth/register",
                {
                    "username": "alice@fabushi.app",
                    "auth_version": 2,
                    "auth_salt_b64": salt1,
                    "auth_verifier_b64": ver1,
                },
            )
            assert st_reg == 200 and reg["expires_at"].endswith("Z")
            tok1 = reg["token"]
            st_kdf, kdf = _json_call(
                base, "GET", "/api/v1/auth/kdf-params?identifier=alice%40fabushi.app"
            )
            assert st_kdf == 200 and kdf["auth_salt_b64"] == salt1
            st_bad, _ = _json_call(
                base,
                "POST",
                "/api/v1/auth/login",
                {"username": "alice@fabushi.app", "auth_verifier_b64": ver2},
            )
            assert st_bad == 401
            st_rot, rot = _json_call(
                base,
                "POST",
                "/api/v1/auth/change-password",
                {"current_auth_verifier_b64": ver1, "new_auth_verifier_b64": ver2},
                bearer=tok1,
            )
            assert st_rot == 200 and rot["revoked_sessions_count"] >= 1
            tok2 = rot["token"]
            st_old_me, _ = _json_call(base, "GET", "/api/v1/auth/me", bearer=tok1)
            assert st_old_me == 401
            st_new_me, new_me = _json_call(base, "GET", "/api/v1/auth/me", bearer=tok2)
            assert st_new_me == 200 and new_me["username"] == "alice@fabushi.app"
            st_lo, _ = _json_call(base, "POST", "/api/v1/auth/logout", {}, bearer=tok2)
            assert st_lo == 200
            st_after, _ = _json_call(base, "GET", "/api/v1/auth/me", bearer=tok2)
            assert st_after == 401

            # Split-Key V2 vs legacy_v1 seed normalization & whitespace upgrade guard (B-01 / A-01)
            st_seed_pw, _ = _json_call(
                base,
                "POST",
                "/__e2e__/seed",
                {
                    "users": [
                        {
                            "username": "legacy_pw_user",
                            "password": "Legacy!Password1",
                            "auth_version": 2,
                        },
                        {
                            "username": "v2_seeded_user",
                            "auth_salt_b64": salt1,
                            "auth_verifier_b64": ver1,
                        },
                    ]
                },
            )
            assert st_seed_pw == 200
            st_lk, lk = _json_call(
                base, "GET", "/api/v1/auth/kdf-params?identifier=legacy_pw_user"
            )
            assert st_lk == 200 and lk["auth_version"] == 1 and lk["kdf_alg"] == "legacy_v1"
            st_pw_ok, pw_login = _json_call(
                base,
                "POST",
                "/api/v1/auth/login",
                {"username": "legacy_pw_user", "password": "Legacy!Password1"},
            )
            assert st_pw_ok == 200
            pw_tok = pw_login["token"]
            st_ws_login, _ = _json_call(
                base,
                "POST",
                "/api/v1/auth/login",
                {"username": "legacy_pw_user", "password": "   "},
            )
            assert st_ws_login == 401
            st_ws_upg, _ = _json_call(
                base,
                "POST",
                "/api/v1/auth/upgrade-split-key",
                {"current_password": "   ", "new_auth_verifier_b64": ver2},
                bearer=pw_tok,
            )
            assert st_ws_upg == 401
            st_upg_ok, upg = _json_call(
                base,
                "POST",
                "/api/v1/auth/upgrade-split-key",
                {"current_password": "Legacy!Password1", "new_auth_verifier_b64": ver2},
                bearer=pw_tok,
            )
            assert st_upg_ok == 200 and upg["auth_version"] == 2
            st_lk2, lk2 = _json_call(
                base, "GET", "/api/v1/auth/kdf-params?identifier=legacy_pw_user"
            )
            assert st_lk2 == 200 and lk2["auth_version"] == 2 and lk2["kdf_alg"] == "split_key_v2"
            st_pw_after, _ = _json_call(
                base,
                "POST",
                "/api/v1/auth/login",
                {"username": "legacy_pw_user", "password": "Legacy!Password1"},
            )
            assert st_pw_after == 401
            st_v2k, v2k = _json_call(
                base, "GET", "/api/v1/auth/kdf-params?identifier=v2_seeded_user"
            )
            assert st_v2k == 200 and v2k["auth_version"] == 2 and v2k["kdf_alg"] == "split_key_v2"
            st_v2_ok, _ = _json_call(
                base,
                "POST",
                "/api/v1/auth/login",
                {"username": "v2_seeded_user", "auth_verifier_b64": ver1},
            )
            assert st_v2_ok == 200
            st_v2_bad, _ = _json_call(
                base,
                "POST",
                "/api/v1/auth/login",
                {"username": "v2_seeded_user", "auth_verifier_b64": ver2},
            )
            assert st_v2_bad == 401
            return "split_key_v2+legacy_v1+rotation+logout verified"
        finally:
            _stop_ephemeral_server(srv, th)

    def check_3_chats_messages_pts_cjk_search() -> str:
        srv, th, base, _ = _start_ephemeral_server()
        try:
            _, reg = _json_call(
                base,
                "POST",
                "/api/v1/auth/register",
                {"username": "bob", "password": "Password1!"},
            )
            tok = reg["token"]
            st1, c1 = _json_call(
                base,
                "POST",
                "/api/v1/chats",
                {
                    "title": "Team",
                    "client_nonce": "nonce_chat_1",
                    "member_ids": [2001, 2002, 2002],
                },
                bearer=tok,
            )
            st2, c2 = _json_call(
                base,
                "POST",
                "/api/v1/chats",
                {
                    "title": "Team",
                    "client_nonce": "nonce_chat_1",
                    "member_ids": [2001, 2002],
                },
                bearer=tok,
            )
            assert st1 == 200 and st2 == 200 and c1["chat_id"] == c2["chat_id"]
            assert c2["deduplicated"] is True and c1["member_count"] == 3
            cid = c1["chat_id"]
            _, m1 = _json_call(
                base,
                "POST",
                f"/api/v1/chats/{cid}/messages",
                {"text": "全球法布施 v1.6.8", "client_nonce": 42},
                bearer=tok,
            )
            _, m1_dup = _json_call(
                base,
                "POST",
                f"/api/v1/chats/{cid}/messages",
                {"text": "全球法布施 v1.6.8", "client_nonce": 42},
                bearer=tok,
            )
            assert m1["message_id"] == m1_dup["message_id"] and m1["pts"] == 1
            _, diff_ok = _json_call(
                base, "POST", "/api/v1/sync/get_difference", {"chat_id": cid, "pts": 0}, bearer=tok
            )
            assert diff_ok["snapshot_required"] is False and len(diff_ok["messages"]) == 1
            _json_call(base, "POST", "/__e2e__/seed", {"journal_floors": {str(cid): 5}})
            _, diff_snap = _json_call(
                base, "POST", "/api/v1/sync/get_difference", {"chat_id": cid, "pts": 1}, bearer=tok
            )
            assert diff_snap["snapshot_required"] is True
            st_q, q_resp = _json_call(
                base,
                "GET",
                "/api/v1/search/messages?q=%E5%85%A8%E7%90%83",
                bearer=tok,
            )
            assert st_q == 200 and q_resp["total"] == 1 and q_resp["query"] == "全球"
            return f"chat_id={cid}, cjk_hits=1"
        finally:
            _stop_ephemeral_server(srv, th)

    def check_4_media_cas_chunked_206_416_revocation() -> str:
        srv, th, base, port = _start_ephemeral_server()
        try:
            _, reg = _json_call(
                base,
                "POST",
                "/api/v1/auth/register",
                {"username": "media_u", "password": "Password1!"},
            )
            tok = reg["token"]
            payload = b"0123456789ABCDEFfabushi_v168_cas_blob"
            chunked_req = (
                f"POST /api/v1/media/upload HTTP/1.1\r\n"
                f"Host: 127.0.0.1:{port}\r\n"
                f"Authorization: Bearer {tok}\r\n"
                f"Transfer-Encoding: chunked\r\n"
                f"Connection: close\r\n\r\n"
                f"{len(payload):x}\r\n"
            ).encode("ascii") + payload + b"\r\n0\r\n\r\n"
            st_up, _, raw_up = _raw_tcp_request("127.0.0.1", port, chunked_req)
            assert st_up == 200
            up_json = json.loads(raw_up.decode("utf-8"))
            assert up_json["sha256"] == hex_sha256(payload)
            mid = up_json["media_id"]

            st_206, hdr_206, b_206 = _http_call(
                base,
                "GET",
                f"/api/v1/media/blob/{mid}",
                headers={"Authorization": f"Bearer {tok}", "Range": "bytes=0-9"},
            )
            assert st_206 == 206 and b_206 == b"0123456789"
            assert hdr_206["content-range"] == f"bytes 0-9/{len(payload)}"
            st_416, _, _ = _http_call(
                base,
                "GET",
                f"/api/v1/media/blob/{mid}",
                headers={"Authorization": f"Bearer {tok}", "Range": "bytes=9999-10000"},
            )
            assert st_416 == 416

            _, init = _json_call(
                base,
                "POST",
                "/api/v1/media/upload/init",
                {
                    "filename": "part.bin",
                    "total_size": 8,
                    "part_size": 4,
                    "expected_sha256": hex_sha256(b"abcdEFGH"),
                },
                bearer=tok,
            )
            uid = init["upload_id"]
            st_p0, _, _ = _http_call(
                base,
                "PUT",
                f"/api/v1/media/upload/{uid}/part/0",
                body=b"abcd",
                headers={"Authorization": f"Bearer {tok}"},
            )
            st_p1, _, _ = _http_call(
                base,
                "PUT",
                f"/api/v1/media/upload/{uid}/part/1",
                body=b"EFGH",
                headers={"Authorization": f"Bearer {tok}"},
            )
            assert st_p0 == 200 and st_p1 == 200
            st_st, stat = _json_call(base, "GET", f"/api/v1/media/upload/{uid}/status", bearer=tok)
            assert st_st == 200 and stat["received_parts"] == [0, 1]
            st_comp, comp = _json_call(
                base, "POST", f"/api/v1/media/upload/{uid}/complete", {}, bearer=tok
            )
            assert st_comp == 200 and comp["status"] == "completed"

            _json_call(base, "POST", "/api/v1/auth/logout", {}, bearer=tok)
            st_rev, _, _ = _http_call(
                base,
                "GET",
                f"/api/v1/media/blob/{mid}",
                headers={"Authorization": f"Bearer {tok}"},
            )
            assert st_rev == 401

            # HTTP/1.1 framing, Expect: 100-continue pre-validation, HEAD & CONNECT checks (B-04)
            for bad_cl in ("+5", "1_000"):
                raw_req = (
                    f"POST /api/v1/messages/send HTTP/1.1\r\n"
                    f"Host: 127.0.0.1:{port}\r\n"
                    f"Content-Length: {bad_cl}\r\n"
                    f"Expect: 100-continue\r\n"
                    f"Connection: close\r\n\r\nhello"
                ).encode("ascii")
                st_bcl, head_bcl, _ = _raw_tcp_request("127.0.0.1", port, raw_req)
                assert st_bcl == 400 and "100 Continue" not in head_bcl, (
                    f"bad_cl={bad_cl!r} expected 400 without 100 Continue, got st={st_bcl} head={head_bcl!r}"
                )
            dup_cl_req = (
                f"POST /api/v1/messages/send HTTP/1.1\r\n"
                f"Host: 127.0.0.1:{port}\r\n"
                f"Content-Length: 5\r\n"
                f"Content-Length: 10\r\n"
                f"Connection: close\r\n\r\nhello"
            ).encode("ascii")
            st_dcl, head_dcl, _ = _raw_tcp_request("127.0.0.1", port, dup_cl_req)
            assert st_dcl == 400, f"dup CL expected 400, got st={st_dcl} head={head_dcl!r}"
            big_req = (
                f"POST /api/v1/messages/send HTTP/1.1\r\n"
                f"Host: 127.0.0.1:{port}\r\n"
                f"Content-Length: 40000000\r\n"
                f"Expect: 100-continue\r\n"
                f"Connection: close\r\n\r\n"
            ).encode("ascii")
            st_413, head_413, _ = _raw_tcp_request("127.0.0.1", port, big_req)
            assert st_413 == 413 and "100 Continue" not in head_413, (
                f"oversized CL expected 413 without 100 Continue, got st={st_413} head={head_413!r}"
            )
            cont_body = b'{"clear":true}'
            cont_headers = (
                f"POST /__e2e__/fault HTTP/1.1\r\n"
                f"Host: 127.0.0.1:{port}\r\n"
                f"Content-Type: application/json\r\n"
                f"Content-Length: {len(cont_body)}\r\n"
                f"Expect: 100-continue\r\n"
                f"Connection: close\r\n\r\n"
            ).encode("ascii")
            with socket.create_connection(("127.0.0.1", port), timeout=5.0) as s_100:
                s_100.settimeout(5.0)
                s_100.sendall(cont_headers)
                interim = bytearray()
                while b"\r\n\r\n" not in interim:
                    b_in = s_100.recv(4096)
                    if not b_in:
                        break
                    interim.extend(b_in)
                assert bytes(interim) == b"HTTP/1.1 100 Continue\r\n\r\n", (
                    f"positive Expect: 100-continue expected exact interim 100 Continue, got {bytes(interim)!r}"
                )
                s_100.sendall(cont_body)
                fin_chunks: list[bytes] = []
                while True:
                    b_fin = s_100.recv(4096)
                    if not b_fin:
                        break
                    fin_chunks.append(b_fin)
                raw_final_100 = b"".join(fin_chunks)
            assert raw_final_100.startswith(b"HTTP/1.1 200 "), (
                f"positive Expect: 100-continue expected final 200, got {raw_final_100!r}"
            )
            fin_sep = raw_final_100.find(b"\r\n\r\n")
            assert fin_sep != -1, f"missing header/body separator in 100-continue response: {raw_final_100!r}"
            fin_json = json.loads(raw_final_100[fin_sep + 4 :].decode("utf-8"))
            assert fin_json.get("ok") is True, f"expected ok:true after 100-continue body, got {fin_json!r}"
            bad_chunk_plus = (
                f"POST /api/v1/messages/send HTTP/1.1\r\n"
                f"Host: 127.0.0.1:{port}\r\n"
                f"Transfer-Encoding: chunked\r\n"
                f"Connection: close\r\n\r\n+a\r\n0123456789\r\n0\r\n\r\n"
            ).encode("ascii")
            st_bc1, _, _ = _raw_tcp_request("127.0.0.1", port, bad_chunk_plus)
            assert st_bc1 == 400, f"bad chunk +a expected 400, got {st_bc1}"
            trunc_trailer = (
                f"POST /api/v1/messages/send HTTP/1.1\r\n"
                f"Host: 127.0.0.1:{port}\r\n"
                f"Transfer-Encoding: chunked\r\n"
                f"Connection: close\r\n\r\n0\r\n"
            ).encode("ascii")
            with socket.create_connection(("127.0.0.1", port), timeout=5.0) as s_tr:
                s_tr.settimeout(5.0)
                s_tr.sendall(trunc_trailer)
                s_tr.shutdown(socket.SHUT_WR)
                tr_chunks: list[bytes] = []
                while True:
                    b_tr = s_tr.recv(4096)
                    if not b_tr:
                        break
                    tr_chunks.append(b_tr)
                raw_tr = b"".join(tr_chunks)
            assert raw_tr.startswith(b"HTTP/1.1 400 "), f"truncated trailer expected 400, got {raw_tr!r}"
            many_trailers = (
                f"POST /api/v1/messages/send HTTP/1.1\r\n"
                f"Host: 127.0.0.1:{port}\r\n"
                f"Transfer-Encoding: chunked\r\n"
                f"Connection: close\r\n\r\n0\r\n"
                + "".join(f"X-T{i}: 1\r\n" for i in range(66))
                + "\r\n"
            ).encode("ascii")
            st_mt, _, _ = _raw_tcp_request("127.0.0.1", port, many_trailers)
            assert st_mt == 400, f"too many trailers expected 400, got {st_mt}"
            st_hd, hdr_hd, body_hd = _http_call(base, "HEAD", "/healthz")
            assert st_hd == 200 and int(hdr_hd.get("content-length", "0")) > 0 and body_hd == b"", (
                f"HEAD /healthz failed: st={st_hd} hdr={hdr_hd} body={body_hd!r}"
            )
            conn_req = (
                f"CONNECT 127.0.0.1:443 HTTP/1.1\r\nHost: 127.0.0.1:443\r\n\r\n"
            ).encode("ascii")
            st_cn, _, _ = _raw_tcp_request("127.0.0.1", port, conn_req)
            assert st_cn == 200, f"CONNECT online expected 200, got {st_cn}"
            _json_call(base, "POST", "/__e2e__/fault", {"offline": True})
            with socket.create_connection(("127.0.0.1", port), timeout=5.0) as s_cn:
                s_cn.settimeout(5.0)
                s_cn.sendall(conn_req)
                try:
                    raw_cn_off = s_cn.recv(4096)
                except OSError:
                    raw_cn_off = b""
            assert raw_cn_off == b"", f"CONNECT must disconnect while offline, got {raw_cn_off!r}"
            _json_call(base, "DELETE", "/__e2e__/fault")
            return f"media_id={mid}, 206/416/revoked=401+framing+connect"
        finally:
            _stop_ephemeral_server(srv, th)

    def check_5_vault_and_device_sync() -> str:
        srv, th, base, _ = _start_ephemeral_server()
        try:
            _, reg = _json_call(
                base,
                "POST",
                "/api/v1/auth/register",
                {"username": "vault_u", "password": "Password1!"},
            )
            tok = reg["token"]
            st_put, _ = _json_call(
                base,
                "PUT",
                "/api/v1/user-vault/envelope",
                {"kdf_alg": "split_key_v2", "key_version": 2},
                bearer=tok,
            )
            st_get, env = _json_call(base, "GET", "/api/v1/user-vault/envelope", bearer=tok)
            assert st_put == 200 and st_get == 200 and env["key_version"] == 2
            st_ip, _ = _json_call(
                base,
                "PUT",
                "/api/v1/user-vault/items/password/github.com%3Aalice",
                {"ciphertext_b64": "YWJj", "nonce_b64": "MTIz"},
                bearer=tok,
            )
            st_ig, item = _json_call(
                base,
                "GET",
                "/api/v1/user-vault/items/password/github.com%3Aalice",
                bearer=tok,
            )
            assert st_ip == 200 and st_ig == 200 and item["item_key"] == "github.com:alice"
            st_id, _ = _json_call(
                base,
                "DELETE",
                "/api/v1/user-vault/items/password/github.com%3Aalice",
                bearer=tok,
            )
            st_gone, _ = _json_call(
                base,
                "GET",
                "/api/v1/user-vault/items/password/github.com%3Aalice",
                bearer=tok,
            )
            assert st_id == 200 and st_gone == 404
            _, dreq = _json_call(
                base,
                "POST",
                "/api/v1/user-vault/device-sync/request",
                {"device_name": "iPad Pro", "ephemeral_x25519_pub_b64": "cHVi"},
                bearer=tok,
            )
            rid = dreq["request_id"]
            _, pend = _json_call(base, "GET", "/api/v1/user-vault/device-sync/pending", bearer=tok)
            assert any(r["request_id"] == rid for r in pend["requests"])
            _, appr = _json_call(
                base,
                "POST",
                "/api/v1/user-vault/device-sync/approve",
                {"request_id": rid, "wrapped_key_b64": "d3JhcA=="},
                bearer=tok,
            )
            assert appr["status"] == "approved"
            _, stat = _json_call(
                base, "GET", f"/api/v1/user-vault/device-sync/status?request_id={rid}", bearer=tok
            )
            assert stat["status"] == "approved" and stat["wrapped_key_b64"] == "d3JhcA=="
            return f"device_sync={rid}"
        finally:
            _stop_ephemeral_server(srv, th)

    def check_6_computer_lease_and_v168_nodes() -> str:
        srv, th, base, _ = _start_ephemeral_server()
        try:
            _, reg = _json_call(
                base,
                "POST",
                "/api/v1/auth/register",
                {"username": "remote_u", "password": "Password1!"},
            )
            tok = reg["token"]
            _json_call(
                base,
                "POST",
                "/__e2e__/seed",
                {
                    "remote_nodes": [
                        {
                            "device_id": "mac_studio_01",
                            "name": "Mac Studio",
                            "online": True,
                            "screen_capture_permission": "not_granted",
                            "accessibility_drag_permission": "not_granted",
                        }
                    ]
                },
            )
            _, lease = _json_call(
                base, "POST", "/api/v1/computer/lease", {"device_id": "mac_studio_01"}, bearer=tok
            )
            gen1 = lease["generation"]
            st_ok, _ = _json_call(
                base,
                "POST",
                "/api/v1/computer/action",
                {"device_id": "mac_studio_01", "generation": gen1, "op": "pty_write"},
                bearer=tok,
            )
            assert st_ok == 200
            st_perm, perm = _json_call(
                base,
                "POST",
                "/api/v1/computer/action",
                {"device_id": "mac_studio_01", "generation": gen1, "op": "capture_screen"},
                bearer=tok,
            )
            assert st_perm == 403 and perm["error"] == "fallback_png_permission_denied"
            _, take = _json_call(
                base,
                "POST",
                "/api/v1/computer/lease/emergency_takeover",
                {"device_id": "mac_studio_01"},
                bearer=tok,
            )
            assert take["new_generation"] == gen1 + 1 and take["latency_sub_10ms"] is True
            st_stale, stale = _json_call(
                base,
                "POST",
                "/api/v1/computer/action",
                {"device_id": "mac_studio_01", "generation": gen1, "op": "pty_write"},
                bearer=tok,
            )
            assert st_stale == 403 and stale["error"] == "stale_generation"
            return "lease+takeover+v168_sck_denied verified"
        finally:
            _stop_ephemeral_server(srv, th)

    def check_7_passkeys_alipay_iv_agent_mcp_miniapps() -> str:
        srv, th, base, _ = _start_ephemeral_server()
        try:
            _, reg = _json_call(
                base,
                "POST",
                "/api/v1/auth/register",
                {"username": "plat_u", "password": "Password1!"},
            )
            tok = reg["token"]
            st_pr, _ = _json_call(
                base,
                "POST",
                "/api/v1/auth/passkey/register",
                {"credential_id": "cred_1", "public_key": "pk_1"},
                bearer=tok,
            )
            _, pl = _json_call(base, "GET", "/api/v1/auth/passkey/list", bearer=tok)
            _, pc = _json_call(base, "POST", "/api/v1/auth/passkey/challenge", {}, bearer=tok)
            st_pv, pv = _json_call(
                base,
                "POST",
                "/api/v1/auth/passkey/verify",
                {"challenge": pc["challenge"], "credential_id": "cred_1"},
            )
            st_replay, _ = _json_call(
                base,
                "POST",
                "/api/v1/auth/passkey/verify",
                {"challenge": pc["challenge"], "credential_id": "cred_1"},
            )
            assert st_pr == 200 and pl["total"] == 1 and st_pv == 200 and pv["sign_count"] == 1
            assert st_replay == 400

            _, a_start = _json_call(base, "POST", "/api/v1/auth/alipay/start", {})
            st_ac, _ = _json_call(
                base,
                "POST",
                "/api/v1/auth/alipay/cancel",
                {"attempt_id": a_start["attempt_id"]},
            )
            st_ap, _ = _json_call(
                base, "GET", f"/api/v1/auth/alipay/poll/{a_start['attempt_id']}"
            )
            assert st_ac == 200 and st_ap == 409

            st_iv, hdr_iv, body_iv = _http_call(base, "GET", "/article/v168")
            assert st_iv == 200 and "text/html" in hdr_iv["content-type"] and b"v168" in body_iv

            st_ag, ag = _json_call(
                base, "POST", "/api/v1/agent/runs", {"prompt": "hello"}, bearer=tok
            )
            assert st_ag == 200 and ag["status"] == "completed"
            st_mcp_ok, _ = _json_call(
                base, "POST", "/api/v1/mcp/call", {"tool_name": "chats.list"}, bearer=tok
            )
            st_mcp_bad, _ = _json_call(base, "POST", "/api/v1/mcp/call", {}, bearer=tok)
            assert st_mcp_ok == 200 and st_mcp_bad == 400
            _json_call(
                base,
                "POST",
                "/__e2e__/seed",
                {"miniapps": [{"app_id": "global_dharma", "title": "全球法布施", "tos_accepted": False}]},
            )
            st_ma, ma = _json_call(base, "GET", "/api/v1/miniapps", bearer=tok)
            assert st_ma == 200 and ma["total"] == 1 and ma["miniapps"][0]["tos_accepted"] is False
            st_tos1, tos1 = _json_call(
                base, "POST", "/api/v1/miniapps/global_dharma/accept-tos", {}, bearer=tok
            )
            assert st_tos1 == 200 and tos1["tos_accepted"] is True
            st_ma2, ma2 = _json_call(base, "GET", "/api/v1/miniapps/global_dharma", bearer=tok)
            assert st_ma2 == 200 and ma2["tos_accepted"] is True
            st_tos2, tos2 = _json_call(
                base,
                "POST",
                "/api/v1/miniapps/global_dharma/tos",
                {"tos_accepted": False},
                bearer=tok,
            )
            assert st_tos2 == 200 and tos2["tos_accepted"] is False

            # Route parity checks: alipay_confirm custom token, .well-known, pair/init+accept, turn, tasks
            st_ac_seed, _ = _json_call(
                base,
                "POST",
                "/__e2e__/seed",
                {"alipay_confirm": {"attempt_id": "alipay_seeded_custom", "token": "tok_alipay_seeded_1"}},
            )
            assert st_ac_seed == 200
            st_ac_me, _ = _json_call(base, "GET", "/api/v1/auth/me", bearer="tok_alipay_seeded_1")
            assert st_ac_me == 200
            st_wk, wk = _json_call(base, "GET", "/.well-known/oauth-authorization-server")
            assert (
                st_wk == 200
                and wk["supported_mcp_versions"][0] == "2025-11-25"
                and wk["registration_endpoint"] == "/api/v1/mcp/oauth/register"
            )
            st_pi, pi = _json_call(base, "POST", "/api/v1/auth/pair/init", {})
            assert st_pi == 200 and pi["qr_url"].startswith("tg://login?token=")
            st_pa1, _ = _json_call(
                base, "POST", "/api/v1/auth/pair/accept", {"pair_token": pi["pair_token"]}, bearer=tok
            )
            st_pa2, _ = _json_call(
                base, "POST", "/api/v1/auth/pair/accept", {"pair_token": pi["pair_token"]}, bearer=tok
            )
            assert st_pa1 == 200 and st_pa2 == 400
            st_turn, turn = _json_call(base, "GET", "/api/v1/webrtc/turn_credentials", bearer=tok)
            assert st_turn == 200 and "urls" in turn and "username" in turn and "credential" in turn
            st_tdec, tdec = _json_call(
                base, "POST", "/api/v1/tasks/t1/approve", {"decision": "rejected"}, bearer=tok
            )
            assert st_tdec == 200 and tdec["approved"] is False and tdec["state"] == "cancelled"
            return "passkey+alipay+iv+agent+mcp+miniapps+parity verified"
        finally:
            _stop_ephemeral_server(srv, th)

    def check_8_e2e_control_seed_fault_offline_reset() -> str:
        srv, th, base, _ = _start_ephemeral_server()
        try:
            st_seed, seed = _json_call(
                base,
                "POST",
                "/__e2e__/seed",
                {
                    "users": [{"username": "e2e_u", "password": "Password1!"}],
                    "chats": [{"title": "Seeded Room"}],
                },
            )
            assert st_seed == 200 and len(seed["seeded"]["chats"]) == 1
            tok = seed["seeded"]["users"][0]["token"]
            _, st_obj = _json_call(base, "GET", "/__e2e__/state")
            assert st_obj["counts"]["chats"] == 1

            # Atomic pre-validation zero-poisoning on /__e2e__/fault (B-03)
            for bad_fault in (
                {"http_fault": {"path_prefix": "/api/v1/chats", "status": 500, "remaining_hits": "1"}},
                {"delay": {"path_prefix": "/api/v1/chats", "delay_ms": True}},
                {"http_fault": {"path_prefix": "/api/v1/chats", "status": 204}},
            ):
                st_bf, bf = _json_call(base, "POST", "/__e2e__/fault", bad_fault)
                assert st_bf == 400 and bf["error"] == "invalid_fault_payload"
            st_after_bf, after_bf = _json_call(base, "GET", "/__e2e__/state")
            assert st_after_bf == 200 and after_bf["active_fault_count"] == 0

            # Crash-free 400 on malformed numeric path/body parameters (B-03)
            st_bad_p, _ = _json_call(base, "GET", "/api/v1/chats/abc/messages", bearer=tok)
            assert st_bad_p == 400
            st_null_cid, _ = _json_call(
                base, "POST", "/api/v1/messages/send", {"chat_id": None, "text": "x"}, bearer=tok
            )
            assert st_null_cid == 400
            st_null_del, _ = _json_call(
                base, "POST", "/api/v1/messages/delete", {"message_ids": None}, bearer=tok
            )
            assert st_null_del == 400

            # 2FA None==None bypass & whitespace/UTF-8 byte length guards (B-03)
            st_p2fa, p2fa = _json_call(
                base,
                "POST",
                "/api/v1/auth/phone/verify",
                {"code": "123456", "require_2fa_account": True},
            )
            assert st_p2fa == 200 and p2fa["requires_2fa"] is True
            st_2fa_byp, _ = _json_call(
                base, "POST", "/api/v1/auth/2fa/verify", {"challenge_token": p2fa["challenge_token"]}
            )
            assert st_2fa_byp == 401
            st_2fa_ws, _ = _json_call(base, "POST", "/api/v1/auth/2fa", {"password": "   "}, bearer=tok)
            assert st_2fa_ws == 400
            st_2fa_cjk, _ = _json_call(
                base, "POST", "/api/v1/auth/2fa", {"password": "法" * 400}, bearer=tok
            )
            assert st_2fa_cjk == 400

            # Integer chat_type 0..=5 preservation & non-destructive GET invite (B-03)
            st_c4, c4 = _json_call(
                base, "POST", "/api/v1/chats", {"title": "Bot Chat", "chat_type": 4}, bearer=tok
            )
            assert st_c4 == 200 and c4["chat_type"] == 4
            c4_id = c4["chat_id"]
            c4_mc0 = c4["member_count"]
            st_inv, inv = _json_call(base, "POST", f"/api/v1/chats/{c4_id}/invites", {}, bearer=tok)
            assert st_inv == 200
            st_inv_get, inv_get = _json_call(
                base, "GET", f"/api/v1/chats/{c4_id}/invites/{inv['invite_id']}", bearer=tok
            )
            assert st_inv_get == 200 and inv_get["revoked"] is False
            st_join1, _ = _json_call(base, "POST", "/api/v1/chats/join", {"code": inv["code"]}, bearer=tok)
            assert st_join1 == 200
            _, c4_after = _json_call(base, "GET", f"/api/v1/chats/{c4_id}", bearer=tok)
            assert c4_after["member_count"] == c4_mc0 + 1
            st_inv_del, _ = _json_call(
                base, "DELETE", f"/api/v1/chats/{c4_id}/invites/{inv['invite_id']}", bearer=tok
            )
            assert st_inv_del == 200
            st_join2, _ = _json_call(base, "POST", "/api/v1/chats/join", {"code": inv["code"]}, bearer=tok)
            assert st_join2 == 400

            # Method-filtered fault + Retry-After header + delay non-blocking concurrency
            _json_call(
                base,
                "POST",
                "/__e2e__/fault",
                {
                    "http_fault": {
                        "path_prefix": "/api/v1/chats",
                        "method": "GET",
                        "status": 429,
                        "retry_after_ms": 2500,
                        "remaining_hits": 1,
                    },
                    "delay": {"path_prefix": "/api/v1/status", "delay_ms": 80},
                },
            )
            st_429, hdr_429, _ = _http_call(
                base, "GET", "/api/v1/chats", headers={"Authorization": f"Bearer {tok}"}
            )
            assert st_429 == 429 and hdr_429.get("retry-after") == "3"
            st_200, _ = _json_call(base, "GET", "/api/v1/chats", bearer=tok)
            assert st_200 == 200
            _json_call(base, "DELETE", "/__e2e__/fault")

            _json_call(base, "POST", "/__e2e__/fault", {"offline": True})
            disconnected = False
            try:
                _http_call(base, "GET", "/api/v1/chats", headers={"Authorization": f"Bearer {tok}"})
            except (
                urllib.error.URLError,
                http.client.RemoteDisconnected,
                http.client.HTTPException,
                ConnectionError,
                OSError,
            ):
                disconnected = True
            assert disconnected, "expected transport disconnect while offline=true"

            st_ctl, ctl = _json_call(base, "GET", "/__e2e__/state")
            assert st_ctl == 200 and ctl["offline"] is True
            _json_call(base, "POST", "/__e2e__/fault", {"offline": False})
            st_rec, _ = _json_call(base, "GET", "/api/v1/chats", bearer=tok)
            assert st_rec == 200

            st_rst, _ = _json_call(base, "POST", "/__e2e__/reset", {})
            assert st_rst == 200
            _, chats_after = _json_call(base, "GET", "/api/v1/chats")
            assert chats_after["chats"] == []
            return "seed+fault_preval+400+2fa+invite+chat_type+offline+reset verified"
        finally:
            _stop_ephemeral_server(srv, th)

    def check_9_fail_closed_404_unknown_and_siblings() -> str:
        srv, th, base, _ = _start_ephemeral_server()
        try:
            bad_paths = [
                "/api/v1/__unknown__",
                "/api/v1/auth/__unknown__",
                "/api/v1/user-vault/__unknown__",
                "/api/v1/computer_bogus/lease",
                "/api/v2/chats",
                "/article_evil",
                "/iv_evil",
                "/__e2e__/__unknown__",
            ]
            for p in bad_paths:
                st, resp = _json_call(base, "GET", p)
                assert st == 404 and resp["error"] == "not_found", f"expected 404 on {p}, got {st}"
            return f"verified {len(bad_paths)} fail-closed 404 routes"
        finally:
            _stop_ephemeral_server(srv, th)

    def check_10_thread_safety_concurrent_requests() -> str:
        srv, th, base, _ = _start_ephemeral_server()
        try:
            _, seed = _json_call(
                base,
                "POST",
                "/__e2e__/seed",
                {
                    "users": [{"username": "conc_u", "password": "Password1!"}],
                    "chats": [{"chat_id": 50, "title": "Concurrency Room"}],
                },
            )
            tok = seed["seeded"]["users"][0]["token"]
            errs: list[str] = []

            def worker(idx: int) -> None:
                try:
                    for j in range(4):
                        st_m, _ = _json_call(
                            base,
                            "POST",
                            "/api/v1/chats/50/messages",
                            {"text": f"t{idx}_m{j}"},
                            bearer=tok,
                        )
                        if st_m != 200:
                            errs.append(f"worker {idx} msg {j} status={st_m}")
                    st_s, _ = _json_call(base, "GET", "/__e2e__/state")
                    if st_s != 200:
                        errs.append(f"worker {idx} state status={st_s}")
                except BaseException as exc:
                    errs.append(f"worker {idx}: {exc}")

            threads = [threading.Thread(target=worker, args=(i,)) for i in range(8)]
            for t in threads:
                t.start()
            for t in threads:
                t.join(timeout=15.0)
            assert not errs, f"concurrent errors: {errs}"
            _, hist = _json_call(base, "GET", "/api/v1/chats/50/messages", bearer=tok)
            pts_list = sorted(m["pts"] for m in hist["messages"])
            assert pts_list == list(range(1, 33)), f"non-contiguous PTS: {pts_list}"
            return "8 threads x 4 messages = 32 contiguous PTS"
        finally:
            _stop_ephemeral_server(srv, th)

    def check_11_rfc6455_websocket() -> str:
        assert (
            compute_websocket_accept("dGhlIHNhbXBsZSBub25jZQ==")
            == "s3pPLMBiTxaQ9kYGzzhZRbK+xOo="
        )
        srv, th, base, port = _start_ephemeral_server()

        class _TestWsClient:
            def __init__(self) -> None:
                self.sock: socket.socket | None = None
                self.buf = bytearray()

            def connect(
                self,
                path: str,
                extra_headers: dict[str, str] | None = None,
                ws_key: str = "dGhlIHNhbXBsZSBub25jZQ==",
            ) -> tuple[int, dict[str, str]]:
                self.sock = socket.create_connection(("127.0.0.1", port), timeout=5.0)
                self.sock.settimeout(5.0)
                hdr_lines = [
                    f"GET {path} HTTP/1.1",
                    f"Host: 127.0.0.1:{port}",
                    "Upgrade: websocket",
                    "Connection: Upgrade",
                    f"Sec-WebSocket-Key: {ws_key}",
                    "Sec-WebSocket-Version: 13",
                ]
                for k, v in (extra_headers or {}).items():
                    hdr_lines.append(f"{k}: {v}")
                raw_req = ("\r\n".join(hdr_lines) + "\r\n\r\n").encode("ascii")
                self.sock.sendall(raw_req)
                while b"\r\n\r\n" not in self.buf:
                    chunk = self.sock.recv(4096)
                    if not chunk:
                        break
                    self.buf.extend(chunk)
                sep = self.buf.find(b"\r\n\r\n")
                assert sep != -1, f"missing WS HTTP response headers: {bytes(self.buf)!r}"
                head = bytes(self.buf[:sep]).decode("utf-8", errors="replace")
                del self.buf[: sep + 4]
                lines = head.splitlines()
                status = int(lines[0].split()[1])
                hdrs: dict[str, str] = {}
                for ln in lines[1:]:
                    if ":" in ln:
                        k, v = ln.split(":", 1)
                        hdrs[k.strip().lower()] = v.strip()
                return status, hdrs

            def send_frame(
                self,
                opcode: int,
                payload: bytes,
                *,
                fin: bool = True,
                masked: bool = True,
                split_delay_s: float = 0.0,
            ) -> None:
                assert self.sock is not None
                b0 = (0x80 if fin else 0x00) | (opcode & 0x0F)
                mask_bit = 0x80 if masked else 0x00
                n = len(payload)
                if n <= 125:
                    hdr = struct.pack("!BB", b0, mask_bit | n)
                elif n <= 65535:
                    hdr = struct.pack("!BBH", b0, mask_bit | 126, n)
                else:
                    hdr = struct.pack("!BBQ", b0, mask_bit | 127, n)
                if masked:
                    mask = b"\x37\xfa\x21\x3d"
                    masked_body = bytes(payload[i] ^ mask[i & 3] for i in range(n))
                    wire = hdr + mask + masked_body
                else:
                    wire = hdr + payload
                if split_delay_s > 0.0 and len(wire) > 1:
                    self.sock.sendall(wire[:1])
                    time.sleep(split_delay_s)
                    self.sock.sendall(wire[1:])
                else:
                    self.sock.sendall(wire)

            def _ensure(self, needed: int, timeout: float = 5.0) -> None:
                assert self.sock is not None
                self.sock.settimeout(timeout)
                while len(self.buf) < needed:
                    chunk = self.sock.recv(4096)
                    assert chunk, f"unexpected WS EOF waiting for {needed} bytes"
                    self.buf.extend(chunk)

            def recv_frame(self, timeout: float = 5.0) -> tuple[bool, int, bytes]:
                self._ensure(2, timeout=timeout)
                b0, b1 = self.buf[0], self.buf[1]
                fin = (b0 & 0x80) != 0
                opcode = b0 & 0x0F
                assert (b1 & 0x80) == 0, "server WS frames must be unmasked"
                len7 = b1 & 0x7F
                if len7 <= 125:
                    offset, plen = 2, len7
                elif len7 == 126:
                    self._ensure(4, timeout=timeout)
                    offset = 4
                    plen = struct.unpack("!H", self.buf[2:4])[0]
                else:
                    self._ensure(10, timeout=timeout)
                    offset = 10
                    plen = struct.unpack("!Q", self.buf[2:10])[0]
                self._ensure(offset + plen, timeout=timeout)
                payload = bytes(self.buf[offset : offset + plen])
                del self.buf[: offset + plen]
                return fin, opcode, payload

            def close(self) -> None:
                if self.sock is not None:
                    try:
                        self.sock.close()
                    except OSError:
                        pass
                    self.sock = None

        try:
            _, reg1 = _json_call(
                base, "POST", "/api/v1/auth/register", {"username": "ws_u1", "password": "Password1!"}
            )
            tok1, sid1 = reg1["token"], reg1["session_id"]

            # Seed push frame + verify 101 + text/binary echo + fragmentation + Ping/Pong + split 16-bit frame
            st_push, _ = _json_call(
                base, "POST", "/__e2e__/seed", {"ws_push_frames": ["pushed_v168_event"]}
            )
            assert st_push == 200
            c1 = _TestWsClient()
            try:
                st, hdrs = c1.connect(f"/api/v1/ws?token={tok1}")
                assert st == 101 and hdrs.get("sec-websocket-accept") == "s3pPLMBiTxaQ9kYGzzhZRbK+xOo="
                _, op_p, pl_p = c1.recv_frame()
                assert op_p == 0x1 and pl_p.decode("utf-8") == "pushed_v168_event"

                c1.send_frame(0x1, b"hello_ws_v168")
                _, op_t, pl_t = c1.recv_frame()
                assert op_t == 0x1 and json.loads(pl_t.decode("utf-8"))["echo"] == "hello_ws_v168"

                c1.send_frame(0x2, b"\x00\x01\xfe\xff")
                _, op_b, pl_b = c1.recv_frame()
                assert op_b == 0x2 and pl_b == b"\x00\x01\xfe\xff"

                # Fragmented text + interleaved Ping/Pong
                c1.send_frame(0x1, b"frag_", fin=False)
                c1.send_frame(0x9, b"ping42", fin=True)
                _, op_pong, pl_pong = c1.recv_frame()
                assert op_pong == 0xA and pl_pong == b"ping42"
                c1.send_frame(0x0, b"joined", fin=True)
                _, op_f, pl_f = c1.recv_frame()
                assert op_f == 0x1 and json.loads(pl_f.decode("utf-8"))["echo"] == "frag_joined"

                # Extended 16-bit payload split across > 50ms poll boundary
                big_msg = "A" * 300
                c1.send_frame(0x1, big_msg.encode("ascii"), split_delay_s=0.065)
                _, op_big, pl_big = c1.recv_frame()
                assert op_big == 0x1 and json.loads(pl_big.decode("utf-8"))["echo"] == big_msg

                c1.send_frame(0x8, struct.pack("!H", 1000))
                _, op_cl, pl_cl = c1.recv_frame()
                assert op_cl == 0x8 and struct.unpack("!H", pl_cl[:2])[0] == 1000
            finally:
                c1.close()

            # Unmasked client frame -> Close 1002
            c_unm = _TestWsClient()
            try:
                st, _ = c_unm.connect("/api/v1/ws")
                assert st == 101
                c_unm.send_frame(0x1, b"bad", masked=False)
                _, op_u, pl_u = c_unm.recv_frame()
                assert op_u == 0x8 and struct.unpack("!H", pl_u[:2])[0] == 1002
            finally:
                c_unm.close()

            # Invalid UTF-8 text frame -> Close 1007
            c_utf = _TestWsClient()
            try:
                st, _ = c_utf.connect("/api/v1/ws")
                assert st == 101
                c_utf.send_frame(0x1, b"\xff\xfe")
                _, op_i, pl_i = c_utf.recv_frame()
                assert op_i == 0x8 and struct.unpack("!H", pl_i[:2])[0] == 1007
            finally:
                c_utf.close()

            # Active session revocation -> Close 4001
            c_rev = _TestWsClient()
            try:
                st, _ = c_rev.connect(f"/api/v1/ws?token={tok1}")
                assert st == 101
                st_del, _ = _json_call(base, "DELETE", f"/api/v1/auth/sessions/{sid1}", bearer=tok1)
                assert st_del == 200
                _, op_r, pl_r = c_rev.recv_frame()
                assert op_r == 0x8 and struct.unpack("!H", pl_r[:2])[0] == 4001
            finally:
                c_rev.close()

            # Pre-upgrade revoked token (?token= and Authorization: Bearer) -> HTTP 401
            for req_path, req_hdrs in (
                (f"/api/v1/ws?token={tok1}", {}),
                ("/api/v1/ws", {"Authorization": f"Bearer {tok1}"}),
            ):
                c_pre = _TestWsClient()
                try:
                    st_pre, _ = c_pre.connect(req_path, extra_headers=req_hdrs)
                    assert st_pre == 401
                finally:
                    c_pre.close()

            # Active offline fault -> Close 1001
            _, reg2 = _json_call(
                base, "POST", "/api/v1/auth/register", {"username": "ws_u2", "password": "Password1!"}
            )
            tok2 = reg2["token"]
            c_off = _TestWsClient()
            try:
                st, _ = c_off.connect("/api/v1/ws", extra_headers={"Authorization": f"Bearer {tok2}"})
                assert st == 101
                _json_call(base, "POST", "/__e2e__/fault", {"offline": True})
                _, op_o, pl_o = c_off.recv_frame()
                assert op_o == 0x8 and struct.unpack("!H", pl_o[:2])[0] == 1001
            finally:
                c_off.close()
            _json_call(base, "DELETE", "/__e2e__/fault")

            # Pre-upgrade fault -> HTTP 503
            _json_call(
                base,
                "POST",
                "/__e2e__/fault",
                {"http_fault": {"path_prefix": "/api/v1/ws", "status": 503, "remaining_hits": 1}},
            )
            c_flt = _TestWsClient()
            try:
                st_f, _ = c_flt.connect("/api/v1/ws")
                assert st_f == 503
            finally:
                c_flt.close()

            # Bogus WS paths -> 404; /ws -> 101 + Close 1000
            for bogus_ws in ("/api/v1/ws_bogus", "/ws_bogus"):
                c_bog = _TestWsClient()
                try:
                    st_b, _ = c_bog.connect(bogus_ws)
                    assert st_b == 404
                finally:
                    c_bog.close()
            c_ws2 = _TestWsClient()
            try:
                st_w2, _ = c_ws2.connect("/ws")
                assert st_w2 == 101
                c_ws2.send_frame(0x8, struct.pack("!H", 1000))
                _, op_w2, pl_w2 = c_ws2.recv_frame()
                assert op_w2 == 0x8 and struct.unpack("!H", pl_w2[:2])[0] == 1000
            finally:
                c_ws2.close()

            return "rfc6455 ws 101+push+echo+frag+ping+1002+1007+4001+1001+401+503+404 verified"
        finally:
            _stop_ephemeral_server(srv, th)

    run_check("cli_atomic_files_healthz_and_sigterm", check_1_cli_atomic_and_sigterm)
    run_check("auth_split_key_v2_rotation_and_logout", check_2_auth_split_key_v2_rotation_logout)
    run_check("chats_messages_idempotency_pts_sync_and_cjk_search", check_3_chats_messages_pts_cjk_search)
    run_check("media_cas_chunked_206_416_and_token_revocation", check_4_media_cas_chunked_206_416_revocation)
    run_check("vault_subpath_percent_decode_and_x25519_device_sync", check_5_vault_and_device_sync)
    run_check("computer_lease_emergency_takeover_and_v168_nodes", check_6_computer_lease_and_v168_nodes)
    run_check("passkeys_alipay_instant_view_agent_mcp_miniapps", check_7_passkeys_alipay_iv_agent_mcp_miniapps)
    run_check("e2e_control_seed_state_fault_500_offline_and_reset", check_8_e2e_control_seed_fault_offline_reset)
    run_check("fail_closed_404_unknown_and_sibling_routes", check_9_fail_closed_404_unknown_and_siblings)
    run_check("thread_safety_concurrent_requests", check_10_thread_safety_concurrent_requests)
    run_check("rfc6455_websocket_handshake_echo_ping_revocation_and_404", check_11_rfc6455_websocket)

    passed = sum(1 for c in checks if c["ok"])
    report = {
        "ok": passed == len(checks),
        "total_checks": len(checks),
        "passed_checks": passed,
        "checks": checks,
    }
    if report_path:
        rp = pathlib.Path(report_path)
        atomic_write_text(rp, json.dumps(report, indent=2, sort_keys=True) + "\n")
    print(json.dumps(report, indent=2, sort_keys=True))
    return 0 if report["ok"] else 1


def main() -> int:
    parser = argparse.ArgumentParser(
        description="Fabushi iOS v1.6.8 Stateful Simulator Loopback HTTP/1.1 Server"
    )
    parser.add_argument("--host", default="127.0.0.1", help="Bind host (default 127.0.0.1)")
    parser.add_argument("--port", type=int, default=0, help="Bind port (0 = OS ephemeral)")
    parser.add_argument("--port-file", default="", help="Atomic output file for bound port")
    parser.add_argument("--url-file", default="", help="Atomic output file for bound URL")
    parser.add_argument(
        "--version",
        default=DEFAULT_SERVER_VERSION,
        help="Server version string (default 1.6.8)",
    )
    parser.add_argument("--quiet", action="store_true", help="Suppress access logs")
    parser.add_argument(
        "--self-test",
        action="store_true",
        help="Run all 11 deterministic self-test checks and exit",
    )
    parser.add_argument("--report", default="", help="Optional JSON report path for --self-test")
    args = parser.parse_args()

    if args.self_test:
        return run_self_test(report_path=args.report)

    stop_event = threading.Event()

    def _handle_signal(_signum: int, _frame: Any) -> None:
        stop_event.set()

    signal.signal(signal.SIGTERM, _handle_signal)
    signal.signal(signal.SIGINT, _handle_signal)

    state = LoopbackState(version=args.version)
    server = LoopbackHTTPServer(
        (args.host, args.port),
        LoopbackHandler,
        state=state,
        quiet=args.quiet,
        stop_event=stop_event,
    )
    bound_host, bound_port = server.server_address[:2]
    url = f"http://{bound_host}:{bound_port}"

    serve_thread = threading.Thread(
        target=server.serve_forever, kwargs={"poll_interval": 0.05}, daemon=True
    )
    serve_thread.start()

    try:
        if args.port_file:
            atomic_write_text(pathlib.Path(args.port_file), f"{bound_port}\n")
        if args.url_file:
            atomic_write_text(pathlib.Path(args.url_file), f"{url}\n")

        if not args.quiet:
            print(f"[loopback] listening on {url}", flush=True)

        while not stop_event.is_set():
            stop_event.wait(0.2)
    finally:
        stop_event.set()
        server.shutdown()
        serve_thread.join(timeout=5.0)
        server.server_close()

    return 0


if __name__ == "__main__":
    raise SystemExit(main())
