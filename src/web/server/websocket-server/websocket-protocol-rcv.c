#include "daemon/common.h"
#include "websocket-internal.h"

static void websocket_protocol_message_done(WEBSOCKET_SERVER_CLIENT *wsc) {
    websocket_message_free(wsc->current_message);
    wsc->current_message = NULL;
    wsc->frame_id = 0;
    wsc->message_id++;
}

// Process a complete WebSocket message
static bool websocket_protocol_process_message(WEBSOCKET_SERVER_CLIENT *wsc) {
    if (!wsc || !wsc->current_message || !wsc->current_message->complete)
        return false;

    WEBSOCKET_MESSAGE *msg = wsc->current_message;

    // Convert message to payload
    WEBSOCKET_PAYLOAD *payload = websocket_message_prepare_payload(wsc, msg);
    if (!payload) {
        websocket_error(wsc, "Failed to prepare payload from message");
        return false;
    }

    // Process the payload (handles decompression if needed)
    bool result = websocket_payload_process(wsc, payload);

    // Free the payload (payload_process will make a copy if needed)
    websocket_payload_free(payload);

    return result;
}

// Helper function to handle frame header parsing
static bool websocket_protocol_parse_header_from_buffer(WEBSOCKET_SERVER_CLIENT *wsc, const char *buffer, size_t length, 
                                                      WEBSOCKET_FRAME_HEADER *header, uint64_t *payload_length, 
                                                      unsigned char *mask_key, size_t *header_size) {
    if (!wsc || !buffer || !header || !payload_length || !mask_key || !header_size || length < 2)
        return false;
        
    // Get first byte - contains FIN bit, RSV bits, and opcode
    unsigned char byte1 = (unsigned char)buffer[0];
    header->fin = (byte1 & WS_FIN) ? 1 : 0;
    header->rsv1 = (byte1 & WS_RSV1) ? 1 : 0;
    header->rsv2 = (byte1 & (WS_RSV1 >> 1)) ? 1 : 0;
    header->rsv3 = (byte1 & (WS_RSV1 >> 2)) ? 1 : 0;
    header->opcode = byte1 & 0x0F;
    
    // Get second byte - contains MASK bit and initial length
    unsigned char byte2 = (unsigned char)buffer[1];
    header->mask = (byte2 & WS_MASK) ? 1 : 0;
    header->len = byte2 & 0x7F;
    
    // Calculate header size and payload length based on length field
    *header_size = 2; // Start with 2 bytes for the basic header
    
    // Determine payload length
    if (header->len < 126) {
        *payload_length = header->len;
    } 
    else if (header->len == 126) {
        // 16-bit length
        if (length < 4) return false; // Not enough data
        
        *payload_length = ((uint64_t)((unsigned char)buffer[2]) << 8) | 
                         ((uint64_t)((unsigned char)buffer[3]));
        *header_size += 2;
    } 
    else if (header->len == 127) {
        // 64-bit length
        if (length < 10) return false; // Not enough data
        
        *payload_length = ((uint64_t)((unsigned char)buffer[2]) << 56) | 
                         ((uint64_t)((unsigned char)buffer[3]) << 48) | 
                         ((uint64_t)((unsigned char)buffer[4]) << 40) | 
                         ((uint64_t)((unsigned char)buffer[5]) << 32) | 
                         ((uint64_t)((unsigned char)buffer[6]) << 24) | 
                         ((uint64_t)((unsigned char)buffer[7]) << 16) | 
                         ((uint64_t)((unsigned char)buffer[8]) << 8) | 
                         ((uint64_t)((unsigned char)buffer[9]));
        *header_size += 8;
    }
    
    // Read masking key if frame is masked
    if (header->mask) {
        if (length < *header_size + 4) return false; // Not enough data
        
        // Copy masking key
        memcpy(mask_key, buffer + *header_size, 4);
        *header_size += 4;
    } else {
        // Clear mask key if not masked
        memset(mask_key, 0, 4);
    }
    
    // Log detailed header information for debugging
    websocket_debug(wsc, "PARSED HEADER: FIN=%d, RSV1=%d, RSV2=%d, RSV3=%d, OPCODE=0x%x, MASK=%d, LEN=%d, PAYLOAD_LEN=%llu, HEADER_SIZE=%zu, BUFFER_POS=%zu, BUFFER_LEN=%zu",
               header->fin, header->rsv1, header->rsv2, header->rsv3, 
               header->opcode, header->mask, header->len, 
               (unsigned long long)*payload_length, *header_size, 
               buffer - wsc->in_buffer->buffer, length);

    return true;
}

