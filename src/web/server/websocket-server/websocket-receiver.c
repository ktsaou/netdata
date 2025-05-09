// SPDX-License-Identifier: GPL-3.0-or-later

#include "daemon/common.h"
#include "websocket-internal.h"

// Process incoming WebSocket data
int websocket_receive_data(WS_CLIENT *wsc) {
    worker_is_busy(WORKERS_WEBSOCKET_SOCK_RECEIVE);

    if (!wsc || !wsb_data(&wsc->in_buffer) || wsc->sock.fd < 0)
        return -1;

    // Log current buffer state
    websocket_debug(wsc, "Before receive: buffer has %zu bytes (out of %zu bytes)",
                wsb_length(&wsc->in_buffer), wsb_size(&wsc->in_buffer));

    // Allocate buffer space for reading - ensure we have enough space
    if(wsb_size(&wsc->in_buffer) < wsc->max_message_size)
        wsb_need_bytes(&wsc->in_buffer, wsc->max_message_size + 1 - wsb_length(&wsc->in_buffer));

    if(wsb_size(&wsc->in_buffer) - wsb_length(&wsc->in_buffer) - 1 < WEBSOCKET_RECEIVE_BUFFER_SIZE / 8)
        wsb_need_bytes(&wsc->in_buffer, WEBSOCKET_RECEIVE_BUFFER_SIZE);

    char *buffer_pos = wsb_data(&wsc->in_buffer) + wsb_length(&wsc->in_buffer);
    
    // Read data from socket into buffer using ND_SOCK
    ssize_t bytes_read = nd_sock_read(&wsc->sock, buffer_pos, wsb_size(&wsc->in_buffer) - wsb_length(&wsc->in_buffer) - 1, 1); // 1 retry for non-blocking read

    if (bytes_read <= 0) {
        if (bytes_read == 0) {
            // Connection closed
            websocket_debug(wsc, "Client closed connection");
            return -1;
        }

        if (errno == EAGAIN || errno == EWOULDBLOCK)
            return 0; // No data available right now

        websocket_error(wsc, "Failed to read from client: %s", strerror(errno));
        return -1;
    }

    // Update buffer length
    wsb_set_length(&wsc->in_buffer, wsb_length(&wsc->in_buffer) + bytes_read);

    // Dump the received data for debugging
    websocket_dump_debug(wsc, buffer_pos, bytes_read, "READ %zd bytes", bytes_read);

    // Update last activity time
    wsc->last_activity_t = now_monotonic_sec();

    // Process received data using the new protocol layers
    // Note: websocket_protocol_process_data works through the buffer and updates bytes_processed
    size_t bytes_processed = 0;
    char *process_buffer = wsb_data(&wsc->in_buffer); // Process from the beginning of the buffer
    size_t buffer_length = wsb_length(&wsc->in_buffer); // Total data available including previous data
    
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
        // We've processed some data - remove it from the front of the buffer
        wsb_trim_front(&wsc->in_buffer, bytes_processed);
    }
    
    // Return the number of bytes we processed from this read
    // Even if bytes_processed is 0, we still read data which will be processed later
    return bytes_read;
}

// Actually write data to the client socket
int websocket_write_data(WS_CLIENT *wsc) {
    worker_is_busy(WORKERS_WEBSOCKET_SOCK_SEND);

    if (!wsc || !wsb_data(&wsc->out_buffer) || wsc->sock.fd < 0 || wsb_length(&wsc->out_buffer) == 0)
        return 0;

    const char *data = wsb_data(&wsc->out_buffer);
    size_t data_length = wsb_length(&wsc->out_buffer);

    // Dump the data being written for debugging
    websocket_dump_debug(wsc, data, data_length, "WRITE %zu bytes", data_length);

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
        size_t remaining = wsb_length(&wsc->out_buffer) - bytes_written;
        if (remaining > 0) {
            // Remove the bytes from the front of the buffer
            wsb_trim_front(&wsc->out_buffer, bytes_written);

            // Still have data to write - keep ND_POLL_WRITE set
            if (wsc->thread && wsc->sock.fd >= 0) {
                websocket_thread_update_client_poll_flags(wsc->thread, wsc, ND_POLL_READ | ND_POLL_WRITE);
            }
        } else {
            // All data written - reset buffer and remove ND_POLL_WRITE
            wsb_reset(&wsc->out_buffer);
            if (wsc->thread && wsc->sock.fd >= 0) {
                websocket_thread_update_client_poll_flags(wsc->thread, wsc, ND_POLL_READ);
            }
        }
    }

    return bytes_written;
}

// Handle socket takeover from web client - similar to stream_receiver_takeover_web_connection
void websocket_takeover_web_connection(struct web_client *w, WS_CLIENT *wsc) {
    // Set the file descriptor and ssl from the web client
    wsc->sock.fd = w->fd;
    wsc->sock.ssl = w->ssl;

    w->ssl = NETDATA_SSL_UNSET_CONNECTION;

    WEB_CLIENT_IS_DEAD(w);

    if(web_server_mode == WEB_SERVER_MODE_STATIC_THREADED) {
        web_client_flag_set(w, WEB_CLIENT_FLAG_DONT_CLOSE_SOCKET);
    }
    else {
        w->fd = -1;
    }

    // Clear web client buffer
    buffer_flush(w->response.data);
}

// Close a WebSocket connection with a specific code and reason
void websocket_client_send_close(WS_CLIENT *wsc, int close_code, const char *reason) {
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
