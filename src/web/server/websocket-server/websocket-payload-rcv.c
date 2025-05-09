#include "daemon/common.h"
#include "websocket-internal.h"
#include "web/server/web_client.h"

#include "libnetdata/json/json-c-parser-inline.h"

// Create a new payload
WEBSOCKET_PAYLOAD *websocket_payload_create(WEBSOCKET_OPCODE opcode, const char *data, size_t length) {
    if (!data || !length)
        return NULL;

    WEBSOCKET_PAYLOAD *payload = mallocz(sizeof(WEBSOCKET_PAYLOAD));

    payload->opcode = opcode;

    // Initialize the buffer with the data
    websocket_buffer_init(&payload->buffer, length + (opcode == WS_OPCODE_TEXT ? 1 : 0));
    memcpy(payload->buffer.data, data, length);
    payload->buffer.length = length;

    // Null-terminate text data
    if (opcode == WS_OPCODE_TEXT && length < payload->buffer.size) {
        payload->buffer.data[length] = '\0';
    }

    return payload;
}

// Free a payload
void websocket_payload_free(WEBSOCKET_PAYLOAD *payload) {
    if (!payload)
        return;

    // Clean up the buffer
    websocket_buffer_cleanup(&payload->buffer);

    // Free the payload structure
    freez(payload);
}

// Process a payload - this is the main entry point for the payload layer
bool websocket_payload_process(WEBSOCKET_SERVER_CLIENT *wsc, WEBSOCKET_PAYLOAD *payload) {
    if (!wsc || !payload)
        return false;

    // All payloads should have a valid buffer (even empty ones)
    // Our payload preparation ensures this
    if (!payload->buffer.data)
        return false;
        
    // Process based on opcode
    switch (payload->opcode) {
        case WS_OPCODE_TEXT:
        case WS_OPCODE_BINARY:
            // Data payload - handle based on client's protocol
            switch (wsc->protocol) {
                case WS_PROTOCOL_NETDATA_JSON:
                    // Default behavior for netdata-json protocol: echo the payload
                    return websocket_payload_handle(wsc, payload);
                    break;
                    
                default:
                    // Unknown protocol - echo payload as a simple default
                    return websocket_payload_echo(wsc, payload);
                    break;
            }
            break;
            
        // Control frame payloads are handled at the protocol layer
        case WS_OPCODE_CLOSE:
        case WS_OPCODE_PING:
        case WS_OPCODE_PONG:
            // These should never reach here
            websocket_error(wsc, "Control frame payload in payload processing");
            return false;
            break;
            
        default:
            // Unknown opcode
            websocket_error(wsc, "Unknown opcode for payload: %d", payload->opcode);
            return false;
    }
    
    return false;
}

// Echo a payload back to the client - useful for testing
int websocket_payload_echo(WEBSOCKET_SERVER_CLIENT *wsc, WEBSOCKET_PAYLOAD *payload) {
    if (!wsc || !payload)
        return -1;

    // All payloads should have a valid buffer (even empty ones)
    // Our payload preparation ensures this
    if (!payload->buffer.data)
        return -1;

    websocket_debug(wsc, "Echoing payload: type=%s, length=%zu",
               (payload->opcode == WS_OPCODE_BINARY) ? "binary" : "text", payload->buffer.length);

    // Dump payload data for debugging
    websocket_dump_debug(wsc, payload->buffer.data, payload->buffer.length,
                      "Echo payload %s data", (payload->opcode == WS_OPCODE_BINARY) ? "binary" : "text");

    int result;

    // Echo the payload with the same opcode
    if (payload->opcode == WS_OPCODE_BINARY) {
        websocket_debug(wsc, "Sending binary echo response (length=%zu)", payload->buffer.length);

        // Handle both empty and non-empty binary messages
        // Our updated protocol_send_binary function can handle NULL/empty data
        result = websocket_protocol_send_binary(wsc, payload->buffer.data, payload->buffer.length);
    } else {
        websocket_debug(wsc, "Sending text echo response (length=%zu)", payload->buffer.length);

        // Text data should already be null-terminated in our buffer
        // This ensures we send only the actual payload data
        if (payload->buffer.data && payload->buffer.length > 0) {
            // Make sure it's null-terminated
            if (payload->buffer.length < payload->buffer.size) {
                payload->buffer.data[payload->buffer.length] = '\0';
                result = websocket_protocol_send_text(wsc, payload->buffer.data);
            } else {
                // We need to make a copy to ensure null-termination
                char *text_copy = mallocz(payload->buffer.length + 1);
                memcpy(text_copy, payload->buffer.data, payload->buffer.length);
                text_copy[payload->buffer.length] = '\0';
                result = websocket_protocol_send_text(wsc, text_copy);
                freez(text_copy);
            }
        } else {
            // Empty text message
            result = websocket_protocol_send_text(wsc, "");
        }
    }

    websocket_debug(wsc, "Echo response result: %d", result);
    return result;
}

