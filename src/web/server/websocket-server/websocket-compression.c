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
    
    // Allocate output buffer - worst case scenario is input size + 0.1% + 12 bytes
    size_t max_out_len = in_len + (in_len / 1000) + 12;
    *out = mallocz(max_out_len);
    
    // ALWAYS reset the deflate stream before each message to avoid state-related issues
    // This is safer than relying on the server_context_takeover flag
    deflateReset(wsc->compression.deflate_stream);
    
    // Set up the stream for this compression operation
    wsc->compression.deflate_stream->next_in = (Bytef *)in;
    wsc->compression.deflate_stream->avail_in = in_len;
    wsc->compression.deflate_stream->next_out = (Bytef *)(*out);
    wsc->compression.deflate_stream->avail_out = max_out_len;
    
    // Compress with sync flush to ensure data is flushed to output
    int ret = deflate(wsc->compression.deflate_stream, Z_SYNC_FLUSH);
    
    if (ret != Z_OK && ret != Z_STREAM_END) {
        netdata_log_error("WEBSOCKET: Compression failed for client %zu: %s (%d)", 
                      wsc->id, zError(ret), ret);
        freez(*out);
        *out = NULL;
        return -1;
    }
    
    // Calculate compressed size
    *out_len = max_out_len - wsc->compression.deflate_stream->avail_out;
    
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
    
    // Per RFC 7692, we need to append 4 bytes (00 00 FF FF) to the compressed data
    // to ensure the inflate operation completes
    size_t actual_in_len = in_len + 4;
    char *actual_in = mallocz(actual_in_len);
    memcpy(actual_in, in, in_len);
    actual_in[in_len] = 0x00;
    actual_in[in_len + 1] = 0x00;
    actual_in[in_len + 2] = 0xFF;
    actual_in[in_len + 3] = 0xFF;
    
    // Start with a reasonably sized output buffer and grow if needed
    size_t max_out_len = in_len * 3;  // Initial guess: compressed data is ~1/3 of original
    *out = mallocz(max_out_len);
    
    // ALWAYS reset the inflate stream before each new message to avoid mixing data
    // This is safer than relying on the context takeover flag
    inflateReset(wsc->compression.inflate_stream);
    
    // Set up the stream for this decompression operation
    wsc->compression.inflate_stream->next_in = (Bytef *)actual_in;
    wsc->compression.inflate_stream->avail_in = actual_in_len;
    wsc->compression.inflate_stream->next_out = (Bytef *)(*out);
    wsc->compression.inflate_stream->avail_out = max_out_len;
    
    // Decompress
    int ret = inflate(wsc->compression.inflate_stream, Z_SYNC_FLUSH);
    
    // Handle the need for more output space
    if (ret == Z_BUF_ERROR || wsc->compression.inflate_stream->avail_out == 0) {
        // Output buffer wasn't large enough, grow and try again
        size_t bytes_decompressed = max_out_len - wsc->compression.inflate_stream->avail_out;
        max_out_len *= 2;  // Double the buffer size
        
        netdata_log_debug(D_WEBSOCKET, "WEBSOCKET: Client %zu - Decompression buffer too small, growing to %zu bytes",
                       wsc->id, max_out_len);
        
        *out = reallocz(*out, max_out_len);
        
        // Update stream to use the larger buffer, continuing from where we left off
        wsc->compression.inflate_stream->next_out = (Bytef *)(*out + bytes_decompressed);
        wsc->compression.inflate_stream->avail_out = max_out_len - bytes_decompressed;
        
        ret = inflate(wsc->compression.inflate_stream, Z_SYNC_FLUSH);
    }
    
    if (ret != Z_OK && ret != Z_STREAM_END) {
        netdata_log_error("WEBSOCKET: Decompression failed for client %zu: %s (%d)", 
                      wsc->id, zError(ret), ret);
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