#include "daemon/common.h"
#include "websocket-internal.h"
#include "websocket-structures.h"
#include "libnetdata/json/json-c-parser-inline.h"

// Forward declarations
WEBSOCKET_PAYLOAD *websocket_payload_create_text(const char *text);
WEBSOCKET_PAYLOAD *websocket_payload_create_binary(const void *data, size_t length);
WEBSOCKET_PAYLOAD *websocket_payload_create_json(struct json_object *json);

// Send a payload to a WebSocket client
int websocket_payload_send(WEBSOCKET_SERVER_CLIENT *wsc, WEBSOCKET_PAYLOAD *payload) {
    if (!wsc || !payload || !payload->buffer.data || payload->buffer.length == 0)
        return -1;

    // Use compression by default for data frames if compression is enabled
    bool use_compression = wsc->compression.enabled &&
                          !websocket_frame_is_control_opcode(payload->opcode) &&
                          payload->buffer.length >= WS_COMPRESS_MIN_SIZE;

    // Send the frame
    return websocket_protocol_send_frame(wsc, payload->buffer.data, payload->buffer.length, payload->opcode, use_compression);
}

// Create and send a text payload
int websocket_payload_send_text(WEBSOCKET_SERVER_CLIENT *wsc, const char *text) {
    if (!wsc || !text)
        return -1;
    
    // Create payload
    WEBSOCKET_PAYLOAD *payload = websocket_payload_create_text(text);
    if (!payload)
        return -1;
    
    // Send payload
    int result = websocket_payload_send(wsc, payload);
    
    // Free payload
    websocket_payload_free(payload);
    
    return result;
}

// Create and send a binary payload
int websocket_payload_send_binary(WEBSOCKET_SERVER_CLIENT *wsc, const void *data, size_t length) {
    if (!wsc || !data || length == 0)
        return -1;
    
    // Create payload
    WEBSOCKET_PAYLOAD *payload = websocket_payload_create_binary(data, length);
    if (!payload)
        return -1;
    
    // Send payload
    int result = websocket_payload_send(wsc, payload);
    
    // Free payload
    websocket_payload_free(payload);
    
    return result;
}

// Create and send a JSON payload
int websocket_payload_send_json(WEBSOCKET_SERVER_CLIENT *wsc, struct json_object *json) {
    if (!wsc || !json)
        return -1;
    
    // Create payload
    WEBSOCKET_PAYLOAD *payload = websocket_payload_create_json(json);
    if (!payload)
        return -1;
    
    // Send payload
    int result = websocket_payload_send(wsc, payload);
    
    // Free payload
    websocket_payload_free(payload);
    
    return result;
}

// Helper function to create a text payload
WEBSOCKET_PAYLOAD *websocket_payload_create_text(const char *text) {
    if (!text)
        return NULL;
    
    return websocket_payload_create(WS_OPCODE_TEXT, text, strlen(text));
}

// Helper function to create a binary payload
WEBSOCKET_PAYLOAD *websocket_payload_create_binary(const void *data, size_t length) {
    if (!data || length == 0)
        return NULL;
    
    WEBSOCKET_PAYLOAD *payload = websocket_payload_create(WS_OPCODE_BINARY, data, length);
    
    return payload;
}

// Helper function to create a JSON payload
WEBSOCKET_PAYLOAD *websocket_payload_create_json(struct json_object *json) {
    if (!json)
        return NULL;
    
    // Convert JSON to string using json-c functionality
    const char *json_str = json_object_to_json_string_ext(json, JSON_C_TO_STRING_PLAIN);
    if (!json_str)
        return NULL;
    
    // Create payload from JSON string
    WEBSOCKET_PAYLOAD *payload = websocket_payload_create_text(json_str);
    
    return payload;
}

