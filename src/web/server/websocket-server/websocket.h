// SPDX-License-Identifier: GPL-3.0-or-later

#ifndef NETDATA_WEB_SERVER_WEBSOCKET_H
#define NETDATA_WEB_SERVER_WEBSOCKET_H 1

#include "libnetdata/libnetdata.h"

// WebSocket frame opcodes as per RFC 6455
typedef enum __attribute__((packed)) {
    WS_OPCODE_CONTINUATION = 0x0,
    WS_OPCODE_TEXT         = 0x1,
    WS_OPCODE_BINARY       = 0x2,
    WS_OPCODE_CLOSE        = 0x8,
    WS_OPCODE_PING         = 0x9,
    WS_OPCODE_PONG         = 0xA
} WEBSOCKET_OPCODE;

// WebSocket subprotocols supported by Netdata
typedef enum __attribute__((packed)) {
    WS_PROTOCOL_UNKNOWN    = 0,     // Unknown or unsupported protocol
    WS_PROTOCOL_NETDATA_JSON = 1,   // Basic JSON protocol
    // Additional protocols will be added here in the future
} WEBSOCKET_PROTOCOL;

// WebSocket connection states
typedef enum __attribute__((packed)) {
    WS_STATE_HANDSHAKE  = 0, // Initial handshake in progress
    WS_STATE_OPEN       = 1, // Connection established
    WS_STATE_CLOSING    = 2, // Closing in progress
    WS_STATE_CLOSED     = 3  // Connection closed
} WEBSOCKET_STATE;

// Forward declarations for websocket client
typedef struct websocket_server_client WEBSOCKET_SERVER_CLIENT;

// Public WebSocket API functions
struct web_client;

// WebSocket detection and handshake
bool websocket_detect_handshake_request(struct web_client *w);
short int websocket_handle_handshake(struct web_client *w);

// WebSocket socket takeover
void websocket_takeover_connection(struct web_client *w, WEBSOCKET_SERVER_CLIENT *wsc);

// Initialize the WebSocket subsystem
void websocket_initialize(void);

// WebSocket message sending functions
int websocket_send_message(WEBSOCKET_SERVER_CLIENT *wsc, const char *message, size_t length, WEBSOCKET_OPCODE opcode);
int websocket_broadcast_message(const char *message, size_t length, WEBSOCKET_OPCODE opcode);

// WebSocket connection management
void websocket_close_client(WEBSOCKET_SERVER_CLIENT *wsc, int close_code, const char *reason);
void websocket_ping_client(WEBSOCKET_SERVER_CLIENT *wsc);

#endif // NETDATA_WEB_SERVER_WEBSOCKET_H