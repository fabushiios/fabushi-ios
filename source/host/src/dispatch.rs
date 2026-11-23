use serde::Deserialize;
use serde_json::{json, Value};

use crate::error::{make_error_json, make_success_json, HostError, HostResult};
use crate::handlers_agent::{
    handle_coding, handle_fabushi, handle_magpie, handle_mcp, handle_multi_agent,
};
use crate::handlers_core::{
    handle_actions, handle_auth, handle_generation, handle_proxy, handle_push, handle_session,
    handle_settings, handle_wallet,
};
use crate::handlers_messaging::{
    handle_chats, handle_contacts, handle_e2ee, handle_folders, handle_media, handle_messages,
    handle_outbox, handle_search, handle_stickers, handle_stories, handle_sync, handle_topics,
};
use crate::handlers_platform::{
    handle_calls, handle_computers, handle_miniapp, handle_remote, handle_tailnet, handle_talk,
    handle_vault, handle_webmcp,
};
use crate::host::FabushiAppHost;

#[derive(Debug, Deserialize)]
struct RequestEnvelope {
    #[serde(default)]
    pub id: Option<Value>,
    #[serde(alias = "command")]
    pub method: Option<String>,
    #[serde(default)]
    pub generation: Option<u64>,
    #[serde(default)]
    #[allow(dead_code)]
    pub account_id: Option<String>,
    #[serde(default)]
    pub params: Option<Value>,
}

fn extract_request_id(id_opt: Option<&Value>) -> String {
    match id_opt {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Number(n)) => n.to_string(),
        _ => String::new(),
    }
}

fn normalize_legacy_method(raw: &str) -> (&str, &str) {
    match raw {
        "telegram.ping" | "ping" => ("session", "ping"),
        "telegram.getChats" | "chat.getChats" => ("chats", "list"),
        "telegram.getMessages" | "chat.getMessages" => ("messages", "localList"),
        "telegram.sendMessage" | "chat.sendMessage" => ("messages", "send"),
        "telegram.search" => ("search", "localMessages"),
        "telegram.syncDifference" => ("sync", "getDifference"),
        "telegram.media.encodeWaveform" => ("media", "encodeWaveform"),
        "telegram.media.decodeWaveform" => ("media", "decodeWaveform"),
        "telegram.uploadMediaChunk" => ("media", "chunkedSessionReceivePart"),
        "telegram.finalizeMediaUpload" => ("media", "chunkedSessionFinalize"),
        "telegram.downloadMedia" => ("media", "download"),
        "telegram.agent.listTasks" | "agent.listTasks" => ("fabushi", "listTasks"),
        "telegram.agent.createTask" | "agent.createTask" => ("fabushi", "createTask"),
        "telegram.agent.decideApproval" | "agent.decideApproval" => ("fabushi", "approvalDecide"),
        "telegram.generation.current" => ("generation", "current"),
        "telegram.generation.bump" => ("generation", "bump"),
        "getHostSettings" => ("settings", "localAll"),
        "setHostSettings" => ("settings", "localSet"),
        other => match other.split_once('.') {
            Some((ns, act)) => (ns, act),
            None => (other, ""),
        },
    }
}

