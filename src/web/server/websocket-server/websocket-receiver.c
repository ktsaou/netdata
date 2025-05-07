#include "daemon/common.h"
#include "websocket-internal.h"

// Process incoming WebSocket data
int websocket_receive_data(WEBSOCKET_SERVER_CLIENT *wsc) {
    if(!wsc || !wsc->in_buffer || wsc->sock.fd < 0)
        return -1;

    // Log current buffer state
    netdata_log_debug(D_WEBSOCKET, "Client %zu - Before receive: buffer has %zu bytes",
                     wsc->id, buffer_strlen(wsc->in_buffer));

    // Allocate buffer space for reading - ensure we have enough space
    buffer_need_bytes(wsc->in_buffer, 4096);
    char *buffer_pos = &wsc->in_buffer->buffer[wsc->in_buffer->len];
    
    // Read data from socket into buffer using ND_SOCK
    ssize_t bytes_read = nd_sock_read(&wsc->sock, buffer_pos, 4096, 1); // 1 retry for non-blocking read

    if(bytes_read <= 0) {
        if(bytes_read == 0) {
            // Connection closed
            netdata_log_info("WEBSOCKET: Client %zu closed connection", wsc->id);
            return -1;
        }

        if(errno == EAGAIN || errno == EWOULDBLOCK)
            return 0; // No data available right now

        netdata_log_error("WEBSOCKET: Failed to read from client %zu: %s", wsc->id, strerror(errno));
        return -1;
    }

    // Update buffer length
    wsc->in_buffer->len += bytes_read;
    
    // Log received data for debugging
    char hex_dump[128] = "";
    for(ssize_t i = 0; i < bytes_read && i < 16; i++) {
        char *pos = hex_dump + strlen(hex_dump);
        snprintf(pos, sizeof(hex_dump) - strlen(hex_dump), "%02x ", (unsigned char)buffer_pos[i]);
    }
    netdata_log_debug(D_WEBSOCKET, "Client %zu - Received %zd bytes: %s",
                    wsc->id, bytes_read, hex_dump);
    
    // Update last activity time
    wsc->last_activity_t = now_monotonic_sec();

    // Process frames
    int processed = websocket_process_frames_buffered(wsc);
    
    return processed;
}

// Function to unmask a WebSocket frame
static void websocket_unmask_payload(unsigned char *payload, size_t payload_len, const unsigned char mask_key[4]) {
    for(size_t i = 0; i < payload_len; i++) {
        payload[i] ^= mask_key[i % 4];
    }
}

