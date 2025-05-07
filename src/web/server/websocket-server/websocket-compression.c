#include "daemon/common.h"
#include "websocket-compression.h"
#include "websocket-internal.h"

// Check if zlib is available and working
bool websocket_compression_supported(void) {
    // Simple version check for zlib
    const char *version = zlibVersion();
    if (!version || version[0] == '\0')
        return false;
    
    return true;
}

// Initialize compression for a WebSocket client
bool websocket_compression_init(struct websocket_server_client *wsc, const char *extensions) {
    if (!wsc)
        return false;
    
    // Initialize compression context to default values
    wsc->compression.type = WS_COMPRESS_NONE;
    wsc->compression.enabled = false;
    wsc->compression.client_context_takeover = true;
    wsc->compression.server_context_takeover = true;
    wsc->compression.window_bits = WS_COMPRESS_WINDOW_BITS;
    wsc->compression.compression_level = WS_COMPRESS_DEFAULT_LEVEL;
    wsc->compression.deflate_stream = NULL;
    wsc->compression.inflate_stream = NULL;
    wsc->compression.fragmented_compression = false;
    wsc->compression.in_fragmented_message = false;
    
    // If extensions is NULL or empty, compression is disabled
    if (!extensions || *extensions == '\0')
        return true;
    
    // Check if "permessage-deflate" is requested
    if (strstr(extensions, "permessage-deflate") != NULL) {
        // Parse extension parameters
        char extension_copy[1024];
        strncpy(extension_copy, extensions, sizeof(extension_copy) - 1);
        extension_copy[sizeof(extension_copy) - 1] = '\0';
        
        char *token, *saveptr;
        token = strtok_r(extension_copy, ",", &saveptr);
        
        while (token) {
            // Trim leading/trailing spaces
            char *ext = token;
            while (*ext && isspace(*ext)) ext++;
            char *end = ext + strlen(ext) - 1;
            while (end > ext && isspace(*end)) *end-- = '\0';
            
            // Check if this is permessage-deflate extension
            if (strncmp(ext, "permessage-deflate", 18) == 0) {
                wsc->compression.type = WS_COMPRESS_DEFLATE;
                wsc->compression.enabled = true;
                
                // Parse parameters
                char *params = ext + 18;
                if (*params == ';') {
                    params++;
                    
                    char *param, *param_saveptr;
                    param = strtok_r(params, ";", &param_saveptr);
                    
                    while (param) {
                        // Trim leading/trailing spaces
                        while (*param && isspace(*param)) param++;
                        end = param + strlen(param) - 1;
                        while (end > param && isspace(*end)) *end-- = '\0';
                        
                        // Client no context takeover
                        if (strcmp(param, "client_no_context_takeover") == 0) {
                            wsc->compression.client_context_takeover = false;
                        }
                        // Server no context takeover
                        else if (strcmp(param, "server_no_context_takeover") == 0) {
                            wsc->compression.server_context_takeover = false;
                        }
                        // Client max window bits
                        else if (strncmp(param, "client_max_window_bits", 22) == 0) {
                            // Just acknowledge, we'll use our defaults
                        }
                        // Server max window bits
                        else if (strncmp(param, "server_max_window_bits=", 23) == 0) {
                            int bits = atoi(param + 23);
                            if (bits >= 8 && bits <= 15) {
                                wsc->compression.window_bits = bits;
                            }
                        }
                        
                        param = strtok_r(NULL, ";", &param_saveptr);
                    }
                }
                
                break;  // Found and parsed permessage-deflate
            }
            
            token = strtok_r(NULL, ",", &saveptr);
        }
    }
    
    // Initialize zlib contexts if compression is enabled
    if (wsc->compression.enabled) {
        // Initialize deflate (compression) context
        wsc->compression.deflate_stream = mallocz(sizeof(z_stream));
        wsc->compression.deflate_stream->zalloc = Z_NULL;
        wsc->compression.deflate_stream->zfree = Z_NULL;
        wsc->compression.deflate_stream->opaque = Z_NULL;
        
        // Initialize with negative window bits for raw deflate (no zlib/gzip header)
        int ret = deflateInit2(
            wsc->compression.deflate_stream,
            wsc->compression.compression_level,
            Z_DEFLATED,
            -wsc->compression.window_bits,
            WS_COMPRESS_MEMLEVEL,
            Z_DEFAULT_STRATEGY
        );
        
        if (ret != Z_OK) {
            netdata_log_error("WEBSOCKET: Failed to initialize deflate context for client %zu: %s (%d)", 
                          wsc->id, zError(ret), ret);
            freez(wsc->compression.deflate_stream);
            wsc->compression.deflate_stream = NULL;
            wsc->compression.enabled = false;
            return false;
        }
        
        // Initialize inflate (decompression) context
        wsc->compression.inflate_stream = mallocz(sizeof(z_stream));
        wsc->compression.inflate_stream->zalloc = Z_NULL;
        wsc->compression.inflate_stream->zfree = Z_NULL;
        wsc->compression.inflate_stream->opaque = Z_NULL;
        
        // Initialize with negative window bits for raw deflate (no zlib/gzip header)
        ret = inflateInit2(
            wsc->compression.inflate_stream,
            -wsc->compression.window_bits
        );
        
        if (ret != Z_OK) {
            netdata_log_error("WEBSOCKET: Failed to initialize inflate context for client %zu: %s (%d)", 
                          wsc->id, zError(ret), ret);
            deflateEnd(wsc->compression.deflate_stream);
            freez(wsc->compression.deflate_stream);
            wsc->compression.deflate_stream = NULL;
            freez(wsc->compression.inflate_stream);
            wsc->compression.inflate_stream = NULL;
            wsc->compression.enabled = false;
            return false;
        }
        
        netdata_log_info("WEBSOCKET: Compression enabled for client %zu (permessage-deflate, window bits: %d)",
                      wsc->id, wsc->compression.window_bits);
    }
    
    return true;
}

