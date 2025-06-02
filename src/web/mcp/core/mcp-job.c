#include "mcp-job.h"
#include <libnetdata/clocks/clocks.h> // For now_monotonic_usec, if used for created_usec
#include "libnetdata/memory/nd-mallocz.h" // For callocz, freez, strdupz
#include <uuid/uuid.h>
#include <string.h>      // For strlen (though strdupz might not need it explicitly)
#include <json-c/json.h> // Ensure json_object_put is declared

// Typedefs for USER_AUTH, MCP_CLIENT, WEB_CLIENT should be resolved by includes in mcp-job.h
// For example, USER_AUTH is in web/api/http_auth.h which mcp-job.h includes.

MCP_REQ_JOB *mcp_req_job_create(struct json_object *params, const char *jobname, USER_AUTH *auth_ctx) {
    MCP_REQ_JOB *job = callocz(1, sizeof(MCP_REQ_JOB));
    if (!job) {
        // Failed to allocate job struct, params ref is still held by caller, but we were given it.
        // As per revised strategy, if callocz for job fails, we must release the params ref.
        if (params) json_object_put(params);
        return NULL;
    }

    job->auth = auth_ctx; // Assign user auth context (pointer, not deep copy)
    job->params = params; // Take ownership of the passed params (already new reference from caller)

    uuid_generate_random(job->uuid);
    job->id = callocz(1, 37); // 36 chars for UUID + 1 for null terminator
    if (!job->id) {
        if (job->params) json_object_put(job->params); // Clean up owned params
        freez(job);
        return NULL;
    }
    uuid_unparse_lower(job->uuid, job->id); // uuid_unparse_lower expects char *

    if (jobname) {
        job->name = strdupz(jobname);
        if (!job->name) {
            if (job->params) json_object_put(job->params);
            freez(job->id);
            freez(job);
            return NULL;
        }
    }

    // job->tool_name is set by adapter job create functions
    // job->source, job->endpoint, job->context are set by adapter job creation
    // job->run, job->callback are set by dispatcher
    // job->created_usec = now_monotonic_usec();

    return job;
}

void mcp_req_job_free(MCP_REQ_JOB *job) {
    if (!job) {
        return;
    }
    freez(job->id);
    freez(job->name);
    freez((void *)job->tool_name); // tool_name is const char*, but strdupz result is char*
    freez(job->source);
    freez(job->endpoint);

    if (job->params) {
        json_object_put(job->params);
    }

    // Free response buffers if any - this should be done here to ensure cleanup
    struct mcp_response_buffer *rb = job->response_buffers;
    while (rb) {
       struct mcp_response_buffer *next = rb->next;
       // Assuming mcp_response_buffer_free is compatible with freez for rb itself
       // and buffer_free for rb->data, json_object_put for rb->json
       if (rb->data) buffer_free(rb->data); // mcp_response_buffer_free should do this
       if (rb->json) json_object_put(rb->json); // mcp_response_buffer_free should do this
       freez(rb); // mcp_response_buffer_free should do this
       rb = next;
    }
    job->response_buffers = NULL;

    freez(job);
}

MCP_HTTP_ADAPTER_JOB *mcp_http_adapter_job_create(struct web_client *wclient, const char *urlpath, const char *querystr, struct json_object *params_for_job, const char *toolname_from_url, USER_AUTH *user_auth_ctx) {
    (void)wclient;
    (void)querystr;

    MCP_HTTP_ADAPTER_JOB *http_job = callocz(1, sizeof(MCP_HTTP_ADAPTER_JOB));
    if (!http_job) {
        // params_for_job is a new reference given to us. If we fail here, we must release it.
        if (params_for_job) json_object_put(params_for_job);
        return NULL;
    }

    // mcp_req_job_create will take ownership of params_for_job if successful.
    // If mcp_req_job_create fails, it's responsible for putting params_for_job.
    http_job->req_job = mcp_req_job_create(params_for_job, urlpath, user_auth_ctx);
    if (!http_job->req_job) {
        // mcp_req_job_create failed and should have handled params_for_job.
        freez(http_job);
        return NULL;
    }

    if (toolname_from_url) {
        http_job->req_job->tool_name = strdupz(toolname_from_url);
        if (!http_job->req_job->tool_name) {
            mcp_req_job_free(http_job->req_job); // Frees req_job and its params
            freez(http_job);
            return NULL;
        }
    }

    http_job->req_job->source = strdupz("HTTP");
    if (!http_job->req_job->source) {
        mcp_req_job_free(http_job->req_job);
        freez(http_job);
        return NULL;
    }

    if (urlpath) {
        http_job->req_job->endpoint = strdupz(urlpath);
        if (!http_job->req_job->endpoint) {
            mcp_req_job_free(http_job->req_job);
            freez(http_job);
            return NULL;
        }
    }
    http_job->req_job->context = http_job;

    return http_job;
}

