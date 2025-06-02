#include "mcp-response.h"
#include "libnetdata/memory/nd-mallocz.h" // For callocz, freez
#include <libnetdata/buffer/buffer.h>     // For buffer_free
#include <json-c/json.h>                  // For json_object_put

MCP_RESPONSE_BUFFER *mcp_response_buffer_create(BUFFER *buf, const char *resp_type) {
    (void)resp_type; // This parameter can be used later for categorizing responses if needed.

    MCP_RESPONSE_BUFFER *rb = callocz(1, sizeof(MCP_RESPONSE_BUFFER));
    if (!rb) {
        // The caller of mcp_response_buffer_create is responsible for freeing 'buf' if this allocation fails,
        // as 'buf' was passed in and not yet assigned to rb->data.
        return NULL;
    }

    rb->data = buf; // MCP_RESPONSE_BUFFER takes ownership of the passed BUFFER.
    // rb->json can be set separately if needed. If set, it must be a 'gotten' reference.

    return rb;
}

void mcp_response_buffer_free(MCP_RESPONSE_BUFFER *rb) {
    if (!rb) {
        return;
    }

    if (rb->data) {
        buffer_free(rb->data); // Free the BUFFER.
    }
    if (rb->json) {
        json_object_put(rb->json); // Release reference to the json object.
    }
    freez(rb); // Free the MCP_RESPONSE_BUFFER struct itself.
}
