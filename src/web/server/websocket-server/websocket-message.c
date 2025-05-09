// SPDX-License-Identifier: GPL-3.0-or-later

#include "daemon/common.h"
#include "websocket-internal.h"

// Helper function to determine if an opcode is a control opcode
bool websocket_frame_is_control_opcode(WEBSOCKET_OPCODE opcode) {
    return (opcode == WS_OPCODE_CLOSE ||
            opcode == WS_OPCODE_PING ||
            opcode == WS_OPCODE_PONG);
}

// Reset a client's message state for a new message
void websocket_client_message_reset(WS_CLIENT *wsc) {
    if (!wsc)
        return;

    // Reset message buffer
    wsb_reset(&wsc->payload);

    // Also reset uncompressed buffer to avoid keeping stale data
    wsb_reset(&wsc->u_payload);

    // Reset client's message state
    wsc->message_complete = true; // Initial state is complete (no fragmented message in progress)
    wsc->is_compressed = false;
    wsc->opcode = WS_OPCODE_TEXT; // Default opcode
    wsc->frame_id = 0;
}

// Process a complete message (decompress if needed and call handler)
bool websocket_client_process_message(WS_CLIENT *wsc) {
    if (!wsc || !wsc->message_complete)
        return false;

    worker_is_busy(WORKERS_WEBSOCKET_MESSAGE);

    websocket_debug(wsc, "Processing message (opcode=0x%x, is_compressed=%d, length=%zu)",
               wsc->opcode, wsc->is_compressed,
        wsb_length(&wsc->payload));

    // Handle control frames immediately
    if (wsc->opcode != WS_OPCODE_TEXT && wsc->opcode != WS_OPCODE_BINARY) {
        websocket_debug(wsc, "Control frame (opcode=0x%x) should not be handled by %s()", wsc->opcode, __FUNCTION__);
        return false;
    }

    // At this point, we know we're dealing with a data frame (text or binary)

    // Handle decompression if needed
    if (wsc->is_compressed) {
        if (!websocket_client_decompress_message(wsc)) {
            websocket_client_send_close(wsc, WS_CLOSE_INTERNAL_ERROR, "Decompression failed");
            return false;
        }
    } else {
        // For uncompressed messages, we just use payload buffer directly

        // Reset the uncompressed buffer (maintain allocated memory)
        wsb_reset(&wsc->u_payload);

        if (wsc->payload.length == 0) {
            // Empty message - handle specially
            websocket_debug(wsc, "Handling empty %s message",
                          (wsc->opcode == WS_OPCODE_BINARY) ? "binary" : "text");

            // For text messages, ensure null termination
            if (wsc->opcode == WS_OPCODE_TEXT) {
                wsb_null_terminate(&wsc->u_payload);
            }
        } else {
            // Normal non-empty message - copy from payload buffer
            // The buffer should already be pre-allocated with sufficient size

            // Copy data from payload to uncompressed buffer
            wsb_append(&wsc->u_payload, wsb_data(&wsc->payload), wsb_length(&wsc->payload));

            // Ensure null termination for text data
            if (wsc->opcode == WS_OPCODE_TEXT) {
                wsb_null_terminate(&wsc->u_payload);
            }
        }
    }

    // Now handle the uncompressed message - using the new function
    // that contains the actual handler logic
    bool result = websocket_payload_handle_message(wsc);

    if (!result) {
        // If message handling failed, just call the on_message callback directly as a fallback
        if (wsc->on_message) {
            wsc->on_message(wsc,
                wsb_data(&wsc->u_payload), wsb_length(&wsc->u_payload),
                          wsc->opcode);
        }
    }

    // Update client message stats
    wsc->message_id++;
    wsc->frame_id = 0;

    // Reset for the next message
    websocket_client_message_reset(wsc);

    return true;
}