void mcp_http_adapter_job_free(MCP_HTTP_ADAPTER_JOB *http_job) {
    if (!http_job) {
        return;
    }
    mcp_req_job_free(http_job->req_job); // This now handles response_buffers too.
    freez(http_job);
}

MCP_JSONRPC_ADAPTER_JOB *mcp_jsonrpc_adapter_job_create(MCP_CLIENT *mcpc, struct json_object *id_obj, const char *methodname, struct json_object *params_for_job, const char *toolname_from_method, USER_AUTH *user_auth_ctx) {
    (void)mcpc;

    MCP_JSONRPC_ADAPTER_JOB *jsonrpc_job = callocz(1, sizeof(MCP_JSONRPC_ADAPTER_JOB));
    if (!jsonrpc_job) {
        if (params_for_job) json_object_put(params_for_job);
        if (id_obj) json_object_put(id_obj); // Release the ref if we fail early
        return NULL;
    }

    if (id_obj) {
        jsonrpc_job->jsonrpc_id_obj = json_object_get(id_obj); // Take ownership of the ID object
    } else {
        jsonrpc_job->jsonrpc_id_obj = NULL; // Explicitly NULL for notifications
    }

    jsonrpc_job->req_job = mcp_req_job_create(params_for_job, methodname, user_auth_ctx);
    if (!jsonrpc_job->req_job) {
        // mcp_req_job_create failed and handled params_for_job.
        if (jsonrpc_job->jsonrpc_id_obj) json_object_put(jsonrpc_job->jsonrpc_id_obj);
        freez(jsonrpc_job);
        return NULL;
    }

    if (toolname_from_method) {
        jsonrpc_job->req_job->tool_name = strdupz(toolname_from_method);
        if (!jsonrpc_job->req_job->tool_name) {
            // No need to put jsonrpc_id_obj here, mcp_req_job_free won't touch it
            // but adapter job free will. Let mcp_jsonrpc_adapter_job_free handle it via mcp_req_job_free path.
            // Actually, mcp_req_job_free does not free jsonrpc_id_obj.
            // So, if strdupz fails, jsonrpc_job->req_job is valid and will be freed by mcp_req_job_free.
            // jsonrpc_job->jsonrpc_id_obj also needs to be freed.
            mcp_req_job_free(jsonrpc_job->req_job); // This frees params_for_job
            if (jsonrpc_job->jsonrpc_id_obj) json_object_put(jsonrpc_job->jsonrpc_id_obj);
            freez(jsonrpc_job);
            return NULL;
        }
    }

    jsonrpc_job->req_job->source = strdupz("JSONRPC");
    if (!jsonrpc_job->req_job->source) {
        mcp_req_job_free(jsonrpc_job->req_job);
        if (jsonrpc_job->jsonrpc_id_obj) json_object_put(jsonrpc_job->jsonrpc_id_obj);
        freez(jsonrpc_job);
        return NULL;
    }

    if (methodname) {
        jsonrpc_job->req_job->endpoint = strdupz(methodname);
         if (!jsonrpc_job->req_job->endpoint) {
            mcp_req_job_free(jsonrpc_job->req_job);
            if (jsonrpc_job->jsonrpc_id_obj) json_object_put(jsonrpc_job->jsonrpc_id_obj);
            freez(jsonrpc_job);
            return NULL;
        }
    }
    jsonrpc_job->req_job->context = jsonrpc_job;

    return jsonrpc_job;
}

void mcp_jsonrpc_adapter_job_free(MCP_JSONRPC_ADAPTER_JOB *jsonrpc_job) {
    if (!jsonrpc_job) {
        return;
    }
    if (jsonrpc_job->jsonrpc_id_obj) {
        json_object_put(jsonrpc_job->jsonrpc_id_obj);
    }
    mcp_req_job_free(jsonrpc_job->req_job);
    freez(jsonrpc_job);
}