// Clean up compression resources for a WebSocket client
void websocket_compression_cleanup(struct websocket_server_client *wsc) {
    if (!wsc || !wsc->compression.enabled)
        return;
    
    // Clean up deflate context
    if (wsc->compression.deflate_stream) {
        deflateEnd(wsc->compression.deflate_stream);
        freez(wsc->compression.deflate_stream);
        wsc->compression.deflate_stream = NULL;
    }
    
    // Clean up inflate context
    if (wsc->compression.inflate_stream) {
        inflateEnd(wsc->compression.inflate_stream);
        freez(wsc->compression.inflate_stream);
        wsc->compression.inflate_stream = NULL;
    }
    
    wsc->compression.enabled = false;
}

// Generate the extension string for the WebSocket handshake response
char *websocket_compression_negotiate(struct websocket_server_client *wsc, const char *requested_extensions) {
    if (!wsc || !requested_extensions || !*requested_extensions)
        return NULL;
    
    // Initialize compression with the requested extensions
    if (!websocket_compression_init(wsc, requested_extensions))
        return NULL;
    
    // If compression wasn't enabled, return NULL
    if (!wsc->compression.enabled)
        return NULL;
    
    // Build the response extension string
    BUFFER *wb = buffer_create(256, NULL);
    
    switch (wsc->compression.type) {
        case WS_COMPRESS_DEFLATE:
            buffer_strcat(wb, "permessage-deflate");
            
            // Add parameters if different from defaults
            if (!wsc->compression.client_context_takeover)
                buffer_strcat(wb, "; client_no_context_takeover");
            
            if (!wsc->compression.server_context_takeover)
                buffer_strcat(wb, "; server_no_context_takeover");
            
            if (wsc->compression.window_bits != WS_COMPRESS_WINDOW_BITS)
                buffer_sprintf(wb, "; server_max_window_bits=%d", wsc->compression.window_bits);
            break;
            
        default:
            // Unsupported compression type
            buffer_free(wb);
            return NULL;
    }
    
    char *result = strdupz(buffer_tostring(wb));
    buffer_free(wb);
    return result;
}

