#include "daemon/common.h"
#include "websocket-internal.h"

// Process incoming WebSocket data
int websocket_receive_data(WEBSOCKET_SERVER_CLIENT *wsc) {
    if (!wsc || !wsc->in_buffer || wsc->sock.fd < 0)
        return -1;

    // Log current buffer state
    websocket_debug(wsc, "Before receive: buffer has %zu bytes (out of %zu bytes)",
                buffer_strlen(wsc->in_buffer), (size_t)wsc->in_buffer->size);

    // Allocate buffer space for reading - ensure we have enough space
    if(wsc->in_buffer->size < wsc->max_message_size)
        buffer_need_bytes(wsc->in_buffer, wsc->max_message_size + 1 - buffer_strlen(wsc->in_buffer));

    if(wsc->in_buffer->size - wsc->in_buffer->len - 1 < WEBSOCKET_RECEIVE_BUFFER_SIZE / 8)
        buffer_need_bytes(wsc->in_buffer, WEBSOCKET_RECEIVE_BUFFER_SIZE);

    char *buffer_pos = &wsc->in_buffer->buffer[wsc->in_buffer->len];
    
    // Read data from socket into buffer using ND_SOCK
    ssize_t bytes_read = nd_sock_read(&wsc->sock, buffer_pos, wsc->in_buffer->size - wsc->in_buffer->len - 1, 1); // 1 retry for non-blocking read

    if (bytes_read <= 0) {
        if (bytes_read == 0) {
            // Connection closed
            websocket_info(wsc, "Client closed connection");
            return -1;
        }

        if (errno == EAGAIN || errno == EWOULDBLOCK)
            return 0; // No data available right now

        websocket_error(wsc, "Failed to read from client: %s", strerror(errno));
        return -1;
    }

    // Update buffer length
    wsc->in_buffer->len += bytes_read;
    
    // Update last activity time
    wsc->last_activity_t = now_monotonic_sec();

    // Process received data using the new protocol layers
    // Note: websocket_protocol_process_data works through the buffer and updates bytes_processed
    size_t bytes_processed = 0;
    char *process_buffer = wsc->in_buffer->buffer; // Process from the beginning of the buffer
    size_t buffer_length = wsc->in_buffer->len; // Total data available including previous data
    
    bool result = websocket_protocol_got_data(wsc, process_buffer, buffer_length, &bytes_processed);
    if (!result) {
        websocket_error(wsc, "Failed to process received data");
        return -1;
    }
    
    // The protocol layer has now processed some bytes from the buffer
    // It might be all the bytes, or it might be just part of them
    // We need to remove the processed bytes from the buffer
    
    // Check if bytes_processed is 0 but this was a successful call
    // This means we have an incomplete frame and need to keep the entire buffer
    if (result && bytes_processed == 0) {
        websocket_debug(wsc, "Incomplete frame detected - keeping all %zu bytes in buffer for next read", buffer_length);
        return bytes_read;  // Return the bytes read so caller knows we made progress
    }
    
    if (bytes_processed > 0) {
        // We've processed some data - remove it from the buffer
        if (bytes_processed < buffer_length) {
            // More data in buffer - shift remaining data to beginning
            size_t remaining = buffer_length - bytes_processed;
            
            // Shift the remaining data to the beginning of the buffer
            memmove(wsc->in_buffer->buffer, 
                    wsc->in_buffer->buffer + bytes_processed, 
                    remaining);
            
            // Update buffer length to reflect the shift
            wsc->in_buffer->len = remaining;
        } else {
            // All data was processed - reset buffer
            wsc->in_buffer->len = 0;
        }
    }
    
    // Return the number of bytes we processed from this read
    // Even if bytes_processed is 0, we still read data which will be processed later
    return bytes_read;
}

