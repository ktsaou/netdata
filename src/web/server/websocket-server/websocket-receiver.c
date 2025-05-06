#include "daemon/common.h"
#include "websocket-internal.h"

// Process incoming WebSocket data
int websocket_receive_data(WEBSOCKET_SERVER_CLIENT *wsc) {
    if(!wsc || !wsc->in_buffer || wsc->sock.fd < 0)
        return -1;

    // Allocate buffer space for reading
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
            WEBSOCKET_FRAME_HEADER *header = (WEBSOCKET_FRAME_HEADER *)data;
            
            // Initialize frame processing state
            wsc->current_frame = *header;
            wsc->frame_in_progress = true;
            wsc->frame_length = header->len;
            wsc->frame_read = 0;

            // Determine full frame length based on payload length indicator
            size_t header_size = 2; // Basic 2-byte header
            
            if(header->len == 126) {
                // 16-bit extended payload length
                if(buffer_strlen(wsc->in_buffer) < 4) {
                    // Not enough data yet
                    process_more = false;
                    continue;
                }
                
                wsc->frame_length = (data[2] << 8) | data[3];
                header_size += 2;
            }
            else if(header->len == 127) {
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
            }

            // Get masking key if frame is masked
            if(header->mask) {
                if(buffer_strlen(wsc->in_buffer) < header_size + 4) {
                    // Not enough data for masking key
                    process_more = false;
                    continue;
                }
                
                // Read 4-byte masking key
                memcpy(wsc->mask_key, data + header_size, 4);
                header_size += 4;
            }

            // Check if we have enough data for the complete frame
            if(buffer_strlen(wsc->in_buffer) < header_size + wsc->frame_length) {
                // Not enough data for complete frame
                process_more = false;
                continue;
            }

            // Process the frame
            unsigned char *payload = data + header_size;
            
            // Unmask payload if needed
            if(header->mask) {
                websocket_unmask_payload(payload, wsc->frame_length, wsc->mask_key);
            }

            // Handle different frame types
            switch(header->opcode) {
                case WS_OPCODE_CONTINUATION:
                    // TODO: Handle continuation frames
                    break;

                case WS_OPCODE_TEXT:
                case WS_OPCODE_BINARY:
                    // Call message handler if registered
                    if(wsc->on_message) {
                        wsc->on_message(wsc, (char *)payload, wsc->frame_length, header->opcode);
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
                            
                            if(wsc->frame_length > 2) {
                                reason = (char *)payload + 2;
                            }
                        }
                        
                        netdata_log_info("WEBSOCKET: Client %zu sent close frame (code: %d, reason: %s)",
                             wsc->id, close_code, reason);
                        
                        // Call close handler if registered
                        if(wsc->on_close) {
                            wsc->on_close(wsc, close_code, reason);
                        }
                        
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
                    netdata_log_error("WEBSOCKET: Client %zu sent unknown frame type: %d", wsc->id, header->opcode);
                    break;
            }

            // Remove processed frame from buffer (shift remaining data to the front)
            size_t processed_size = header_size + wsc->frame_length;
            size_t remaining = wsc->in_buffer->len - processed_size;
            if (remaining > 0) {
                memmove(wsc->in_buffer->buffer, 
                        wsc->in_buffer->buffer + processed_size, 
                        remaining);
            }
            wsc->in_buffer->len = remaining;
            
            // Reset frame processing state
            wsc->frame_in_progress = false;
            frames_processed++;
        }
        else {
            // Continue processing an existing frame (for fragmented frames or large payloads)
            // TODO: Implement fragmented frame handling
            process_more = false;
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