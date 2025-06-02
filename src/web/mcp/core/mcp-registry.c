#include "mcp-registry.h"
#include "libnetdata/libnetdata.h" // For simple_hash(), etc.
                                   // Assuming D_MCP is a log channel defined elsewhere or part of libnetdata's logging
#include "libnetdata/log/nd_log.h"      // For netdata_log_debug, netdata_log_error
#include <string.h>               // For strcmp
#include <stdbool.h>              // For bool type if not included by other headers

// Placeholder for tool execution function and schema (will be defined later)
int mcp_tool_execute_function(struct mcp_req_job *job);
#define MCP_EXECUTE_FUNCTION_SCHEMA_JSON "{ \"type\": \"object\" }" // Simplified placeholder

// Global static registry
MCP_TOOL_REGISTRY_ENTRY mcp_tools_registry[] = {
    {
        .name = "execute_function",
        .hash = 0, // Will be calculated on init
        .acl = HTTP_ACL_FUNCTIONS, // Assuming HTTP_ACL_FUNCTIONS is defined in web/api/http_auth.h
        .access = HTTP_ACCESS_ANONYMOUS_DATA, // Assuming HTTP_ACCESS_ANONYMOUS_DATA is defined
        .execute = mcp_tool_execute_function, // Placeholder
        .namespace = MCP_NAMESPACE_TOOLS,
        .title = "Execute Netdata Function",
        .description = "Execute live data collection functions on nodes",
        .input_schema_json = MCP_EXECUTE_FUNCTION_SCHEMA_JSON, // Placeholder
        .supports_pagination = true,
        .supports_streaming = false,
        .cacheable_schema = true,
        .schema_cache_duration = 3600,
    },
    // ... more tools will be added here
    { .name = NULL } // Terminator
};

// It's good practice to define log domains if they are not globally available
#ifndef D_MCP
#define D_MCP ND_LOG_CAT_USER
#warning "Log channel D_MCP not defined, defaulting to ND_LOG_CAT_USER. Please define it in a relevant header."
#endif


void mcp_tools_registry_init(void) {
    for (MCP_TOOL_REGISTRY_ENTRY *tool = mcp_tools_registry; tool->name != NULL; tool++) {
        if (tool->hash == 0) { // Calculate hash if not pre-set
            tool->hash = simple_hash(tool->name);
        }
        netdata_log_debug(D_MCP, "MCP Tool '%s' initialized with hash %u", tool->name, tool->hash);
    }
}

const MCP_TOOL_REGISTRY_ENTRY *mcp_find_tool(const char *tool_name) {
    if (!tool_name || !*tool_name) {
        return NULL;
    }

    uint32_t hash = simple_hash(tool_name);
    for (MCP_TOOL_REGISTRY_ENTRY *tool = mcp_tools_registry; tool->name != NULL; tool++) {
        // Check hash first for performance, then strcmp to resolve collisions
        if (tool->hash == hash && strcmp(tool->name, tool_name) == 0) {
            return tool;
        }
    }
    netdata_log_debug(D_MCP, "MCP Tool '%s' not found in registry.", tool_name);
    return NULL;
}

// Placeholder for mcp_tool_execute_function
int mcp_tool_execute_function(struct mcp_req_job *job) {
    (void)job; // Suppress unused parameter warning
    netdata_log_error(D_MCP, "MCP: mcp_tool_execute_function is not yet implemented.");
    // In a real scenario, this function would process job->params and interact with other parts of the system.
    // It should then populate job->response or indicate an error.
    // For now, returning -1 to signify an error or not implemented.
    return -1;
}

// mcp_get_tools_by_namespace will be implemented later
const MCP_TOOL_REGISTRY_ENTRY **mcp_get_tools_by_namespace(MCP_NAMESPACE namespace, size_t *count) {
    (void)namespace; // Suppress unused parameter warning
    if (!count) return NULL; // Basic error checking

    netdata_log_error(D_MCP, "MCP: mcp_get_tools_by_namespace is not yet implemented.");
    *count = 0;
    // This function would typically iterate through mcp_tools_registry,
    // collect all entries matching the namespace, allocate an array to hold pointers to them,
    // and return that array, setting *count to the number of tools found.
    // The caller would be responsible for freeing the returned array.
    return NULL;
}