// Actually write data to the client socket
int websocket_write_data(WEBSOCKET_SERVER_CLIENT *wsc) {
    if (!wsc || !wsc->out_buffer || wsc->sock.fd < 0 || buffer_strlen(wsc->out_buffer) == 0)
        return 0;

#ifdef NETDATA_INTERNAL_CHECKS
    // Create a hex dump of the first 20 bytes being sent for logging
    char hex_dump[256] = "";
    char ascii_dump[64] = "";
    size_t hex_pos = 0;
    size_t ascii_pos = 0;
    size_t data_length = buffer_strlen(wsc->out_buffer);
    size_t bytes_to_show = data_length < 20 ? data_length : 20;
    const char *data = buffer_tostring(wsc->out_buffer);
    
    // Build the hex and ASCII representations
    for (size_t i = 0; i < bytes_to_show; i++) {
        unsigned char c = (unsigned char)data[i];
        
        // Add to hex dump
        int chars_added = snprintf(hex_dump + hex_pos, sizeof(hex_dump) - hex_pos, "%02x ", c);
        if (chars_added > 0) {
            hex_pos += chars_added;
        }
        
        // Add to ASCII dump
        if (isprint(c)) {
            int chars_added = snprintf(ascii_dump + ascii_pos, sizeof(ascii_dump) - ascii_pos, "%c", c);
            if (chars_added > 0) {
                ascii_pos += chars_added;
            }
        } else {
            int chars_added = snprintf(ascii_dump + ascii_pos, sizeof(ascii_dump) - ascii_pos, ".");
            if (chars_added > 0) {
                ascii_pos += chars_added;
            }
        }
    }
    
    websocket_debug(wsc, "WRITE %zu bytes: [%s] \"%s\"%s",
                data_length, hex_dump, ascii_dump, 
                data_length > 20 ? " (more bytes not shown)" : "");
#else
    const char *data = buffer_tostring(wsc->out_buffer);
    size_t data_length = buffer_strlen(wsc->out_buffer);
#endif /* NETDATA_INTERNAL_CHECKS */

    // In the websocket thread we want non-blocking behavior
    // Use nd_sock_write with a single retry
    ssize_t bytes_written = nd_sock_write(&wsc->sock, data, data_length, 1); // 1 retry for non-blocking write

    if (bytes_written < 0) {
        if (errno == EAGAIN || errno == EWOULDBLOCK) {
            // Would block, try later
            // Keep WRITE flags set so we'll try again when socket is ready
            if (wsc->thread && wsc->sock.fd >= 0) {
                websocket_thread_update_client_poll_flags(wsc->thread, wsc, ND_POLL_READ | ND_POLL_WRITE);
            }
            return 0;
        }

        websocket_error(wsc, "Failed to write to client: %s", strerror(errno));
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
            if (wsc->thread && wsc->sock.fd >= 0) {
                websocket_thread_update_client_poll_flags(wsc->thread, wsc, ND_POLL_READ | ND_POLL_WRITE);
            }
        } else {
            // All data written - remove ND_POLL_WRITE
            wsc->out_buffer->len = 0;
            if (wsc->thread && wsc->sock.fd >= 0) {
                websocket_thread_update_client_poll_flags(wsc->thread, wsc, ND_POLL_READ);
            }
        }
    }

    return bytes_written;
}

