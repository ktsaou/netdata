#include "daemon/common.h"
#include "websocket-internal.h"
#include "websocket-structures.h"

// Create and send a WebSocket frame
int websocket_protocol_send_frame(WEBSOCKET_SERVER_CLIENT *wsc, const char *payload, 
                                 size_t payload_len, WEBSOCKET_OPCODE opcode, bool use_compression) {
    if (!wsc || wsc->sock.fd < 0)
        return -1;
    
    // Validate parameters based on WebSocket protocol
    if (websocket_frame_is_control_opcode(opcode) && payload_len > 125) {
        websocket_error(wsc, "Control frame payload too large: %zu bytes (max: 125)",
                   payload_len);
        return -1;
    }
    
    // Check if we should actually use compression
    bool compress = use_compression && wsc->compression.enabled && wsc->compression.deflate_stream && 
                   payload_len >= WS_COMPRESS_MIN_SIZE && !websocket_frame_is_control_opcode(opcode);
    
    char *frame_payload = (char *)payload;
    size_t frame_payload_len = payload_len;
    bool payload_compressed = false;
    
    // Compress the payload if needed
    if (compress) {
        char *compressed_payload = NULL;
        size_t compressed_len = 0;
        
        int compression_result = websocket_compress_payload(wsc, payload, payload_len, 
                                                          &compressed_payload, &compressed_len);
        
        if (compression_result > 0 && compressed_payload && compressed_len > 0) {
            // Successfully compressed
            frame_payload = compressed_payload;
            frame_payload_len = compressed_len;
            payload_compressed = true;
            
            websocket_debug(wsc, "Compressed payload from %zu to %zu bytes (%.1f%% reduction)",
                       payload_len, compressed_len, 
                       (100.0 - ((double)compressed_len / (double)payload_len) * 100.0));
        }
        else {
            // Compression failed or wasn't effective, use original payload
            compress = false;
            websocket_debug(wsc, "Compression ineffective or failed, using original payload");
        }
    }
    
    // Calculate frame size
    size_t header_size;
    if (frame_payload_len < 126)
        header_size = 2;
    else if (frame_payload_len <= 65535)
        header_size = 4;
    else
        header_size = 10;
    
    size_t frame_size = header_size + frame_payload_len;
    
    // Allocate buffer for the complete frame
    char *frame_buffer = mallocz(frame_size);
    if (!frame_buffer) {
        if (payload_compressed)
            freez(frame_payload);
        return -1;
    }
    
    // Build frame header
    unsigned char *header = (unsigned char *)frame_buffer;
    
    // First byte: FIN(1) + RSV1(compress) + RSV2(0) + RSV3(0) + OPCODE(4)
    header[0] = 0x80 | (compress ? 0x40 : 0) | (opcode & 0x0F);
    
    // Second byte: MASK(0) + payload length
    if (frame_payload_len < 126) {
        header[1] = frame_payload_len & 0x7F;
    }
    else if (frame_payload_len <= 65535) {
        header[1] = 126;
        header[2] = (frame_payload_len >> 8) & 0xFF;
        header[3] = frame_payload_len & 0xFF;
    }
    else {
        header[1] = 127;
        header[2] = (frame_payload_len >> 56) & 0xFF;
        header[3] = (frame_payload_len >> 48) & 0xFF;
        header[4] = (frame_payload_len >> 40) & 0xFF;
        header[5] = (frame_payload_len >> 32) & 0xFF;
        header[6] = (frame_payload_len >> 24) & 0xFF;
        header[7] = (frame_payload_len >> 16) & 0xFF;
        header[8] = (frame_payload_len >> 8) & 0xFF;
        header[9] = frame_payload_len & 0xFF;
    }
    
    // Copy payload to frame buffer
    memcpy(frame_buffer + header_size, frame_payload, frame_payload_len);
    
    // Log frame being sent
    websocket_debug(wsc, "Sending frame: opcode=%d, payload_len=%zu%s",
               opcode, frame_payload_len, compress ? ", compressed=yes" : "");
    
    // If we have an output buffer, add to it
    if (wsc->out_buffer) {
        buffer_need_bytes(wsc->out_buffer, frame_size);
        buffer_memcat(wsc->out_buffer, frame_buffer, frame_size);
        
        // Make sure the client's poll flags include WRITE
        if (wsc->thread && wsc->sock.fd >= 0) {
            websocket_thread_update_client_poll_flags(wsc->thread, wsc, ND_POLL_READ | ND_POLL_WRITE);
        }
        
        // Try to write immediately in case the buffer is getting large
        websocket_write_data(wsc);
        
        // Free temporary buffers
        freez(frame_buffer);
        if (payload_compressed)
            freez(frame_payload);
        
        return (int)frame_size;
    }
    
    // Direct send, no buffer
    ssize_t bytes_sent = nd_sock_write(&wsc->sock, frame_buffer, frame_size, 0);
    
    // Check for errors
    if (bytes_sent < 0) {
        websocket_error(wsc, "Failed to send frame: %s", strerror(errno));
    }
    else if ((size_t)bytes_sent < frame_size) {
        websocket_error(wsc, "Partial frame sent: %zd of %zu bytes",
                   bytes_sent, frame_size);
    }
    else {
        websocket_debug(wsc, "Successfully sent %zd bytes", bytes_sent);
    }
    
    // Free temporary buffers
    freez(frame_buffer);
    if (payload_compressed)
        freez(frame_payload);
    
    return (int)bytes_sent;
}

