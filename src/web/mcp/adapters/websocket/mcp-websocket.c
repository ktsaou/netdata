// SPDX-License-Identifier: GPL-3.0-or-later

#include "mcp-websocket.h"
#include "web/websocket/websocket-internal.h"
#include "web/mcp/core/mcp-job.h"      // For MCP_JSONRPC_ADAPTER_JOB, mcp_jsonrpc_adapter_job_create/free
#include "web/mcp/core/mcp-core.h"     // For mcp_execute_tool
#include "web/mcp/core/mcp-response.h" // For MCP_RESPONSE_BUFFER (used indirectly by mcp_execute_tool)
#include "libnetdata/json/json.h"      // For json_object manipulation, json_tokener_parse, etc.
#include "libnetdata/log/nd_log.h"       // For netdata_error, etc. (already used, but good to note)
#include "libnetdata/buffer/buffer.h"    // For buffer_tostring (already used)

// Store the MCP context in the WebSocket client's data field
void mcp_websocket_set_context(struct websocket_server_client *wsc, MCP_CLIENT *ctx) {
    if (!wsc) return;
    wsc->user_data = ctx;
}

// Get the MCP context from a WebSocket client
MCP_CLIENT *mcp_websocket_get_context(struct websocket_server_client *wsc) {
    if (!wsc) return NULL;
    return (MCP_CLIENT *)wsc->user_data;
}

// WebSocket buffer sender function for the MCP adapter
int mcp_websocket_send_buffer(struct websocket_server_client *wsc, BUFFER *buffer) {
    if (!wsc || !buffer) return -1;

    const char *text = buffer_tostring(buffer);
    if (!text || !*text) return -1;

    // Log the raw outgoing message
    netdata_log_debug(D_MCP, "SND: %s", text);

    return websocket_protocol_send_text(wsc, text);
}

// Create a response context for a WebSocket client
static MCP_CLIENT *mcp_websocket_create_context(struct websocket_server_client *wsc) {
    if (!wsc) return NULL;

    MCP_CLIENT *ctx = mcp_create_client(MCP_TRANSPORT_WEBSOCKET, wsc);
    if (ctx) {
        // Set pointer to the websocket client's user_auth
        ctx->user_auth = &wsc->user_auth;
    }
    mcp_websocket_set_context(wsc, ctx);

    return ctx;
}

// WebSocket connection handler for MCP
void mcp_websocket_on_connect(struct websocket_server_client *wsc) {
    if (!wsc) return;

    // Create the MCP context
    MCP_CLIENT *ctx = mcp_websocket_create_context(wsc);
    if (!ctx) {
        websocket_protocol_send_close(wsc, WS_CLOSE_INTERNAL_ERROR, "Failed to create MCP context");
        return;
    }

    websocket_debug(wsc, "MCP client connected");
}

