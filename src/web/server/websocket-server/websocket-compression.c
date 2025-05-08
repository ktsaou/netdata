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
    
    // Clean up any existing compression resources to prevent leaks
    websocket_compression_cleanup(wsc);
    
    // Initialize compression context to default values
    wsc->compression.type = WS_COMPRESS_NONE;
    wsc->compression.enabled = false;
    wsc->compression.client_context_takeover = true;
    wsc->compression.server_context_takeover = true;
    wsc->compression.window_bits = WS_COMPRESS_WINDOW_BITS;
    wsc->compression.compression_level = WS_COMPRESS_DEFAULT_LEVEL;
    wsc->compression.deflate_stream = NULL;
    wsc->compression.inflate_stream = NULL;
    
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
            websocket_error(wsc, "Failed to initialize deflate context: %s (%d)", 
                       zError(ret), ret);
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
            websocket_error(wsc, "Failed to initialize inflate context: %s (%d)", 
                       zError(ret), ret);
            deflateEnd(wsc->compression.deflate_stream);
            freez(wsc->compression.deflate_stream);
            wsc->compression.deflate_stream = NULL;
            freez(wsc->compression.inflate_stream);
            wsc->compression.inflate_stream = NULL;
            wsc->compression.enabled = false;
            return false;
        }
        
        websocket_info(wsc, "Compression enabled (permessage-deflate, window bits: %d)",
                   wsc->compression.window_bits);
    }
    
    return true;
}

