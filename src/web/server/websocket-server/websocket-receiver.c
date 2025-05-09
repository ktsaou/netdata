// SPDX-License-Identifier: GPL-3.0-or-later

#include "daemon/common.h"
#include "websocket-internal.h"

// Process incoming WebSocket data
ssize_t websocket_receive_data(WS_CLIENT *wsc) {
    worker_is_busy(WORKERS_WEBSOCKET_SOCK_RECEIVE);

    if (!wsc || !wsc->in_buffer.data || wsc->sock.fd < 0)
        return -1;

    // Log current buffer state - properly calculate buffer usage with circular buffer API
    size_t buffer_used = cbuffer_next_unsafe(&wsc->in_buffer, NULL);
    websocket_debug(wsc, "Before receive: buffer has %zu bytes (out of %zu bytes)",
                buffer_used, wsc->in_buffer.size);

    // The circular buffer will handle its own memory management
    // No need to manually resize - if it grows beyond max size, we'll get an error on add

    // Get a pointer to where to write the data
    char buffer[WEBSOCKET_RECEIVE_BUFFER_SIZE];

    // Read data from socket into temporary buffer using ND_SOCK
    ssize_t bytes_read = nd_sock_read(&wsc->sock, buffer, WEBSOCKET_RECEIVE_BUFFER_SIZE, 1); // 1 retry for non-blocking read

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

    // Update last activity time
    wsc->last_activity_t = now_monotonic_sec();

    // Dump the received data for debugging
    websocket_dump_debug(wsc, buffer, bytes_read, "READ %zd bytes", bytes_read);

    // Add data to circular buffer
    if (cbuffer_add_unsafe(&wsc->in_buffer, buffer, bytes_read) != 0) {
        // If this fails, it means the client is sending too much data too quickly
        websocket_error(wsc, "Client buffer full - too much incoming data - disconnecting");

        // Schedule client disconnect
        websocket_client_send_close(wsc, WS_CLOSE_MESSAGE_TOO_BIG, "Too much incoming data");
        return -1;
    }

    char *buffer_pos;
    buffer_used = cbuffer_next_unsafe(&wsc->in_buffer, &buffer_pos);

    // Process received data using the protocol layers
    // First, check if we know the next frame size and ensure it's unwrapped if needed
    if (wsc->next_frame_size > 0) {
        // Try to ensure the frame data is contiguous
        if (!cbuffer_ensure_unwrapped_size(&wsc->in_buffer, wsc->next_frame_size)) {
            // Not enough data for the next frame or couldn't unwrap
            // This is not an error - we'll get more data in the next read
            websocket_debug(wsc, "Need more data for next frame (need %zu bytes, have %zu bytes)",
                       wsc->next_frame_size, buffer_used);
            return bytes_read;
        }

        // Get the updated buffer view after unwrapping
        buffer_used = cbuffer_next_unsafe(&wsc->in_buffer, &buffer_pos);
    }

    // Now we have contiguous data for processing
    ssize_t bytes_consumed = websocket_protocol_got_data(wsc, buffer_pos, buffer_used);
    if (bytes_consumed < 0) {
        if(bytes_consumed < -1) {
            bytes_consumed = -bytes_consumed;
            cbuffer_remove_unsafe(&wsc->in_buffer, bytes_consumed);
        }

        websocket_error(wsc, "Failed to process received data");
        return -1;
    }

    // Check if bytes_processed is 0 but this was a successful call
    // This means we have an incomplete frame and need to keep the entire buffer
    if (bytes_consumed == 0) {
        websocket_debug(wsc, "Incomplete frame detected - keeping all %zu bytes in buffer for next read", buffer_used);
        return bytes_read;  // Return the bytes read so caller knows we made progress
    }

    // We've processed some data - remove it from the circular buffer
    cbuffer_remove_unsafe(&wsc->in_buffer, bytes_consumed);

    // Return the number of bytes we processed from this read
    // Even if bytes_processed is 0, we still read data which will be processed later
    return bytes_read;
}

// Actually write data to the client socket
ssize_t websocket_write_data(WS_CLIENT *wsc) {
    worker_is_busy(WORKERS_WEBSOCKET_SOCK_SEND);

    if (!wsc || !wsc->out_buffer.data || wsc->sock.fd < 0)
        return 0;

    ssize_t bytes_written = 0;

    // Let cbuffer_next_unsafe determine if there's data to write
    // This correctly handles the circular buffer wrap-around cases

    // Get data to write from circular buffer
    char *data;
    size_t data_length = cbuffer_next_unsafe(&wsc->out_buffer, &data);
    if (data_length == 0)
        goto done;

    // Dump the data being written for debugging
    websocket_dump_debug(wsc, data, data_length, "WRITE %zu bytes", data_length);

    // In the websocket thread we want non-blocking behavior
    // Use nd_sock_write with a single retry
    bytes_written = nd_sock_write(&wsc->sock, data, data_length, 1); // 1 retry for non-blocking write

    if (bytes_written < 0) {
        websocket_error(wsc, "Failed to write to client: %s", strerror(errno));
        goto done;
    }

    // Remove written bytes from circular buffer
    if (bytes_written > 0)
        cbuffer_remove_unsafe(&wsc->out_buffer, bytes_written);

done:
    websocket_thread_update_client_poll_flags(wsc);
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
    if (wsc->wth) {
        // Send command to remove client
        websocket_thread_send_command(wsc->wth, WEBSOCKET_THREAD_CMD_REMOVE_CLIENT, &wsc->id, sizeof(wsc->id));
    }
}
