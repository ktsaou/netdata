// SPDX-License-Identifier: GPL-3.0-or-later

#include "web/server/web_client.h"
#include "websocket-internal.h"
#include <openssl/sha.h>

// All constants are now in websocket-internal.h

// Private structure for WebSocket server state
struct websocket_server {
    WS_CLIENTS_JudyLSet clients;     // JudyL array of WebSocket clients
    size_t client_id_counter;        // Counter for generating unique client IDs
    size_t active_clients;           // Number of active clients
    SPINLOCK spinlock;               // Spinlock to protect the registry
};

// The global (but private) instance of the WebSocket server state
static struct websocket_server ws_server = {
    .clients = NULL,
    .client_id_counter = 0,
    .active_clients = 0,
    .spinlock = SPINLOCK_INITIALIZER
};

// Forward declarations for internal functions
static WEBSOCKET_SERVER_CLIENT *websocket_client_create(void);
static void websocket_client_free(WEBSOCKET_SERVER_CLIENT *wsc);
static int websocket_read_frame_header(WEBSOCKET_SERVER_CLIENT *wsc, WEBSOCKET_FRAME_HEADER *header);
static int websocket_recv_frame(WEBSOCKET_SERVER_CLIENT *wsc, char **payload, size_t *payload_len, WEBSOCKET_OPCODE *opcode);
static void websocket_handle_control_frame(WEBSOCKET_SERVER_CLIENT *wsc, WEBSOCKET_OPCODE opcode, const char *payload, size_t payload_len);

// Check if the current HTTP request is a WebSocket handshake request
bool websocket_detect_handshake_request(struct web_client *w) {
    // We need a valid key and to be flagged as a WebSocket request
    if (!web_client_is_websocket(w) || !w->websocket.key)
        return false;
    
    // We must have a supported protocol
    if (w->websocket.protocol == WS_PROTOCOL_UNKNOWN)
        return false;
        
    return true;
}

// Generate the WebSocket accept key as per RFC 6455
char *websocket_generate_handshake_key(const char *client_key) {
    if (!client_key)
        return NULL;
    
    // Concatenate the key with the WebSocket GUID
    char concat_key[256];
    snprintfz(concat_key, sizeof(concat_key), "%s%s", client_key, WS_GUID);
    
    // Create SHA-1 hash
    unsigned char sha_hash[SHA_DIGEST_LENGTH];
    SHA1((unsigned char *)concat_key, strlen(concat_key), sha_hash);
    
    // Convert to base64
    char *accept_key = mallocz(33); // Base64 of SHA-1 is 28 chars + null term
    netdata_base64_encode((unsigned char *)accept_key, sha_hash, SHA_DIGEST_LENGTH);
    
    return accept_key;
}

// Handle the WebSocket handshake procedure
short int websocket_handle_handshake(struct web_client *w) {
    if (!websocket_detect_handshake_request(w))
        return HTTP_RESP_BAD_REQUEST;

    // Generate the accept key
    char *accept_key = websocket_generate_handshake_key(w->websocket.key);
    if (!accept_key)
        return HTTP_RESP_INTERNAL_SERVER_ERROR;
    
    // Build the handshake response
    buffer_flush(w->response.header_output);
    buffer_flush(w->response.data);
    
    buffer_sprintf(w->response.header_output,
        "HTTP/1.1 101 Switching Protocols\r\n"
        "Upgrade: websocket\r\n"
        "Connection: Upgrade\r\n"
        "Sec-WebSocket-Accept: %s\r\n",
        accept_key
    );
    
    // Add the selected subprotocol
    if (w->websocket.protocol == WS_PROTOCOL_NETDATA_JSON) {
        buffer_strcat(w->response.header_output, "Sec-WebSocket-Protocol: netdata-json\r\n");
    } 
    else {
        // We should never get here as we validate protocols in websocket_detect_handshake_request
        freez(accept_key);
        return HTTP_RESP_BAD_REQUEST;
    }
    
    // End of headers
    buffer_strcat(w->response.header_output, "\r\n");
    
    freez(accept_key);
    
    // Take over the connection immediately
    WEBSOCKET_SERVER_CLIENT *wsc = websocket_client_create();
    if (!wsc) {
        netdata_log_error("WEBSOCKET: Failed to create WebSocket client");
        return HTTP_RESP_INTERNAL_SERVER_ERROR;
    }
    
    // Similar to stream_receiver_takeover_web_connection
    websocket_takeover_connection(w, wsc);
    
    w->mode = HTTP_REQUEST_MODE_WEBSOCKET;
    web_client_disable_wait_receive(w);
    
    netdata_log_debug(D_WEB_CLIENT, "%llu: WebSocket handshake successful, protocol: netdata-json", w->id);
    
    // Important: This code doesn't actually get sent to the client since we've already
    // taken over the socket. It's just used by the caller to identify what happened.
    return HTTP_RESP_WEBSOCKET_HANDSHAKE;
}

