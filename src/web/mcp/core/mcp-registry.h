#ifndef NETDATA_MCP_REGISTRY_H
#define NETDATA_MCP_REGISTRY_H

#include "web/api/http_auth.h" // For HTTP_ACL, HTTP_ACCESS
#include "mcp-job.h"           // For MCP_REQ_JOB

// Forward declaration
struct mcp_req_job;

// MCP Namespace enum (as per TODO-LIST, though not explicitly defined there, it's used)
typedef enum mcp_namespace {
    MCP_NAMESPACE_TOOLS,
    // Add other namespaces as needed
    MCP_NAMESPACE_UNKNOWN
} MCP_NAMESPACE;

typedef struct mcp_tool_registry_entry {
    // Tool identification
    const char *name;
    uint32_t hash;

    // Authorization
    HTTP_ACL acl;
    HTTP_ACCESS access;

    // Execution
    int (*execute)(struct mcp_req_job *job);

    // MCP-specific metadata
    MCP_NAMESPACE namespace;
    const char *title;
    const char *description;
    const char *input_schema_json; // Static string for JSON schema

    // Feature flags
    bool supports_pagination;
    bool supports_streaming;

    // Caching hints for adapters
    bool cacheable_schema;
    time_t schema_cache_duration;
} MCP_TOOL_REGISTRY_ENTRY;

// Function declarations
void mcp_tools_registry_init(void);
const MCP_TOOL_REGISTRY_ENTRY *mcp_find_tool(const char *tool_name);
const MCP_TOOL_REGISTRY_ENTRY **mcp_get_tools_by_namespace(MCP_NAMESPACE namespace, size_t *count);

#endif // NETDATA_MCP_REGISTRY_H