// Compress a payload
int websocket_compress_payload(struct websocket_server_client *wsc, const char *in, size_t in_len, char **out, size_t *out_len) {
    if (!wsc || !in || !in_len || !out || !out_len)
        return -1;
    
    // Don't compress tiny payloads - they might actually get larger
    if (in_len < WS_COMPRESS_MIN_SIZE) {
        *out = mallocz(in_len);
        memcpy(*out, in, in_len);
        *out_len = in_len;
        netdata_log_debug(D_WEBSOCKET, "WEBSOCKET: Client %zu - Skipping compression for small payload (%zu bytes)",
                       wsc->id, in_len);
        return 0;
    }
    
    // Check if compression is enabled
    if (!wsc->compression.enabled || !wsc->compression.deflate_stream)
        return -1;
    
    // Log current compression context information
    netdata_log_debug(D_WEBSOCKET, "WEBSOCKET: Client %zu - Compression state before: total_in=%lu, total_out=%lu",
                  wsc->id, 
                  wsc->compression.deflate_stream->total_in,
                  wsc->compression.deflate_stream->total_out);
                  
    // Respect server_context_takeover setting
    // This determines whether the server keeps its compression dictionary between messages
    if (!wsc->compression.server_context_takeover) {
        netdata_log_debug(D_WEBSOCKET, "WEBSOCKET: Client %zu - No server context takeover, resetting deflate stream",
                      wsc->id);
        deflateReset(wsc->compression.deflate_stream);
    } else {
        netdata_log_debug(D_WEBSOCKET, "WEBSOCKET: Client %zu - Server context takeover enabled, preserving compression dictionary",
                      wsc->id);
    }
    
    // Allocate output buffer - worst case scenario for zlib is input size + 0.1% + 12 bytes
    // But add more buffer to be safe, especially for small inputs (minimum 1KB)
    size_t max_out_len = in_len + (in_len / 100) + 1024; 
    *out = mallocz(max_out_len);
    
    // Set up the stream for this compression operation
    wsc->compression.deflate_stream->next_in = (Bytef *)in;
    wsc->compression.deflate_stream->avail_in = in_len;
    wsc->compression.deflate_stream->next_out = (Bytef *)(*out);
    wsc->compression.deflate_stream->avail_out = max_out_len;
    
    // Compress with sync flush to ensure data is flushed to output
    int ret = deflate(wsc->compression.deflate_stream, Z_SYNC_FLUSH);
    
    // Check if we need to expand the buffer
    if (ret == Z_BUF_ERROR || wsc->compression.deflate_stream->avail_out == 0) {
        // This shouldn't happen with our buffer size calculation, but handle it just in case
        netdata_log_error("WEBSOCKET: Client %zu - Compression buffer too small for %zu bytes (this is unexpected)", 
                      wsc->id, in_len);
        
        max_out_len *= 2;
        
        // Reallocate buffer
        *out = reallocz(*out, max_out_len);
        
        // We need to preserve state, so save the current state if server_context_takeover is true
        z_stream saved_stream;
        bool state_saved = false;
        
        if (wsc->compression.server_context_takeover) {
            // Save deflate state before resetting
            memcpy(&saved_stream, wsc->compression.deflate_stream, sizeof(z_stream));
            state_saved = true;
        }
        
        // Reset stream for retry
        deflateReset(wsc->compression.deflate_stream);
        
        // Restore state if needed (this is a simplification; in reality, we might need more complex state restoration)
        if (state_saved) {
            // Copy back only the dictionary-related fields, not the I/O pointers
            // This is a simplification; ideally we would use deflateCopy but it requires more complex handling
        }
        
        // Set up the stream for retry
        wsc->compression.deflate_stream->next_in = (Bytef *)in;
        wsc->compression.deflate_stream->avail_in = in_len;
        wsc->compression.deflate_stream->next_out = (Bytef *)(*out);
        wsc->compression.deflate_stream->avail_out = max_out_len;
        
        ret = deflate(wsc->compression.deflate_stream, Z_SYNC_FLUSH);
    }
    
    if (ret != Z_OK && ret != Z_STREAM_END) {
        netdata_log_error("WEBSOCKET: Compression failed for client %zu: %s (%d)", 
                      wsc->id, zError(ret), ret);
        freez(*out);
        *out = NULL;
        return -1;
    }
    
    // Calculate compressed size
    *out_len = max_out_len - wsc->compression.deflate_stream->avail_out;
    
    // Log compression result details
    netdata_log_debug(D_WEBSOCKET, "WEBSOCKET: Client %zu - Compression state after: total_in=%lu, total_out=%lu",
                  wsc->id, 
                  wsc->compression.deflate_stream->total_in,
                  wsc->compression.deflate_stream->total_out);
    
    // As per RFC 7692, we need to remove the trailing 4 bytes (00 00 FF FF)
    // which are added by Z_SYNC_FLUSH but not needed for WebSocket compression
    if (*out_len >= 4)
        *out_len -= 4;
    
    // If compression didn't actually save space, use the original data
    if (*out_len >= in_len) {
        freez(*out);
        *out = mallocz(in_len);
        memcpy(*out, in, in_len);
        *out_len = in_len;
        netdata_log_debug(D_WEBSOCKET, "WEBSOCKET: Client %zu - Compression ineffective, using original data (%zu bytes)",
                       wsc->id, in_len);
        return 0;
    }
    
    // Log successful compression
    netdata_log_debug(D_WEBSOCKET, "WEBSOCKET: Client %zu - Successfully compressed %zu bytes to %zu bytes (%.1f%% reduction)",
                   wsc->id, in_len, *out_len, (100.0 - ((double)*out_len / (double)in_len) * 100.0));
    
    return 1;  // Successfully compressed
}