// Initialize a WebSocket client structure
void websocket_init_client(WEBSOCKET_SERVER_CLIENT *wsc, int fd, NETDATA_SSL ssl) {
    wsc->state = WS_STATE_OPEN;
    
    // Initialize the ND_SOCK structure with the web server's SSL context
    nd_sock_init(&wsc->sock, netdata_ssl_web_server_ctx, false);
    
    // Now set the fd and ssl
    wsc->sock.fd = fd;
    wsc->sock.ssl = ssl;
    
    wsc->in_buffer = buffer_create(NETDATA_WEB_REQUEST_INITIAL_SIZE, NULL);
    wsc->out_buffer = buffer_create(NETDATA_WEB_RESPONSE_INITIAL_SIZE, NULL);
    
    // Initialize callbacks to NULL
    wsc->on_message = NULL;
    wsc->on_close = NULL;
    wsc->on_error = NULL;
    wsc->user_data = NULL;
}

// Initialize WebSocket subsystem
void websocket_initialize(void) {
    // Initialize thread system
    websocket_threads_init();
    
    netdata_log_info("WebSocket server subsystem initialized");
}

// Create a new WebSocket client with a unique ID
static WEBSOCKET_SERVER_CLIENT *websocket_client_create(void) {
    WEBSOCKET_SERVER_CLIENT *wsc = callocz(1, sizeof(WEBSOCKET_SERVER_CLIENT));
    
    spinlock_lock(&ws_server.spinlock);
    wsc->id = ++ws_server.client_id_counter; // Generate unique ID
    spinlock_unlock(&ws_server.spinlock);
    
    wsc->connected_t = now_realtime_sec();
    wsc->last_activity_t = wsc->connected_t;
    
    return wsc;
}

// Free a WebSocket client
static void websocket_client_free(WEBSOCKET_SERVER_CLIENT *wsc) {
    if (!wsc)
        return;
    
    // First unregister from the client registry
    websocket_client_unregister(wsc);
    
    // Close socket using ND_SOCK abstraction
    nd_sock_close(&wsc->sock);
    
    // Free buffers
    if (wsc->in_buffer)
        buffer_free(wsc->in_buffer);
    
    if (wsc->out_buffer)
        buffer_free(wsc->out_buffer);
    
    freez(wsc);
}

// Register a WebSocket client in the registry
bool websocket_client_register(WEBSOCKET_SERVER_CLIENT *wsc) {
    if (!wsc || wsc->id == 0)
        return false;
    
    spinlock_lock(&ws_server.spinlock);
    
    int added = WS_CLIENTS_SET(&ws_server.clients, wsc->id, wsc);
    if (!added) {
        ws_server.active_clients++;
        netdata_log_debug(D_WEB_CLIENT, "WebSocket client %zu registered, total clients: %zu", 
                         wsc->id, ws_server.active_clients);
    }
    
    spinlock_unlock(&ws_server.spinlock);
    
    return added;
}