// Handle socket takeover from web client - similar to stream_receiver_takeover_web_connection
void websocket_takeover_connection(struct web_client *w, WEBSOCKET_SERVER_CLIENT *wsc) {
    // First initialize the ND_SOCK with the web server's SSL context
    nd_sock_init(&wsc->sock, netdata_ssl_web_server_ctx, false);

    // Now set the file descriptor and ssl from the web client
    wsc->sock.fd = w->fd;
    wsc->sock.ssl = w->ssl;

    // Copy client information
    strncpyz(wsc->client_ip, w->client_ip, sizeof(wsc->client_ip));
    strncpyz(wsc->client_port, w->client_port, sizeof(wsc->client_port));
    wsc->protocol = w->websocket.protocol;

    // Initialize buffers
    wsc->in_buffer = buffer_create(NETDATA_WEB_REQUEST_INITIAL_SIZE, NULL);
    wsc->out_buffer = buffer_create(NETDATA_WEB_RESPONSE_INITIAL_SIZE, NULL);

    // Take over the socket but don't close it when cleaning up the web client
    w->ssl = NETDATA_SSL_UNSET_CONNECTION;
    WEB_CLIENT_IS_DEAD(w);

    if(web_server_mode == WEB_SERVER_MODE_STATIC_THREADED) {
        web_client_flag_set(w, WEB_CLIENT_FLAG_DONT_CLOSE_SOCKET);
    }
    else {
        w->fd = -1;
    }

    // Initialize compression if permessage-deflate extension is enabled
    if (w->websocket.ext_flags & WS_EXTENSION_PERMESSAGE_DEFLATE) {
        // Initialize the compression with the original extension string for parameter parsing
        websocket_compression_init(wsc, w->websocket.extensions);
    }

    // Send the handshake response using ND_SOCK - we're still in the web server thread,
    // so we need to use the persist version to ensure the complete handshake is sent
    ssize_t bytes = nd_sock_write_persist(&wsc->sock, buffer_tostring(w->response.header_output), buffer_strlen(w->response.header_output), 20); // Try up to 20 chunks

    if (bytes != (ssize_t)buffer_strlen(w->response.header_output)) {
        netdata_log_error("WEBSOCKET: Failed to send complete WebSocket handshake response"); // No client yet
        websocket_client_free(wsc);
        return;
    }

    // Now that we've sent the handshake response successfully, set the connection state to open
    wsc->state = WS_STATE_OPEN;

    // Register the client in our registry
    if (!websocket_client_register(wsc)) {
        websocket_error(wsc, "Failed to register WebSocket client");
        websocket_client_free(wsc);
        return;
    }

    // Initialize current message to NULL
    wsc->current_message = NULL;

    // Set socket to non-blocking mode
    if (fcntl(wsc->sock.fd, F_SETFL, O_NONBLOCK) == -1) {
        websocket_error(wsc, "Failed to set WebSocket socket to non-blocking mode");
        websocket_client_free(wsc);
        return;
    }

    // Assign to a thread
    WEBSOCKET_THREAD *wth = websocket_thread_assign_client(wsc);
    if (!wth) {
        websocket_error(wsc, "Failed to assign WebSocket client to a thread");
        websocket_client_free(wsc);
        return;
    }

    nd_log(NDLS_DAEMON, NDLP_DEBUG,
           "WebSocket connection established with %s:%s using protocol: %s (client ID: %zu, thread: %zu)",
           wsc->client_ip, wsc->client_port,
           (wsc->protocol == WS_PROTOCOL_NETDATA_JSON) ? "netdata-json" : "unknown",
           wsc->id, wth->id);
           
    websocket_info(wsc, "Connection established on thread %zu using protocol: %s", 
                wth->id, (wsc->protocol == WS_PROTOCOL_NETDATA_JSON) ? "netdata-json" : "unknown");
}

// Legacy function to ping client - redirects to our new protocol layer
void websocket_ping_client(WEBSOCKET_SERVER_CLIENT *wsc) {
    if (!wsc)
        return;
    
    // Use new protocol layer to send ping
    websocket_protocol_send_ping(wsc, NULL, 0);
}

// Close a WebSocket connection with a specific code and reason
void websocket_close_client(WEBSOCKET_SERVER_CLIENT *wsc, int close_code, const char *reason) {
    if (!wsc || wsc->state == WS_STATE_CLOSED)
        return;

    // Log close information
    websocket_info(wsc, "Closing client with code %d%s%s",
              close_code, reason ? ": " : "", reason ? reason : "");

    // Send close frame using new protocol layer
    websocket_protocol_send_close(wsc, close_code, reason);

    // Update state
    wsc->state = WS_STATE_CLOSING;

    // Call close callback if set
    if (wsc->on_close)
        wsc->on_close(wsc, close_code, reason);

    // Mark client for removal from thread
    if (wsc->thread) {
        // Send command to remove client
        websocket_thread_send_command(wsc->thread, WEBSOCKET_THREAD_CMD_REMOVE_CLIENT, &wsc->id, sizeof(wsc->id));
    }
}
