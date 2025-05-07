#ifndef NETDATA_WEBSOCKET_INTERNAL_H
#define NETDATA_WEBSOCKET_INTERNAL_H

#include "daemon/common.h"
#include "websocket.h"

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
#define WS_FIN                     0x80
#define WS_MASK                    0x80
#define WS_MAX_FRAME_LENGTH        (1 << 20) // 1MB max frame size

// Maximum number of WebSocket threads
#define WEBSOCKET_MAX_THREADS 2

// WebSocket frame header structure
typedef struct websocket_frame_header {
    unsigned char fin:1;
    unsigned char rsv1:1;
    unsigned char rsv2:1;
    unsigned char rsv3:1;
    unsigned char opcode:4;
    unsigned char mask:1;
    unsigned char len:7;
} WEBSOCKET_FRAME_HEADER;

// Forward declaration for thread structure
struct websocket_thread;

// WebSocket connection context - full structure definition
struct websocket_server_client {
    WEBSOCKET_STATE state;
    ND_SOCK sock;        // Socket with SSL abstraction
    size_t id;           // Unique client ID
    time_t connected_t;  // Connection timestamp
    time_t last_activity_t; // Last activity timestamp
    
    // Buffer for message data
    BUFFER *in_buffer;   // Incoming data
    BUFFER *out_buffer;  // Outgoing data
    
    // Connection info
    char client_ip[INET6_ADDRSTRLEN];
    char client_port[NI_MAXSERV];
    WEBSOCKET_PROTOCOL protocol; // The negotiated subprotocol
    
    // Thread management
    struct websocket_thread *thread; // The thread handling this client
    struct websocket_server_client *prev; // Linked list for thread's client management
    struct websocket_server_client *next; // Linked list for thread's client management
    
    // Frame processing state
    bool frame_in_progress;         // Currently processing a frame
    WEBSOCKET_FRAME_HEADER current_frame; // Current frame being processed
    uint64_t frame_length;          // Length of the current frame
    uint64_t frame_read;            // Bytes read for current frame
    unsigned char mask_key[4];      // Masking key for current frame
    
    // Fragmented message state
    bool fragmented_message;        // Currently receiving a fragmented message
    WEBSOCKET_OPCODE fragmented_opcode; // Opcode of the fragmented message
    BUFFER *message_buffer;         // Buffer for accumulating fragmented message
    
    // Message handling callbacks
    void (*on_message)(struct websocket_server_client *wsc, const char *message, size_t length, WEBSOCKET_OPCODE opcode);
    void (*on_close)(struct websocket_server_client *wsc, int code, const char *reason);
    void (*on_error)(struct websocket_server_client *wsc, const char *error);
    
    // User data for application use
    void *user_data;
};

// WebSocket thread structure
typedef struct websocket_thread {
    size_t id;                       // Thread ID
    struct {
        ND_THREAD *thread;           // Thread handle
        bool running;                // Thread running status
        SPINLOCK spinlock;           // Thread spinlock
    };

    size_t clients_current;                   // Current number of clients in the thread
    SPINLOCK clients_spinlock;                // Spinlock for client operations
    struct websocket_server_client *clients_first; // First client in the thread
    struct websocket_server_client *clients_last;  // Last client in the thread

    nd_poll_t *ndpl;             // Poll instance

    struct {
        int pipe[2];                 // Command pipe [0] = read, [1] = write
    } cmd;

} WEBSOCKET_THREAD;

// Global array of WebSocket threads
extern WEBSOCKET_THREAD websocket_threads[WEBSOCKET_MAX_THREADS];

// Define JudyL typed structure for WebSocket clients
DEFINE_JUDYL_TYPED(WS_CLIENTS, struct websocket_server_client *);

// WebSocket thread commands
#define WEBSOCKET_THREAD_CMD_EXIT            1
#define WEBSOCKET_THREAD_CMD_ADD_CLIENT      2
#define WEBSOCKET_THREAD_CMD_REMOVE_CLIENT   3
#define WEBSOCKET_THREAD_CMD_BROADCAST       4

// Thread management
void websocket_threads_init(void);
WEBSOCKET_THREAD *websocket_thread_assign_client(struct websocket_server_client *wsc);
bool websocket_thread_add_client_to_poll(WEBSOCKET_THREAD *wth, struct websocket_server_client *wsc);
void websocket_thread_remove_client_from_poll(WEBSOCKET_THREAD *wth, struct websocket_server_client *wsc);
void websocket_threads_cancel(void);
bool websocket_thread_send_command(WEBSOCKET_THREAD *wth, uint8_t cmd, void *data, size_t data_len);
void websocket_thread_process_commands(WEBSOCKET_THREAD *wth);
void *websocket_thread(void *ptr);
void websocket_thread_enqueue_client(WEBSOCKET_THREAD *wth, struct websocket_server_client *wsc);
void websocket_thread_dequeue_client(WEBSOCKET_THREAD *wth, struct websocket_server_client *wsc);
bool websocket_thread_update_client_poll_flags(WEBSOCKET_THREAD *wth, struct websocket_server_client *wsc, nd_poll_event_t events);
WEBSOCKET_THREAD *websocket_thread_by_id(size_t id);

// Frame processing
int websocket_process_frames(struct websocket_server_client *wsc);
int websocket_process_frames_buffered(struct websocket_server_client *wsc);
int websocket_receive_data(struct websocket_server_client *wsc);
int websocket_write_data(struct websocket_server_client *wsc);

// Client registry internals
bool websocket_client_register(struct websocket_server_client *wsc);
void websocket_client_unregister(struct websocket_server_client *wsc);
struct websocket_server_client *websocket_client_find_by_id(size_t id);
void websocket_clients_foreach(void (*callback)(struct websocket_server_client *wsc, void *data), void *data);
size_t websocket_clients_count(void);

// Utility functions
char *websocket_generate_handshake_key(const char *client_key);

// Frame handling functions
int websocket_send_frame(struct websocket_server_client *wsc, const char *payload, size_t payload_len, WEBSOCKET_OPCODE opcode);
void websocket_ping_client(struct websocket_server_client *wsc);

#endif // NETDATA_WEBSOCKET_INTERNAL_H