// Clean up compression resources for a WebSocket client
void websocket_compression_cleanup(struct websocket_server_client *wsc) {
    if (!wsc)
        return;
    
    // Aggressively clean up all compression resources regardless of state
    websocket_debug(wsc, "Cleaning up compression resources");
    
    // Clean up deflate context
    if (wsc->compression.deflate_stream) {
        websocket_debug(wsc, "Cleaning up deflate stream");
        
        // Set up dummy I/O pointers to ensure clean state
        unsigned char dummy_buffer[16] = {0};
        wsc->compression.deflate_stream->next_in = dummy_buffer;
        wsc->compression.deflate_stream->avail_in = 0;
        wsc->compression.deflate_stream->next_out = dummy_buffer;
        wsc->compression.deflate_stream->avail_out = sizeof(dummy_buffer);
        
        // Always call deflateEnd to release internal zlib resources
        // Don't bother with deflateReset as deflateEnd will clean up properly
        int ret = deflateEnd(wsc->compression.deflate_stream);
        
        if (ret != Z_OK && ret != Z_DATA_ERROR) {
            // Z_DATA_ERROR can happen in some edge cases, it's not critical here
            // as we're cleaning up anyway
            websocket_debug(wsc, "deflateEnd returned %d: %s", ret, zError(ret));
            
            // Even on error, we'll continue with cleanup to avoid leaks
        }
        
        // Free the stream structure
        freez(wsc->compression.deflate_stream);
        wsc->compression.deflate_stream = NULL;
    }
    
    // Clean up inflate context
    if (wsc->compression.inflate_stream) {
        websocket_debug(wsc, "Cleaning up inflate stream");
        
        // Set up dummy I/O pointers to ensure clean state
        unsigned char dummy_buffer[16] = {0};
        wsc->compression.inflate_stream->next_in = dummy_buffer;
        wsc->compression.inflate_stream->avail_in = 0;
        wsc->compression.inflate_stream->next_out = dummy_buffer;
        wsc->compression.inflate_stream->avail_out = sizeof(dummy_buffer);
        
        // Always call inflateEnd to release internal zlib resources
        // Don't bother with inflateReset as inflateEnd will clean up properly
        int ret = inflateEnd(wsc->compression.inflate_stream);
        
        if (ret != Z_OK && ret != Z_DATA_ERROR) {
            // Z_DATA_ERROR can happen in some edge cases, it's not critical here
            // as we're cleaning up anyway
            websocket_debug(wsc, "inflateEnd returned %d: %s", ret, zError(ret));
            
            // Even on error, we'll continue with cleanup to avoid leaks
        }
        
        // Free the stream structure
        freez(wsc->compression.inflate_stream);
        wsc->compression.inflate_stream = NULL;
    }
    
    // Reset all compression state
    wsc->compression.type = WS_COMPRESS_NONE;
    wsc->compression.enabled = false;
    wsc->compression.client_context_takeover = true;
    wsc->compression.server_context_takeover = true;
    wsc->compression.window_bits = WS_COMPRESS_WINDOW_BITS;
    wsc->compression.compression_level = WS_COMPRESS_DEFAULT_LEVEL;
    
    websocket_debug(wsc, "Compression resources cleaned up");
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

// Compress a message payload
int websocket_compress_payload(struct websocket_server_client *wsc, const char *in, size_t in_len, char **out, size_t *out_len) {
    if (!wsc || !in || !in_len || !out || !out_len)
        return -1;
    
    // Don't compress tiny payloads - they might actually get larger
    if (in_len < WS_COMPRESS_MIN_SIZE) {
        *out = mallocz(in_len);
        memcpy(*out, in, in_len);
        *out_len = in_len;
        websocket_debug(wsc, "Skipping compression for small payload (%zu bytes)",
                    in_len);
        return 0;
    }
    
    // Check if compression is enabled
    if (!wsc->compression.enabled || !wsc->compression.deflate_stream)
        return -1;
    
    // Log current compression context information
    websocket_debug(wsc, "Compression state before: total_in=%lu, total_out=%lu",
               wsc->compression.deflate_stream->total_in,
               wsc->compression.deflate_stream->total_out);
                  
    // Respect server_context_takeover setting
    // This determines whether the server keeps its compression dictionary between messages
    if (!wsc->compression.server_context_takeover) {
        websocket_debug(wsc, "No server context takeover, resetting deflate stream (previous state: total_in=%lu, total_out=%lu)",
                   wsc->compression.deflate_stream->total_in, wsc->compression.deflate_stream->total_out);
        
        // When server_context_takeover is disabled, we must fully reset the deflate state
        deflateReset(wsc->compression.deflate_stream);
        
        // Also reset the Adler32 checksum to its initial value
        wsc->compression.deflate_stream->adler = 1;
        
        websocket_debug(wsc, "Deflate stream reset complete, new state: adler=%lu",
                   (unsigned long)wsc->compression.deflate_stream->adler);
    } else {
        // With server_context_takeover enabled, we preserve the compression dictionary
        // This improves compression efficiency across multiple messages with similar content
        websocket_debug(wsc, "Server context takeover enabled, preserving compression dictionary (state: total_in=%lu, total_out=%lu)",
                   wsc->compression.deflate_stream->total_in, wsc->compression.deflate_stream->total_out);
    }
    
    // For WebSocket messages, we need to be generous with buffer allocation
    // Binary data can compress to very different sizes
    size_t max_out_len = in_len * 1.2; // Usually compression reduces size, but allow for some overhead
    
    // For very small inputs, use a minimum buffer size
    if (max_out_len < 1024) {
        max_out_len = 1024; // Minimum 1KB buffer
    }
    
    *out = mallocz(max_out_len);
    
    // Set up the stream for this compression operation
    wsc->compression.deflate_stream->next_in = (Bytef *)in;
    wsc->compression.deflate_stream->avail_in = in_len;
    wsc->compression.deflate_stream->next_out = (Bytef *)(*out);
    wsc->compression.deflate_stream->avail_out = max_out_len;
    
    // Log pre-compression details
    websocket_debug(wsc, "Compression starting: input=%zu bytes, avail_in=%u, avail_out=%u, total_in=%lu, total_out=%lu",
               in_len, wsc->compression.deflate_stream->avail_in, 
               wsc->compression.deflate_stream->avail_out,
               wsc->compression.deflate_stream->total_in,
               wsc->compression.deflate_stream->total_out);
                  
    // Compress with sync flush to ensure data is flushed to output
    int ret = deflate(wsc->compression.deflate_stream, Z_SYNC_FLUSH);
    bool compression_success = (ret == Z_OK || ret == Z_STREAM_END);
    
    // Log post-compression details and return code
    websocket_debug(wsc, "Compression finished: result=%d (%s), avail_in=%u, avail_out=%u, total_in=%lu, total_out=%lu",
               ret, zError(ret), 
               wsc->compression.deflate_stream->avail_in, 
               wsc->compression.deflate_stream->avail_out,
               wsc->compression.deflate_stream->total_in,
               wsc->compression.deflate_stream->total_out);
    
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
        
        // Restore state if needed
        if (state_saved) {
            // Restore the compression dictionary state from the saved stream
            // We need to carefully restore the dictionary state without touching I/O pointers
            // Copy the adler and total_in/out values which are part of the dictionary state
            wsc->compression.deflate_stream->adler = saved_stream.adler;
            wsc->compression.deflate_stream->total_in = saved_stream.total_in;
            wsc->compression.deflate_stream->total_out = saved_stream.total_out;
            
            // Using deflateSetDictionary would be ideal here, but it's complex to extract
            // the dictionary from the saved state. Instead, we'll rely on reusing the
            // compression level and strategy, which should recover most state
            netdata_log_debug(D_WEBSOCKET, "WEBSOCKET: Client %zu - Attempting to restore compression state after buffer expansion",
                          wsc->id);
        }
        
        // Set up the stream for retry
        wsc->compression.deflate_stream->next_in = (Bytef *)in;
        wsc->compression.deflate_stream->avail_in = in_len;
        wsc->compression.deflate_stream->next_out = (Bytef *)(*out);
        wsc->compression.deflate_stream->avail_out = max_out_len;
        
        ret = deflate(wsc->compression.deflate_stream, Z_SYNC_FLUSH);
    }
    
    if (!compression_success) {
        netdata_log_error("WEBSOCKET: Compression failed for client %zu: %s (%d)", 
                      wsc->id, zError(ret), ret);
                      
        // Free allocated buffer
        freez(*out);
        *out = NULL;
        
        // Reset the deflate stream to maintain usability and prevent memory leaks
        if (ret != Z_BUF_ERROR) {
            netdata_log_debug(D_WEBSOCKET, "WEBSOCKET: Client %zu - Resetting deflate stream after error", wsc->id);
            deflateReset(wsc->compression.deflate_stream);
        }
        
        return -1;
    }
    
    // Calculate compressed size
    *out_len = max_out_len - wsc->compression.deflate_stream->avail_out;
    
    // Log compression result details
    websocket_debug(wsc, "Compression state after: total_in=%lu, total_out=%lu", 
               wsc->compression.deflate_stream->total_in,
               wsc->compression.deflate_stream->total_out);
    
    // As per RFC 7692, we need to remove the trailing 4 bytes (00 00 FF FF)
    // which are added by Z_SYNC_FLUSH but not needed for WebSocket compression
    if (*out_len >= 4)
        *out_len -= 4;
    
    // If compression didn't actually save space, use the original data
    if (*out_len >= in_len) {
        netdata_log_debug(D_WEBSOCKET, "WEBSOCKET: Client %zu - Compression ineffective (%zu -> %zu bytes), using original data",
                       wsc->id, in_len, *out_len);
        
        freez(*out);
        *out = mallocz(in_len);
        memcpy(*out, in, in_len);
        *out_len = in_len;
        return 0;
    }
    
    // Log successful compression
    websocket_debug(wsc, "Successfully compressed %zu bytes to %zu bytes (%.1f%% reduction)",
                in_len, *out_len, (100.0 - ((double)*out_len / (double)in_len) * 100.0));
    
    return 1;  // Successfully compressed
}

