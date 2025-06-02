#include "mcp-http-adapter.h"
#include "web/server/web_server.h"      // For web_client_api_request_v3_register_static, web_client_api_request_v1_info_send_message
#include "web/api/web_api_v3.h"         // For web_client_api_request_v3_callback_t (though not directly used, good for context)
#include "web/api/http_auth.h"          // For RRDHOST, struct web_client, USER_AUTH
#include "config.h"                     // For netdata_configured_host_prefix (usually included via common.h or libnetdata.h)
#include "libnetdata/libnetdata.h"        // For debug_log, etc. (includes common.h which includes config.h)
#include "libnetdata/log/nd_log.h"          // For netdata_error, netdata_log_info, etc.
#include "libnetdata/json/json.h"         // For json_tokener_parse_verbose, json_object_put, json_object_new_object, json_object_get
#include "libnetdata/buffer/buffer.h"       // For BUFFER, buffer_tostring
#include "libnetdata/url/url.h"           // For extract_url_segment_by_position
#include "web/mcp/core/mcp-core.h"        // For mcp_execute_tool
#include "web/mcp/core/mcp-job.h"         // For MCP_HTTP_ADAPTER_JOB, mcp_http_adapter_job_create/free, MCP_REQ_JOB fields
#include "web/mcp/core/mcp-response.h"    // For MCP_RESPONSE_BUFFER
#include "web/mcp/core/mcp-registry.h"    // For mcp_find_tool, MCP_TOOL_REGISTRY_ENTRY
#include <stdio.h>                      // For snprintf

// Define D_WEB_CLIENT if not already defined (e.g. by web_server.h or similar)
#ifndef D_WEB_CLIENT
#define D_WEB_CLIENT ND_LOG_CAT_WEB_CLIENT
#pragma message "D_WEB_CLIENT not defined in mcp-http-adapter.c, falling back to ND_LOG_CAT_WEB_CLIENT. Please include appropriate headers."
#endif

// Placeholder for D_MCP if not defined
#ifndef D_MCP
#define D_MCP D_WEB_CLIENT // Fallback to D_WEB_CLIENT if D_MCP is not specifically available
#pragma message "D_MCP not defined in mcp-http-adapter.c, falling back to D_WEB_CLIENT. Define D_MCP in a central MCP header or logging config."
#endif