// Decompress a payload
int websocket_decompress_payload(struct websocket_server_client *wsc, const char *in, size_t in_len, char **out, size_t *out_len) {
    if (!wsc || !in || !out || !out_len)
        return -1;
    
    // Check if compression is enabled
    if (!wsc->compression.enabled || !wsc->compression.inflate_stream)
        return -1;
    
    // Log the detailed compression state for debugging
    netdata_log_debug(D_WEBSOCKET, "WEBSOCKET: Client %zu - Compression state: in_fragmented_message=%d, fragmented_compression=%d",
                  wsc->id, wsc->compression.in_fragmented_message ? 1 : 0, 
                  wsc->compression.fragmented_compression ? 1 : 0);
    
    // Per RFC 7692, we need to append 4 bytes (00 00 FF FF) to the compressed data
    // to ensure the inflate operation completes, but ONLY for the final frame of a message or standalone messages
    bool is_final_fragment = !wsc->compression.in_fragmented_message || 
                            (wsc->compression.in_fragmented_message && wsc->current_frame.fin);
                            
    netdata_log_debug(D_WEBSOCKET, "WEBSOCKET: Client %zu - is_final_fragment=%d, in_fragmented_message=%d, fin=%d",
                  wsc->id, is_final_fragment ? 1 : 0, 
                  wsc->compression.in_fragmented_message ? 1 : 0,
                  wsc->current_frame.fin);
    
    size_t actual_in_len;
    char *actual_in;
    
    if (is_final_fragment) {
        // For final fragments, add the termination bytes
        actual_in_len = in_len + 4;
        actual_in = mallocz(actual_in_len);
        memcpy(actual_in, in, in_len);
        actual_in[in_len] = 0x00;
        actual_in[in_len + 1] = 0x00;
        actual_in[in_len + 2] = 0xFF;
        actual_in[in_len + 3] = 0xFF;
        
        netdata_log_debug(D_WEBSOCKET, "WEBSOCKET: Client %zu - Final fragment or complete message, adding termination bytes",
                      wsc->id);
    } else {
        // For intermediate fragments, do not add the termination bytes
        actual_in_len = in_len;
        actual_in = mallocz(actual_in_len);
        memcpy(actual_in, in, in_len);
        
        netdata_log_debug(D_WEBSOCKET, "WEBSOCKET: Client %zu - Intermediate fragment, not adding termination bytes",
                      wsc->id);
    }
    
    // Debug dump the input buffer for diagnostic purposes (on every decompression attempt)
    netdata_log_debug(D_WEBSOCKET, "WEBSOCKET: Client %zu - Input buffer for decompression:", wsc->id);
    websocket_dump_buffer(in, in_len, 32, "Decompression input", wsc->id);
    
    // Start with a reasonably sized output buffer and grow if needed
    // For fragmented messages, expand the initial buffer size to better handle higher compression ratios
    size_t max_out_len = wsc->compression.in_fragmented_message ? (in_len * 20) : (in_len * 5);
    *out = mallocz(max_out_len);
    
    // Handle the inflate stream appropriately based on message type and context takeover settings
    if (!wsc->compression.in_fragmented_message && !wsc->compression.fragmented_compression) {
        // New standalone message (not fragmented)
        // For new messages, we respect client_context_takeover setting
        if (!wsc->compression.client_context_takeover) {
            netdata_log_debug(D_WEBSOCKET, "WEBSOCKET: Client %zu - New message with no context takeover, resetting inflate stream",
                        wsc->id);
            inflateReset(wsc->compression.inflate_stream);
        } else {
            netdata_log_debug(D_WEBSOCKET, "WEBSOCKET: Client %zu - New message with context takeover, preserving inflate state",
                        wsc->id);
        }
    } else {
        netdata_log_debug(D_WEBSOCKET, "WEBSOCKET: Client %zu - Continuing fragmented message, preserving inflate state",
                      wsc->id);
    }
    
    // Set up the stream for this decompression operation
    wsc->compression.inflate_stream->next_in = (Bytef *)actual_in;
    wsc->compression.inflate_stream->avail_in = actual_in_len;
    wsc->compression.inflate_stream->next_out = (Bytef *)(*out);
    wsc->compression.inflate_stream->avail_out = max_out_len;
    
    // Decompress with loop for multiple buffer expansions if needed
    int ret;
    int max_attempts = WS_COMPRESS_MAX_BUFFER_EXPANSIONS; // Limit number of expansions to prevent excessive memory usage
    
    // Debug dump the first few bytes of input data
    netdata_log_debug(D_WEBSOCKET, "WEBSOCKET: Client %zu - Decompressing %zu bytes of data, first bytes:", wsc->id, in_len);
    for (size_t i = 0; i < 16 && i < in_len; i++) {
        netdata_log_debug(D_WEBSOCKET, "  Byte %2zu: 0x%02x '%c'", i, (unsigned char)in[i], 
                      isprint((unsigned char)in[i]) ? in[i] : '.');
    }
    
    for (int attempt = 0; attempt < max_attempts; attempt++) {
        // Try to decompress
        ret = inflate(wsc->compression.inflate_stream, Z_SYNC_FLUSH);
        
        // Log inflate result for debugging
        netdata_log_debug(D_WEBSOCKET, "WEBSOCKET: Client %zu - inflate() returned %d (%s), "
                      "avail_in=%u, avail_out=%u, attempt %d/%d",
                      wsc->id, ret, zError(ret),
                      wsc->compression.inflate_stream->avail_in, 
                      wsc->compression.inflate_stream->avail_out,
                      attempt + 1, max_attempts);
        
        // Check if we need more output space
        if (ret == Z_BUF_ERROR || (ret == Z_OK && wsc->compression.inflate_stream->avail_out == 0 && 
                                  wsc->compression.inflate_stream->avail_in > 0)) {
            // We've used up the output buffer but there's still input data to process
            size_t bytes_decompressed = max_out_len - wsc->compression.inflate_stream->avail_out;
            size_t old_max_out_len = max_out_len;
            
            // Double the buffer size
            max_out_len *= 2;
            
            netdata_log_debug(D_WEBSOCKET, "WEBSOCKET: Client %zu - Decompression buffer too small (%zu bytes used), "
                           "growing from %zu to %zu bytes (attempt %d/%d)",
                           wsc->id, bytes_decompressed, old_max_out_len, max_out_len, attempt + 1, max_attempts);
            
            // Reallocate the buffer
            *out = reallocz(*out, max_out_len);
            
            // Update stream to use the larger buffer, continuing from where we left off
            wsc->compression.inflate_stream->next_out = (Bytef *)(*out + bytes_decompressed);
            wsc->compression.inflate_stream->avail_out = max_out_len - bytes_decompressed;
        }
        else {
            // Either an error or we've finished processing the input
            break;
        }
    }
    
    if (ret != Z_OK && ret != Z_STREAM_END && ret != Z_BUF_ERROR) {
        netdata_log_error("WEBSOCKET: Decompression failed for client %zu: %s (%d)", 
                      wsc->id, zError(ret), ret);
                      
        // Dump the input buffer for debugging
        netdata_log_debug(D_WEBSOCKET, "WEBSOCKET: Client %zu - Dumping input buffer that caused decompression failure:", wsc->id);
        websocket_dump_buffer(in, in_len, 32, "Failed input", wsc->id);
        
        freez(*out);
        *out = NULL;
        freez(actual_in);
        return -1;
    }
    
    freez(actual_in);
    
    // Calculate decompressed size
    *out_len = max_out_len - wsc->compression.inflate_stream->avail_out;
    
    // Verify we don't have a zero-size output (which indicates a problem)
    if (*out_len == 0) {
        netdata_log_error("WEBSOCKET: Client %zu - Decompression resulted in zero bytes output from %zu input bytes",
                      wsc->id, in_len);
        freez(*out);
        *out = NULL;
        return -1;
    }
    
    // Log successful decompression
    netdata_log_debug(D_WEBSOCKET, "WEBSOCKET: Client %zu - Successfully decompressed %zu bytes to %zu bytes",
                   wsc->id, in_len, *out_len);
    
    return 1;  // Successfully decompressed
}