// Unregister a WebSocket client from the registry
void websocket_client_unregister(WEBSOCKET_SERVER_CLIENT *wsc) {
    if (!wsc || wsc->id == 0)
        return;
    
    spinlock_lock(&ws_server.spinlock);
    
    WEBSOCKET_SERVER_CLIENT *existing = WS_CLIENTS_GET(&ws_server.clients, wsc->id);
    if (existing && existing == wsc) {
        WS_CLIENTS_DEL(&ws_server.clients, wsc->id);
        if (ws_server.active_clients > 0)
            ws_server.active_clients--;
        
        netdata_log_debug(D_WEB_CLIENT, "WebSocket client %zu unregistered, total clients: %zu", 
                         wsc->id, ws_server.active_clients);
    }
    
    spinlock_unlock(&ws_server.spinlock);
}

// Find a WebSocket client by ID
WEBSOCKET_SERVER_CLIENT *websocket_client_find_by_id(size_t id) {
    if (id == 0)
        return NULL;
    
    WEBSOCKET_SERVER_CLIENT *wsc = NULL;
    
    spinlock_lock(&ws_server.spinlock);
    wsc = WS_CLIENTS_GET(&ws_server.clients, id);
    spinlock_unlock(&ws_server.spinlock);
    
    return wsc;
}

// Helper structure for foreach callback
struct foreach_callback_params {
    void (*callback)(WEBSOCKET_SERVER_CLIENT *wsc, void *data);
    void *data;
};

// Iterate through all WebSocket clients using JudyL FIRST/NEXT
void websocket_clients_foreach(void (*callback)(WEBSOCKET_SERVER_CLIENT *wsc, void *data), void *data) {
    if (!callback)
        return;
    
    spinlock_lock(&ws_server.spinlock);
    
    Word_t index = 0;
    WEBSOCKET_SERVER_CLIENT *wsc;
    
    // Get the first entry
    wsc = WS_CLIENTS_FIRST(&ws_server.clients, &index);
    
    // Iterate through all entries
    while (wsc) {
        callback(wsc, data);
        wsc = WS_CLIENTS_NEXT(&ws_server.clients, &index);
    }
    
    spinlock_unlock(&ws_server.spinlock);
}

// Get the count of active WebSocket clients
size_t websocket_clients_count(void) {
    size_t count;
    
    spinlock_lock(&ws_server.spinlock);
    count = ws_server.active_clients;
    spinlock_unlock(&ws_server.spinlock);
    
    return count;
}

// Helper structure for broadcast
struct broadcast_params {
    const char *message;
    size_t length;
    WEBSOCKET_OPCODE opcode;
    int success_count;
};

// Callback function for broadcast
static void websocket_broadcast_callback(WEBSOCKET_SERVER_CLIENT *wsc, void *data) {
    struct broadcast_params *params = (struct broadcast_params *)data;
    
    if (wsc && wsc->state == WS_STATE_OPEN) {
        if (websocket_send_message(wsc, params->message, params->length, params->opcode) > 0)
            params->success_count++;
    }
}