// Validate a parsed frame header according to WebSocket protocol rules
static bool websocket_protocol_validate_header(WEBSOCKET_SERVER_CLIENT *wsc, WEBSOCKET_FRAME_HEADER *header, 
                                            uint64_t payload_length, bool in_fragment_sequence) {
    if (!wsc || !header)
        return false;
        
    // Check RSV bits - must be 0 unless extensions are negotiated
    if (header->rsv2 || header->rsv3) {
        websocket_error(wsc, "Invalid frame: RSV2 or RSV3 bits set");
        return false;
    }
    
    // RSV1 is only valid if compression is enabled
    if (header->rsv1 && (!wsc->compression.enabled)) {
        websocket_error(wsc, "Invalid frame: RSV1 bit set but compression not enabled");
        return false;
    }
    
    // For continuation frames in a compressed message, RSV1 must be 0 per RFC 7692
    // Continuation frames for a compressed message must not have RSV1 set
    if (header->opcode == WS_OPCODE_CONTINUATION && in_fragment_sequence && header->rsv1) {
        websocket_error(wsc, "Invalid frame: Continuation frame should not have RSV1 bit set");
        return false;
    }
    
    // Check opcode validity
    switch (header->opcode) {
        case WS_OPCODE_CONTINUATION:
            // Continuation frames must be in a fragment sequence
            if (!in_fragment_sequence) {
                websocket_error(wsc, "Invalid frame: Continuation frame without initial frame");
                return false;
            }
            break;
            
        case WS_OPCODE_TEXT:
        case WS_OPCODE_BINARY:
            // New data frames cannot start inside a fragment sequence
            if (in_fragment_sequence) {
                websocket_error(wsc, "Invalid frame: New data frame during fragmented message");
                return false;
            }
            break;
            
        case WS_OPCODE_CLOSE:
        case WS_OPCODE_PING:
        case WS_OPCODE_PONG:
            // Control frames must not be fragmented
            if (!header->fin) {
                websocket_error(wsc, "Invalid frame: Fragmented control frame");
                return false;
            }
            
            // Control frames must have payload ≤ 125 bytes
            if (payload_length > 125) {
                websocket_error(wsc, "Invalid frame: Control frame payload too large (%llu bytes)",
                           (unsigned long long)payload_length);
                return false;
            }
            break;
            
        default:
            // Unknown opcode
            websocket_error(wsc, "Invalid frame: Unknown opcode: 0x%x", (unsigned int)header->opcode);
            return false;
    }
    
    // Validate payload length against limits
    if (payload_length > WS_MAX_FRAME_LENGTH) {
        websocket_error(wsc, "Invalid frame: Payload too large (%llu bytes)", 
                   (unsigned long long)payload_length);
        return false;
    }
    
    // All checks passed
    return true;
}

// Create a new message from an initial frame
static WEBSOCKET_MESSAGE *websocket_protocol_create_message_from_header(WEBSOCKET_SERVER_CLIENT *wsc, 
                                                                      WEBSOCKET_FRAME_HEADER *header,
                                                                      uint64_t payload_length) {
    if (!wsc || !header)
        return NULL;
        
    // Create new message
    WEBSOCKET_MESSAGE *msg = websocket_message_create(payload_length);

    // Initialize message from the frame
    msg->opcode = (WEBSOCKET_OPCODE)header->opcode;
    msg->is_compressed = header->rsv1 ? true : false;
    
    // Set message state based on FIN bit
    msg->complete = header->fin ? true : false;
    msg->in_fragmented_sequence = !header->fin;
    msg->total_length = payload_length; // Initial payload length
    
    return msg;
}