// Process all complete frames in the buffer
int websocket_process_frames_buffered(WEBSOCKET_SERVER_CLIENT *wsc) {
    if(!wsc || !wsc->in_buffer)
        return -1;

    int frames_processed = 0;
    bool process_more = true;

    while(process_more && buffer_strlen(wsc->in_buffer) > 0) {
        // If we're not currently processing a frame, start a new one
        if(!wsc->frame_in_progress) {
            // We need at least 2 bytes for the basic frame header
            if(buffer_strlen(wsc->in_buffer) < 2) {
                process_more = false;
                continue;
            }

            // Read the basic frame header (2 bytes)
            unsigned char *data = (unsigned char *)buffer_tostring(wsc->in_buffer);
            
            // Manually extract frame header bits for better reliability
            unsigned char byte1 = data[0];
            unsigned char byte2 = data[1];
            
            // Extract bits from first byte
            unsigned char fin = (byte1 >> 7) & 0x01;
            unsigned char rsv1 = (byte1 >> 6) & 0x01;
            unsigned char rsv2 = (byte1 >> 5) & 0x01;
            unsigned char rsv3 = (byte1 >> 4) & 0x01;
            unsigned char opcode = byte1 & 0x0F;
            
            // Extract bits from second byte
            unsigned char mask = (byte2 >> 7) & 0x01;
            unsigned char len = byte2 & 0x7F;
            
            // Create and populate header structure
            WEBSOCKET_FRAME_HEADER header;
            header.fin = fin;
            header.rsv1 = rsv1;
            header.rsv2 = rsv2;
            header.rsv3 = rsv3;
            header.opcode = opcode;
            header.mask = mask;
            header.len = len;
            
            // Log frame header details for debugging
            netdata_log_debug(D_WEBSOCKET, "Client %zu - Frame header: FIN=%d, RSV1=%d, RSV2=%d, RSV3=%d, OPCODE=%d, MASK=%d, LEN=%d",
                           wsc->id, fin, rsv1, rsv2, rsv3, opcode, mask, len);
            
            // Validate RSV bits - if compression is enabled, RSV1 may be set for data frames
            bool is_compressed = false;
            
            if (rsv1) {
                // RSV1 is set - check if compression is enabled
                if (wsc->compression.enabled && (opcode == WS_OPCODE_TEXT || opcode == WS_OPCODE_BINARY)) {
                    // RSV1 is valid for compression on data frames
                    is_compressed = true;
                    netdata_log_debug(D_WEBSOCKET, "Client %zu - Received compressed frame (opcode=%d)", 
                                  wsc->id, opcode);
                } else {
                    // RSV1 set but compression not enabled or not a data frame
                    netdata_log_error("WEBSOCKET: Client %zu - Invalid RSV1 bit in frame - compression not enabled or not allowed for this frame type",
                                  wsc->id);
                    websocket_close_client(wsc, WS_CLOSE_PROTOCOL_ERROR, "Invalid RSV1 bit");
                    process_more = false;
                    frames_processed = -1;
                    break;
                }
            }
            
            // RSV2 and RSV3 must always be 0
            if (rsv2 || rsv3) {
                netdata_log_error("WEBSOCKET: Client %zu - Invalid RSV bits in frame (RSV2=%d, RSV3=%d) - protocol violation",
                              wsc->id, rsv2, rsv3);
                websocket_close_client(wsc, WS_CLOSE_PROTOCOL_ERROR, "Invalid RSV bits");
                process_more = false;
                frames_processed = -1;
                break;
            }
            
            // Initialize frame processing state
            wsc->current_frame = header;
            wsc->frame_in_progress = true;
            wsc->frame_length = len;
            wsc->frame_read = 0;
            
            // Validate control frames according to the WebSocket spec
            if (opcode >= 0x8) { // Control frames: 0x8 (Close), 0x9 (Ping), 0xA (Pong)
                // Control frames must not be fragmented
                if (!fin) {
                    netdata_log_error("WEBSOCKET: Client %zu - Fragmented control frame (opcode=%d) - protocol violation", 
                                  wsc->id, opcode);
                    websocket_close_client(wsc, WS_CLOSE_PROTOCOL_ERROR, "Fragmented control frame");
                    process_more = false;
                    frames_processed = -1;
                    break;
                }
                
                // Control frames must have a payload length of 125 bytes or less
                if (len > 125 || (len == 126 && wsc->frame_length > 125) || (len == 127)) {
                    netdata_log_error("WEBSOCKET: Client %zu - Control frame too large: %zu bytes (max: 125)", 
                                  wsc->id, wsc->frame_length);
                    websocket_close_client(wsc, WS_CLOSE_PROTOCOL_ERROR, "Control frame too large");
                    process_more = false;
                    frames_processed = -1;
                    break;
                }
            }

            // Determine full frame length based on payload length indicator
            size_t header_size = 2; // Basic 2-byte header
            
            if(len == 126) {
                // 16-bit extended payload length
                if(buffer_strlen(wsc->in_buffer) < 4) {
                    // Not enough data yet
                    process_more = false;
                    continue;
                }
                
                wsc->frame_length = (data[2] << 8) | data[3];
                header_size += 2;
                
                netdata_log_debug(D_WEBSOCKET, "Client %zu - Extended payload length (16-bit): %zu", 
                                wsc->id, wsc->frame_length);
                                
                // Check for excessive frame size
                if (wsc->frame_length > WS_MAX_FRAME_LENGTH) {
                    netdata_log_error("WEBSOCKET: Client %zu - Frame too large: %zu bytes (max: %d)", 
                                  wsc->id, wsc->frame_length, WS_MAX_FRAME_LENGTH);
                    websocket_close_client(wsc, WS_CLOSE_MESSAGE_TOO_BIG, "Frame too large");
                    process_more = false;
                    frames_processed = -1;
                    break;
                }
            }
            else if(len == 127) {
                // 64-bit extended payload length
                if(buffer_strlen(wsc->in_buffer) < 10) {
                    // Not enough data yet
                    process_more = false;
                    continue;
                }
                
                // Read 64-bit length as 8 bytes in network byte order (big-endian)
                wsc->frame_length = 0;
                for(int i = 0; i < 8; i++) {
                    wsc->frame_length = (wsc->frame_length << 8) | data[2 + i];
                }
                header_size += 8;
                
                netdata_log_debug(D_WEBSOCKET, "Client %zu - Extended payload length (64-bit): %zu", 
                                wsc->id, wsc->frame_length);
                                
                // Check for excessive frame size
                if (wsc->frame_length > WS_MAX_FRAME_LENGTH) {
                    netdata_log_error("WEBSOCKET: Client %zu - Frame too large: %zu bytes (max: %d)", 
                                  wsc->id, wsc->frame_length, WS_MAX_FRAME_LENGTH);
                    websocket_close_client(wsc, WS_CLOSE_MESSAGE_TOO_BIG, "Frame too large");
                    process_more = false;
                    frames_processed = -1;
                    break;
                }
            }

            // Get masking key if frame is masked
            if(mask) {
                if(buffer_strlen(wsc->in_buffer) < header_size + 4) {
                    // Not enough data for masking key
                    process_more = false;
                    continue;
                }
                
                // Read 4-byte masking key
                memcpy(wsc->mask_key, data + header_size, 4);
                header_size += 4;
                
                netdata_log_debug(D_WEBSOCKET, "Client %zu - Mask key: %02x %02x %02x %02x", 
                               wsc->id, 
                               wsc->mask_key[0], wsc->mask_key[1], 
                               wsc->mask_key[2], wsc->mask_key[3]);
            }

            // Check if we have enough data for the complete frame
            if(buffer_strlen(wsc->in_buffer) < header_size + wsc->frame_length) {
                // Not enough data for complete frame
                netdata_log_debug(D_WEBSOCKET, "Client %zu - Partial frame: buffer has %zu bytes, need %zu bytes (header=%zu, payload=%zu)",
                               wsc->id, buffer_strlen(wsc->in_buffer), header_size + wsc->frame_length, 
                               header_size, wsc->frame_length);
                
                // Also log the first few bytes to help debug
                char hex_dump[128] = "";
                for(size_t i = 0; i < buffer_strlen(wsc->in_buffer) && i < 16; i++) {
                    char *pos = hex_dump + strlen(hex_dump);
                    snprintf(pos, sizeof(hex_dump) - strlen(hex_dump), "%02x ", (unsigned char)wsc->in_buffer->buffer[i]);
                }
                netdata_log_debug(D_WEBSOCKET, "Client %zu - Frame bytes: %s", wsc->id, hex_dump);
                
                process_more = false;
                break; // Exit the loop instead of continue
            }

            // Process the frame
            unsigned char *payload = data + header_size;
            
            // Unmask payload if needed
            if(mask) {
                websocket_unmask_payload(payload, wsc->frame_length, wsc->mask_key);
            }
            
            // Decompress payload if needed (RSV1 bit set and compression enabled)
            char *decompressed_payload = NULL;
            size_t decompressed_len = 0;
            size_t original_frame_length = wsc->frame_length; // Keep original for buffer management
            
            if(is_compressed && wsc->compression.enabled) {
                // Log compression info before attempting decompression
                netdata_log_debug(D_WEBSOCKET, "Client %zu - Attempting to decompress payload of %zu bytes",
                              wsc->id, wsc->frame_length);
                
                int result = websocket_decompress_payload(wsc, (const char *)payload, wsc->frame_length, 
                                                     &decompressed_payload, &decompressed_len);
                if(result > 0) {
                    netdata_log_debug(D_WEBSOCKET, "Client %zu - Successfully decompressed payload from %zu to %zu bytes",
                                  wsc->id, wsc->frame_length, decompressed_len);
                    
                    // Use decompressed payload but keep original frame_length for buffer management
                    payload = (unsigned char *)decompressed_payload;
                    
                    // For debugging, store the old size temporarily
                    size_t old_size = wsc->frame_length;
                    
                    // Update size for message processing
                    wsc->frame_length = decompressed_len;
                    
                    netdata_log_debug(D_WEBSOCKET, "Client %zu - Frame size updated from %zu to %zu bytes for processing",
                                  wsc->id, old_size, wsc->frame_length);
                } else {
                    netdata_log_error("WEBSOCKET: Client %zu - Failed to decompress frame payload of size %zu", 
                                  wsc->id, wsc->frame_length);
                    websocket_close_client(wsc, WS_CLOSE_INTERNAL_ERROR, "Decompression failed");
                    process_more = false;
                    frames_processed = -1;
                    break;
                }
            }

            // Handle different frame types
            switch(opcode) {
                case WS_OPCODE_CONTINUATION:
                    // Handle continuation of a fragmented message
                    if (!wsc->fragmented_message) {
                        netdata_log_error("WEBSOCKET: Client %zu - Received continuation frame without starting frame", wsc->id);
                        break;
                    }
                    
                    // Append this payload to the message buffer
                    buffer_need_bytes(wsc->message_buffer, wsc->frame_length);
                    buffer_memcat(wsc->message_buffer, (char *)payload, wsc->frame_length);
                    
                    netdata_log_debug(D_WEBSOCKET, "Client %zu - Added %zu bytes to fragmented message (total: %zu)",
                                  wsc->id, wsc->frame_length, buffer_strlen(wsc->message_buffer));
                    
                    // If this is the final frame of the message, process it
                    if (fin) {
                        // Process the complete message
                        netdata_log_debug(D_WEBSOCKET, "Client %zu - Final continuation frame received, message complete",
                                      wsc->id);
                        
                        if (wsc->on_message) {
                            wsc->on_message(wsc, buffer_tostring(wsc->message_buffer), 
                                         buffer_strlen(wsc->message_buffer), 
                                         wsc->fragmented_opcode);
                        }
                        else {
                            // Default behavior: echo the message back
                            netdata_log_debug(D_WEBSOCKET, "Client %zu - No message handler registered, echoing fragmented message back",
                                         wsc->id);
                            websocket_send_frame(wsc, buffer_tostring(wsc->message_buffer),
                                            buffer_strlen(wsc->message_buffer),
                                            wsc->fragmented_opcode);
                        }
                        
                        // Reset fragmented message state
                        wsc->fragmented_message = false;
                        buffer_flush(wsc->message_buffer);
                    }
                    break;

                case WS_OPCODE_TEXT:
                case WS_OPCODE_BINARY:
                    // Log the received message for debugging
                    {
                        char msg_preview[64] = "";
                        size_t copy_len = wsc->frame_length < sizeof(msg_preview) - 1 ? 
                                        wsc->frame_length : sizeof(msg_preview) - 1;
                        memcpy(msg_preview, payload, copy_len);
                        msg_preview[copy_len] = '\0';
                        
                        netdata_log_debug(D_WEBSOCKET, "Client %zu - Received message: '%s'", 
                                       wsc->id, msg_preview);
                    }
                    
                    // Check if this is a fragmented message (FIN bit not set)
                    if (!fin) {
                        // Start of a fragmented message
                        wsc->fragmented_message = true;
                        wsc->fragmented_opcode = opcode;
                        
                        // Reset and initialize the message buffer
                        buffer_flush(wsc->message_buffer);
                        buffer_need_bytes(wsc->message_buffer, wsc->frame_length);
                        buffer_memcat(wsc->message_buffer, (char *)payload, wsc->frame_length);
                        
                        netdata_log_debug(D_WEBSOCKET, "Client %zu - Started fragmented %s message (%zu bytes)",
                                      wsc->id, 
                                      (opcode == WS_OPCODE_TEXT) ? "text" : "binary",
                                      wsc->frame_length);
                    }
                    else {
                        // This is a complete, unfragmented message
                        // Call message handler if registered
                        if(wsc->on_message) {
                            wsc->on_message(wsc, (char *)payload, wsc->frame_length, opcode);
                        }
                        else {
                            // Default behavior: echo the message back as-is
                            netdata_log_debug(D_WEBSOCKET, "Client %zu - No message handler registered, echoing back", wsc->id);
                            websocket_send_frame(wsc, (char *)payload, wsc->frame_length, opcode);
                        }
                    }
                    break;

                case WS_OPCODE_CLOSE:
                    // Handle close frame
                    {
                        uint16_t close_code = 1000; // Normal closure
                        const char *reason = "";
                        
                        // Extract close code and reason if present
                        if(wsc->frame_length >= 2) {
                            close_code = (payload[0] << 8) | payload[1];
                            
                            // Validate close code according to RFC 6455
                            if (!websocket_validate_close_code(close_code)) {
                                netdata_log_error("WEBSOCKET: Client %zu sent invalid close code: %d", 
                                              wsc->id, close_code);
                                // Use protocol error as response code
                                close_code = WS_CLOSE_PROTOCOL_ERROR;
                                reason = "Invalid close code";
                            }
                            else if(wsc->frame_length > 2) {
                                reason = (char *)payload + 2;
                                
                                // Validate that reason text is valid UTF-8 (per RFC 6455)
                                // Assuming that we trust our implementation to be correct,
                                // we just check for null bytes in the middle of the string
                                size_t reason_len = wsc->frame_length - 2;
                                bool valid = true;
                                for (size_t i = 0; i < reason_len; i++) {
                                    if (reason[i] == '\0' && i < reason_len - 1) {
                                        valid = false;
                                        break;
                                    }
                                }
                                if (!valid) {
                                    netdata_log_error("WEBSOCKET: Client %zu sent invalid UTF-8 in close reason", 
                                                  wsc->id);
                                    close_code = WS_CLOSE_PROTOCOL_ERROR;
                                    reason = "Invalid UTF-8 in close reason";
                                }
                            }
                        }
                        
                        netdata_log_info("WEBSOCKET: Client %zu sent close frame (code: %d, reason: %s)",
                             wsc->id, close_code, reason);
                        
                        // Call close handler if registered
                        if(wsc->on_close) {
                            wsc->on_close(wsc, close_code, reason);
                        }
                        
                        // Echo the close frame back to the client (with potentially corrected code)
                        websocket_close_client(wsc, close_code, reason);
                        
                        // Mark for disconnection
                        process_more = false;
                        frames_processed = -1;
                    }
                    break;

                case WS_OPCODE_PING:
                    // Respond with pong - use pre-declared function from websocket.c
                    {
                        char *ping_payload = (char *)payload;
                        size_t ping_len = wsc->frame_length;
                        websocket_send_frame(wsc, ping_payload, ping_len, WS_OPCODE_PONG);
                    }
                    break;

                case WS_OPCODE_PONG:
                    // Update last activity time
                    wsc->last_activity_t = now_monotonic_sec();
                    break;

                default:
                    netdata_log_error("WEBSOCKET: Client %zu sent unknown frame type: %d", wsc->id, opcode);
                    break;
            }

            // Remove processed frame from buffer (shift remaining data to the front)
            // When decompression happens, we need to use the original compressed length for buffer management
            size_t processed_size = header_size + (is_compressed ? original_frame_length : wsc->frame_length);
            
            // Make sure we don't overflow the buffer
            if (processed_size > wsc->in_buffer->len) {
                netdata_log_error("WEBSOCKET: Client %zu - Buffer processing error, processed_size (%zu) > buffer length (%zu)",
                              wsc->id, processed_size, (size_t)wsc->in_buffer->len);
                processed_size = wsc->in_buffer->len;
            }
            
            size_t remaining = wsc->in_buffer->len - processed_size;
            if (remaining > 0) {
                memmove(wsc->in_buffer->buffer, 
                        wsc->in_buffer->buffer + processed_size, 
                        remaining);
            }
            wsc->in_buffer->len = remaining;
            
            // Free decompressed payload if allocated
            if (decompressed_payload)
                freez(decompressed_payload);
            
            // Reset frame processing state
            wsc->frame_in_progress = false;
            frames_processed++;
            
            // Log remaining buffer size after processing this frame
            netdata_log_debug(D_WEBSOCKET, "Client %zu - Frame processed, %zu bytes remaining in buffer",
                          wsc->id, (size_t)wsc->in_buffer->len);
        }
        else {
            // Continue processing an existing frame (for large payloads split across multiple TCP packets)
            netdata_log_debug(D_WEBSOCKET, "Client %zu - Continuing with frame_in_progress, buffer has %zu bytes",
                           wsc->id, buffer_strlen(wsc->in_buffer));
            
            // Check if this is a compressed frame (RSV1 bit set)
            bool is_compressed = (wsc->current_frame.rsv1 == 1);
            
            // Check if we have enough data to complete the frame
            if(buffer_strlen(wsc->in_buffer) < wsc->frame_length - wsc->frame_read) {
                // Not enough data yet
                netdata_log_debug(D_WEBSOCKET, "Client %zu - Not enough data for full payload: have %zu, need %zu more",
                               wsc->id, buffer_strlen(wsc->in_buffer), wsc->frame_length - wsc->frame_read);
                process_more = false;
                break;
            }
            
            // Now we have enough data to complete the frame
            netdata_log_debug(D_WEBSOCKET, "Client %zu - Continuing frame with %zu more bytes of data",
                          wsc->id, buffer_strlen(wsc->in_buffer));
            
            // Get the payload from the buffer
            unsigned char *data = (unsigned char *)buffer_tostring(wsc->in_buffer);
            unsigned char *payload = data;
            size_t payload_length = wsc->frame_length - wsc->frame_read;
            
            // Unmask if needed (using the mask key saved from the header)
            if(wsc->current_frame.mask) {
                websocket_unmask_payload(payload, payload_length, wsc->mask_key);
            }
            
            // Decompress payload if needed (RSV1 bit set and compression enabled)
            char *decompressed_payload = NULL;
            size_t decompressed_len = 0;
            size_t original_payload_length = payload_length; // Keep original length for buffer management
            
            if(is_compressed && wsc->compression.enabled) {
                // Log compression info before attempting decompression
                netdata_log_debug(D_WEBSOCKET, "Client %zu - Attempting to decompress continuation payload of %zu bytes",
                              wsc->id, payload_length);
                
                int result = websocket_decompress_payload(wsc, (const char *)payload, payload_length, 
                                                     &decompressed_payload, &decompressed_len);
                if(result > 0) {
                    netdata_log_debug(D_WEBSOCKET, "Client %zu - Successfully decompressed continuation payload from %zu to %zu bytes",
                                  wsc->id, payload_length, decompressed_len);
                    
                    // Use decompressed payload but keep original length for buffer management
                    payload = (unsigned char *)decompressed_payload;
                    payload_length = decompressed_len;
                } else {
                    netdata_log_error("WEBSOCKET: Client %zu - Failed to decompress continuation frame payload of size %zu", 
                                  wsc->id, payload_length);
                    websocket_close_client(wsc, WS_CLOSE_INTERNAL_ERROR, "Decompression failed");
                    process_more = false;
                    frames_processed = -1;
                    break;
                }
            }
            
            // Process based on opcode from saved header
            WEBSOCKET_OPCODE opcode = wsc->current_frame.opcode;
            bool fin = wsc->current_frame.fin;
            
            // Similar processing as in the main switch statement
            switch(opcode) {
                case WS_OPCODE_CONTINUATION:
                    if(wsc->fragmented_message) {
                        // Append this payload to the message buffer
                        buffer_need_bytes(wsc->message_buffer, payload_length);
                        buffer_memcat(wsc->message_buffer, (char *)payload, payload_length);
                        
                        netdata_log_debug(D_WEBSOCKET, "Client %zu - Added %zu bytes to fragmented message (total: %zu)",
                                      wsc->id, payload_length, buffer_strlen(wsc->message_buffer));
                        
                        // If this is the final frame of the message, process it
                        if(fin) {
                            netdata_log_debug(D_WEBSOCKET, "Client %zu - Final continuation frame completed, message complete",
                                          wsc->id);
                            
                            if(wsc->on_message) {
                                wsc->on_message(wsc, buffer_tostring(wsc->message_buffer), 
                                             buffer_strlen(wsc->message_buffer), 
                                             wsc->fragmented_opcode);
                            }
                            else {
                                // Default behavior: echo the message back
                                netdata_log_debug(D_WEBSOCKET, "Client %zu - No message handler registered, echoing fragmented message back", 
                                             wsc->id);
                                websocket_send_frame(wsc, buffer_tostring(wsc->message_buffer),
                                                buffer_strlen(wsc->message_buffer),
                                                wsc->fragmented_opcode);
                            }
                            
                            // Reset fragmented message state
                            wsc->fragmented_message = false;
                            buffer_flush(wsc->message_buffer);
                        }
                    }
                    break;
                    
                case WS_OPCODE_TEXT:
                case WS_OPCODE_BINARY:
                    if(!fin) {
                        // This is part of a fragmented message
                        if(!wsc->fragmented_message) {
                            // Start of a new fragmented message
                            wsc->fragmented_message = true;
                            wsc->fragmented_opcode = opcode;
                            buffer_flush(wsc->message_buffer);
                        }
                        
                        // Add payload to message buffer
                        buffer_need_bytes(wsc->message_buffer, payload_length);
                        buffer_memcat(wsc->message_buffer, (char *)payload, payload_length);
                        
                        netdata_log_debug(D_WEBSOCKET, "Client %zu - Added %zu bytes to fragmented message (total: %zu)",
                                      wsc->id, payload_length, buffer_strlen(wsc->message_buffer));
                    }
                    else {
                        // This is a complete message
                        char msg_preview[64] = "";
                        size_t copy_len = payload_length < sizeof(msg_preview) - 1 ? 
                                         payload_length : sizeof(msg_preview) - 1;
                        memcpy(msg_preview, payload, copy_len);
                        msg_preview[copy_len] = '\0';
                        
                        netdata_log_debug(D_WEBSOCKET, "Client %zu - Completed full message: '%s'", 
                                       wsc->id, msg_preview);
                        
                        if(wsc->on_message) {
                            wsc->on_message(wsc, (char *)payload, payload_length, opcode);
                        }
                        else {
                            // Echo back
                            netdata_log_debug(D_WEBSOCKET, "Client %zu - No message handler registered, echoing back", wsc->id);
                            websocket_send_frame(wsc, (char *)payload, payload_length, opcode);
                        }
                    }
                    break;
                    
                case WS_OPCODE_PING:
                    // Respond with pong
                    websocket_send_frame(wsc, (char *)payload, payload_length, WS_OPCODE_PONG);
                    break;
                    
                case WS_OPCODE_PONG:
                    // Just update activity timestamp
                    wsc->last_activity_t = now_monotonic_sec();
                    break;
                    
                case WS_OPCODE_CLOSE:
                    {
                        uint16_t close_code = 1000; // Normal closure
                        const char *reason = "";
                        
                        if(payload_length >= 2) {
                            close_code = (payload[0] << 8) | payload[1];
                            
                            if(payload_length > 2) {
                                reason = (char *)payload + 2;
                            }
                        }
                        
                        netdata_log_info("WEBSOCKET: Client %zu sent close frame (code: %d, reason: %s)",
                                         wsc->id, close_code, reason);
                        
                        if(wsc->on_close) {
                            wsc->on_close(wsc, close_code, reason);
                        }
                        
                        process_more = false;
                        frames_processed = -1;
                    }
                    break;
                    
                default:
                    netdata_log_error("WEBSOCKET: Client %zu sent unknown frame type: %d", wsc->id, opcode);
                    break;
            }
            
            // Remove processed bytes from buffer
            // When decompression happens, we need to use the original compressed length for buffer management
            size_t processed_size = (is_compressed ? original_payload_length : payload_length);
            
            // Make sure we don't overflow the buffer
            if (processed_size > wsc->in_buffer->len) {
                netdata_log_error("WEBSOCKET: Client %zu - Buffer processing error, processed_size (%zu) > buffer length (%zu)",
                              wsc->id, processed_size, (size_t)wsc->in_buffer->len);
                processed_size = wsc->in_buffer->len;
            }
            
            size_t remaining = wsc->in_buffer->len - processed_size;
            if(remaining > 0) {
                memmove(wsc->in_buffer->buffer, 
                        wsc->in_buffer->buffer + processed_size, 
                        remaining);
            }
            wsc->in_buffer->len = remaining;
            
            // Free decompressed payload if it was allocated
            if (is_compressed && decompressed_payload) {
                freez(decompressed_payload);
                decompressed_payload = NULL;
            }
            
            // Free decompressed payload if it was allocated
            if (is_compressed && decompressed_payload) {
                freez(decompressed_payload);
                decompressed_payload = NULL;
            }
            
            // Reset frame processing state
            wsc->frame_in_progress = false;
            frames_processed++;
            
            netdata_log_debug(D_WEBSOCKET, "Client %zu - Frame completed, %zu bytes remaining in buffer",
                          wsc->id, (size_t)wsc->in_buffer->len);
        }
    }

    return frames_processed;
}