// Send multiple text messages as fragments
int websocket_payload_send_text_fragmented(WEBSOCKET_SERVER_CLIENT *wsc, const char **fragments, int count) {
    if (!wsc || !fragments || count <= 0)
        return -1;
    
    websocket_debug(wsc, "Sending fragmented text message with %d fragments", count);
    
    int total_bytes = 0;
    
    // Send fragments with appropriate FIN and opcode settings
    for (int i = 0; i < count; i++) {
        const char *fragment = fragments[i];
        if (!fragment)
            continue;
        
        size_t length = strlen(fragment);
        bool is_first = (i == 0);
        WEBSOCKET_OPCODE opcode = is_first ? WS_OPCODE_TEXT : WS_OPCODE_CONTINUATION;
        
        // Only compress fragments larger than minimum size
        bool compress = wsc->compression.enabled && length >= WS_COMPRESS_MIN_SIZE;
        
        websocket_debug(wsc, "Sending fragment %d/%d: length=%zu, opcode=%d, compress=%d", 
                   i+1, count, length, opcode, compress);
        
        // Use the protocol send frame function directly
        int result = websocket_protocol_send_frame(wsc, fragment, length, opcode, compress);
        
        if (result < 0) {
            websocket_error(wsc, "Failed to send fragment %d/%d", i+1, count);
            return result;
        }
        
        total_bytes += result;
    }
    
    websocket_debug(wsc, "Completed sending fragmented text message, total bytes=%d", total_bytes);
    return total_bytes;
}

// Send a large binary payload as fragments
int websocket_payload_send_binary_fragmented(WEBSOCKET_SERVER_CLIENT *wsc, const void *data, 
                                            size_t length, size_t fragment_size) {
    if (!wsc || !data || length == 0 || fragment_size == 0)
        return -1;
    
    // Use reasonable default fragment size if not specified
    if (fragment_size == 0)
        fragment_size = 64 * 1024; // 64KB fragments
    
    // Calculate number of fragments
    int count = (length + fragment_size - 1) / fragment_size;
    int total_bytes = 0;
    
    websocket_debug(wsc, "Sending fragmented binary message: total_length=%zu, fragments=%d, fragment_size=%zu", 
               length, count, fragment_size);
    
    // Send fragments
    const char *ptr = (const char *)data;
    size_t remaining = length;
    
    for (int i = 0; i < count; i++) {
        bool is_first = (i == 0);
        size_t this_size = (remaining < fragment_size) ? remaining : fragment_size;
        WEBSOCKET_OPCODE opcode = is_first ? WS_OPCODE_BINARY : WS_OPCODE_CONTINUATION;
        
        // Only compress fragments larger than minimum size
        bool compress = wsc->compression.enabled && this_size >= WS_COMPRESS_MIN_SIZE;
        
        websocket_debug(wsc, "Sending binary fragment %d/%d: size=%zu, opcode=%d, compress=%d", 
                   i+1, count, this_size, opcode, compress);
        
        // Send the fragment
        int result = websocket_protocol_send_frame(wsc, ptr, this_size, opcode, compress);
        
        if (result < 0) {
            websocket_error(wsc, "Failed to send binary fragment %d/%d", i+1, count);
            return result;
        }
        
        total_bytes += result;
        ptr += this_size;
        remaining -= this_size;
    }
    
    websocket_debug(wsc, "Completed sending fragmented binary message, total bytes=%d", total_bytes);
    return total_bytes;
}

// Send a formatted text message (printf style)
int websocket_payload_send_text_fmt(WEBSOCKET_SERVER_CLIENT *wsc, const char *format, ...) {
    if (!wsc || !format)
        return -1;
    
    websocket_debug(wsc, "Preparing formatted text message");
    
    char buffer[4096];
    va_list args;
    
    va_start(args, format);
    int length = vsnprintf(buffer, sizeof(buffer), format, args);
    va_end(args);
    
    if (length < 0) {
        websocket_error(wsc, "Failed to format text message");
        return -1;
    }
    
    // If the message fits in our buffer, send it directly
    if (length < (int)sizeof(buffer)) {
        websocket_debug(wsc, "Sending formatted text message, length=%d", length);
        return websocket_payload_send_text(wsc, buffer);
    }
    
    // Otherwise, allocate a larger buffer
    websocket_debug(wsc, "Message too large for stack buffer (%d bytes), allocating heap buffer", length);
    char *large_buffer = mallocz(length + 1);
    if (!large_buffer) {
        websocket_error(wsc, "Failed to allocate memory for large formatted text message");
        return -1;
    }
    
    va_start(args, format);
    vsnprintf(large_buffer, length + 1, format, args);
    va_end(args);
    
    websocket_debug(wsc, "Sending large formatted text message, length=%d", length);
    int result = websocket_payload_send_text(wsc, large_buffer);
    freez(large_buffer);
    
    return result;
}