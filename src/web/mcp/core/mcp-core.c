#include "mcp-core.h"
#include "mcp-registry.h" // To find and execute tools
#include "mcp-response.h" // For mcp_response_buffer_create
#include "libnetdata/libnetdata.h" // For debug_log, etc.
#include "libnetdata/log/nd_log.h"   // For netdata_fatal, netdata_error, etc.
#include "libnetdata/simple_pattern/simple_pattern.h" // For acl checks - though not directly used in provided snippet, good for context
#include "libnetdata/json/json.h" // For json_escape_string
#include "libnetdata/clocks/clocks.h" // For now_monotonic_usec
#include "web/api/http_auth.h" // For USER_AUTH and ACL defines
#include <string.h>           // For strlen, snprintf
#include <stdio.h>            // For snprintf

// It's good practice to define log domains if they are not globally available
#ifndef D_WEB_CLIENT // Assuming D_WEB_CLIENT is the intended log category for mcp_execute_tool
#define D_WEB_CLIENT ND_LOG_CAT_USER
#warning "Log channel D_WEB_CLIENT not defined, defaulting to ND_LOG_CAT_USER. Please define it in a relevant header."
#endif

#ifndef D_MCP // For other MCP specific logs
#define D_MCP ND_LOG_CAT_USER
#warning "Log channel D_MCP not defined, defaulting to ND_LOG_CAT_USER. Please define it in a relevant header."
#endif


// Core MCP execution function
int mcp_execute_tool(struct mcp_req_job *job) {
    if (!job || !job->tool_name) {
        netdata_error(D_MCP, "Invalid job or tool name provided to mcp_execute_tool.");
        // Optionally, add an error response to the job if possible
        // This early, job might not be suitable for mcp_job_add_error_response if other fields are also null
        return -1;
    }

    const MCP_TOOL_REGISTRY_ENTRY *tool = mcp_find_tool(job->tool_name);
    if (!tool) {
        netdata_error(D_MCP, "Tool '%s' not found.", job->tool_name);
        mcp_job_add_error_response(job, "Tool not found", 404); // HTTP 404 Not Found
        // job->status_code = 404; // mcp_job_add_error_response should set this
        return -1;
    }

    // Authorization check
    // Assuming job->auth is populated by the adapter (e.g. HTTP adapter from web_client)
    if (job->auth) {
        if (tool->acl != HTTP_ACL_NOCHECK && !(job->auth->acl & tool->acl)) {
            char msg[256];
            snprintf(msg, sizeof(msg), "Access to tool '%s' denied due to ACL restrictions. User ACL: %u, Tool ACL: %u", tool->name, job->auth->acl, tool->acl);
            netdata_error(D_MCP, "%s", msg);
            mcp_job_add_error_response(job, msg, 403); // HTTP 403 Forbidden
            // job->status_code = 403; // mcp_job_add_error_response should set this
            return -1;
        }
    } else if (tool->acl != HTTP_ACL_NOCHECK && tool->acl != HTTP_ACL_NONE) { // HTTP_ACL_NONE might mean no auth required for specific tools
        char msg[256];
        snprintf(msg, sizeof(msg), "Authentication required for tool '%s'. Tool ACL: %u", tool->name, tool->acl);
        netdata_error(D_MCP, "%s", msg);
        mcp_job_add_error_response(job, msg, 401); // HTTP 401 Unauthorized
        // job->status_code = 401; // mcp_job_add_error_response should set this
        return -1;
    }

    netdata_log_debug(D_WEB_CLIENT, "Executing MCP tool: %s (ID: %s)", tool->name, job->id ? job->id : "N/A");
    job->status_code = 200; // Default to OK, tool can change it
    int ret = tool->execute(job); // The tool's execute function is responsible for populating job->response_buffers

    // job->completed and job->completed_usec should be set by the caller of mcp_execute_tool
    // or by the dispatcher, once the execution (including any async parts) is truly finished.
    // For now, we'll mark it based on synchronous execution.
    job->completed = true;
    job->completed_usec = now_monotonic_usec();
    return ret;
}