// WebSocket message handler for MCP - receives message and routes to MCP
void mcp_websocket_on_message(struct websocket_server_client *wsc, const char *message, size_t length, WEBSOCKET_OPCODE opcode) {
    if (!wsc || !message || length == 0)
        return;

    // Log the raw incoming message
    netdata_log_debug(D_MCP, "RCV: %s", message);

    // Only handle text messages
    if (opcode != WS_OPCODE_TEXT) {
        websocket_error(wsc, "Ignoring binary message - mcp supports only TEXT messages");
        return;
    }

    // Silently ignore standalone "PING" messages (legacy MCP client behavior)
    if (length == 4 && strncmp(message, "PING", 4) == 0) {
        websocket_debug(wsc, "Ignoring legacy PING message");
        return;
    }

    // Get the MCP context
    MCP_CLIENT *mcpc = mcp_websocket_get_context(wsc);
    if (!mcpc) {
        websocket_error(wsc, "MCP context not found");
        return;
    }

    // Parse the JSON-RPC request
    struct json_object *request = NULL;
    enum json_tokener_error jerr = json_tokener_success;
    request = json_tokener_parse_verbose(message, &jerr);

    if (!request || jerr != json_tokener_success) {
        // Log the full error with payload for debugging
        websocket_error(wsc, "Failed to parse JSON-RPC request: %s | Payload (length=%zu): '%.*s'",
                        json_tokener_error_desc(jerr),
                        length,
                        (int)(length > 1000 ? 1000 : length), // Limit to 1000 chars in log
                        message);

        // Also log the hex dump of first few bytes to catch non-printable characters
        if (length > 0) {
            char hex_dump[256];
            size_t hex_len = 0;
            size_t bytes_to_dump = (length > 32) ? 32 : length;

            for (size_t i = 0; i < bytes_to_dump && hex_len < sizeof(hex_dump) - 6; i++) {
                hex_len += snprintf(hex_dump + hex_len, sizeof(hex_dump) - hex_len,
                                   "%02X ", (unsigned char)message[i]);
            }
            if (bytes_to_dump < length) {
                hex_len += snprintf(hex_dump + hex_len, sizeof(hex_dump) - hex_len, "...");
            }

            websocket_error(wsc, "First %zu bytes hex dump: %s", bytes_to_dump, hex_dump);
        }

        return;
    }

    // Pass the request to the MCP handler
    mcp_handle_request(mcpc, request);

    // Free the request object
    json_object_put(request);
}

// WebSocket close handler for MCP
void mcp_websocket_on_close(struct websocket_server_client *wsc, WEBSOCKET_CLOSE_CODE code, const char *reason) {
    if (!wsc) return;

    websocket_debug(wsc, "MCP client closing (code: %d, reason: %s)", code, reason ? reason : "none");

    // Clean up the MCP context
    MCP_CLIENT *ctx = mcp_websocket_get_context(wsc);
    if (ctx) {
        mcp_free_client(ctx);
        mcp_websocket_set_context(wsc, NULL);
    }
}

// WebSocket disconnect handler for MCP
void mcp_websocket_on_disconnect(struct websocket_server_client *wsc) {
    if (!wsc) return;

    websocket_debug(wsc, "MCP client disconnected");

    // Clean up the MCP context
    MCP_CLIENT *ctx = mcp_websocket_get_context(wsc);
    if (ctx) {
        mcp_free_client(ctx);
        mcp_websocket_set_context(wsc, NULL);
    }
}

// Register WebSocket callbacks for MCP
void mcp_websocket_adapter_initialize(void) {
    mcp_initialize_subsystem();
    // Ensure core MCP registry is also initialized if not done elsewhere centrally
    // mcp_tools_registry_init(); // This should be called once globally, e.g. in mcp_initialize_subsystem or daemon start
    netdata_log_info("MCP WebSocket adapter initialized");
}

// Helper to send JSON-RPC error response
static void mcp_websocket_send_jsonrpc_error(struct websocket_server_client *wsc, struct json_object *id, int code, const char *message) {
    if (!wsc) return;

    struct json_object *resp_json = json_object_new_object();
    if (!resp_json) return;

    json_object_object_add(resp_json, "jsonrpc", json_object_new_string("2.0"));
    if (id) { // For notifications (id == NULL), no id field in response, but errors are usually for requests.
        json_object_object_add(resp_json, "id", json_object_get(id)); // get new ref for resp_json
    } else {
        json_object_object_add(resp_json, "id", NULL); // JSON-RPC null id
    }

    struct json_object *error_obj = json_object_new_object();
    json_object_object_add(error_obj, "code", json_object_new_int(code));
    json_object_object_add(error_obj, "message", json_object_new_string(message));
    json_object_object_add(resp_json, "error", error_obj);

    const char *response_str = json_object_to_json_string_ext(resp_json, JSON_C_TO_STRING_PLAIN);
    if (response_str) {
        websocket_protocol_send_text(wsc, response_str);
    }
    json_object_put(resp_json); // a new ref was gotten for id, and new objects created
}