// Helper function to dump buffer contents for debugging
void websocket_dump_buffer(const char *buffer, size_t len, size_t max_bytes, const char *prefix, size_t client_id) {
    if (!buffer || !len)
        return;
    
    // Limit the number of bytes to dump
    size_t bytes_to_dump = len < max_bytes ? len : max_bytes;
    
    // Build hex representation
    char hex_dump[1024] = "";
    char ascii_dump[256] = "";
    size_t hex_pos = 0;
    size_t ascii_pos = 0;
    
    for (size_t i = 0; i < bytes_to_dump; i++) {
        unsigned char c = (unsigned char)buffer[i];
        
        // Add to hex dump
        int chars_added = snprintf(hex_dump + hex_pos, sizeof(hex_dump) - hex_pos, "%02x ", c);
        if (chars_added > 0) {
            hex_pos += chars_added;
        }
        
        // Add to ASCII dump
        if (isprint(c)) {
            int chars_added = snprintf(ascii_dump + ascii_pos, sizeof(ascii_dump) - ascii_pos, "%c", c);
            if (chars_added > 0) {
                ascii_pos += chars_added;
            }
        } else {
            int chars_added = snprintf(ascii_dump + ascii_pos, sizeof(ascii_dump) - ascii_pos, ".");
            if (chars_added > 0) {
                ascii_pos += chars_added;
            }
        }
        
        // Print a full line (16 bytes) or at the end
        if ((i + 1) % 16 == 0 || i == bytes_to_dump - 1) {
            netdata_log_debug(D_WEBSOCKET, "WEBSOCKET: Client %zu - %s [%04zx]: %-48s | %s", 
                          client_id, prefix, i & ~0xF, hex_dump, ascii_dump);
            hex_pos = 0;
            ascii_pos = 0;
            hex_dump[0] = '\0';
            ascii_dump[0] = '\0';
        }
    }
    
    // If there's more data, indicate it
    if (len > bytes_to_dump) {
        netdata_log_debug(D_WEBSOCKET, "WEBSOCKET: Client %zu - %s: (%zu more bytes not shown)", 
                       client_id, prefix, len - bytes_to_dump);
    }
}