static void websocket_unmask(char *dst, const char *src, size_t length, const unsigned char *mask_key) {
    for (size_t i = 0; i < length; i++)
        dst[i] = (char)((unsigned char)src[i] ^ mask_key[i % 4]);
}

// Process a control frame directly without creating a message structure
static bool websocket_protocol_process_control_message(WEBSOCKET_SERVER_CLIENT *wsc, WEBSOCKET_OPCODE opcode,
                                                    char *payload, size_t payload_length,
                                                    bool is_masked, const unsigned char *mask_key) {
    websocket_debug(wsc, "Processing control frame opcode=0x%x, payload_length=%zu, is_masked=%d",
                  opcode, payload_length, is_masked);

    // If payload is masked, unmask it first
    if (is_masked && mask_key && payload && payload_length > 0)
        websocket_unmask(payload, payload, payload_length, mask_key);

    switch (opcode) {
        case WS_OPCODE_CLOSE: {
            uint16_t code = WS_CLOSE_NORMAL;
            char reason[124];
            reason[0] = '\0';  // Initialize reason string
            
            // Parse close code if present
            if (payload && payload_length >= 2) {
                code = ((uint16_t)((unsigned char)payload[0]) << 8) |
                       ((uint16_t)((unsigned char)payload[1]));
                
                // Validate close code
                if (!websocket_validate_close_code(code)) {
                    websocket_error(wsc, "Invalid close code: %u", code);
                    code = WS_CLOSE_PROTOCOL_ERROR;
                }
                
                // Extract reason if present
                if (payload_length > 2)
                    strncpyz(reason, payload + 2, MIN(payload_length - 2, sizeof(reason) - 1));
            }
            
            // Send close frame in response
            websocket_protocol_send_close(wsc, code, reason);
            
            // Update client state
            wsc->state = WS_STATE_CLOSING;
            
            // Call close callback if set
            if (wsc->on_close)
                wsc->on_close(wsc, code, reason);

            return true;
        }
        
        case WS_OPCODE_PING:
            // Ping frame - respond with pong
            websocket_debug(wsc, "Received PING frame with %zu bytes, responding with PONG", payload_length);
            
            // Send pong with the same payload
            return websocket_protocol_send_pong(wsc, payload, payload_length) > 0;

        case WS_OPCODE_PONG:
            // Pong frame - update last activity time
            websocket_debug(wsc, "Received PONG frame, updating last activity time");
            wsc->last_activity_t = now_monotonic_sec();
            return true;

        default:
            break;
    }

    websocket_error(wsc, "Unknown control opcode: %d", opcode);
    return false;
}