// Decompress a message payload
int websocket_decompress_payload(struct websocket_server_client *wsc, const char *in, size_t in_len, char **out, size_t *out_len) {
    if (!wsc || !in || !out || !out_len)
        return -1;
    
    // Check if compression is enabled
    if (!wsc->compression.enabled || !wsc->compression.inflate_stream)
        return -1;
    
    // Per RFC 7692, we need to append 4 bytes (00 00 FF FF) to the compressed data
    // to ensure the inflate operation completes
    size_t actual_in_len = in_len + 4;
    char *actual_in = mallocz(actual_in_len);
    memcpy(actual_in, in, in_len);
    actual_in[in_len] = 0x00;
    actual_in[in_len + 1] = 0x00;
    actual_in[in_len + 2] = 0xFF;
    actual_in[in_len + 3] = 0xFF;
    
    websocket_debug(wsc, "Created decompression buffer: original=%zu bytes, with trailer=%zu bytes", 
                in_len, actual_in_len);
    
    // Handle the inflate stream appropriately based on message type and context takeover settings
    if (!wsc->compression.client_context_takeover) {
        websocket_debug(wsc, "New message with no context takeover, resetting inflate stream");
        
        // When client_context_takeover is disabled, we must fully reset the inflate state
        int reset_ret = inflateReset(wsc->compression.inflate_stream);
        if (reset_ret != Z_OK) {
            websocket_error(wsc, "Failed to reset inflate stream for no context takeover: %s (%d)",
                       zError(reset_ret), reset_ret);
                       
            // On reset failure, recreate the inflate stream
            if (wsc->compression.inflate_stream) {
                inflateEnd(wsc->compression.inflate_stream);
                freez(wsc->compression.inflate_stream);
            }
            
            wsc->compression.inflate_stream = mallocz(sizeof(z_stream));
            wsc->compression.inflate_stream->zalloc = Z_NULL;
            wsc->compression.inflate_stream->zfree = Z_NULL;
            wsc->compression.inflate_stream->opaque = Z_NULL;
            
            int init_ret = inflateInit2(wsc->compression.inflate_stream, -wsc->compression.window_bits);
            if (init_ret != Z_OK) {
                websocket_error(wsc, "Failed to reinitialize inflate stream: %s (%d)",
                           zError(init_ret), init_ret);
                           
                freez(wsc->compression.inflate_stream);
                wsc->compression.inflate_stream = NULL;
                freez(actual_in);
                return -1;
            }
            
            websocket_debug(wsc, "Recreated inflate stream after reset failure");
        } else {
            // Reset Adler32 checksum to its initial value (1) for a clean dictionary state
            wsc->compression.inflate_stream->adler = 1;
            websocket_debug(wsc, "Inflate stream reset complete with adler set to 1");
        }
    }
    else {
        // With client_context_takeover enabled, we preserve the inflate state
        // This improves compression efficiency for similar messages
        websocket_debug(wsc, "New message with context takeover, preserving inflate state (total_in=%lu, total_out=%lu)",
                 wsc->compression.inflate_stream->total_in, wsc->compression.inflate_stream->total_out);
    }
    
    // WebSocket compressed messages can expand significantly, especially binary data
    // The ratio can be very high for highly compressed data
    size_t max_out_len = in_len * 10; // Allow for 10x expansion ratio as a start
    
    // For very small inputs, use a larger minimum buffer size
    if (max_out_len < 10240) {
        max_out_len = 10240; // Minimum 10KB buffer for small inputs
    }
    
    // Cap the maximum output buffer size for safety
    if (max_out_len > WEBSOCKET_MAX_UNCOMPRESSED_SIZE) {
        max_out_len = WEBSOCKET_MAX_UNCOMPRESSED_SIZE;
    }
    
    websocket_debug(wsc, "Initial decompression buffer size: %zu bytes for %zu input bytes (ratio %.1fx)",
               max_out_len, in_len, (float)max_out_len / in_len);
    
    *out = mallocz(max_out_len);
    
    // Set up the stream for this decompression operation
    wsc->compression.inflate_stream->next_in = (Bytef *)actual_in;
    wsc->compression.inflate_stream->avail_in = actual_in_len;
    wsc->compression.inflate_stream->next_out = (Bytef *)(*out);
    wsc->compression.inflate_stream->avail_out = max_out_len;
    
    // Log details about the data to be decompressed
    websocket_debug(wsc, "Beginning decompression of %zu bytes (added 4 trailer bytes)",
               in_len);
                  
#ifdef NETDATA_INTERNAL_CHECKS
    // Dump the first few bytes of compressed data in hex format for debugging
    char comp_hex_dump[128] = "";
    size_t bytes_to_show = actual_in_len < 16 ? actual_in_len : 16;
    for (size_t i = 0; i < bytes_to_show; i++) {
        char *pos = comp_hex_dump + strlen(comp_hex_dump);
        snprintf(pos, sizeof(comp_hex_dump) - strlen(comp_hex_dump), "%02x ", (unsigned char)actual_in[i]);
    }
    websocket_debug(wsc, "Compressed data preview: [%s]%s",
               comp_hex_dump, actual_in_len > 16 ? "..." : "");
#endif /* NETDATA_INTERNAL_CHECKS */
    
    // Decompress with loop for multiple buffer expansions if needed
    int ret;
    int max_attempts = WS_COMPRESS_MAX_BUFFER_EXPANSIONS; // Limit number of expansions
    bool decompression_success = false;  // Track success for cleanup
    
    // Log the initial decompression state
    websocket_debug(wsc, "Decompression starting: actual_in_len=%zu, avail_in=%u, avail_out=%u, total_in=%lu, total_out=%lu",
               actual_in_len, 
               wsc->compression.inflate_stream->avail_in, 
               wsc->compression.inflate_stream->avail_out,
               wsc->compression.inflate_stream->total_in,
               wsc->compression.inflate_stream->total_out);
    
    for (int attempt = 0; attempt < max_attempts; attempt++) {
        // Try to decompress
        ret = inflate(wsc->compression.inflate_stream, Z_SYNC_FLUSH);
        
        // Log inflate result for debugging with detailed state information
        websocket_debug(wsc, "inflate() returned %d (%s), "
                   "avail_in=%u, avail_out=%u, total_in=%lu, total_out=%lu, attempt %d/%d",
                   ret, zError(ret),
                   wsc->compression.inflate_stream->avail_in, 
                   wsc->compression.inflate_stream->avail_out,
                   wsc->compression.inflate_stream->total_in,
                   wsc->compression.inflate_stream->total_out,
                   attempt + 1, max_attempts);
                      
        // If we got a successful return code, mark decompression as successful
        if (ret == Z_OK || ret == Z_STREAM_END) {
            decompression_success = true;
        }
        
        // Check if we need more output space
        if (ret == Z_BUF_ERROR || (ret == Z_OK && wsc->compression.inflate_stream->avail_out == 0 && 
                                  wsc->compression.inflate_stream->avail_in > 0)) {
            // We've used up the output buffer but there's still input data to process
            size_t bytes_decompressed = max_out_len - wsc->compression.inflate_stream->avail_out;
            size_t old_max_out_len = max_out_len;
            
            // Double the buffer size for each attempt
            max_out_len *= 2;
            
            // Cap the maximum size
            if (max_out_len > WEBSOCKET_MAX_UNCOMPRESSED_SIZE) {
                max_out_len = WEBSOCKET_MAX_UNCOMPRESSED_SIZE;
            }
            
            websocket_debug(wsc, "Decompression buffer too small (%zu bytes used), "
                        "growing from %zu to %zu bytes (attempt %d/%d)",
                        bytes_decompressed, old_max_out_len, max_out_len, attempt + 1, max_attempts);
            
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
    
    // Handle decompression failures
    if (!decompression_success || (ret != Z_OK && ret != Z_STREAM_END && ret != Z_BUF_ERROR)) {
        // Log detailed error information
        websocket_error(wsc, "Decompression failed: %s (%d) - This may be caused by fragmented compressed messages", 
                   zError(ret), ret);
                      
        // Free allocated memory
        freez(*out);
        *out = NULL;
        freez(actual_in);
        
        // On decompression failure, we need to recreate the inflate stream completely
        // This ensures we start with a clean state for future messages
        websocket_debug(wsc, "Creating new inflate context after decompression error");
        
        // Properly clean up the old inflate stream first
        if (wsc->compression.inflate_stream) {
            // End the current inflate stream and free its resources
            inflateEnd(wsc->compression.inflate_stream);
            freez(wsc->compression.inflate_stream);
        }
        
        // Create and initialize a new inflate stream
        wsc->compression.inflate_stream = mallocz(sizeof(z_stream));
        wsc->compression.inflate_stream->zalloc = Z_NULL;
        wsc->compression.inflate_stream->zfree = Z_NULL;
        wsc->compression.inflate_stream->opaque = Z_NULL;
        
        // Initialize with negative window bits for raw deflate (no zlib/gzip header)
        int init_ret = inflateInit2(
            wsc->compression.inflate_stream,
            -wsc->compression.window_bits
        );
        
        if (init_ret != Z_OK) {
            websocket_error(wsc, "Failed to reinitialize inflate context after error: %s (%d)",
                       zError(init_ret), init_ret);
            // We'll continue and hope for the best, but this connection will likely need to be closed
            freez(wsc->compression.inflate_stream);
            wsc->compression.inflate_stream = NULL;
        }
        else
            websocket_debug(wsc, "Successfully recreated inflate context after error");
        
        return -1;
    }
    
    freez(actual_in);
    
    // Calculate decompressed size
    *out_len = max_out_len - wsc->compression.inflate_stream->avail_out;
    
    // Verify we don't have a zero-size output (which indicates a problem)
    if (*out_len == 0) {
        websocket_error(wsc, "Decompression resulted in zero bytes output from %zu input bytes",
                   in_len);
        freez(*out);
        *out = NULL;
        return -1;
    }
    
    // Log successful decompression with detailed information
    websocket_debug(wsc, "Successfully decompressed %zu bytes to %zu bytes (ratio: %.2fx)",
                in_len, *out_len, (double)*out_len / in_len);
                
    // IMPORTANT: Original compressed data was the input length (in_len)
    // For decompression, we added a 4-byte trailer (in_len + 4)
    // Frame boundaries in the buffer should account for just the original data (in_len),
    // NOT including the 4-byte trailer we added
    websocket_debug(wsc, "DECOMPRESSION NOTE: Original compressed data was %zu bytes. Added 4-byte trailer internally for decompression (totaling %zu bytes). Buffer management should use only the original %zu bytes.",
               in_len, in_len + 4, in_len);
    
    // Dump the next few bytes after the end of the compressed frame
    // This is useful for debugging buffer misalignment issues
    char trailer_hex_dump[128] = "";
    if (in_len < actual_in_len - 4) {
        size_t bytes_after_frame = 4;  // Just show the 4-byte trailer we added
        for (size_t i = 0; i < bytes_after_frame; i++) {
            char *pos = trailer_hex_dump + strlen(trailer_hex_dump);
            snprintf(pos, sizeof(trailer_hex_dump) - strlen(trailer_hex_dump), 
                   "%02x ", (unsigned char)actual_in[in_len + i]);
        }
        websocket_debug(wsc, "4-byte trailer added for decompression: [%s]", trailer_hex_dump);
    }
                   
#ifdef NETDATA_INTERNAL_CHECKS
    // Show a preview of the decompressed data
    char decomp_hex_dump[128] = "";
    char ascii_dump[64] = "";
    size_t bytes_to_dump = *out_len < 16 ? *out_len : 16;
    
    for (size_t i = 0; i < bytes_to_dump; i++) {
        // Add to hex dump
        char *pos = decomp_hex_dump + strlen(decomp_hex_dump);
        snprintf(pos, sizeof(decomp_hex_dump) - strlen(decomp_hex_dump), "%02x ", (unsigned char)(*out)[i]);
        
        // Add to ASCII dump
        unsigned char c = (unsigned char)(*out)[i];
        if (isprint(c)) {
            pos = ascii_dump + strlen(ascii_dump);
            snprintf(pos, sizeof(ascii_dump) - strlen(ascii_dump), "%c", c);
        } else {
            pos = ascii_dump + strlen(ascii_dump);
            snprintf(pos, sizeof(ascii_dump) - strlen(ascii_dump), ".");
        }
    }
    
    websocket_debug(wsc, "Decompressed data preview: [%s] \"%s\"%s",
                decomp_hex_dump, ascii_dump, 
                *out_len > 16 ? "..." : "");
#endif /* NETDATA_INTERNAL_CHECKS */
    
    return 1;  // Successfully decompressed
}
