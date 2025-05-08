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
    payload->is_binary = (opcode == WS_OPCODE_BINARY);
    payload->data = strndupz(data, length);
    payload->length = length;
    payload->needs_free = true;
    
    return payload;
}

// Free a payload
void websocket_payload_free(WEBSOCKET_PAYLOAD *payload) {
    if (!payload)
        return;
        
    // Free data if needed
    if (payload->needs_free && payload->data)
        freez(payload->data);
        
    // Free the payload structure
    freez(payload);
}

// Process a payload - this is the main entry point for the payload layer
bool websocket_payload_process(WEBSOCKET_SERVER_CLIENT *wsc, WEBSOCKET_PAYLOAD *payload) {
    if (!wsc || !payload || !payload->data || !payload->length)
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
    if (!wsc || !payload || !payload->data || !payload->length)
        return -1;
        
    websocket_debug(wsc, "Echoing payload: type=%s, length=%zu",
               payload->is_binary ? "binary" : "text", payload->length);
    
#ifdef NETDATA_INTERNAL_CHECKS
    // Log the payload data (hex dump and text for small payloads)
    char payload_dump[128] = "";
    for (size_t i = 0; i < payload->length && i < 16; i++) {
        char *pos = payload_dump + strlen(payload_dump);
        snprintf(pos, sizeof(payload_dump) - strlen(payload_dump), "%02x ", (unsigned char)payload->data[i]);
    }
    websocket_debug(wsc, "Echo payload data: %s%s",
               payload_dump, payload->length > 16 ? "..." : "");
    
    // For small text payloads, log the actual content
    if (!payload->is_binary && payload->length < 100) {
        char text_preview[101];
        strncpy(text_preview, payload->data, payload->length > 100 ? 100 : payload->length);
        text_preview[payload->length > 100 ? 100 : payload->length] = '\0';
        websocket_debug(wsc, "Echo text content: '%s'", text_preview);
    }
#endif /* NETDATA_INTERNAL_CHECKS */
    
    int result;
    
    // Echo the payload with the same opcode
    if (payload->is_binary) {
        websocket_debug(wsc, "Sending binary echo response");
        result = websocket_protocol_send_binary(wsc, payload->data, payload->length);
    } else {
        websocket_debug(wsc, "Sending text echo response");
        
        // Create a properly null-terminated copy of the text data for sending
        // This prevents sending garbage bytes beyond the actual payload length
        char *text_copy = strndupz(payload->data, payload->length);
        result = websocket_protocol_send_text(wsc, text_copy);
        freez(text_copy);
    }
    
    websocket_debug(wsc, "Echo response result: %d", result);
    return result;
}

// Parse a JSON payload
struct json_object *websocket_payload_parse_json(WEBSOCKET_PAYLOAD *payload) {
    if (!payload || !payload->data || !payload->length)
        return NULL;
        
    // Make sure it's a text payload
    if (payload->opcode != WS_OPCODE_TEXT) {
        // We can't use websocket_error here because we don't have a client pointer
        netdata_log_error("WEBSOCKET: Attempted to parse binary data as JSON");
        return NULL;
    }
    
    // Parse JSON using json-c
    struct json_object *json = json_tokener_parse(payload->data);
    if (!json) {
        // We can't use websocket_error here because we don't have a client pointer
        netdata_log_error("WEBSOCKET: Failed to parse JSON payload: %s", payload->data);
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
    if (!wsc || !payload || !payload->data || !payload->length)
        return false;
    
    websocket_debug(wsc, "Handling payload: type=%s, length=%zu", 
               payload->is_binary ? "binary" : "text", payload->length);
        
    // For now, we just echo back any message
    // This can be enhanced to handle specific operations based on the protocol
    
    // Call the message callback if set
    if (wsc->on_message) {
        websocket_debug(wsc, "Calling client message handler");
        wsc->on_message(wsc, payload->data, payload->length, payload->opcode);
    }
    
    // Echo the payload
    int result = websocket_payload_echo(wsc, payload);
    if (result <= 0) {
        websocket_error(wsc, "Failed to echo payload");
        return false;
    }
    
    return true;
}