// Parse a frame from a buffer and append it to the current message if applicable.
// Returns one of the following:
// - WS_FRAME_ERROR: An error occurred, connection should be closed
// - WS_FRAME_NEED_MORE_DATA: More data is needed to complete the frame
// - WS_FRAME_COMPLETE: Frame was successfully parsed and handled, but is not a complete message yet
// - WS_FRAME_MESSAGE_READY: Message is ready for processing
static WEBSOCKET_FRAME_RESULT
websocket_protocol_consume_frame(WEBSOCKET_SERVER_CLIENT *wsc, char *data, size_t length, size_t *bytes_processed) {
    if (!wsc || !data || !length || !bytes_processed)
        return WS_FRAME_ERROR;
        
    size_t bytes = *bytes_processed; // Make a copy to avoid modifying the original unless successful

    // Local variables for frame processing
    WEBSOCKET_FRAME_HEADER header;
    uint64_t payload_length = 0;
    unsigned char mask_key[4];
    size_t header_size = 0;

    // Step 1: Parse the frame header
    if (!websocket_protocol_parse_header_from_buffer(wsc, data + bytes, length - bytes,
                                               &header, &payload_length, mask_key, &header_size)) {
        // Not enough data to parse a complete header
        return WS_FRAME_NEED_MORE_DATA;
    }
    
    if(payload_length + header_size > wsc->max_message_size)
        wsc->max_message_size = payload_length + header_size;

    // Check if we have enough data for the complete frame (header + payload)
    // If not, don't consume any bytes and wait for more data
    if (bytes + header_size + payload_length > length) {
        websocket_debug(wsc, "INCOMPLETE FRAME: header size %zu + payload %llu exceeds available data %zu. Have %zu bytes, need %zu more.",
                   header_size, (unsigned long long)payload_length, length - bytes, 
                   length - bytes, (bytes + header_size + payload_length) - length);
        return WS_FRAME_NEED_MORE_DATA;
    }
    
    // Step 2: Validate the frame header
    if (!websocket_protocol_validate_header(wsc, &header, payload_length, wsc->current_message != NULL)) {
        // Invalid frame, close the connection
        websocket_client_send_close(wsc, WS_CLOSE_PROTOCOL_ERROR, "Invalid frame header");
        return WS_FRAME_ERROR;
    }

    // Advance past the header
    bytes += header_size;
    
    if (websocket_frame_is_control_opcode(header.opcode)) {
        // Handle control frames (PING, PONG, CLOSE) directly
        websocket_debug(wsc, "Handling control frame: opcode=0x%x, payload_length=%llu",
                        (unsigned)header.opcode, (unsigned long long)payload_length);

        // Process the control frame with optional payload
        char *payload = (payload_length > 0) ? (data + bytes) : NULL;

        // Process control message directly without creating a message object
        if (!websocket_protocol_process_control_message(
                wsc, (WEBSOCKET_OPCODE)header.opcode,
                payload, (size_t)payload_length,
                header.mask ? true : false,
                mask_key)) {
            websocket_error(wsc, "Failed to process control frame");
            return WS_FRAME_ERROR;
        }

        // Update bytes processed
        if (payload_length > 0)
            bytes += (size_t)payload_length;

        *bytes_processed = bytes;

        return WS_FRAME_COMPLETE; // Return COMPLETE so we continue processing other frames
    }

    // Step 3: Handle the frame based on its opcode
    if (header.opcode == WS_OPCODE_CONTINUATION) {
        // This is a continuation frame - need an existing message
        if (!wsc->current_message) {
            websocket_error(wsc, "Received continuation frame with no message in progress");
            websocket_client_send_close(wsc, WS_CLOSE_PROTOCOL_ERROR, "Invalid continuation frame");
            return WS_FRAME_ERROR;
        }
        
        // If it's a zero-length frame, we don't need to append any data
        if (payload_length == 0) {
            // For zero-length non-final frames, just update and continue
            if (!header.fin) {
                // Non-final zero-length frame
                websocket_debug(wsc, "Zero-length non-final continuation frame");
                *bytes_processed = bytes;
                wsc->frame_id++;
                return WS_FRAME_COMPLETE;
            }
            
            // Final zero-length frame - mark message as complete
            *bytes_processed = bytes;
            return WS_FRAME_MESSAGE_READY;
        }

        // update the total length of the current message
        wsc->current_message->total_length += payload_length;
    }
    else {
        if(payload_length == 0) {
            websocket_debug(wsc, "Received data frame with zero-length payload (fin=%d)", header.fin);

            // Create an empty message to be processed
            WEBSOCKET_MESSAGE *msg = websocket_protocol_create_message_from_header(wsc, &header, 0);
            if (!msg) {
                websocket_error(wsc, "Failed to create message for empty payload");
                return WS_FRAME_ERROR;
            }

            // Store the message
            wsc->current_message = msg;

            // Check if this is a final frame
            if (header.fin) {
                // Final frame - mark message as complete and return message ready
                wsc->current_message->complete = true;
                *bytes_processed = bytes;
                return WS_FRAME_MESSAGE_READY;
            } else {
                // Non-final frame - continue to next frame
                *bytes_processed = bytes;
                wsc->frame_id++;
                return WS_FRAME_COMPLETE;
            }
        }

        // This is a new data frame (TEXT or BINARY)
        // If we have an existing message, it's an error
        if (wsc->current_message) {
            websocket_error(wsc, "Received new data frame while another message is in progress");
            websocket_client_send_close(wsc, WS_CLOSE_PROTOCOL_ERROR, "Invalid new data frame");
            return WS_FRAME_ERROR;
        }
        
        // Create a new message
        WEBSOCKET_MESSAGE *msg = websocket_protocol_create_message_from_header(wsc, &header, payload_length);
        if (!msg) {
            websocket_error(wsc, "Failed to create message");
            return WS_FRAME_ERROR;
        }
        
        // Store the message
        wsc->current_message = msg;
    }

    // Step 4: Append payload data to the message
    websocket_buffer_expand(&wsc->current_message->buffer, payload_length);

    char *dst = &wsc->current_message->buffer.data[wsc->current_message->buffer.length];
    if(header.mask)
        websocket_unmask(dst, data + header_size, payload_length, mask_key);
    else
        memcpy(dst, data + header_size, payload_length);

    wsc->current_message->buffer.length += payload_length;

    // Step 5: At this point, we know we've processed a complete frame
    wsc->frame_id++;

    bytes += payload_length;
    *bytes_processed = bytes;

    // If this is a final frame, mark the message as complete
    if (header.fin)
        return WS_FRAME_MESSAGE_READY;

    // Non-final frame, message is incomplete - move to next frame
    return WS_FRAME_COMPLETE;
}