// WebSocket message handler for MCP - receives message and routes to MCP
void mcp_websocket_on_message(struct websocket_server_client *wsc, const char *message, size_t length, WEBSOCKET_OPCODE opcode) {
    if (!wsc || !message || length == 0) return;

    netdata_log_debug(D_MCP, "RCV (WebSocket): %s", message);

    if (opcode != WS_OPCODE_TEXT) {
        websocket_error(wsc, "Ignoring binary message - MCP via WebSocket supports only TEXT messages (JSON-RPC).");
        return;
    }

    MCP_CLIENT *mcpc = mcp_websocket_get_context(wsc); // mcpc is the old context, might be deprecated for jobs
    if (!mcpc) { // Should not happen if on_connect succeeded
        websocket_error(wsc, "MCP context not found for WebSocket client.");
        // No ID to send error with yet
        return;
    }

    enum json_tokener_error jerr;
    struct json_object *req_json = json_tokener_parse_verbose(message, &jerr);

    if (jerr != json_tokener_success) {
        char err_msg[128];
        snprintf(err_msg, sizeof(err_msg), "Parse error: %s", json_tokener_error_desc(jerr));
        mcp_websocket_send_jsonrpc_error(wsc, NULL, -32700, err_msg); // Parse error
        if (req_json) json_object_put(req_json); // tokener might return a partial object
        return;
    }
    if (!req_json || json_object_get_type(req_json) != json_type_object) {
        mcp_websocket_send_jsonrpc_error(wsc, NULL, -32600, "Invalid Request: not a JSON object.");
        if (req_json) json_object_put(req_json);
        return;
    }

    struct json_object *jsonrpc_id_obj = NULL;
    json_object_object_get_ex(req_json, "id", &jsonrpc_id_obj); // OK if id is missing (notification)

    struct json_object *method_obj = NULL;
    if (!json_object_object_get_ex(req_json, "method", &method_obj) || json_object_get_type(method_obj) != json_type_string) {
        mcp_websocket_send_jsonrpc_error(wsc, jsonrpc_id_obj, -32600, "Invalid Request: missing or invalid 'method'.");
        json_object_put(req_json);
        return;
    }
    const char *method_name_full = json_object_get_string(method_obj);

    struct json_object *params_obj = NULL;
    struct json_object *params_for_job = NULL;
    if (json_object_object_get_ex(req_json, "params", &params_obj)) {
        if (json_object_get_type(params_obj) != json_type_object && json_object_get_type(params_obj) != json_type_array) {
             // Support for array params could be added if tools need it, but generally object is better for named params.
            mcp_websocket_send_jsonrpc_error(wsc, jsonrpc_id_obj, -32602, "Invalid params: must be object or array.");
            json_object_put(req_json);
            return;
        }
        params_for_job = json_object_get(params_obj); // New reference for the job
    } else {
        params_for_job = json_object_new_object(); // New empty object for the job
    }


    if (strcmp(method_name_full, "ping") == 0) {
        struct json_object *pong_resp = json_object_new_object();
        json_object_object_add(pong_resp, "jsonrpc", json_object_new_string("2.0"));
        if (jsonrpc_id_obj) json_object_object_add(pong_resp, "id", json_object_get(jsonrpc_id_obj));
        else json_object_object_add(pong_resp, "id", NULL);
        json_object_object_add(pong_resp, "result", json_object_new_string("pong"));
        const char *pong_str = json_object_to_json_string_ext(pong_resp, JSON_C_TO_STRING_PLAIN);
        websocket_protocol_send_text(wsc, pong_str);
        json_object_put(pong_resp);
        json_object_put(req_json);
        json_object_put(params_for_job); // We created this ref, and it's not passed to a job
        return;
    }

    // Assuming tool_name is the method_name_full for now
    // In future, method_name_full might be "namespace/tool_name"
    const char *tool_name = method_name_full;

    MCP_JSONRPC_ADAPTER_JOB *jrpc_job = mcp_jsonrpc_adapter_job_create(mcpc, jsonrpc_id_obj, method_name_full, params_for_job, tool_name, &wsc->user_auth);

    if (!jrpc_job) {
        // mcp_jsonrpc_adapter_job_create is responsible for params_for_job if it fails.
        mcp_websocket_send_jsonrpc_error(wsc, jsonrpc_id_obj, -32000, "Server error: Failed to create job.");
        json_object_put(req_json);
        // params_for_job was handled by mcp_jsonrpc_adapter_job_create
        return;
    }

    mcp_execute_tool(jrpc_job->req_job);

    // Process response for non-notifications
    if (jrpc_job->jsonrpc_id_obj) { // Only send response if there was an ID.
        struct json_object *resp_json = json_object_new_object();
        json_object_object_add(resp_json, "jsonrpc", json_object_new_string("2.0"));
        json_object_object_add(resp_json, "id", json_object_get(jrpc_job->jsonrpc_id_obj)); // get new ref for resp_json

        if (jrpc_job->req_job->status_code >= 400 || (jrpc_job->req_job->response_buffers && buffer_strstr(jrpc_job->req_job->response_buffers->data, "\"error\""))) {
            // Error case
            if (jrpc_job->req_job->response_buffers && jrpc_job->req_job->response_buffers->data) {
                const char* error_buffer_str = buffer_tostring(jrpc_job->req_job->response_buffers->data);
                struct json_object *error_content_json = json_tokener_parse(error_buffer_str);
                if (error_content_json && json_object_object_get(error_content_json, "error")) {
                     json_object_object_add(resp_json, "error", json_object_get(json_object_object_get(error_content_json, "error")));
                } else {
                    // Fallback if parsing or structure is unexpected
                    struct json_object *err_detail = json_object_new_object();
                    json_object_object_add(err_detail, "code", json_object_new_int(jrpc_job->req_job->status_code));
                    json_object_object_add(err_detail, "message", json_object_new_string(error_buffer_str ? error_buffer_str : "Tool execution failed with no specific error message."));
                    json_object_object_add(resp_json, "error", err_detail);
                }
                if (error_content_json) json_object_put(error_content_json);
            } else { // No response buffer, but status code indicates error
                struct json_object *err_detail = json_object_new_object();
                json_object_object_add(err_detail, "code", json_object_new_int(jrpc_job->req_job->status_code));
                json_object_object_add(err_detail, "message", json_object_new_string("Tool execution failed."));
                json_object_object_add(resp_json, "error", err_detail);
            }
        } else if (jrpc_job->req_job->response_buffers && jrpc_job->req_job->response_buffers->data) {
            // Success case
            const char* result_buffer_str = buffer_tostring(jrpc_job->req_job->response_buffers->data);
            struct json_object *result_json = NULL;
            enum json_tokener_error result_jerr;
            // Try to parse as JSON. If it's not JSON, send as string.
            result_json = json_tokener_parse_verbose(result_buffer_str, &result_jerr);
            if (result_jerr == json_tokener_success && result_json) {
                 json_object_object_add(resp_json, "result", result_json); // tokener_parse creates new ref
            } else {
                 if(result_json) json_object_put(result_json); // Should not happen if error
                 json_object_object_add(resp_json, "result", json_object_new_string(result_buffer_str));
            }
        } else { // Success but no data (e.g. 204 No Content like)
             json_object_object_add(resp_json, "result", NULL); // JSON null
        }

        const char *response_str = json_object_to_json_string_ext(resp_json, JSON_C_TO_STRING_PLAIN);
        if (response_str) {
            websocket_protocol_send_text(wsc, response_str);
        }
        json_object_put(resp_json);
    }

    mcp_jsonrpc_adapter_job_free(jrpc_job);
    json_object_put(req_json);
    // params_for_job was owned by jrpc_job, so it's freed by mcp_jsonrpc_adapter_job_free
}
