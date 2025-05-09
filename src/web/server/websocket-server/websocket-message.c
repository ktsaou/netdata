#include "daemon/common.h"
#include "websocket-internal.h"

// Initialize an already allocated buffer structure
void websocket_buffer_init(WEBSOCKET_BUFFER *buffer, size_t initial_size) {
    if (!buffer)
        return;
    
    buffer->data = mallocz(initial_size);
    buffer->alloc_size = initial_size;
    buffer->length = 0;
    buffer->pos = 0;
}

// Clean up an embedded buffer (free data but not the buffer structure itself)
void websocket_buffer_cleanup(WEBSOCKET_BUFFER *buffer) {
    if (!buffer)
        return;

    freez(buffer->data);
    buffer->data = NULL;
    buffer->alloc_size = 0;
    buffer->length = 0;
    buffer->pos = 0;
}

// Initialize buffer structure
WEBSOCKET_BUFFER *websocket_buffer_create(size_t initial_size) {
    WEBSOCKET_BUFFER *buffer = mallocz(sizeof(WEBSOCKET_BUFFER));

    websocket_buffer_init(buffer, MAX(initial_size, 1024));

    return buffer;
}

// Free buffer structure
void websocket_buffer_free(WEBSOCKET_BUFFER *buffer) {
    if (!buffer)
        return;

    freez(buffer->data);
    freez(buffer);
}

// Resize buffer to a new size
void websocket_buffer_resize(WEBSOCKET_BUFFER *buffer, size_t new_size) {
    // If the buffer is already large enough, consider it a success
    if (new_size <= buffer->alloc_size)
        return;
    
    netdata_log_debug(D_WEBSOCKET, "WEBSOCKET: Buffer resize: from %zu to %zu bytes (current length: %zu)",
                   buffer->alloc_size, new_size, buffer->length);

    // Allocate new buffer
    buffer->data = reallocz(buffer->data, new_size);
    buffer->alloc_size = new_size;
}

// Append data to buffer
void websocket_buffer_expand(WEBSOCKET_BUFFER *buffer, size_t length) {
    if (!buffer || !length)
        return;
    
    // Check if we need to resize the buffer
    if (buffer->length + length > buffer->alloc_size) {
        // Calculate new size
        size_t new_size = buffer->alloc_size * 2;
        if (new_size < buffer->length + length)
            new_size = buffer->length + length + 1024; // Add some extra space
        
        websocket_buffer_resize(buffer, new_size);
    }
}

// Reset buffer
void websocket_buffer_reset(WEBSOCKET_BUFFER *buffer) {
    if (!buffer)
        return;

    buffer->length = 0;
    buffer->pos = 0;
}

// Create a new WebSocket message
NEVERNULL
WEBSOCKET_MESSAGE *websocket_message_create(size_t initial_size) {
    WEBSOCKET_MESSAGE *msg = callocz(1, sizeof(WEBSOCKET_MESSAGE));
    
    // Initialize the embedded buffer directly
    websocket_buffer_init(&msg->buffer, initial_size);
    msg->opcode = WS_OPCODE_TEXT;

    return msg;
}

// Free a WebSocket message
void websocket_message_free(WEBSOCKET_MESSAGE *msg) {
    if (!msg)
        return;
        
    // Clean up the embedded buffer using our buffer cleanup function
    websocket_buffer_cleanup(&msg->buffer);
    
    // Free the message itself
    freez(msg);
}

// Reset a WebSocket message for reuse
void websocket_message_reset(WEBSOCKET_MESSAGE *msg) {
    if (!msg)
        return;
        
    // Reset buffer using the buffer reset function
    websocket_buffer_reset(&msg->buffer);
    
    // Reset message state
    msg->complete = true; // Initial state is complete (no fragmented message in progress)
    msg->is_compressed = false;
    msg->opcode = WS_OPCODE_TEXT; // Default opcode
    msg->total_length = 0;
    
    // No current_frame to reset anymore
}

// Check if a message is a control message
bool websocket_message_is_control(const WEBSOCKET_MESSAGE *msg) {
    if (!msg)
        return false;
        
    return (msg->opcode == WS_OPCODE_CLOSE || 
            msg->opcode == WS_OPCODE_PING || 
            msg->opcode == WS_OPCODE_PONG);
}

// Helper function to determine if an opcode is a control opcode
bool websocket_frame_is_control_opcode(WEBSOCKET_OPCODE opcode) {
    return (opcode == WS_OPCODE_CLOSE || 
            opcode == WS_OPCODE_PING || 
            opcode == WS_OPCODE_PONG);
}