// Broadcast a message to all connected WebSocket clients
int websocket_broadcast_message(const char *message, size_t length, WEBSOCKET_OPCODE opcode) {
    if (!message || length == 0 || (opcode != WS_OPCODE_TEXT && opcode != WS_OPCODE_BINARY))
        return -1;
    
    // Check if there are any active clients
    size_t client_count = websocket_clients_count();
    if (client_count == 0)
        return 0;
    
    // First, try the older method for backward compatibility - useful for testing
    if(!websocket_threads[0].thread) {
        struct broadcast_params params = {
            .message = message,
            .length = length,
            .opcode = opcode,
            .success_count = 0
        };
        
        // Use the foreach function to iterate through all clients
        websocket_clients_foreach(websocket_broadcast_callback, &params);
        
        return params.success_count;
    }
    
    // New approach: send broadcast command to all active threads
    int success_count = 0;
    
    // Prepare the broadcast command data
    size_t cmd_size = sizeof(size_t) + sizeof(WEBSOCKET_OPCODE) + length;
    char *cmd_data = mallocz(cmd_size);
    
    // Set message length
    memcpy(cmd_data, &length, sizeof(size_t));
    
    // Set opcode
    memcpy(cmd_data + sizeof(size_t), &opcode, sizeof(WEBSOCKET_OPCODE));
    
    // Set message
    memcpy(cmd_data + sizeof(size_t) + sizeof(WEBSOCKET_OPCODE), message, length);
    
    // Send broadcast command to all active threads
    for(size_t i = 0; i < WEBSOCKET_MAX_THREADS; i++) {
        if(websocket_threads[i].thread && websocket_threads[i].running) {
            if(websocket_thread_send_command(&websocket_threads[i], WEBSOCKET_THREAD_CMD_BROADCAST, 
                                            cmd_data, cmd_size)) {
                success_count++;
            }
        }
    }
    
    freez(cmd_data);
    return success_count;
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
    
    // Send the handshake response using ND_SOCK - we're still in the web server thread,
    // so we need to use the persist version to ensure the complete handshake is sent
    ssize_t bytes = nd_sock_write_persist(&wsc->sock, buffer_tostring(w->response.header_output), buffer_strlen(w->response.header_output), 20); // Try up to 20 chunks
    
    if (bytes != (ssize_t)buffer_strlen(w->response.header_output)) {
        netdata_log_error("WEBSOCKET: Failed to send complete WebSocket handshake response");
        websocket_client_free(wsc);
        return;
    }
    
    // Now that we've sent the handshake response successfully, set the connection state to open
    wsc->state = WS_STATE_OPEN;
    
    // Register the client in our registry
    if (!websocket_client_register(wsc)) {
        netdata_log_error("WEBSOCKET: Failed to register WebSocket client");
        websocket_client_free(wsc);
        return;
    }
    
    // Initialize frame processing state
    wsc->frame_in_progress = false;
    wsc->frame_length = 0;
    wsc->frame_read = 0;
    memset(wsc->mask_key, 0, sizeof(wsc->mask_key));
    
    // Set socket to non-blocking mode
    if (fcntl(wsc->sock.fd, F_SETFL, O_NONBLOCK) == -1) {
        netdata_log_error("WEBSOCKET: Failed to set WebSocket socket to non-blocking mode");
        websocket_client_free(wsc);
        return;
    }
    
    // Assign to a thread
    WEBSOCKET_THREAD *wth = websocket_thread_assign_client(wsc);
    if (!wth) {
        netdata_log_error("WEBSOCKET: Failed to assign WebSocket client to a thread");
        websocket_client_free(wsc);
        return;
    }
    
    netdata_log_info("WebSocket connection established with %s:%s using protocol: %s (client ID: %zu, thread: %zu)", 
                    wsc->client_ip, wsc->client_port, 
                    (wsc->protocol == WS_PROTOCOL_NETDATA_JSON) ? "netdata-json" : "unknown",
                    wsc->id, wth->id);
}

// Close a WebSocket connection with a specific code and reason
void websocket_close_client(WEBSOCKET_SERVER_CLIENT *wsc, int close_code, const char *reason) {
    if (!wsc || wsc->state == WS_STATE_CLOSED)
        return;
    
    // Create close frame payload (code + reason)
    size_t reason_len = reason ? strlen(reason) : 0;
    size_t payload_len = 2 + reason_len; // 2 bytes for code, rest for reason
    char *payload = mallocz(payload_len);
    
    // Set close code (network byte order)
    uint16_t code_be = htons((uint16_t)close_code);
    memcpy(payload, &code_be, 2);
    
    // Add reason text if provided
    if (reason_len > 0)
        memcpy(payload + 2, reason, reason_len);
    
    // Send close frame
    websocket_send_frame(wsc, payload, payload_len, WS_OPCODE_CLOSE);
    freez(payload);
    
    // Update state
    wsc->state = WS_STATE_CLOSING;
    
    // Call close callback if set
    if (wsc->on_close)
        wsc->on_close(wsc, close_code, reason);
}