// Process incoming data from the WebSocket client
// This function's job is to:
// 1. Consume frames from the input buffer 
// 2. Build messages
// 3. Process complete messages
bool websocket_protocol_got_data(WEBSOCKET_SERVER_CLIENT *wsc, char *data, size_t length, size_t *bytes_processed) {
    if (!wsc || !data || !length || !bytes_processed)
        return false;
    
    // Initialize output parameter
    *bytes_processed = 0;
    
    // Keep processing frames until we can't process any more
    size_t entry_bytes = *bytes_processed;
    while (*bytes_processed < length) {

        // Try to consume one complete frame
        size_t before = *bytes_processed;
        WEBSOCKET_FRAME_RESULT result = websocket_protocol_consume_frame(wsc, data, length, bytes_processed);
        size_t consumed = *bytes_processed - before;
        
        websocket_debug(wsc, "Frame processing result: %d, bytes_processed: %zu, consumed: %zu, total processed: %zu/%zu", 
                      result, *bytes_processed, consumed, *bytes_processed - entry_bytes, length);
        
        // Safety check to ensure we always move forward in the buffer
        if (consumed == 0 && result != WS_FRAME_NEED_MORE_DATA) {
            // If we're processing a frame but not consuming bytes, we might be stuck
            websocket_error(wsc, "Protocol processing stalled - consumed 0 bytes but not waiting for more data");
            return false;
        }
        
        switch (result) {
            case WS_FRAME_ERROR:
                // Error occurred during frame processing
                websocket_error(wsc, "Error processing WebSocket frame");
                return false;
                
            case WS_FRAME_NEED_MORE_DATA:
                // Need more data to complete the current frame
                websocket_debug(wsc, "Need more data to complete the current frame");
                return true;
                
            case WS_FRAME_COMPLETE:
                // Frame was processed successfully, but more frames are needed for a complete message
                websocket_debug(wsc, "Frame complete, but message not yet complete");
                continue;
                
            case WS_FRAME_MESSAGE_READY:
                wsc->current_message->complete = true;

                if (!websocket_protocol_process_message(wsc)) {
                    websocket_error(wsc, "Failed to process message");
                    return WS_FRAME_ERROR;
                }
                websocket_protocol_message_done(wsc);
                continue;
        }
    }
    
    return true;
}

