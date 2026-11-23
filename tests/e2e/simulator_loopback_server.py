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
from typing import Any

MAX_HTTP_BODY_BYTES = 32 * 1024 * 1024
MAX_HTTP_TRAILER_LINES = 64
MAX_UPLOAD_TOTAL_SIZE = 4 * 1024 * 1024 * 1024
MAX_UPLOAD_PART_SIZE = 16 * 1024 * 1024
MAX_RECORDED_BODY_PREVIEW = 16 * 1024
DEFAULT_SERVER_VERSION = "1.6.8"
DEVICE_SYNC_CODE_DOMAIN_V1 = b"fabushi.vault.device_sync.code.v1"

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
    h.update(b"|")
    h.update(request_id.encode("utf-8"))
    h.update(b"|")
    h.update(ephemeral_pub_b64.encode("utf-8"))
    digest = h.digest()
    val = int.from_bytes(digest[:4], "big") % 1_000_000
    return f"{val:06d}"


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
    try:
        if not left and right:
            suffix = int(right)
            if suffix <= 0:
                return False, 0, 0
            start = max(0, total_len - suffix)
            return True, start, total_len - 1
        start = int(left)
        end = total_len - 1 if not right else int(right)
    except ValueError:
        return False, 0, 0
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
                "auth_version": 2,
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
    return {
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


def _poll_alipay_attempt(
    state: LoopbackState, attempt_id: str, ts: int
) -> tuple[int, dict[str, Any]]:
    with state.lock:
        rec = state.alipay_attempts.get(attempt_id)
        if rec is None:
            return 404, {"error": "oauth_attempt_not_found"}
        if rec["status"] == "cancelled":
            return 409, {"error": "oauth_cancelled", "status": "cancelled"}
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
            seeded_users: list[dict[str, Any]] = []
            seeded_chats: list[dict[str, Any]] = []
            seeded_messages: list[dict[str, Any]] = []
            seeded_media_ids: list[str] = []
            seeded_vault_count = 0
            seeded_node_ids: list[str] = []
            seeded_miniapp_ids: list[str] = []

            try:
                for u_spec in body.get("users", []):
                    if not isinstance(u_spec, dict):
                        raise ValueError("users[] entries must be objects")
                    uname = _str_nonempty_trim(u_spec.get("username"))
                    if not uname:
                        raise ValueError("users[].username must be non-empty")
                    uid = state.username_to_id.get(uname)
                    if uid is None:
                        state.next_user_id += 1
                        uid = state.next_user_id
                    first_name = _str_nonempty_trim(u_spec.get("first_name")) or uname
                    auth_version = int(u_spec.get("auth_version", 2))
                    auth_salt_b64 = _str_nonempty_trim(
                        u_spec.get("auth_salt_b64")
                    ) or base64.b64encode(bytes([0x42] * 16)).decode("ascii")
                    auth_verifier_b64 = _str_nonempty_trim(u_spec.get("auth_verifier_b64"))
                    password = (
                        None
                        if auth_verifier_b64 is not None
                        else _str_nonblank_raw(u_spec.get("password"))
                    )
                    user = {
                        "user_id": uid,
                        "username": uname,
                        "first_name": first_name,
                        "auth_version": auth_version,
                        "key_version": int(u_spec.get("key_version", 1)),
                        "password": password,
                        "auth_salt_b64": auth_salt_b64,
                        "auth_verifier_b64": auth_verifier_b64,
                        "kdf_params": u_spec.get(
                            "kdf_params",
                            {"m_cost_kib": 65536, "t_cost": 3, "p_cost": 4},
                        ),
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
                    }
                    state.username_to_id[uname] = uid
                    state.users_by_id[uid] = user
                    sid = None
                    tok = None
                    if u_spec.get("mint_session", True):
                        dev_name = (
                            _str_nonempty_trim(u_spec.get("device_name"))
                            or "iOS Simulator"
                        )
                        sess = state.mint_session(uid, dev_name, ts)
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

                for c_spec in body.get("chats", []):
                    if not isinstance(c_spec, dict):
                        raise ValueError("chats[] entries must be objects")
                    title = _str_nonempty_trim(c_spec.get("title"))
                    if not title:
                        raise ValueError("chats[].title must be non-empty")
                    if c_spec.get("chat_id") is not None:
                        cid = int(c_spec["chat_id"])
                        if cid >= state.next_chat_id:
                            state.next_chat_id = cid + 1
                    else:
                        state.next_chat_id += 1
                        cid = state.next_chat_id
                    chat_type = _str_nonempty_trim(c_spec.get("chat_type")) or "group"
                    member_count = int(c_spec.get("member_count", 2))
                    state.chats[cid] = {
                        "id": cid,
                        "chat_id": cid,
                        "title": title,
                        "chat_type": chat_type,
                        "member_count": member_count,
                        "is_group": chat_type in ("group", "supergroup", "channel"),
                    }
                    state.chat_messages.setdefault(cid, [])
                    state.chat_pts.setdefault(cid, 0)
                    seeded_chats.append({"chat_id": cid, "title": title})

                for m_spec in body.get("messages", []):
                    if not isinstance(m_spec, dict):
                        raise ValueError("messages[] entries must be objects")
                    cid = int(m_spec.get("chat_id", 1))
                    text = m_spec.get("text")
                    if text is None:
                        text = m_spec.get("content", "")
                    if not isinstance(text, str):
                        raise ValueError("messages[].text must be a string")
                    mid = state.next_msg_id
                    state.next_msg_id += 1
                    pts = state.chat_pts.get(cid, 0) + 1
                    state.chat_pts[cid] = pts
                    msg = {
                        "message_id": mid,
                        "chat_id": cid,
                        "pts": pts,
                        "text": text,
                        "client_nonce": m_spec.get("client_nonce"),
                        "sender_id": int(
                            m_spec.get("sender_id", state.default_user_id())
                        ),
                        "timestamp_ms": ts,
                    }
                    state.chat_messages.setdefault(cid, []).append(msg)
                    seeded_messages.append(
                        {"message_id": mid, "chat_id": cid, "pts": pts}
                    )

                for b_spec in body.get("media_blobs", []):
                    if not isinstance(b_spec, dict):
                        raise ValueError("media_blobs[] entries must be objects")
                    fid = _str_nonempty_trim(b_spec.get("file_id"))
                    data_b64 = b_spec.get("data_b64")
                    if not fid or not isinstance(data_b64, str):
                        raise ValueError(
                            "media_blobs[] requires file_id and base64 data_b64"
                        )
                    raw = base64.b64decode(data_b64.encode("ascii"))
                    state.media_blobs[fid] = raw
                    seeded_media_ids.append(fid)

                for v_spec in body.get("vault_items", []):
                    if not isinstance(v_spec, dict):
                        raise ValueError("vault_items[] entries must be objects")
                    uid = int(v_spec.get("user_id", state.default_user_id()))
                    cat = _str_nonempty_trim(v_spec.get("category")) or "general"
                    key = _str_nonempty_trim(v_spec.get("item_key"))
                    if not key:
                        raise ValueError("vault_items[].item_key must be non-empty")
                    state.vault_items[(uid, cat, key)] = dict(v_spec)
                    seeded_vault_count += 1

                if "journal_floors" in body:
                    jf = body["journal_floors"]
                    if not isinstance(jf, dict):
                        raise ValueError("journal_floors must be an object")
                    for k, v in jf.items():
                        state.journal_floors[int(k)] = int(v)

                for n_spec in body.get("remote_nodes", []):
                    if not isinstance(n_spec, dict):
                        raise ValueError("remote_nodes[] entries must be objects")
                    did = _str_nonempty_trim(n_spec.get("device_id"))
                    if not did:
                        raise ValueError("remote_nodes[].device_id must be non-empty")
                    node = {
                        "device_id": did,
                        "name": _str_nonempty_trim(n_spec.get("name")) or did,
                        "online": bool(n_spec.get("online", True)),
                        "is_current_device": bool(
                            n_spec.get("is_current_device", False)
                        ),
                        "screen_capture_permission": str(
                            n_spec.get("screen_capture_permission", "granted_active")
                        ),
                        "accessibility_drag_permission": str(
                            n_spec.get("accessibility_drag_permission", "granted_active")
                        ),
                    }
                    state.remote_nodes[did] = node
                    seeded_node_ids.append(did)

                for app_spec in body.get("miniapps", []):
                    if not isinstance(app_spec, dict):
                        raise ValueError("miniapps[] entries must be objects")
                    aid = _str_nonempty_trim(app_spec.get("app_id"))
                    if not aid:
                        raise ValueError("miniapps[].app_id must be non-empty")
                    state.miniapps[aid] = {
                        "app_id": aid,
                        "title": _str_nonempty_trim(app_spec.get("title")) or aid,
                        "tos_accepted": bool(app_spec.get("tos_accepted", False)),
                        "webmcp_enabled": bool(app_spec.get("webmcp_enabled", True)),
                    }
                    seeded_miniapp_ids.append(aid)

                if "alipay_confirm" in body and body["alipay_confirm"] is not None:
                    ac = body["alipay_confirm"]
                    if not isinstance(ac, dict):
                        raise ValueError("alipay_confirm must be an object")
                    aid = _str_nonempty_trim(ac.get("attempt_id"))
                    if not aid or aid not in state.alipay_attempts:
                        raise ValueError("alipay_confirm.attempt_id not found")
                    rec = state.alipay_attempts[aid]
                    uid = int(ac.get("user_id", state.default_user_id()))
                    sess = state.mint_session(uid, "iOS Alipay Seeded", ts)
                    rec["status"] = "confirmed"
                    rec["issued_session"] = {
                        "user_id": uid,
                        "session_id": ac.get("session_id") or sess["session_id"],
                        "token": ac.get("token") or sess["token"],
                        "expires_at": ms_to_rfc3339(sess["expires_at_ms"]),
                        "expires_at_ms": sess["expires_at_ms"],
                    }

                if "alipay_expire" in body and body["alipay_expire"] is not None:
                    exp_id = _str_nonempty_trim(body["alipay_expire"])
                    if not exp_id or exp_id not in state.alipay_attempts:
                        raise ValueError("alipay_expire attempt_id not found")
                    state.alipay_attempts[exp_id]["status"] = "expired"
                    state.alipay_attempts[exp_id]["expires_at_ms"] = ts - 1000
            except (ValueError, TypeError, binascii.Error) as exc:
                return 400, {
                    "ok": False,
                    "error": "invalid_seed_payload",
                    "message": str(exc),
                }

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
        with state.lock:
            try:
                if body.get("clear") is True:
                    state.faults.clear()
                    state.delays.clear()
                    state.offline = False
                if "offline" in body and body["offline"] is not None:
                    if not isinstance(body["offline"], bool):
                        raise ValueError("offline must be boolean")
                    state.offline = body["offline"]
                fault_entries: list[dict[str, Any]] = []
                if "http_fault" in body and body["http_fault"] is not None:
                    fault_entries.append(body["http_fault"])
                if "http_faults" in body and body["http_faults"] is not None:
                    if not isinstance(body["http_faults"], list):
                        raise ValueError("http_faults must be a list")
                    fault_entries.extend(body["http_faults"])
                for f in fault_entries:
                    if not isinstance(f, dict):
                        raise ValueError("http_fault entry must be an object")
                    prefix = _str_nonempty_trim(f.get("path_prefix"))
                    status = f.get("status")
                    if not prefix or not isinstance(status, int) or status < 100 or status > 599:
                        raise ValueError(
                            "http_fault requires non-empty path_prefix and valid integer status"
                        )
                    state.faults.append(
                        {
                            "path_prefix": prefix,
                            "method": _str_nonempty_trim(f.get("method")),
                            "status": status,
                            "body": f.get("body", {"error": f"injected_http_{status}"}),
                            "retry_after_ms": f.get("retry_after_ms"),
                            "remaining_hits": f.get("remaining_hits"),
                        }
                    )
                if "delay" in body and body["delay"] is not None:
                    d = body["delay"]
                    if not isinstance(d, dict):
                        raise ValueError("delay must be an object")
                    prefix = _str_nonempty_trim(d.get("path_prefix"))
                    delay_ms = d.get("delay_ms")
                    if not prefix or not isinstance(delay_ms, int) or delay_ms < 0:
                        raise ValueError(
                            "delay requires non-empty path_prefix and non-negative delay_ms"
                        )
                    state.delays[prefix] = delay_ms
            except (ValueError, TypeError) as exc:
                return 400, {
                    "ok": False,
                    "error": "invalid_fault_payload",
                    "message": str(exc),
                }

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
                "authorization_endpoint": "/api/v1/oauth/authorize",
                "token_endpoint": "/api/v1/oauth/token",
                "mcp_protocol_version": "2025-11-25",
                "code_challenge_methods_supported": ["S256"],
                "scopes_supported": [
                    "messages:read",
                    "messages:write",
                    "chats:read",
                    "chats:write",
                    "contacts:read",
                    "media:read",
                    "media:write",
                    "search:read",
                    "agent:run",
                    "mcp:tools",
                    "computers:read",
                    "computers:control",
                    "miniapp:read",
                    "miniapp:invoke",
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
                    return (
                        200,
                        {
                            "auth_version": user["auth_version"],
                            "key_version": user["key_version"],
                            "kdf_alg": (
                                "split_key_v2"
                                if user["auth_version"] >= 2
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
            state.next_user_id += 1
            uid = state.next_user_id
            first_name = _str_nonempty_trim(body.get("first_name")) or uname
            auth_version = int(body.get("auth_version", 2))
            auth_salt_b64 = _str_nonempty_trim(
                body.get("auth_salt_b64")
            ) or base64.b64encode(bytes([0x42] * 16)).decode("ascii")
            vault_envelope = body.get("vault_envelope")
            key_version = int(
                body.get(
                    "key_version",
                    (
                        vault_envelope.get("key_version", 1)
                        if isinstance(vault_envelope, dict)
                        else 1
                    ),
                )
            )
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
            state.next_user_id += 1
            uid = state.next_user_id
            uname = uname_opt or f"user_{uid}"
            auth_version = int(body.get("auth_version", 2))
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
            cur_pw = _str_nonblank_raw(body.get("current_password"))
            cur_v = _str_nonempty_trim(body.get("current_auth_verifier_b64"))
            if user["password"] is not None and cur_pw is not None and cur_pw != user["password"]:
                return 401, {"error": "invalid_credentials"}
            if (
                user["auth_verifier_b64"] is not None
                and cur_v is not None
                and cur_v != user["auth_verifier_b64"]
            ):
                return 401, {"error": "invalid_credentials"}
            new_salt = _str_nonempty_trim(
                body.get("new_auth_salt_b64") or body.get("auth_salt_b64")
            )
            new_v = _str_nonempty_trim(
                body.get("new_auth_verifier_b64") or body.get("auth_verifier_b64")
            )
            if new_salt:
                user["auth_salt_b64"] = new_salt
            if new_v:
                user["auth_verifier_b64"] = new_v
                user["password"] = None
            new_env = body.get("rewrapped_vault_envelope") or body.get("vault_envelope")
            if new_env is not None:
                user["vault_envelope"] = new_env
            user["auth_version"] = int(body.get("auth_version", 2))
            if body.get("key_version") is not None:
                user["key_version"] = int(body["key_version"])
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
            if new_salt:
                user["auth_salt_b64"] = new_salt
            if new_v is not None:
                user["auth_verifier_b64"] = new_v
                user["password"] = None
                user["auth_version"] = int(body.get("auth_version", 2))
            else:
                user["password"] = new_pw
                user["auth_verifier_b64"] = None
            new_env = body.get("rewrapped_vault_envelope") or body.get("vault_envelope")
            if new_env is not None:
                user["vault_envelope"] = new_env
            user["key_version"] = int(body.get("key_version", user["key_version"] + 1))

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
        new_pw = body.get("new_password") if "new_password" in body else body.get("password")
        if not isinstance(new_pw, str) or not new_pw or len(new_pw) > 1024:
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
        with state.lock:
            rec = state.two_factor_challenges.get(chal or "")
            if rec is None:
                return 401, {"error": "invalid_challenge"}
            if ts > rec["expires_at_ms"]:
                return 401, {"error": "challenge_expired"}
            user = state.ensure_user_by_id(rec["user_id"])
            if user["two_factor_password"] != pw:
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
            qr_url = f"fabushi://pair?token={tok}"
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
            if rec is None or ts > rec["expires_at_ms"]:
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
                if user["two_factor_password"] != pw:
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
        if not code or len(code) != 6 or not code.isdigit():
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
            uid = body.get("user_id")
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

            state.next_chat_id += 1
            cid = state.next_chat_id
            title = _str_nonempty_trim(body.get("title")) or f"Chat {cid}"
            chat_type = _str_nonempty_trim(body.get("chat_type")) or "group"
            raw_members = body.get("member_ids") or body.get("recipients") or []
            unique_members: set[str] = {str(uid)}
            if isinstance(raw_members, list):
                for m in raw_members:
                    unique_members.add(str(m))
            member_count = max(len(unique_members), 1)
            chat = {
                "id": cid,
                "chat_id": cid,
                "title": title,
                "chat_type": chat_type,
                "member_count": member_count,
                "creator_id": uid,
                "is_group": chat_type in ("group", "supergroup", "channel"),
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
            if inv is None or inv["revoked"] or (
                inv["max_uses"] is not None and inv["uses"] >= inv["max_uses"]
            ):
                return 404, {"error": "invalid_invite"}
            inv["uses"] += 1
            return 200, {"chat_id": inv["chat_id"], "joined": True}

    if path.startswith("/api/v1/chats/") and path.endswith("/invites"):
        cid_str = path[len("/api/v1/chats/") : -len("/invites")]
        try:
            cid = int(cid_str)
        except ValueError:
            return 404, {"error": "not_found"}
        with state.lock:
            if method == "POST":
                seq = state.alloc_seq()
                inv_id = f"inv_{cid}_{seq}"
                code = f"join_{cid}_{seq}"
                max_uses = body.get("max_uses") or body.get("usage_limit")
                inv = {
                    "invite_id": inv_id,
                    "chat_id": cid,
                    "code": code,
                    "url": f"https://fabushi.app/+{code}",
                    "max_uses": int(max_uses) if max_uses is not None else None,
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
        with state.lock:
            inv = state.chat_invites.get(inv_id)
            if inv is None:
                return 404, {"error": "invite_not_found"}
            inv["revoked"] = True
            return 200, {"revoked": True, "invite_id": inv["invite_id"]}

    if method == "GET" and path.startswith("/api/v1/chats/") and path.endswith("/topics"):
        cid_str = path[len("/api/v1/chats/") : -len("/topics")]
        try:
            cid = int(cid_str)
        except ValueError:
            cid = 1
        return 200, {"chat_id": cid, "topics": []}

    # Messages send/history/edit/delete/reaction/read
    if method == "POST" and (
        path == "/api/v1/messages/send"
        or (path.startswith("/api/v1/chats/") and path.endswith("/messages"))
    ):
        if path.startswith("/api/v1/chats/"):
            cid = int(path[len("/api/v1/chats/") : -len("/messages")])
        else:
            cid = int(body.get("chat_id", 1))
        text = body.get("text")
        if text is None:
            text = body.get("content", "")
        text_str = str(text)
        nonce = body.get("client_nonce")
        nonce_int = int(nonce) if isinstance(nonce, int) and not isinstance(nonce, bool) else None
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
            cid = int(path[len("/api/v1/chats/") : -len("/messages")])
        else:
            cid_s = extract_query_param(query, "chat_id")
            cid = int(cid_s) if cid_s and cid_s.lstrip("-").isdigit() else 1
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
        if cid_str.lstrip("-").isdigit():
            cid = int(cid_str)
            with state.lock:
                chat = state.chats.get(cid)
                if chat is None:
                    return 404, {"error": "chat_not_found"}
                return 200, dict(chat)

    if method == "POST" and path == "/api/v1/messages/edit":
        cid = int(body.get("chat_id", 1))
        mid = int(body.get("message_id", 0))
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
        cid = int(body.get("chat_id", 1))
        revoke = bool(body.get("revoke", False))
        ids = {int(x) for x in body.get("message_ids", [])}
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
        cid = int(body.get("chat_id", 1))
        with state.lock:
            pts = state.chat_pts.get(cid, 0) + 1
            state.chat_pts[cid] = pts
            return 200, {"ok": True, "success": True, "pts": pts, "status": "reaction_updated"}

    if method == "POST" and path == "/api/v1/messages/read":
        cid = int(body.get("chat_id", 1))
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
        cid = int(body.get("chat_id", int(cid_q) if cid_q and cid_q.lstrip("-").isdigit() else 1))
        req_pts = int(body.get("pts", int(pts_q) if pts_q and pts_q.isdigit() else 0))
        limit = max(1, int(body.get("limit", int(lim_q) if lim_q and lim_q.isdigit() else 100)))
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
        cid = int(cid_q) if cid_q and cid_q.lstrip("-").isdigit() else 1
        with state.lock:
            cur_pts = state.chat_pts.get(cid, 0)
            return 200, {"chat_id": cid, "pts": cur_pts, "current_pts": cur_pts, "seq": state.next_seq}

    # Media JSON upload control & Instant View
    if method == "POST" and path == "/api/v1/media/upload/init":
        total_size = int(body.get("total_size", body.get("size_bytes", 0)))
        part_size = int(body.get("part_size", body.get("chunk_size", 1024 * 1024)))
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
        req_gen = body.get("generation")
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
                if isinstance(req_gen, int) and req_gen < lease["generation"]:
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
                    "generation": int(req_gen) if isinstance(req_gen, int) else 1,
                    "lease_id": "lease_default",
                    "target_device_id": did,
                },
            )

    # MiniApps catalog
    if method == "GET" and path == "/api/v1/miniapps":
        with state.lock:
            apps = [dict(state.miniapps[k]) for k in sorted(state.miniapps.keys())]
            return 200, {"miniapps": apps, "total": len(apps)}

    # Search, WebRTC, Tasks, MCP, Talk, Magpie, Updates
    if method == "GET" and path == "/api/v1/search/messages":
        q = extract_query_param(query, "q") or ""
        cid_q = extract_query_param(query, "chat_id")
        cid_filter = int(cid_q) if cid_q and cid_q.lstrip("-").isdigit() else None
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
                "ice_servers": [{"urls": ["stun:127.0.0.1:3478"]}],
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
            approved = bool(body.get("approved", body.get("decision") != "deny"))
            task["approved"] = approved
            task["state"] = "active" if approved else "cancelled"
            return 200, dict(task)

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
    ) -> None:
        self.state = state
        self.quiet = quiet
        super().__init__(server_address, handler_cls)


class LoopbackHandler(http.server.BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"

    def handle_one_request(self) -> None:
        try:
            super().handle_one_request()
        except (BrokenPipeError, ConnectionResetError, OSError):
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

    def _read_request_body(self) -> bytes:
        expect = self.headers.get("Expect", "")
        if "100-continue" in expect.lower():
            self.wfile.write(b"HTTP/1.1 100 Continue\r\n\r\n")
            self.wfile.flush()

        te = self.headers.get("Transfer-Encoding", "").lower()
        if "chunked" in te:
            out = bytearray()
            while True:
                line = self.rfile.readline(65536)
                if not line:
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
                        tline = self.rfile.readline(65536)
                        if not tline or tline in (b"\r\n", b"\n"):
                            break
                        trailer_count += 1
                        if trailer_count > MAX_HTTP_TRAILER_LINES:
                            raise BadRequestError("too_many_trailers")
                    break
                if len(out) + chunk_size > MAX_HTTP_BODY_BYTES:
                    raise PayloadTooLargeError("chunked_body_too_large")
                chunk = self.rfile.read(chunk_size)
                if len(chunk) != chunk_size:
                    raise BadRequestError("truncated_chunk_data")
                out.extend(chunk)
                crlf = self.rfile.readline(4)
                if crlf not in (b"\r\n", b"\n"):
                    raise BadRequestError("missing_chunk_crlf")
            return bytes(out)

        cl_raw = self.headers.get("Content-Length")
        if cl_raw is None:
            return b""
        try:
            cl = int(cl_raw.strip())
        except ValueError as exc:
            raise BadRequestError("invalid_content_length") from exc
        if cl < 0:
            raise BadRequestError("negative_content_length")
        if cl > MAX_HTTP_BODY_BYTES:
            raise PayloadTooLargeError("body_too_large")
        if cl == 0:
            return b""
        data = self.rfile.read(cl)
        if len(data) != cl:
            raise BadRequestError("truncated_body")
        return data

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

    def _dispatch(self) -> None:
        assert isinstance(self.server, LoopbackHTTPServer)
        state = self.server.state
        method = self.command.upper()
        is_head = method == "HEAD"
        path, query = normalize_request_target(self.path)
        headers = {k.lower(): v for k, v in self.headers.items()}

        # Control plane `/__e2e__/*` is immune to offline, faults, and delays, and excluded from recorded
        if matches_segment_prefix(path, "/__e2e__"):
            try:
                body_bytes = self._read_request_body()
            except PayloadTooLargeError as exc:
                self._send_json(413, {"error": "payload_too_large", "message": str(exc)}, is_head=is_head)
                return
            except BadRequestError as exc:
                self._send_json(400, {"error": "bad_request", "message": str(exc)}, is_head=is_head)
                return
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

        try:
            body_bytes = self._read_request_body()
        except PayloadTooLargeError as exc:
            self._send_json(413, {"error": "payload_too_large", "message": str(exc)}, is_head=is_head)
            return
        except BadRequestError as exc:
            self._send_json(400, {"error": "bad_request", "message": str(exc)}, is_head=is_head)
            return

        state.record_request(method, path, query, headers, body_bytes)

        # Delay injection slept strictly OUTSIDE lock
        delay_ms = state.lookup_delay_ms(path)
        if delay_ms > 0:
            time.sleep(delay_ms / 1000.0)

        # Deterministic fault injection
        fault = state.pop_matching_fault(method, path)
        if fault is not None:
            extra: list[tuple[str, str]] = []
            if isinstance(fault.get("retry_after_ms"), int) and fault["retry_after_ms"] > 0:
                sec = (fault["retry_after_ms"] + 999) // 1000
                extra.append(("Retry-After", str(sec)))
            body_obj = fault.get("body")
            if not isinstance(body_obj, dict):
                body_obj = {"error": f"injected_http_{fault['status']}"}
            self._send_json(fault["status"], body_obj, extra_headers=extra, is_head=is_head)
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
            if not part_s.isdigit():
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
        self.close_connection = True
        self.wfile.write(b"HTTP/1.1 200 Connection Established\r\n\r\n")
        self.wfile.flush()


# ---------------------------------------------------------------------------
# Deterministic --self-test suite (10 checks, zero external deps, zero repo files)
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
            return "split_key_v2+rotation+logout verified"
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
            return f"media_id={mid}, 206/416/revoked=401"
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
            assert st_ma == 200 and ma["total"] == 1
            return "passkey+alipay+iv+agent+mcp+miniapps verified"
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

            _json_call(
                base,
                "POST",
                "/__e2e__/fault",
                {"http_fault": {"path_prefix": "/api/v1/chats", "status": 500, "remaining_hits": 1}},
            )
            st_500, _ = _json_call(base, "GET", "/api/v1/chats", bearer=tok)
            st_200, _ = _json_call(base, "GET", "/api/v1/chats", bearer=tok)
            assert st_500 == 500 and st_200 == 200

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
            return "seed+500+offline_disconnect+reset verified"
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
        help="Run all 10 deterministic self-test checks and exit",
    )
    parser.add_argument("--report", default="", help="Optional JSON report path for --self-test")
    args = parser.parse_args()

    if args.self_test:
        return run_self_test(report_path=args.report)

    state = LoopbackState(version=args.version)
    server = LoopbackHTTPServer((args.host, args.port), LoopbackHandler, state=state, quiet=args.quiet)
    bound_host, bound_port = server.server_address[:2]
    url = f"http://{bound_host}:{bound_port}"

    if args.port_file:
        atomic_write_text(pathlib.Path(args.port_file), f"{bound_port}\n")
    if args.url_file:
        atomic_write_text(pathlib.Path(args.url_file), f"{url}\n")

    if not args.quiet:
        print(f"[loopback] listening on {url}", flush=True)

    stop_event = threading.Event()

    def _handle_signal(_signum: int, _frame: Any) -> None:
        stop_event.set()

    signal.signal(signal.SIGTERM, _handle_signal)
    signal.signal(signal.SIGINT, _handle_signal)

    serve_thread = threading.Thread(
        target=server.serve_forever, kwargs={"poll_interval": 0.05}, daemon=True
    )
    serve_thread.start()

    try:
        while not stop_event.is_set():
            stop_event.wait(0.2)
    finally:
        server.shutdown()
        serve_thread.join(timeout=5.0)
        server.server_close()

    return 0


if __name__ == "__main__":
    raise SystemExit(main())
