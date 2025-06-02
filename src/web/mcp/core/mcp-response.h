#ifndef NETDATA_MCP_RESPONSE_H
#define NETDATA_MCP_RESPONSE_H

#include <libnetdata/buffer/buffer.h>
#include <json-c/json.h>

// Forward declaration
struct mcp_response_buffer;

typedef struct mcp_response_buffer {
    BUFFER *data; // response data (e.g. text, binary)
    struct json_object *json; // response json (if applicable, could be parsed from data or primary representation)

    // For linked list functionality
    struct mcp_response_buffer *next;
    struct mcp_response_buffer *prev;

    // Add any other relevant members, e.g., content_type if not solely on BUFFER
    // const char *response_type; // e.g., "data", "error", "metadata_chunk" - passed to create
} MCP_RESPONSE_BUFFER;

#endif // NETDATA_MCP_RESPONSE_H
