#include "mcp-response.h"
#include <libnetdata/memory/nd-mallocz.h>
#include <libnetdata/buffer/buffer.h> // For buffer_free

// MCP_RESPONSE_BUFFER *mcp_response_buffer_create(BUFFER *buffer, const char *response_type) {
//     (void)response_type; // Temporarily unused, might be used later for content-type or other metadata
MCP_RESPONSE_BUFFER *mcp_response_buffer_create(BUFFER *buffer, const char *response_type) {
    (void)response_type; // Temporarily unused
    MCP_RESPONSE_BUFFER *rb = (MCP_RESPONSE_BUFFER *)nd_callocz(1, sizeof(MCP_RESPONSE_BUFFER));
    if (!rb) {
        // Error log: Failed to allocate MCP_RESPONSE_BUFFER
        return NULL;
    }

    rb->data = buffer; // The buffer is passed in, MCP_RESPONSE_BUFFER takes ownership or a reference based on design.
                       // Assuming it takes ownership for now. If it's a reference, buffer_free should not be called here.
                       // For json_object, it would be json_object_get() if passed and stored.

    // rb->json is not initialized here, assuming it's set separately if the response is JSON.
    // If data buffer is meant to contain JSON string which is then parsed to rb->json,
    // that logic would be elsewhere.

    return rb;
}

void mcp_response_buffer_free(MCP_RESPONSE_BUFFER *rb) {
    if (!rb) {
        return;
    }

    if (rb->data) {
        buffer_free(rb->data); // Free the BUFFER if MCP_RESPONSE_BUFFER owns it.
    }
    if (rb->json) {
        json_object_put(rb->json); // Decrement ref count for the json object
    }
    nd_free(rb);
}
