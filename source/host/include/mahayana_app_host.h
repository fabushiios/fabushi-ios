#ifndef MAHAYANA_APP_HOST_H
#define MAHAYANA_APP_HOST_H

#include <stddef.h>
#include <stdint.h>
#include <stdbool.h>

#ifdef __cplusplus
extern "C" {
#endif

/* ============================================================================
 * 1. Canonical Fabushi iOS Host C-ABI Exports (Primary v1.6.7 API)
 * ============================================================================ */

/**
 * Creates a canonical Fabushi App Host runtime from a JSON configuration string.
 *
 * Config JSON fields (all optional; NULL or empty string uses safe defaults):
 *   - "app_data_dir" / "storage_dir": sandbox directory for SQLite WAL, CAS, Tantivy, Vault
 *   - "storage_passphrase": optional storage passphrase
 *   - "server_url": backend server base URL (defaults to http://34.46.77.133:8080)
 *   - "http_timeout_secs": HTTP timeout seconds (default 10)
 *   - "test_mode": boolean enabling test-only diagnostics (default false)
 *   - "ios_sandbox_strict": boolean enforcing fail-closed iOS sandbox capability policy
 *
 * @param config_json  Optional NUL-terminated UTF-8 JSON object string, or NULL.
 * @return             Opaque host handle on success, or NULL on error (inspect fabushi_host_last_error).
 */
void *fabushi_host_create(const char *config_json);

/**
 * Synchronously dispatches a fail-closed JSON-RPC request to the Fabushi App Host.
 *
 * Request envelope:
 *   {"id":"<uuid>","method":"<namespace>.<action>","generation":<u64?>,"account_id":"<id?>","params":{...}}
 *   (Also accepts legacy "command" alias for "method".)
 *
 * Response envelope:
 *   Success: {"id":"<uuid>","ok":true,"generation":<u64>,"result":{...}}
 *   Error:   {"id":"<uuid>","ok":false,"generation":<u64>,"error":{"code":"...","message":"...","retryable":<bool>}}
 *
 * @param handle        Live opaque handle returned by fabushi_host_create (or legacy create aliases).
 * @param request_json  NUL-terminated UTF-8 JSON request string.
 * @return              Newly allocated NUL-terminated UTF-8 JSON response string.
 *                      Caller MUST free with fabushi_host_free_string.
 */
char *fabushi_host_dispatch(void *handle, const char *request_json);

/**
 * Polls up to `max_count` queued domain/WebSocket/outbox events, blocking up to `timeout_ms`
 * milliseconds until at least one event arrives or the timeout expires.
 *
 * Response envelope:
 *   {"id":"","ok":true,"generation":<u64>,"result":{"events":[...]}}
 *
 * @param handle      Live opaque host handle.
 * @param max_count   Maximum number of events to drain in one call (0 defaults to 64).
 * @param timeout_ms  Maximum wait time in milliseconds (0 performs a non-blocking poll).
 * @return            Newly allocated NUL-terminated UTF-8 JSON response string.
 *                    Caller MUST free with fabushi_host_free_string.
 */
char *fabushi_host_poll_events(void *handle, uint32_t max_count, uint32_t timeout_ms);

/**
 * Requests cooperative cancellation of an in-flight RPC identified by `request_id`.
 *
 * @param handle      Live opaque host handle.
 * @param request_id  NUL-terminated UTF-8 request ID string matching the in-flight request's "id".
 * @return            true if an active in-flight request matched and was cancelled; false otherwise.
 */
bool fabushi_host_cancel(void *handle, const char *request_id);

/**
 * Destroys a Fabushi App Host instance, stops background pumps, checkpoints state, and frees memory.
 *
 * @param handle  Opaque host handle, or NULL (safe no-op).
 */
void fabushi_host_destroy(void *handle);

/**
 * Frees a UTF-8 JSON string returned by any host C-ABI entrypoint.
 *
 * @param ptr  Pointer returned by dispatch/poll/compat functions, or NULL (safe no-op).
 */
void fabushi_host_free_string(char *ptr);

/**
 * Returns the last initialization error recorded on the calling thread, or NULL if none.
 * Returned pointer is borrowed; caller must copy immediately and MUST NOT free it.
 */
const char *fabushi_host_last_error(void);

/* ============================================================================
 * 2. C Callback Function Pointer & Legacy Telegram Bridge Aliases
 *    (Routed strictly to the exact same canonical FabushiAppHost engine)
 * ============================================================================ */

typedef void (*TelegramBridgeEventCallback)(
    const char *event_type,
    const char *payload_json,
    void *user_context
);

void *telegram_app_host_create(
    const char *app_data_dir,
    const char *storage_passphrase
);

void *telegram_app_host_create_test(
    const char *app_data_dir
);

const char *telegram_app_host_last_error(void);

char *telegram_app_host_dispatch(
    void *host,
    const char *request_json
);

char *telegram_app_host_dispatch_with_handle(
    void *host,
    const char *request_json
);

/**
 * Registers or unregisters a push event callback.
 * Pass NULL for `callback` to unregister safely without undefined behavior.
 */
void telegram_bridge_register_event_callback(
    void *host,
    TelegramBridgeEventCallback callback,
    void *user_context
);

void telegram_app_host_free_string(char *pointer);

void telegram_app_host_destroy(void *host);

/* ============================================================================
 * 3. Legacy Mahayana Compatibility Aliases
 *    (Routed strictly to the exact same canonical FabushiAppHost engine)
 * ============================================================================ */

void *mahayana_app_host_create(const char *app_data_dir);
void *mahayana_app_host_create_with_storage_passphrase(const char *app_data_dir, const char *storage_passphrase);
void *mahayana_app_host_create_test(const char *app_data_dir);
const char *mahayana_app_host_last_error(void);
char *mahayana_app_host_dispatch_with_handle(void *host, const char *request_json);
char *mahayana_app_host_dispatch(const char *request_json);
void mahayana_app_host_free_string(char *pointer);
void mahayana_app_host_destroy(void *host);
char *mahayana_cloud_agent_trace_rows(const unsigned char *bytes, size_t len);
char *mahayana_spreadsheet_parse_file(const char *path, size_t max_bytes, size_t max_rows);

/* ============================================================================
 * 4. Sliding Replay Window & Protocol Limit C-ABI Exports (E2EE)
 * ============================================================================ */

void *telegram_replay_window_create(void);
bool telegram_replay_window_check_and_add(void *window, uint64_t seq);
void telegram_replay_window_destroy(void *window);
size_t telegram_proto_max_packet_size(void);

#ifdef __cplusplus
}
#endif

#endif /* MAHAYANA_APP_HOST_H */