// Send a ping frame to check client connection
void websocket_ping_client(WEBSOCKET_SERVER_CLIENT *wsc) {
    if(!wsc || wsc->state != WS_STATE_OPEN)
        return;

    // Simple ping with no payload
    websocket_send_frame(wsc, NULL, 0, WS_OPCODE_PING);
}

// Send a WebSocket message to the client
int websocket_send_message(WEBSOCKET_SERVER_CLIENT *wsc, const char *message, size_t length, WEBSOCKET_OPCODE opcode) {
    if (!wsc || !message || wsc->state != WS_STATE_OPEN)
        return -1;
    
    return websocket_send_frame(wsc, message, length, opcode);
}

// Send a WebSocket frame to the client
int websocket_send_frame(WEBSOCKET_SERVER_CLIENT *wsc, const char *payload, size_t payload_len, WEBSOCKET_OPCODE opcode) {
    if (!wsc || wsc->state != WS_STATE_OPEN)
        return -1;
    
    // Calculate frame header size based on payload length
    size_t header_size = 2; // Base header size (2 bytes)
    if (payload_len > 125 && payload_len <= 65535)
        header_size += 2; // Extended 16-bit length
    else if (payload_len > 65535)
        header_size += 8; // Extended 64-bit length
    
    // Allocate buffer for the complete frame
    char *frame = mallocz(header_size + payload_len);
    
    // Set first byte: FIN bit and opcode
    frame[0] = WS_FIN | (opcode & 0x0F);
    
    // Set second byte: payload length
    if (payload_len <= 125) {
        frame[1] = (unsigned char)payload_len;
    } else if (payload_len <= 65535) {
        frame[1] = 126;
        uint16_t len16 = htons((uint16_t)payload_len);
        memcpy(frame + 2, &len16, 2);
    } else {
        frame[1] = 127;
        uint64_t len64 = htobe64((uint64_t)payload_len);
        memcpy(frame + 2, &len64, 8);
    }
    
    // Copy payload (if any)
    if (payload_len > 0)
        memcpy(frame + header_size, payload, payload_len);
    
    // We're in the websocket thread, so use non-blocking writes
    // and buffer any data that couldn't be sent
    ssize_t sent = nd_sock_write(&wsc->sock, frame, header_size + payload_len, 1); // 1 retry
    
    if (sent < 0) {
        freez(frame);
        if (wsc->on_error)
            wsc->on_error(wsc, "Failed to send WebSocket frame");
        return -1;
    }
    
    // Check if we sent all data
    if (sent < (ssize_t)(header_size + payload_len)) {
        // We couldn't send all data - buffer the rest for later sending
        size_t remaining = (header_size + payload_len) - sent;
        buffer_need_bytes(wsc->out_buffer, remaining);
        memcpy(&wsc->out_buffer->buffer[wsc->out_buffer->len], frame + sent, remaining);
        wsc->out_buffer->len += remaining;
        
        // Mark for poll to include ND_POLL_WRITE
        if (wsc->thread && wsc->sock.fd >= 0) {
            websocket_thread_update_client_poll_flags(wsc->thread, wsc, ND_POLL_READ | ND_POLL_WRITE);
        }
    }
    
    freez(frame);
    return (int)(header_size + payload_len); // Return full size as if all was sent
}