// Start handling a fragmented compressed message
void websocket_compression_fragmented_start(struct websocket_server_client *wsc) {
    if (!wsc || !wsc->compression.enabled)
        return;
    
    netdata_log_debug(D_WEBSOCKET, "WEBSOCKET: Client %zu - Starting fragmented compressed message, preserving zlib state", wsc->id);
    
    // Mark that we're in a fragmented message
    wsc->compression.in_fragmented_message = true;
    wsc->compression.fragmented_compression = true;
    
    // Initialize for a new fragmented message, but respect context takeover settings
    if (wsc->compression.inflate_stream) {
        // For a new fragmented message, we generally want to preserve the inflate state
        // unless it's explicitly set to no context takeover
        netdata_log_debug(D_WEBSOCKET, "WEBSOCKET: Client %zu - Starting a new fragmented message with compression",
                    wsc->id);
        
        // We don't reset here because this might be a continuation of previous compression context
        // if client_context_takeover is true
    }
    
    // Log the current state of the inflate stream for debugging
    netdata_log_debug(D_WEBSOCKET, "WEBSOCKET: Client %zu - Inflate stream: total_in=%lu, total_out=%lu, avail_in=%u, avail_out=%u",
                  wsc->id, 
                  wsc->compression.inflate_stream->total_in,
                  wsc->compression.inflate_stream->total_out,
                  wsc->compression.inflate_stream->avail_in,
                  wsc->compression.inflate_stream->avail_out);
}