// Convert a message to a payload structure ready for application processing
WEBSOCKET_PAYLOAD *websocket_message_prepare_payload(WEBSOCKET_SERVER_CLIENT *wsc, WEBSOCKET_MESSAGE *msg) {
    if (!wsc || !msg)
        return NULL;

    // Empty messages (length=0) are valid in WebSocket and should create an empty payload
    // We only need to check for msg->buffer.data if length is > 0
    if (msg->buffer.length > 0 && !msg->buffer.data)
        return NULL;
        
    websocket_debug(wsc, "Preparing payload from message (opcode=0x%x, is_compressed=%d, length=%zu, fragmented=%d)",
               msg->opcode, msg->is_compressed, msg->buffer.length, !msg->complete);
    
    // Create a new payload
    WEBSOCKET_PAYLOAD *payload = mallocz(sizeof(WEBSOCKET_PAYLOAD));
    
    // Set payload type
    payload->opcode = msg->opcode;
    payload->is_binary = (msg->opcode == WS_OPCODE_BINARY);
    
    // Handle compression if needed
    if (msg->is_compressed && wsc->compression.enabled) {
        websocket_debug(wsc, "Message is compressed, decompressing %zu bytes",
                   msg->buffer.length);
        
        // Compressed message - needs decompression
        char *uncompressed_data = NULL;
        size_t uncompressed_length = 0;

        // Dump compressed data and buffer metadata for debugging
        websocket_dump_debug(wsc, msg->buffer.data, msg->buffer.length,
                         "Compressed data, buffer metadata: length=%zu, alloc_size=%zu, pos=%zu",
                         msg->buffer.length, msg->buffer.alloc_size, msg->buffer.pos);

        // Verify that the buffer is well-formed before attempting decompression
        if (msg->buffer.length > msg->buffer.alloc_size) {
            websocket_error(wsc, "Buffer corruption detected: length (%zu) > alloc_size (%zu)",
                        msg->buffer.length, msg->buffer.alloc_size);
            freez(payload);
            return NULL;
        }
        
        // Ensure buffer is null-terminated to avoid potential overruns
        // This is defensive programming - the buffer may not need to be null-terminated
        // for binary data, but it doesn't hurt and may prevent certain issues
        if (msg->buffer.length < msg->buffer.alloc_size) {
            msg->buffer.data[msg->buffer.length] = '\0';
        }
        
        // Decompress the message payload
        int result = websocket_decompress_payload(wsc, msg->buffer.data, msg->buffer.length, 
                                             &uncompressed_data, &uncompressed_length);
        
        if (result < 0 || !uncompressed_data || uncompressed_length == 0) {
            // Decompression failed
            websocket_error(wsc, "Failed to decompress message payload - was this a fragmented compressed message?");
            
            // Additional debug information about the buffer
            websocket_debug(wsc, "Message details: total_length=%llu, is_fragmented=%d, complete=%d",
                        (unsigned long long)msg->total_length, !msg->complete, msg->complete);
            
            freez(payload);
            return NULL;
        }
        
        // Dump decompressed data for debugging with complete stats
        websocket_dump_debug(wsc, uncompressed_data, uncompressed_length,
                         "Successfully decompressed message from %zu to %zu bytes (ratio: %.2fx)",
                         msg->buffer.length, uncompressed_length,
                         (double)uncompressed_length / (double)msg->buffer.length);

        // If it's a text message, try to show the decompressed content
        if (msg->opcode == WS_OPCODE_TEXT && uncompressed_length < 100 && uncompressed_data) {
            char text_preview[101];
            size_t copy_len = (uncompressed_length < 100) ? uncompressed_length : 100;
            memcpy(text_preview, uncompressed_data, copy_len);
            text_preview[copy_len] = '\0';
            websocket_debug(wsc, "Decompressed text: '%s'", text_preview);
        }

        // Use the uncompressed data as payload
        payload->data = uncompressed_data;
        payload->length = uncompressed_length;
        payload->needs_free = true; // We'll need to free this data
    }
    else {
        // Uncompressed message - directly use buffer data
        websocket_debug(wsc, "Message not compressed, copying %zu bytes directly",
                   msg->buffer.length);

        // Handle empty messages
        if (msg->buffer.length == 0) {
            // For empty messages, create appropriate empty payload
            if (msg->opcode == WS_OPCODE_BINARY) {
                // Empty binary payload - empty data but valid pointer
                payload->data = mallocz(1);  // Just allocate 1 byte to have a valid pointer
            } else {
                // Empty text payload - empty string (just null terminator)
                payload->data = mallocz(1);
                ((char*)payload->data)[0] = '\0';
            }
            payload->length = 0;
            payload->needs_free = true;
        }
        else {
            // Non-empty message - copy data normally
            // Don't use strndupz for binary data as it expects null-terminated strings
            if (msg->opcode == WS_OPCODE_BINARY) {
                // For binary data, allocate exact size and use memcpy
                payload->data = mallocz(msg->buffer.length);
                memcpy(payload->data, msg->buffer.data, msg->buffer.length);
            } else {
                // For text data, manually allocate memory and copy data to ensure proper handling
                // Don't use strndupz for safety, as it can cause issues with malformed text
                payload->data = mallocz(msg->buffer.length + 1);
                memcpy(payload->data, msg->buffer.data, msg->buffer.length);
                ((char*)payload->data)[msg->buffer.length] = '\0';
            }
            payload->length = msg->buffer.length;
            payload->needs_free = true; // We'll need to free this data
        }
    }
    
    websocket_debug(wsc, "Payload prepared: opcode=0x%x, is_binary=%d, length=%zu",
               payload->opcode, payload->is_binary, payload->length);
    
    return payload;
}