// Actually write data to the client socket
int websocket_write_data(WEBSOCKET_SERVER_CLIENT *wsc) {
    if(!wsc || !wsc->out_buffer || wsc->sock.fd < 0 || buffer_strlen(wsc->out_buffer) == 0)
        return 0;

    // In the websocket thread we want non-blocking behavior
    // Use nd_sock_write with a single retry
    ssize_t bytes_written = nd_sock_write(&wsc->sock, buffer_tostring(wsc->out_buffer), buffer_strlen(wsc->out_buffer), 1); // 1 retry for non-blocking write

    if(bytes_written < 0) {
        if(errno == EAGAIN || errno == EWOULDBLOCK) {
            // Would block, try later
            // Keep WRITE flags set so we'll try again when socket is ready
            if(wsc->thread && wsc->sock.fd >= 0) {
                websocket_thread_update_client_poll_flags(wsc->thread, wsc, ND_POLL_READ | ND_POLL_WRITE);
            }
            return 0;
        }

        netdata_log_error("WEBSOCKET: Failed to write to client %zu: %s", wsc->id, strerror(errno));
        return -1;
    }

    // Consume written bytes (shift remaining data to the front of the buffer)
    if (bytes_written > 0) {
        size_t remaining = wsc->out_buffer->len - bytes_written;
        if (remaining > 0) {
            memmove(wsc->out_buffer->buffer, 
                    wsc->out_buffer->buffer + bytes_written, 
                    remaining);
            wsc->out_buffer->len = remaining;
            
            // Still have data to write - keep ND_POLL_WRITE set
            if(wsc->thread && wsc->sock.fd >= 0) {
                websocket_thread_update_client_poll_flags(wsc->thread, wsc, ND_POLL_READ | ND_POLL_WRITE);
            }
        } else {
            // All data written - remove ND_POLL_WRITE
            wsc->out_buffer->len = 0;
            if(wsc->thread && wsc->sock.fd >= 0) {
                websocket_thread_update_client_poll_flags(wsc->thread, wsc, ND_POLL_READ);
            }
        }
    }

    return bytes_written;
}