fn dispatch_inner(
    host: &FabushiAppHost,
    req_id: &str,
    start_gen: u64,
    method: &str,
    params: &Value,
) -> HostResult<(Value, bool)> {
    if matches!(
        method,
        "telegram.proto.getConfig" | "proto.getConfig" | "telegram.proto.maxPacketSize"
    ) {
        return Ok((
            json!({
                "max_packet_size": 16 * 1024 * 1024,
                "current_layer": 188,
                "min_layer": 18,
            }),
            false,
        ));
    }

    let (namespace, action) = normalize_legacy_method(method);
    if action.is_empty() {
        return Err(HostError::unknown_command(method));
    }

    match namespace {
        "auth" => handle_auth(host, req_id, start_gen, action, params),
        "vault" => handle_vault(host, req_id, action, params).map(|v| (v, false)),
        "session" => handle_session(host, req_id, start_gen, action, params),
        "chats" => handle_chats(host, req_id, action, params).map(|v| (v, false)),
        "folders" => handle_folders(host, req_id, action, params).map(|v| (v, false)),
        "topics" => handle_topics(host, req_id, action, params).map(|v| (v, false)),
        "contacts" => handle_contacts(host, req_id, action, params).map(|v| (v, false)),
        "messages" => handle_messages(host, req_id, action, params).map(|v| (v, false)),
        "sync" => handle_sync(host, req_id, action, params).map(|v| (v, false)),
        "outbox" => handle_outbox(host, req_id, action, params).map(|v| (v, false)),
        "e2ee" => handle_e2ee(host, req_id, action, params).map(|v| (v, false)),
        "search" => handle_search(host, req_id, action, params).map(|v| (v, false)),
        "media" => handle_media(host, req_id, action, params).map(|v| (v, false)),
        "stories" => handle_stories(host, req_id, action, params).map(|v| (v, false)),
        "stickers" => handle_stickers(host, req_id, action, params).map(|v| (v, false)),
        "fabushi" => handle_fabushi(host, req_id, action, params).map(|v| (v, false)),
        "multi_agent" => handle_multi_agent(host, req_id, action, params).map(|v| (v, false)),
        "magpie" => handle_magpie(host, req_id, action, params).map(|v| (v, false)),
        "coding" => handle_coding(host, req_id, action, params).map(|v| (v, false)),
        "mcp" => handle_mcp(host, req_id, action, params).map(|v| (v, false)),
        "computers" => handle_computers(host, req_id, action, params).map(|v| (v, false)),
        "remote" => handle_remote(host, req_id, action, params).map(|v| (v, false)),
        "tailnet" => handle_tailnet(host, req_id, action, params).map(|v| (v, false)),
        "miniapp" => handle_miniapp(host, req_id, action, params).map(|v| (v, false)),
        "webmcp" => handle_webmcp(host, req_id, action, params).map(|v| (v, false)),
        "calls" => handle_calls(host, req_id, action, params).map(|v| (v, false)),
        "talk" => handle_talk(host, req_id, action, params).map(|v| (v, false)),
        "wallet" => handle_wallet(host, req_id, action, params).map(|v| (v, false)),
        "settings" => handle_settings(host, req_id, action, params),
        "proxy" => handle_proxy(host, req_id, action, params).map(|v| (v, false)),
        "push" => handle_push(host, req_id, action, params).map(|v| (v, false)),
        "actions" => handle_actions(host, req_id, action, params).map(|v| (v, false)),
        "generation" => handle_generation(host, req_id, action, params).map(|v| (v, true)),
        _ => Err(HostError::unknown_command(method)),
    }
}

pub fn route_command(host: &FabushiAppHost, request_json: &str) -> String {
    let current_gen = host.current_generation();
    let req: RequestEnvelope = match serde_json::from_str(request_json) {
        Ok(r) => r,
        Err(e) => {
            return make_error_json(
                "",
                current_gen,
                &HostError::invalid_json(format!("JSON parse error: {e}")),
            );
        }
    };

    let req_id = extract_request_id(req.id.as_ref());
    let method = match req.method.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        Some(m) => m,
        None => {
            return make_error_json(
                &req_id,
                current_gen,
                &HostError::invalid_arguments("Request missing 'method' field"),
            );
        }
    };

    // Panic isolation diagnostic strictly gated to test_mode handles.
    if method == "debug.trigger_panic" {
        if host.test_mode {
            panic!("Intentional test panic for catch_unwind verification");
        }
        return make_error_json(&req_id, current_gen, &HostError::unknown_command(method));
    }

    // Generation fencing: reject any stale generation both BEFORE and AFTER dispatch_inner
    // so an in-flight Account 1 RPC (including conditional auth polls that did NOT establish a
    // session) can never return stamped with Account 2's newer generation.
    let start_gen = host.current_generation();
    let (ns, act) = normalize_legacy_method(method);
    let exempt_from_fence = ns == "generation" || (ns == "session" && act == "ping");
    if !exempt_from_fence {
        if let Some(requested_gen) = req.generation {
            if requested_gen < start_gen {
                return make_error_json(
                    &req_id,
                    start_gen,
                    &HostError::stale_generation(requested_gen, start_gen),
                );
            }
        }
    }

    let params = req.params.unwrap_or(Value::Null);
    let outcome = dispatch_inner(host, &req_id, start_gen, method, &params);
    let end_gen = host.current_generation();
    match outcome {
        Ok((result, generation_advanced)) => {
            if !exempt_from_fence && !generation_advanced {
                let effective_req_gen = req.generation.unwrap_or(start_gen);
                if end_gen != start_gen || effective_req_gen < end_gen {
                    return make_error_json(
                        &req_id,
                        end_gen,
                        &HostError::stale_generation(effective_req_gen, end_gen),
                    );
                }
            }
            make_success_json(&req_id, end_gen, &result)
        }
        Err(err) => {
            let is_gen_control = matches!(
                (ns, act),
                ("auth", "logout")
                    | ("session", "clearSession" | "switchAccount" | "setServerUrl")
                    | ("settings", "serverUrlSet")
            );
            if !exempt_from_fence
                && !is_gen_control
                && !matches!(
                    err.code.as_str(),
                    "stale_generation" | "cancelled" | "host_closed"
                )
            {
                let effective_req_gen = req.generation.unwrap_or(start_gen);
                if end_gen != start_gen || effective_req_gen < end_gen {
                    return make_error_json(
                        &req_id,
                        end_gen,
                        &HostError::stale_generation(effective_req_gen, end_gen),
                    );
                }
            }
            make_error_json(&req_id, end_gen, &err)
        }
    }
}
