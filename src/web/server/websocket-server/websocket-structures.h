#ifndef NETDATA_WEBSOCKET_STRUCTURES_H
#define NETDATA_WEBSOCKET_STRUCTURES_H

#include "daemon/common.h"
#include "websocket.h"
#include "websocket-compression.h"

// WebSocket protocol constants
#define WS_GUID "258EAFA5-E914-47DA-95CA-C5AB0DC85B11"

// WebSocket close codes (RFC 6455)
#define WS_CLOSE_NORMAL            1000
#define WS_CLOSE_GOING_AWAY        1001
#define WS_CLOSE_PROTOCOL_ERROR    1002
#define WS_CLOSE_UNSUPPORTED_DATA  1003
#define WS_CLOSE_POLICY_VIOLATION  1008
#define WS_CLOSE_MESSAGE_TOO_BIG   1009
#define WS_CLOSE_INTERNAL_ERROR    1011

// WebSocket frame constants
#define WS_FIN                     0x80  // Final frame bit
#define WS_RSV1                    0x40  // Reserved bit 1 (used for compression)
#define WS_MASK                    0x80  // Mask bit
// Frame size limit - affects fragmentation but not total message size
#define WS_MAX_FRAME_LENGTH        (20 * 1024 * 1024) // 20MB max frame size

// Total message size limits - these prevent resource exhaustion
#define WEBSOCKET_MAX_COMPRESSED_SIZE    (20ULL * 1024 * 1024) // 20MB max compressed message
#define WEBSOCKET_MAX_UNCOMPRESSED_SIZE  (200ULL * 1024 * 1024) // 200MB max uncompressed message

// WebSocket frame header structure - used for processing frame headers
typedef struct websocket_frame_header {
    unsigned char fin:1;
    unsigned char rsv1:1;
    unsigned char rsv2:1;
    unsigned char rsv3:1;
    unsigned char opcode:4;
    unsigned char mask:1;
    unsigned char len:7;

    unsigned char mask_key[4];      // Masking key (if present)
    size_t frame_size;              // Size of the entire frame
    size_t header_size;             // Size of the header
    size_t payload_length;          // Length of the payload data
    void *payload;                  // Pointer to the payload data
} WEBSOCKET_FRAME_HEADER;

// Buffer for message data (used for reassembly of fragmented messages)
typedef struct websocket_buffer {
    char *data;                  // Buffer holding message data
    size_t length;               // Current buffer length
    size_t alloc_size;           // Allocated buffer size
    size_t pos;                  // Current position for reading/writing
} WEBSOCKET_BUFFER;

// WebSocket message structure - contains a complete or partially complete message
typedef struct websocket_message {
    // Message metadata
    WEBSOCKET_OPCODE opcode;         // Message type (from first frame)
    bool is_compressed;              // Whether the message is compressed (RSV1 bit set)

    // Message data
    WEBSOCKET_BUFFER buffer;         // Buffer for complete message data

    // Processing state
    bool complete;                   // Whether the message is complete (all fragments received)
    uint64_t total_length;           // Total length of the reassembled message
    // Note: message is in a fragmented sequence when complete=false

} WEBSOCKET_MESSAGE;

// WebSocket payload structure - contains the processed, unmasked, uncompressed message data
typedef struct websocket_payload {
    WEBSOCKET_OPCODE opcode;         // Payload type
    char *data;                      // Payload data (unmasked and uncompressed)
    size_t length;                   // Payload length
    bool is_binary;                  // Whether payload is binary data
    bool needs_free;                 // Whether data needs to be freed
} WEBSOCKET_PAYLOAD;

// Forward declaration for client structure
struct websocket_server_client;

// Function prototypes for message/payload handling

// Buffer functions
void websocket_buffer_init(WEBSOCKET_BUFFER *buffer, size_t initial_size);
void websocket_buffer_cleanup(WEBSOCKET_BUFFER *buffer);
WEBSOCKET_BUFFER *websocket_buffer_create(size_t initial_size);
void websocket_buffer_free(WEBSOCKET_BUFFER *buffer);
void websocket_buffer_resize(WEBSOCKET_BUFFER *buffer, size_t new_size);
void websocket_buffer_expand(WEBSOCKET_BUFFER *buffer, size_t length);
void websocket_buffer_reset(WEBSOCKET_BUFFER *buffer);

// Message functions
NEVERNULL
WEBSOCKET_MESSAGE *websocket_message_create(size_t initial_size);

void websocket_message_free(WEBSOCKET_MESSAGE *msg);
void websocket_message_reset(WEBSOCKET_MESSAGE *msg);
bool websocket_message_process_frame_data(struct websocket_server_client *wsc, WEBSOCKET_MESSAGE *msg, const char *data, size_t length);
bool websocket_message_is_complete(const WEBSOCKET_MESSAGE *msg);
bool websocket_message_is_control(const WEBSOCKET_MESSAGE *msg);
WEBSOCKET_PAYLOAD *websocket_message_to_payload(struct websocket_server_client *wsc, WEBSOCKET_MESSAGE *msg);

// Payload functions
WEBSOCKET_PAYLOAD *websocket_payload_create(WEBSOCKET_OPCODE opcode, const char *data, size_t length);
void websocket_payload_free(WEBSOCKET_PAYLOAD *payload);
int websocket_payload_send(struct websocket_server_client *wsc, WEBSOCKET_PAYLOAD *payload);
WEBSOCKET_PAYLOAD *websocket_payload_create_text(const char *text);
WEBSOCKET_PAYLOAD *websocket_payload_create_binary(const void *data, size_t length);
int websocket_payload_send_text(struct websocket_server_client *wsc, const char *text);
int websocket_payload_send_binary(struct websocket_server_client *wsc, const void *data, size_t length);
int websocket_payload_send_text_fmt(struct websocket_server_client *wsc, const char *format, ...);

// Additional functions to help with transition from old code to new
bool websocket_frame_is_control_opcode(WEBSOCKET_OPCODE opcode);

#endif // NETDATA_WEBSOCKET_STRUCTURES_H