// End handling a fragmented compressed message
void websocket_compression_fragmented_end(struct websocket_server_client *wsc) {
    if (!wsc || !wsc->compression.enabled)
        return;
    
    // Log the current state of the inflate stream before ending
    netdata_log_debug(D_WEBSOCKET, "WEBSOCKET: Client %zu - Ending fragmented compressed message", wsc->id);
    
    // Log the final state of the inflate stream for debugging
    netdata_log_debug(D_WEBSOCKET, "WEBSOCKET: Client %zu - Final inflate stream state: total_in=%lu, total_out=%lu, avail_in=%u, avail_out=%u",
                  wsc->id, 
                  wsc->compression.inflate_stream->total_in,
                  wsc->compression.inflate_stream->total_out,
                  wsc->compression.inflate_stream->avail_in,
                  wsc->compression.inflate_stream->avail_out);
    
    // Mark that we're no longer in a fragmented message
    wsc->compression.in_fragmented_message = false;
    wsc->compression.fragmented_compression = false;
    
    // Handle compression state according to context takeover settings
    if (wsc->compression.inflate_stream) {
        // Reset inflate stream only if client_context_takeover is false
        if (!wsc->compression.client_context_takeover) {
            netdata_log_debug(D_WEBSOCKET, "WEBSOCKET: Client %zu - Resetting inflate stream after fragmented message (no client context takeover)", 
                          wsc->id);
            inflateReset(wsc->compression.inflate_stream);
        } else {
            netdata_log_debug(D_WEBSOCKET, "WEBSOCKET: Client %zu - Preserving inflate stream context for next message (client context takeover)", 
                          wsc->id);
        }
    }
}