// Send a text message
int websocket_protocol_send_text(WEBSOCKET_SERVER_CLIENT *wsc, const char *text) {
    if (!wsc || !text)
        return -1;
    
    size_t text_len = strlen(text);
    
    websocket_debug(wsc, "Sending text message, length=%zu", text_len);
    
#ifdef NETDATA_INTERNAL_CHECKS
    // Log the text content for small messages
    if (text_len < 100) {
        websocket_debug(wsc, "Text message content: '%s'", text);
    }
    
    // Log the first few bytes of the text message in hex
    char hex_dump[128] = "";
    for (size_t i = 0; i < text_len && i < 16; i++) {
        char *pos = hex_dump + strlen(hex_dump);
        snprintf(pos, sizeof(hex_dump) - strlen(hex_dump), "%02x ", (unsigned char)text[i]);
    }
    websocket_debug(wsc, "Text message data: %s%s", 
               hex_dump, text_len > 16 ? "..." : "");
#endif /* NETDATA_INTERNAL_CHECKS */
    
    // Enable compression for text messages by default
    return websocket_protocol_send_frame(wsc, text, text_len, WS_OPCODE_TEXT, true);
}

// Send a binary message
int websocket_protocol_send_binary(WEBSOCKET_SERVER_CLIENT *wsc, const void *data, size_t length) {
    if (!wsc || !data || length == 0)
        return -1;
    
    return websocket_protocol_send_frame(wsc, data, length, WS_OPCODE_BINARY, true);
}

// Send a close frame
int websocket_protocol_send_close(WEBSOCKET_SERVER_CLIENT *wsc, uint16_t code, const char *reason) {
    if (!wsc)
        return -1;
    
    // Validate close code
    if (!websocket_validate_close_code(code)) {
        websocket_error(wsc, "Invalid close code: %d", code);
        code = WS_CLOSE_PROTOCOL_ERROR;
        reason = "Invalid close code";
    }
    
    // Prepare close payload: 2-byte code + optional reason text
    size_t reason_len = reason ? strlen(reason) : 0;
    size_t payload_len = 2 + reason_len;
    
    // Control frames max size is 125 bytes
    if (payload_len > 125) {
        websocket_error(wsc, "Close frame payload too large: %zu bytes (max: 123 for reason)",
                   payload_len);
        payload_len = 125;
        reason_len = 123; // Truncate reason to fit
    }
    
    char *payload = mallocz(payload_len);
    if (!payload)
        return -1;
    
    // Set status code in network byte order (big-endian)
    payload[0] = (code >> 8) & 0xFF;
    payload[1] = code & 0xFF;
    
    // Add reason if provided (truncate if necessary)
    if (reason && reason_len > 0)
        memcpy(payload + 2, reason, reason_len);
    
    // Send close frame (never compressed)
    int result = websocket_protocol_send_frame(wsc, payload, payload_len, WS_OPCODE_CLOSE, false);
    
    // Free temporary buffer
    freez(payload);
    
    return result;
}

// Send a ping frame
int websocket_protocol_send_ping(WEBSOCKET_SERVER_CLIENT *wsc, const char *data, size_t length) {
    if (!wsc)
        return -1;
    
    // Control frames max size is 125 bytes
    if (length > 125) {
        websocket_error(wsc, "Ping frame payload too large: %zu bytes (max: 125)",
                   length);
        return -1;
    }
    
    // If no data provided, use empty ping
    if (!data || length == 0) {
        data = "";
        length = 0;
    }
    
    // Send ping frame (never compressed)
    return websocket_protocol_send_frame(wsc, data, length, WS_OPCODE_PING, false);
}

// Send a pong frame
int websocket_protocol_send_pong(WEBSOCKET_SERVER_CLIENT *wsc, const char *data, size_t length) {
    if (!wsc)
        return -1;
    
    // Control frames max size is 125 bytes
    if (length > 125) {
        websocket_error(wsc, "Pong frame payload too large: %zu bytes (max: 125)",
                   length);
        return -1;
    }
    
    // If no data provided, use empty pong
    if (!data || length == 0) {
        data = "";
        length = 0;
    }
    
    // Send pong frame (never compressed)
    return websocket_protocol_send_frame(wsc, data, length, WS_OPCODE_PONG, false);
}
