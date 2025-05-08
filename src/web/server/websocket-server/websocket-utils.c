#include "daemon/common.h"
#include "websocket-internal.h"
#include <stdarg.h>

// Debug log function with client, message, and frame IDs
void websocket_debug(WEBSOCKET_SERVER_CLIENT *wsc, const char *format, ...) {
#ifdef NETDATA_INTERNAL_CHECKS
    if (!wsc || !format)
        return;
        
    char formatted_message[1024];
    va_list args;
    va_start(args, format);
    vsnprintf(formatted_message, sizeof(formatted_message), format, args);
    va_end(args);
    
    // Format the debug message with client, message, and frame IDs
    netdata_log_debug(D_WEBSOCKET, "WEBSOCKET: C=%zu M=%zu F=%zu %s",
                  wsc->id, wsc->message_id, wsc->frame_id, formatted_message);
#endif /* NETDATA_INTERNAL_CHECKS */
}

// Info log function with client, message, and frame IDs
void websocket_info(WEBSOCKET_SERVER_CLIENT *wsc, const char *format, ...) {
    if (!wsc || !format)
        return;
        
    char formatted_message[1024];
    va_list args;
    va_start(args, format);
    vsnprintf(formatted_message, sizeof(formatted_message), format, args);
    va_end(args);
    
    // Format the info message with client, message, and frame IDs
    netdata_log_info("WEBSOCKET: C=%zu M=%zu F=%zu %s",
                 wsc->id, wsc->message_id, wsc->frame_id, formatted_message);
}

// Error log function with client, message, and frame IDs
void websocket_error(WEBSOCKET_SERVER_CLIENT *wsc, const char *format, ...) {
    if (!wsc || !format)
        return;
        
    char formatted_message[1024];
    va_list args;
    va_start(args, format);
    vsnprintf(formatted_message, sizeof(formatted_message), format, args);
    va_end(args);
    
    // Format the error message with client, message, and frame IDs
    netdata_log_error("WEBSOCKET: C=%zu M=%zu F=%zu %s",
                  wsc->id, wsc->message_id, wsc->frame_id, formatted_message);
}