MCP_RESPONSE_BUFFER *mcp_job_add_response_buffer(struct mcp_req_job *job, BUFFER *buffer, const char *response_type) {
    if (!job || !buffer) {
        if (buffer) buffer_free(buffer); // Free buffer if job is null
        return NULL;
    }

    MCP_RESPONSE_BUFFER *rb = mcp_response_buffer_create(buffer, response_type);
    if (!rb) {
        buffer_free(buffer); // Free buffer if response buffer creation fails
        return NULL;
    }

    mcp_job_append_response_buffer(job, rb);
    return rb;
}

void mcp_job_prepend_response_buffer(struct mcp_req_job *job, struct mcp_response_buffer *item) {
    if (!job || !item) return;

    item->next = job->response_buffers;
    item->prev = NULL; // Item becomes the new head, so its prev is NULL
    if (job->response_buffers) {
        job->response_buffers->prev = item;
    }
    job->response_buffers = item;
}

void mcp_job_append_response_buffer(struct mcp_req_job *job, struct mcp_response_buffer *item) {
    if (!job || !item) return;

    item->next = NULL; // Item becomes the new tail, so its next is NULL
    if (!job->response_buffers) {
        job->response_buffers = item;
        item->prev = NULL;
    } else {
        MCP_RESPONSE_BUFFER *current = job->response_buffers;
        while (current->next) {
            current = current->next;
        }
        current->next = item;
        item->prev = current;
    }
}

MCP_RESPONSE_BUFFER *mcp_job_add_text_response(struct mcp_req_job *job, const char *text, int http_status, HTTP_CONTENT_TYPE content_type) {
    if (!job || !text) return NULL;

    BUFFER *buf = buffer_create(strlen(text) + 1); // No need for , NULL with buffer_create
    if (!buf) return NULL;

    buffer_strcat(buf, text); // Use buffer_strcat or buffer_strcpy for plain strings
    buffer_set_content_type(buf, content_type);

    job->status_code = http_status;
    // The "text" type here is conceptual for MCP_RESPONSE_BUFFER, not directly HTTP content type.
    return mcp_job_add_response_buffer(job, buf, "text/plain");
}

MCP_RESPONSE_BUFFER *mcp_job_add_json_response(struct mcp_req_job *job, BUFFER *json_buffer) {
    if (!job || !json_buffer) {
        if (json_buffer) buffer_free(json_buffer);
        return NULL;
    }

    buffer_set_content_type(json_buffer, CT_APPLICATION_JSON);
    // The "data" type here is conceptual.
    return mcp_job_add_response_buffer(job, json_buffer, "application/json");
}

MCP_RESPONSE_BUFFER *mcp_job_add_error_response(struct mcp_req_job *job, const char *error_msg, int http_status) {
    if (!job || !error_msg) return NULL;

    BUFFER *buf = buffer_create(256); // Initial size, will grow if needed
    if (!buf) return NULL;

    // Construct JSON error object: {"error": {"message": "...", "code": ...}}
    buffer_sprintf(buf, "{\"error\": {\"message\": \"");
    json_escape_string(buf, error_msg, strlen(error_msg)); // Ensure error_msg is properly escaped for JSON
    buffer_sprintf(buf, "\", \"code\": %d}}", http_status);

    buffer_set_content_type(buf, CT_APPLICATION_JSON);
    job->status_code = http_status;

    // Error responses are often prepended so they are found first.
    MCP_RESPONSE_BUFFER *rb = mcp_response_buffer_create(buf, "application/json_error");
    if (rb) {
        mcp_job_prepend_response_buffer(job, rb); // Prepend error to make it primary
    } else {
        buffer_free(buf); // Free buffer if response buffer creation fails
        return NULL;
    }
    return rb;
}