// Read and parse a WebSocket frame header
static int websocket_read_frame_header(WEBSOCKET_SERVER_CLIENT *wsc, WEBSOCKET_FRAME_HEADER *header) {
    // Read initial 2 bytes using ND_SOCK
    unsigned char buf[2];
    ssize_t bytes = nd_sock_read(&wsc->sock, buf, 2, 2); // 2 retries
    
    if (bytes != 2)
        return -1;
    
    // Parse header fields
    header->fin = (buf[0] & 0x80) >> 7;
    header->rsv1 = (buf[0] & 0x40) >> 6;
    header->rsv2 = (buf[0] & 0x20) >> 5;
    header->rsv3 = (buf[0] & 0x10) >> 4;
    header->opcode = buf[0] & 0x0F;
    header->mask = (buf[1] & 0x80) >> 7;
    header->len = buf[1] & 0x7F;
    
    return 0;
}

// Receive and process a complete WebSocket frame
// Note: This function is kept for compatibility, but we should use the buffered approach
// from websocket_process_frames() in websocket-receiver.c for handling frames
static int websocket_recv_frame(WEBSOCKET_SERVER_CLIENT *wsc, char **payload, size_t *payload_len, WEBSOCKET_OPCODE *opcode) {
    // This function can only be used if we have a valid input buffer
    if (!wsc || !wsc->in_buffer) {
        netdata_log_error("WEBSOCKET: Cannot receive frame - invalid client or missing buffer");
        return -1;
    }
    
    // First, try to receive some data into the buffer
    ssize_t bytes_read = websocket_receive_data(wsc);
    if (bytes_read < 0) {
        return -1;  // Error or connection closed
    }
    
    // Process one frame from the buffer
    int frames = websocket_process_frames_buffered(wsc);
    if (frames <= 0) {
        // No complete frames available yet, or error
        *payload = NULL;
        *payload_len = 0;
        return -1;
    }
    
    // The frame was already processed by websocket_process_frames
    // which calls the appropriate handlers. Just return success.
    *payload = NULL;
    *payload_len = 0;
    *opcode = WS_OPCODE_TEXT; // Default
    
    return 0;
}

// Handle a WebSocket control frame (Close, Ping, Pong)
static void websocket_handle_control_frame(WEBSOCKET_SERVER_CLIENT *wsc, WEBSOCKET_OPCODE opcode, const char *payload, size_t payload_len) {
    switch (opcode) {
        case WS_OPCODE_CLOSE:
            // Process close frame
            uint16_t close_code = WS_CLOSE_NORMAL;
            const char *reason = NULL;
            
            if (payload_len >= 2) {
                // Extract close code from payload
                memcpy(&close_code, payload, 2);
                close_code = ntohs(close_code);
                
                // Extract reason if present
                if (payload_len > 2)
                    reason = payload + 2;
            }
            
            // If we initiated the close, just set to CLOSED state
            // If client initiated, send a close frame back
            if (wsc->state != WS_STATE_CLOSING) {
                websocket_close_client(wsc, close_code, reason);
            }
            
            wsc->state = WS_STATE_CLOSED;
            break;
            
        case WS_OPCODE_PING:
            // Respond with a pong frame
            websocket_send_frame(wsc, payload, payload_len, WS_OPCODE_PONG);
            break;
            
        case WS_OPCODE_PONG:
            // Just ignore pongs - we can use them for latency measurement if needed
            break;
            
        default:
            // Should never happen as this function is only called for control frames
            break;
    }
}

// Process incoming WebSocket data
// This function is a wrapper around websocket_receive_and_process_data
int websocket_process_frames(WEBSOCKET_SERVER_CLIENT *wsc) {
    if (!wsc || wsc->state == WS_STATE_CLOSED)
        return -1;
    
    // First try to receive data
    if(websocket_receive_data(wsc) < 0) {
        return -1;  // Error or connection closed
    }
    
    // Then process any complete frames in the buffer
    // This is the buffered implementation in websocket-receiver.c
    extern int websocket_process_frames_buffered(WEBSOCKET_SERVER_CLIENT *wsc);
    return websocket_process_frames_buffered(wsc);
}