// Generic handler for /api/v3/mcp/tools/{tool_name}/call
static int mcp_http_handle_tool_call(RRDHOST *host, struct web_client *w, char *url) {
    (void)host; // May not be used directly if all info comes from web_client or global state

    // URL: /api/v3/mcp/tools/{tool_name}/call
    // Segments: 0="", 1="api", 2="v3", 3="mcp", 4="tools", 5="{tool_name}", 6="call"
    // The actual tool name segment index depends on netdata_configured_host_prefix
    // If prefix is "/", segments are as above. If "/netdata", then 0="", 1="netdata", 2="api", ... , 6="{tool_name}"
    // extract_url_segment_by_position needs the path part *after* the host prefix.
    // web_server should pass the path part after the prefix to the handler.
    // Assuming `url` passed here is relative to `netdata_configured_host_prefix`
    // So, if full URL is /netdata/api/v3/mcp/tools/T/call, url here should be /api/v3/mcp/tools/T/call
    // Then segments are: 0="", 1="api", 2="v3", 3="mcp", 4="tools", 5="{tool_name}"
    // The function `web_client_api_request_v3_register_static` registers paths including the prefix.
    // The `url` parameter to the handler, however, is typically the path *without* the prefix.

    const char *tool_name = extract_url_segment_by_position(url, 5);
    if (!tool_name || !*tool_name) {
        netdata_log_debug(D_WEB_CLIENT, "MCP HTTP: Tool name not extracted from URL '%s' at segment 5.", url);
        web_client_api_request_v1_info_send_message(w, 400, "text/plain", "Tool name not specified or invalid in URL.");
        return HTTP_RESP_BAD_REQUEST;
    }

    const MCP_TOOL_REGISTRY_ENTRY *tool_entry = mcp_find_tool(tool_name);
    if (!tool_entry) {
        char msg[256];
        snprintf(msg, sizeof(msg), "Tool '%s' not found.", tool_name);
        web_client_api_request_v1_info_send_message(w, 404, "text/plain", msg);
        return HTTP_RESP_NOT_FOUND;
    }

    struct json_object *params_json = NULL;
    if (w->request_content_type == CT_APPLICATION_JSON && w->request_content_length > 0) {
        // web_client should have parsed JSON into w->json_params if Content-Type is application/json
        if (w->json_params) {
            params_json = json_object_get(w->json_params); // Take ownership/increment refcount
        } else {
            // This case might indicate a problem in web_client's JSON parsing or an empty JSON body "{}" not parsed to w->json_params
            // Or if the JSON was malformed and web_client nulled w->json_params.
            // For robustness, attempt to parse from w->request_buffer if w->json_params is NULL.
            // However, the primary expectation is that web_client handles this.
            // If w->json_params is NULL and body was expected, it's likely an error.
            netdata_log_debug(D_WEB_CLIENT, "MCP HTTP: Content-Type is JSON, but w->json_params is null. Body length: %zu", w->request_content_length);
            // For now, we'll trust that if w->json_params is NULL, it's an empty or invalid JSON.
            // Let's create an empty object to avoid passing NULL.
            params_json = json_object_new_object();
        }
    } else if (w->request_method == WEB_CLIENT_REQUEST_POST && w->request_content_length > 0) { // Check method for clarity
        web_client_api_request_v1_info_send_message(w, 415, "text/plain", "Unsupported Media Type. Expecting application/json for POST body.");
        return HTTP_RESP_UNSUPPORTED_MEDIA_TYPE;
    } else {
        // No body or not a POST with body, create an empty params object for GET or POST with no body
        params_json = json_object_new_object();
    }

    // Create MCP HTTP Adapter Job
    MCP_HTTP_ADAPTER_JOB *http_job = mcp_http_adapter_job_create(w, url, w->url_querystring, params_json, tool_name, &w->user_auth);
    // mcp_http_adapter_job_create calls mcp_req_job_create, which calls json_object_get on params_json.
    // So, we can release our reference if http_job is created. If not, we need to free params_json.
    if (!http_job) {
        json_object_put(params_json);
        web_client_api_request_v1_info_send_message(w, 500, "text/plain", "Failed to create MCP job.");
        return HTTP_RESP_INTERNAL_SERVER_ERROR;
    }
    // If job created successfully, it now owns params_json. We don't put it here.

    // Execute the tool - mcp_execute_tool will use http_job->req.tool_name
    mcp_execute_tool(http_job->req_job); // Pass the embedded MCP_REQ_JOB

    // Process responses
    if (!http_job->req_job->response_buffers) {
        if (http_job->req_job->status_code >= 200 && http_job->req_job->status_code < 300) {
            // No content, but success
            web_client_api_request_v1_info_send_message(w, http_job->req_job->status_code, "application/json", "{}");
        } else {
             char error_payload[256]; // Increased size
             snprintf(error_payload, sizeof(error_payload),
                     "{\"error\": {\"message\": \"Tool executed but no response generated by tool. Status: %d\", \"code\": %d}}",
                     http_job->req_job->status_code, http_job->req_job->status_code);
            web_client_api_request_v1_info_send_message(w, http_job->req_job->status_code, "application/json", error_payload);
        }
    } else {
        MCP_RESPONSE_BUFFER *rb = http_job->req_job->response_buffers; // Send first buffer

        w->response_code = http_job->req_job->status_code;

        const char *response_content = rb->data ? buffer_tostring(rb->data) : "{}"; // rb->data is BUFFER*
        HTTP_CONTENT_TYPE ct = rb->data ? rb->data->content_type_provided : CT_APPLICATION_JSON;
        const char *ct_str = http_content_type_name_r(ct);

        web_client_api_request_v1_info_send_message(w, w->response_code, ct_str, response_content);
    }

    mcp_http_adapter_job_free(http_job); // This will free the req_job, which frees response_buffers and params_json
    return w->response_code >= 400 ? HTTP_RESP_ERROR_SENT_BY_CALLER : HTTP_RESP_OK_SENT_BY_CALLER;
}


int mcp_http_adapter_init_routes(void) {
    char path_buffer[WEB_SERVER_MAX_PATH_LENGTH]; // Use defined constant for max path length
    snprintf(path_buffer, sizeof(path_buffer), "%sapi/v3/mcp/tools/*/call", netdata_configured_host_prefix);

    // Note: web_client_api_request_v3_register_static might not be the correct function name.
    // It's often web_server_add_api_v3_entry or similar from web_api_v3_defs.h or web_server_v3.c
    // For now, assuming web_client_api_request_v3_register_static is correct as per prompt.
    // The callback type is web_client_api_request_v3_callback_t.
    // int callback(RRDHOST *host, struct web_client *w, char *url_path_without_prefix)
    if (!web_client_api_request_v3_register_static(path_buffer, mcp_http_handle_tool_call, WEB_CLIENT_API_REQUEST_V3_FLAG_DEFAULT)) {
        netdata_error(D_MCP, "MCP HTTP Adapter: Failed to register route: %s", path_buffer);
        return -1;
    }
    netdata_log_info(D_MCP, "MCP HTTP Adapter: Registered route %s for tool calls.", path_buffer);

    // TODO: Implement /api/v3/mcp/tools and /api/v3/mcp/tools/{tool_name}/schema

    return 0;
}
