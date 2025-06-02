#ifndef NETDATA_MCP_JOB_H
#define NETDATA_MCP_JOB_H

#include <uuid/uuid.h>
#include <json-c/json.h>
#include <stdbool.h> // For bool type
#include <stdint.h>  // For uint64_t

// Forward declarations
struct mcp_req_job;
struct mcp_response_buffer; // For response_buffers field
struct user_auth; // For auth field (USER_AUTH typedef'd later or in included headers)

typedef struct mcp_http_adapter_job {
    struct mcp_req_job *req_job; // Parent
    // Add http specific members
} MCP_HTTP_ADAPTER_JOB;

typedef struct mcp_jsonrpc_adapter_job {
    struct mcp_req_job *req_job; // Parent
    // Add jsonrpc specific members
} MCP_JSONRPC_ADAPTER_JOB;

typedef struct mcp_req_job {
    // Identification & Naming
    char *id; // job id -- uuidv4 string
    nd_uuid_t uuid; // job id -- binary uuid
    char *name;     // job name -- e.g. /api/v1/info (often the full path or method)
    const char *tool_name; // actual tool name to be executed, e.g., "execute_function"

    // Source & Context
    char *source;   // job source -- e.g. HTTP, JSONRPC, etc.
    char *endpoint; // job endpoint -- e.g. /api/v1/info or JSON-RPC method string
    void *context;  // job context -- e.g. MCP_HTTP_ADAPTER_JOB, MCP_JSONRPC_ADAPTER_JOB, etc.

    // Parameters & Authentication
    struct json_object *params; // job params -- e.g. query params, request body, etc.
    struct user_auth *auth;     // User authentication context

    // Execution & Lifecycle
    int (*run)(struct mcp_req_job *job, struct json_object **resp); // job run function (tool specific)
    void (*callback)(struct mcp_req_job *job, struct json_object *resp); // job callback function
    // uint64_t created_usec; // Timestamp of creation - add if libnetdata/clocks.h is used
    bool completed;
    uint64_t completed_usec; // Timestamp of completion

    // Response Handling
    struct mcp_response_buffer *response_buffers; // Linked list of response buffers
    int status_code; // Final status code of the job (e.g., HTTP status)

} MCP_REQ_JOB;

#endif // NETDATA_MCP_JOB_H