// Parse a JSON payload
struct json_object *websocket_payload_parse_json(WEBSOCKET_PAYLOAD *payload) {
    if (!payload || !payload->buffer.data || !payload->buffer.length)
        return NULL;

    // Make sure it's a text payload
    if (payload->opcode != WS_OPCODE_TEXT) {
        // We can't use websocket_error here because we don't have a client pointer
        netdata_log_error("WEBSOCKET: Attempted to parse binary data as JSON");
        return NULL;
    }

    // Ensure the text data is null-terminated
    if (payload->buffer.length < payload->buffer.size) {
        payload->buffer.data[payload->buffer.length] = '\0';
    } else {
        // This shouldn't happen with our buffer management
        netdata_log_error("WEBSOCKET: JSON text data not null-terminated");
        return NULL;
    }

    // Parse JSON using json-c
    struct json_object *json = json_tokener_parse(payload->buffer.data);
    if (!json) {
        // We can't use websocket_error here because we don't have a client pointer
        netdata_log_error("WEBSOCKET: Failed to parse JSON payload: %s", payload->buffer.data);
        return NULL;
    }

    return json;
}

// Send an error response back to the client
void websocket_payload_error(WEBSOCKET_SERVER_CLIENT *wsc, const char *error_message) {
    if (!wsc || !error_message)
        return;
    
    websocket_error(wsc, "Sending error response: %s", error_message);
        
    // Create a JSON error response using json-c
    struct json_object *error_obj = json_object_new_object();
    
    // Add an error object with message
    json_object_object_add(error_obj, "error", json_object_new_string(error_message));
    
    // Add status
    json_object_object_add(error_obj, "status", json_object_new_string("error"));
    
    // Convert to string
    const char *error_json = json_object_to_json_string_ext(error_obj, JSON_C_TO_STRING_PLAIN);
    
    // Send to client
    int result = websocket_protocol_send_text(wsc, error_json);
    
    if (result < 0) {
        websocket_error(wsc, "Failed to send error response");
    }
    
    // Free JSON object
    json_object_put(error_obj);
}

// Handle a payload based on client protocol
bool websocket_payload_handle(WEBSOCKET_SERVER_CLIENT *wsc, WEBSOCKET_PAYLOAD *payload) {
    if (!wsc || !payload || !payload->buffer.data || !payload->buffer.length)
        return false;

    websocket_debug(wsc, "Handling payload: type=%s, length=%zu",
               (payload->opcode == WS_OPCODE_BINARY) ? "binary" : "text", payload->buffer.length);

    // For now, we just echo back any message
    // This can be enhanced to handle specific operations based on the protocol

    // Call the message callback if set
    if (wsc->on_message) {
        websocket_debug(wsc, "Calling client message handler");
        wsc->on_message(wsc, payload->buffer.data, payload->buffer.length, payload->opcode);
    }

    // Echo the payload
    int result = websocket_payload_echo(wsc, payload);
    if (result <= 0) {
        websocket_error(wsc, "Failed to echo payload");
        return false;
    }

    return true;
}