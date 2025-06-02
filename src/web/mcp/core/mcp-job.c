#include "mcp-job.h"
#include <libnetdata/clocks/clocks.h>
#include <libnetdata/memory/nd-mallocz.h>
#include <uuid/uuid.h>
#include <string.h> // For strdup

// Forward declarations for USER_AUTH and MCP_CLIENT if not fully defined in headers
// This might be necessary if their full definitions are in other modules and only forward declarations are available.
// For now, we assume their pointers can be used opaquely or their definitions are accessible.
typedef struct user_auth USER_AUTH;
typedef struct mcp_client MCP_CLIENT;
typedef struct web_client WEB_CLIENT;


MCP_REQ_JOB *mcp_req_job_create(struct json_object *params, const char *tool_name, USER_AUTH *auth) {
    (void)auth; // Temporarily unused
    MCP_REQ_JOB *job = (MCP_REQ_JOB *)nd_callocz(1, sizeof(MCP_REQ_JOB));
    if (!job) {
        // Error log: Failed to allocate MCP_REQ_JOB
        return NULL;
    }

    uuid_generate_random(job->uuid);
    job->id = (char *)nd_callocz(1, 37); // 36 chars for UUID + 1 for null terminator
    if (!job->id) {
        // Error log: Failed to allocate job ID string
        nd_free(job);
        return NULL;
    }
    uuid_unparse_lower(job->uuid, job->id);

    if (tool_name) {
        job->name = nd_strdup(tool_name);
        if (!job->name) {
            // Error log: Failed to duplicate tool_name
            nd_free(job->id);
            nd_free(job);
            return NULL;
        }
    }

    // job->source, job->endpoint, job->context are set by adapter job creation
    // job->run, job->callback are set by dispatcher

    if (params) {
        job->params = json_object_get(params); // Increment ref count
    }

    // job->created_usec = now_realtime_usec(); // Requires linking with libnetdata clocks, will add later if needed.

    return job;
}

void mcp_req_job_free(MCP_REQ_JOB *job) {
    if (!job) {
        return;
    }
    nd_free(job->id);
    nd_free(job->name);
    if (job->params) {
        json_object_put(job->params); // Decrement ref count
    }
    // job->context is freed by the adapter-specific free function
    nd_free(job);
}

MCP_HTTP_ADAPTER_JOB *mcp_http_adapter_job_create(struct web_client *web_client, const char *url_path, const char *query_string, struct json_object *params, const char *tool_name, USER_AUTH *auth) {
    (void)web_client; // Temporarily unused
    (void)query_string; // Temporarily unused
    MCP_HTTP_ADAPTER_JOB *http_job = (MCP_HTTP_ADAPTER_JOB *)nd_callocz(1, sizeof(MCP_HTTP_ADAPTER_JOB));
    if (!http_job) {
        // Error log: Failed to allocate MCP_HTTP_ADAPTER_JOB
        return NULL;
    }

    http_job->req_job = mcp_req_job_create(params, tool_name, auth);
    if (!http_job->req_job) {
        // Error log: Failed to create MCP_REQ_JOB for HTTP adapter
        nd_free(http_job);
        return NULL;
    }

    http_job->req_job->source = nd_strdup("HTTP");
    if (url_path) {
        http_job->req_job->endpoint = nd_strdup(url_path);
    }
    http_job->req_job->context = http_job;

    return http_job;
}

void mcp_http_adapter_job_free(MCP_HTTP_ADAPTER_JOB *http_job) {
    if (!http_job) {
        return;
    }
    // The embedded req_job->context points to http_job, so no separate free for context within mcp_req_job_free
    // mcp_req_job_free will handle freeing common members.
    // We must free members specific to MCP_HTTP_ADAPTER_JOB here, if any, before freeing req_job.
    mcp_req_job_free(http_job->req_job);
    nd_free(http_job);
}

MCP_JSONRPC_ADAPTER_JOB *mcp_jsonrpc_adapter_job_create(MCP_CLIENT *mcpc, uint64_t jsonrpc_id, const char *jsonrpc_method, struct json_object *params, const char *tool_name, USER_AUTH *auth) {
    (void)mcpc; // Temporarily unused
    (void)jsonrpc_id; // Temporarily unused
    MCP_JSONRPC_ADAPTER_JOB *jsonrpc_job = (MCP_JSONRPC_ADAPTER_JOB *)nd_callocz(1, sizeof(MCP_JSONRPC_ADAPTER_JOB));
    if (!jsonrpc_job) {
        // Error log: Failed to allocate MCP_JSONRPC_ADAPTER_JOB
        return NULL;
    }

    jsonrpc_job->req_job = mcp_req_job_create(params, tool_name, auth);
    if (!jsonrpc_job->req_job) {
        // Error log: Failed to create MCP_REQ_JOB for JSONRPC adapter
        nd_free(jsonrpc_job);
        return NULL;
    }

    jsonrpc_job->req_job->source = nd_strdup("JSONRPC");
    if (jsonrpc_method) {
        jsonrpc_job->req_job->endpoint = nd_strdup(jsonrpc_method);
    }
    jsonrpc_job->req_job->context = jsonrpc_job;

    return jsonrpc_job;
}

void mcp_jsonrpc_adapter_job_free(MCP_JSONRPC_ADAPTER_JOB *jsonrpc_job) {
    if (!jsonrpc_job) {
        return;
    }
    // Similar to HTTP, free specific members first, then the req_job.
    mcp_req_job_free(jsonrpc_job->req_job);
    nd_free(jsonrpc_job);
}
