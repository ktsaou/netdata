// SPDX-License-Identifier: GPL-3.0-or-later

#include "webrtc.h"

#include "../server/web_client.h"
#include "../server/web_client_cache.h"
#ifdef ENABLE_PROMQL
#include "../api/promql/rtc-body.h"
#include "../api/promql/operations.h"
#endif

#ifdef HAVE_LIBDATACHANNEL

#include <limits.h>

#include "rtc/rtc.h"

#if defined(ENABLE_LZ4)
#include <lz4.h>
#endif

#define WEBRTC_OUR_MAX_MESSAGE_SIZE (5 * 1024 * 1024)
#define WEBRTC_DEFAULT_REMOTE_MAX_MESSAGE_SIZE (65536)
#define WEBRTC_COMPRESSED_HEADER_SIZE 200
#define WEBRTC_REQUEST_PARSER_PADDING 10

static void webrtc_log(rtcLogLevel level, const char *message) {
    switch(level) {
        case RTC_LOG_NONE:
            break;

        case RTC_LOG_WARNING:
        case RTC_LOG_ERROR:
        case RTC_LOG_FATAL:
            netdata_log_error("WEBRTC: %s", message);
            break;

        case RTC_LOG_INFO:
            netdata_log_info("WEBRTC: %s", message);
            break;

        default:
        case RTC_LOG_DEBUG:
        case RTC_LOG_VERBOSE:
            internal_error(true, "WEBRTC: %s", message);
            break;

    }
}

typedef struct webrtc_request {
    struct webrtc_request *next;
    BUFFER *data;
    size_t size;
    bool binary, too_long;
} WEBRTC_REQUEST;

typedef struct webrtc_datachannel {
    int dc;
    char *label;
    struct webrtc_connection *conn;

    bool open; // atomic
    bool stopped, ready, cleaning;
    pthread_mutex_t mutex;
    pthread_cond_t changed;
    uint64_t generation, operation_id;
    ND_THREAD *worker;
    WEBRTC_REQUEST *requests, *tail;

    struct {
        struct webrtc_datachannel *prev;
        struct webrtc_datachannel *next;
    } link;
} WEBRTC_DC;

typedef struct webrtc_connection {
    int pc;
    rtcConfiguration config;
    rtcState state;
    rtcGatheringState gathering_state;
    bool stopped, setup_ready, retiring;
    size_t pins;

    struct {
        SPINLOCK spinlock;
        BUFFER *wb;
        bool sdp;
        bool candidates;
    } response;

    struct {
        SPINLOCK spinlock;
        WEBRTC_DC *head;
    } channels;

    struct {
        struct webrtc_connection *prev;
        struct webrtc_connection *next;
    } link;
} WEBRTC_CONN;

#define WEBRTC_MAX_ICE_SERVERS 100

static struct {
    bool enabled;
    char *iceServers[WEBRTC_MAX_ICE_SERVERS];
    int iceServersCount;
    const char *proxyServer;
    const char *bindAddress;

    struct {
        SPINLOCK spinlock;
        WEBRTC_CONN *head;
    } unsafe;
    pthread_mutex_t manager_mutex;
    pthread_cond_t manager_changed;
    ND_THREAD *manager;
    bool manager_wake, stopping;

} webrtc_base = {
#ifdef NETDATA_INTERNAL_CHECKS
        .enabled = true,
#else
        .enabled = false,
#endif
        .iceServers = {
                // Format:
                // [("stun"|"turn"|"turns") (":"|"://")][username ":" password "@"]hostname[":" port]["?transport=" ("udp"|"tcp"|"tls")]
                //
                // Note transports TCP and TLS are only available for a TURN server with libnice as ICE backend and govern only the
                // TURN control connection, meaning relaying is always performed over UDP.
                //
                // If the username or password of a URI contains reserved special characters, they must be percent-encoded.
                // In particular, ":" must be encoded as "%3A" and "@" must by encoded as "%40".

                "stun://stun.l.google.com:19302",
                NULL, // terminator
        },
        .iceServersCount = 1,
        .proxyServer = NULL, // [("http"|"socks5") (":"|"://")][username ":" password "@"]hostname["    :" port]
        .bindAddress = NULL,
        .unsafe = {
                .spinlock = SPINLOCK_INITIALIZER,
                .head = NULL,
        },
        .manager_mutex = PTHREAD_MUTEX_INITIALIZER,
        .manager_changed = PTHREAD_COND_INITIALIZER,
};

static inline bool webrtc_dc_is_open(WEBRTC_DC *chan) {
    return __atomic_load_n(&chan->open, __ATOMIC_RELAXED);
}

static inline rtcState webrtc_conn_state(WEBRTC_CONN *conn) {
    return __atomic_load_n(&conn->state, __ATOMIC_RELAXED);
}

static inline void webrtc_conn_set_state(WEBRTC_CONN *conn, rtcState state) {
    __atomic_store_n(&conn->state, state, __ATOMIC_RELAXED);
}

static inline rtcGatheringState webrtc_conn_gathering_state(WEBRTC_CONN *conn) {
    return __atomic_load_n(&conn->gathering_state, __ATOMIC_RELAXED);
}

static inline void webrtc_conn_set_gathering_state(WEBRTC_CONN *conn, rtcGatheringState state) {
    __atomic_store_n(&conn->gathering_state, state, __ATOMIC_RELAXED);
}

static void cleanupConnections(void) {
    pthread_mutex_lock(&webrtc_base.manager_mutex);
    webrtc_base.manager_wake = true;
    pthread_cond_signal(&webrtc_base.manager_changed);
    pthread_mutex_unlock(&webrtc_base.manager_mutex);
}

static void channel_stop(WEBRTC_DC *chan) {
    pthread_mutex_lock(&chan->mutex);
    chan->stopped = true;
    __atomic_store_n(&chan->open, false, __ATOMIC_RELAXED);
    uint64_t id = chan->operation_id;
    chan->generation++;
    pthread_cond_broadcast(&chan->changed);
    pthread_mutex_unlock(&chan->mutex);
#ifdef ENABLE_PROMQL
    promql_operations_cancel_id(id);
#else
    (void)id;
#endif
    cleanupConnections();
}

static WEBRTC_CONN *connection_acquire(int pc) {
    WEBRTC_CONN *result = NULL;
    spinlock_lock(&webrtc_base.unsafe.spinlock);
    for(WEBRTC_CONN *conn = webrtc_base.unsafe.head; conn; conn = conn->link.next)
        if(conn->pc == pc && !conn->retiring) { conn->pins++; result = conn; break; }
    spinlock_unlock(&webrtc_base.unsafe.spinlock);
    return result;
}

static void connection_release(WEBRTC_CONN *conn) {
    spinlock_lock(&webrtc_base.unsafe.spinlock);
    internal_fatal(!conn->pins, "WebRTC owner pin underflow");
    conn->pins--;
    spinlock_unlock(&webrtc_base.unsafe.spinlock);
    cleanupConnections();
}

static void connection_stop(WEBRTC_CONN *conn) {
    __atomic_store_n(&conn->stopped, true, __ATOMIC_RELEASE);
    spinlock_lock(&conn->channels.spinlock);
    for(WEBRTC_DC *chan = conn->channels.head; chan; chan = chan->link.next) channel_stop(chan);
    spinlock_unlock(&conn->channels.spinlock);
    cleanupConnections();
}

static void response_worker(void *data);
static void cleanup_manager(void *data);

static bool channel_ready(void *data) {
    WEBRTC_DC *chan = data;
    for(;;) {
        pthread_mutex_lock(&chan->mutex);
        uint64_t generation = chan->generation;
        bool open = !chan->stopped && webrtc_dc_is_open(chan);
        pthread_mutex_unlock(&chan->mutex);
        if(!open) return false;
        int amount = rtcGetBufferedAmount(chan->dc);
        if(amount <= 0) return amount == 0;
        pthread_mutex_lock(&chan->mutex);
        while(!chan->stopped && chan->generation == generation) pthread_cond_wait(&chan->changed, &chan->mutex);
        pthread_mutex_unlock(&chan->mutex);
    }
}

static void myBufferedLowCallback(int id __maybe_unused, void *user_ptr) {
    WEBRTC_DC *chan = user_ptr;
    if(!chan) return;
    pthread_mutex_lock(&chan->mutex);
    chan->generation++;
    pthread_cond_broadcast(&chan->changed);
    pthread_mutex_unlock(&chan->mutex);
}

#ifdef ENABLE_PROMQL
static void channel_operation(void *data, uint64_t id) {
    WEBRTC_DC *chan = data;
    pthread_mutex_lock(&chan->mutex);
    chan->operation_id = id;
    bool stopped = chan->stopped;
    pthread_mutex_unlock(&chan->mutex);
    if(id && stopped) promql_operations_cancel_id(id);
}

static bool channel_send_text(void *data, const char *text, size_t length) {
    WEBRTC_DC *chan = data;
    return length < INT_MAX && rtcSendMessage(chan->dc, text, -(int)length) == RTC_ERR_SUCCESS;
}
#endif

static void webrtc_config_ice_servers(void) {
    BUFFER *wb = buffer_create(0, NULL);

    int i;
    for(i = 0; i < WEBRTC_MAX_ICE_SERVERS ;i++) {
        if (webrtc_base.iceServers[i]) {
            if (buffer_strlen(wb))
                buffer_strcat(wb, " ");

            internal_error(true, "WEBRTC: default ice server No %d is: '%s'", i, webrtc_base.iceServers[i]);
            buffer_strcat(wb, webrtc_base.iceServers[i]);
        }
        else
            break;
    }
    webrtc_base.iceServersCount = i;
    internal_error(true, "WEBRTC: there are %d default ice servers: '%s'", webrtc_base.iceServersCount, buffer_tostring(wb));

    const char *servers = inicfg_get(&netdata_config, CONFIG_SECTION_WEBRTC, "ice servers", buffer_tostring(wb));

    webrtc_base.iceServersCount = 0;
    char tmp[strlen(servers) + 1];
    strcpy(tmp, servers);
    char *s = tmp, *e;
    while(*s) {
        if(isspace((uint8_t)*s))
            s++;

        e = s;
        while(*e && !isspace((uint8_t)*e))
            e++;

        if(s != e && webrtc_base.iceServersCount < WEBRTC_MAX_ICE_SERVERS) {
            char old = *e;
            *e = '\0';
            internal_error(true, "WEBRTC: ice server No %d is: '%s'", webrtc_base.iceServersCount, s);
            webrtc_base.iceServers[webrtc_base.iceServersCount++] = strdupz(s);
            *e = old;
        }

        if(*e)
            s = e + 1;
        else
            break;
    }

    buffer_free(wb);
}

void webrtc_initialize() {
    webrtc_base.enabled = inicfg_get_boolean(&netdata_config, CONFIG_SECTION_WEBRTC, "enabled", webrtc_base.enabled);
    internal_error(true, "WEBRTC: is %s", webrtc_base.enabled ? "enabled" : "disabled");

    webrtc_config_ice_servers();

    webrtc_base.proxyServer = inicfg_get(&netdata_config, CONFIG_SECTION_WEBRTC, "proxy server", webrtc_base.proxyServer ? webrtc_base.proxyServer : "");
    if(!webrtc_base.proxyServer || !*webrtc_base.proxyServer)
        webrtc_base.proxyServer = NULL;

    internal_error(true, "WEBRTC: proxy server is: '%s'", webrtc_base.proxyServer ? webrtc_base.proxyServer : "");

    webrtc_base.bindAddress = inicfg_get(&netdata_config, CONFIG_SECTION_WEBRTC, "bind address", webrtc_base.bindAddress ? webrtc_base.bindAddress : "");
    if(!webrtc_base.bindAddress || !*webrtc_base.bindAddress)
        webrtc_base.bindAddress = NULL;

    internal_error(true, "WEBRTC: bind address is: '%s'", webrtc_base.bindAddress ? webrtc_base.bindAddress : "");

    if(!webrtc_base.enabled)
        return;

    rtcLogLevel level;
#ifdef NETDATA_INTERNAL_CHECKS
    level = RTC_LOG_INFO;
#else
    level = RTC_LOG_WARNING;
#endif

    rtcInitLogger(level, webrtc_log);
    rtcPreload();
    webrtc_base.manager = nd_thread_create("WEBRTC_CLEANUP", NETDATA_THREAD_OPTION_DONT_LOG, cleanup_manager, NULL);
    if(!webrtc_base.manager) { rtcCleanup(); webrtc_base.enabled = false; }
}

void webrtc_close_all_connections() {
    if(!webrtc_base.enabled)
        return;

    spinlock_lock(&webrtc_base.unsafe.spinlock);
    webrtc_base.stopping = true;
    for(WEBRTC_CONN *conn = webrtc_base.unsafe.head; conn; conn = conn->link.next) connection_stop(conn);
    spinlock_unlock(&webrtc_base.unsafe.spinlock);
    cleanupConnections();
    if(webrtc_base.manager) { nd_thread_join(webrtc_base.manager); webrtc_base.manager = NULL; }
    rtcCleanup();
}

// ----------------------------------------------------------------------------
// execute web API requests

static bool web_client_stop_callback(struct web_client *w __maybe_unused, void *data) {
    WEBRTC_DC *chan = data;
    return !webrtc_dc_is_open(chan);
}

static size_t webrtc_send_in_chunks(WEBRTC_DC *chan, const char *data, size_t size, int code, const char *message_type, HTTP_CONTENT_TYPE content_type, size_t max_message_size, bool binary) {
    size_t sent_bytes = 0;
    size_t chunk = 0;
    size_t total_chunks = size / max_message_size;
    if(total_chunks * max_message_size < size)
        total_chunks++;

    size_t frame_size = max_message_size + WEBRTC_COMPRESSED_HEADER_SIZE + 1;
    char *send_buffer = mallocz(frame_size);

    char *s = (char *)data;
    size_t remaining = size;
    while(remaining > 0) {
        if(!channel_ready(chan)) break;
        chunk++;

        size_t message_size = MIN(remaining, max_message_size);

        int len = snprintfz(send_buffer, WEBRTC_COMPRESSED_HEADER_SIZE, "%d %s %zu %zu %zu %s\r\n",
                            code,
                            message_type,
                            message_size,
                            chunk,
                            total_chunks,
                            content_type_id2string(content_type)
        );

        internal_fatal((size_t)len != strlen(send_buffer), "WEBRTC compressed header line mismatch");
        internal_fatal((size_t)len + message_size >= frame_size, "WEBRTC message exceeds max message size");

        memcpy(&send_buffer[len], s, message_size);
        send_buffer[len + message_size] = 0;

        int total_message_size = (int)(len + message_size);
        sent_bytes += total_message_size;

        if(!binary)
            total_message_size = -total_message_size;

        if(rtcSendMessage(chan->dc, send_buffer, total_message_size) != RTC_ERR_SUCCESS) {
            netdata_log_error("WEBRTC[%d],DC[%d]: failed to send LZ4 chunk %zu of %zu", chan->conn->pc, chan->dc, chunk, total_chunks);
            channel_stop(chan); break;
        }
        else
            internal_error(true, "WEBRTC[%d],DC[%d]: sent chunk %zu of %zu, size %zu (total %d)",
                           chan->conn->pc, chan->dc, chunk, total_chunks, message_size, total_message_size);

        s = s + message_size;
        remaining -= message_size;
    }

    if(!remaining) channel_ready(chan);

    freez(send_buffer);
    return sent_bytes;
}

static void webrtc_execute_api_request(WEBRTC_DC *chan, const char *request, size_t size, bool binary, bool too_long) {
    ND_LOG_STACK lgs[] = {
            ND_LOG_FIELD_TXT(NDF_SRC_TRANSPORT, "webrtc"),
            ND_LOG_FIELD_END(),
    };
    ND_LOG_STACK_PUSH(lgs);

    internal_error(true, "WEBRTC[%d],DC[%d]: got request '%s' of size %zu and type %s.",
                   chan->conn->pc, chan->dc, request, size, binary?"binary":"text");

    struct web_client *w = web_client_get_from_cache();
    w->statistics.received_bytes = size;
    w->interrupt.callback = web_client_stop_callback;
    w->interrupt.callback_data = chan;
#ifdef ENABLE_PROMQL
    w->promql_operation_hook = channel_operation;
    w->promql_operation_hook_data = chan;
#endif
    web_client_set_conn_webrtc(w);

    w->port_acl = HTTP_ACL_WEBRTC | HTTP_ACL_ALL_FEATURES;
    w->acl = w->port_acl;

    char *path = (char *)request;
    if(strncmp(request, "POST ", 5) == 0) {
        w->mode = HTTP_REQUEST_MODE_POST;
        path += 5;
    }
    else if(strncmp(request, "GET ", 4) == 0) {
        w->mode = HTTP_REQUEST_MODE_GET;
        path += 4;
    }

    web_client_timeout_checkpoint_set(w, 0);
    bool routed = false;
    if(too_long) {
        w->response.code = HTTP_RESP_CONTENT_TOO_LONG;
        buffer_strcat(w->response.data, "Request exceeds the native request size.\r\n");
    }
    else if(strstr(request, "\r\n")) {
        buffer_memcat(w->response.data, request, size);
        HTTP_VALIDATION validation = http_request_validate(w);
        if(validation == HTTP_VALIDATION_OK) routed = true;
        else {
            buffer_flush(w->response.data);
            buffer_strcat(w->response.data, "Invalid HTTP request.\r\n");
            w->response.code = validation == HTTP_VALIDATION_URI_TOO_LONG ? HTTP_RESP_URI_TOO_LONG : HTTP_RESP_BAD_REQUEST;
        }
    }
    else if(unlikely(!web_client_decode_path_and_query_string(w, path))) {
        buffer_flush(w->url_as_received);
        buffer_strcat(w->url_as_received, "too long request URI");
        buffer_flush(w->response.data);
        buffer_strcat(w->response.data, "Request URI is too long.\r\n");
        w->response.code = HTTP_RESP_URI_TOO_LONG;
    }
    else routed = true;
    if(routed) {
        path = (char *)buffer_tostring(w->url_path_decoded);
        w->response.code = (short)web_client_api_request_with_node_selection(localhost, w, path);
    }
    web_client_timeout_checkpoint_response_ready(w, NULL);

    size_t sent_bytes = 0;
    size_t response_size = buffer_strlen(w->response.data);

    bool send_plain = true;
    int negotiated = rtcMaxMessageSize(chan->dc);
    if(negotiated <= WEBRTC_COMPRESSED_HEADER_SIZE + 1) { channel_stop(chan); goto cleanup; }
    size_t max_message_size = (size_t)negotiated - WEBRTC_COMPRESSED_HEADER_SIZE - 1;

#ifdef ENABLE_PROMQL
    if(w->response.promql_body) {
        PROMQL_RTC_WRITER writer = {.context = chan, .ready = channel_ready, .send = channel_send_text};
        uint64_t sent = 0;
        int error = 0;
        bool complete = promql_rtc_body_send(w->response.promql_body, w->response.code,
            content_type_id2string(w->response.data->content_type), (size_t)negotiated, &writer, &sent, &error);
        w->statistics.sent_bytes = sent;
        if(complete) {
            w->response.promql_sent = promql_body_length(w->response.promql_body);
            goto cleanup;
        }
        if(sent || error == ECANCELED) { channel_stop(chan); goto cleanup; }
        w->response.code = error == EMSGSIZE ? HTTP_RESP_CONTENT_TOO_LONG : HTTP_RESP_INTERNAL_SERVER_ERROR;
        buffer_flush(w->response.data);
        buffer_strcat(w->response.data, error == EMSGSIZE ? "Response cannot fit the negotiated text frame.\r\n" :
                                                            "PromQL response read failed.\r\n");
        w->response.data->content_type = CT_TEXT_PLAIN;
        response_size = buffer_strlen(w->response.data);
    }
#endif

    if(!webrtc_dc_is_open(chan)) {
        internal_error(true, "WEBRTC[%d],DC[%d]: ignoring API response on closed data channel.", chan->conn->pc, chan->dc);
        goto cleanup;
    }
    else {
        internal_error(true, "WEBRTC[%d],DC[%d]: prepared response with code %d, size %zu.",
                       chan->conn->pc, chan->dc, w->response.code, response_size);
    }

#if defined(ENABLE_LZ4)
    if(response_size <= LZ4_MAX_INPUT_SIZE) {
        int max_compressed_size = LZ4_compressBound((int)response_size);
        char *compressed = mallocz(max_compressed_size);

        int compressed_size = LZ4_compress_default(buffer_tostring(w->response.data), compressed,
                                                   (int)response_size, max_compressed_size);

        if(compressed_size > 0) {
            send_plain = false;
            sent_bytes = webrtc_send_in_chunks(chan, compressed, compressed_size,
                                               w->response.code, "LZ4", w->response.data->content_type,
                                               max_message_size, true);
        }
        freez(compressed);
    }
#endif

    if(send_plain)
        sent_bytes = webrtc_send_in_chunks(chan, buffer_tostring(w->response.data), buffer_strlen(w->response.data),
                              w->response.code, "PLAIN", w->response.data->content_type,
                              max_message_size, false);

    w->statistics.sent_bytes = sent_bytes;

cleanup:
    web_client_log_completed_request(w, false);
    web_client_release_to_cache(w);
}

static void request_free(WEBRTC_REQUEST *request) {
    buffer_free(request->data);
    freez(request);
}

static void response_worker(void *data) {
    nd_thread_set_manual_join(nd_thread_self());
    WEBRTC_DC *chan = data;
    pthread_mutex_lock(&chan->mutex);
    while(!chan->stopped) {
        while(!chan->stopped && (!chan->requests || !webrtc_dc_is_open(chan)))
            pthread_cond_wait(&chan->changed, &chan->mutex);
        if(chan->stopped) break;
        WEBRTC_REQUEST *request = chan->requests;
        chan->requests = request->next;
        if(!chan->requests) chan->tail = NULL;
        pthread_mutex_unlock(&chan->mutex);
        webrtc_execute_api_request(chan, buffer_tostring(request->data), request->size, request->binary, request->too_long);
        request_free(request);
        pthread_mutex_lock(&chan->mutex);
    }
    WEBRTC_REQUEST *request = chan->requests;
    chan->requests = chan->tail = NULL;
    pthread_mutex_unlock(&chan->mutex);
    while(request) { WEBRTC_REQUEST *next = request->next; request_free(request); request = next; }
    cleanupConnections();
}

// ----------------------------------------------------------------------------
// webrtc data channel

static void myOpenCallback(int id __maybe_unused, void *user_ptr) {
    webrtc_set_thread_name();

    WEBRTC_DC *chan = user_ptr;
    if(!chan) return;
    internal_fatal(chan->dc != id, "WEBRTC[%d],DC[%d]: dc mismatch, expected %d, got %d", chan->conn->pc, chan->dc, chan->dc, id);

    nd_log(NDLS_ACCESS, NDLP_DEBUG, "WEBRTC[%d],DC[%d]: %d DATA CHANNEL '%s' OPEN", chan->conn->pc, chan->dc, gettid_cached(), chan->label);
    internal_error(true, "WEBRTC[%d],DC[%d]: data channel opened.", chan->conn->pc, chan->dc);
    pthread_mutex_lock(&chan->mutex);
    if(!chan->stopped) __atomic_store_n(&chan->open, true, __ATOMIC_RELAXED);
    chan->generation++;
    pthread_cond_broadcast(&chan->changed);
    pthread_mutex_unlock(&chan->mutex);
}

static void myClosedCallback(int id __maybe_unused, void *user_ptr) {
    webrtc_set_thread_name();

    WEBRTC_DC *chan = user_ptr;
    if(chan) channel_stop(chan);
}

static void myErrorCallback(int id __maybe_unused, const char *error, void *user_ptr) {
    webrtc_set_thread_name();

    WEBRTC_DC *chan = user_ptr;
    if(!chan) return;
    internal_fatal(chan->dc != id, "WEBRTC[%d],DC[%d]: dc mismatch, expected %d, got %d", chan->conn->pc, chan->dc, chan->dc, id);

    netdata_log_error("WEBRTC[%d],DC[%d]: ERROR: '%s'", chan->conn->pc, chan->dc, error);
    channel_stop(chan);
}

static void myMessageCallback(int id __maybe_unused, const char *message, int size, void *user_ptr) {
    webrtc_set_thread_name();

    WEBRTC_DC *chan = user_ptr;
    if(!chan) return;
    internal_fatal(chan->dc != id, "WEBRTC[%d],DC[%d]: dc mismatch, expected %d, got %d", chan->conn->pc, chan->dc, chan->dc, id);

    bool binary = (size >= 0);
    if(size == INT_MIN) {
        netdata_log_error("WEBRTC[%d],DC[%d]: invalid message size.", chan->conn->pc, chan->dc);
        return;
    }

    if(unlikely(!message)) {
        netdata_log_error("WEBRTC[%d],DC[%d]: received NULL message.", chan->conn->pc, chan->dc);
        return;
    }

    // The SDK includes a terminating NUL in text callback sizes, outside the wire payload.
    size_t request_size = binary ? (size_t)size : (size_t)(-size - 1);
    WEBRTC_REQUEST *request = callocz(1, sizeof(*request));
    request->too_long = request_size > NETDATA_WEB_REQUEST_MAX_SIZE;
    request->size = request->too_long ? 0 : request_size;
    request->binary = binary;
    request->data = buffer_create(request->size + 1 + WEBRTC_REQUEST_PARSER_PADDING, &netdata_buffers_statistics.buffers_web);
    if(request->size) buffer_memcat(request->data, message, request->size);
    memset(request->data->buffer + request->data->len, 0, 1 + WEBRTC_REQUEST_PARSER_PADDING);
    pthread_mutex_lock(&chan->mutex);
    if(chan->stopped) { pthread_mutex_unlock(&chan->mutex); request_free(request); return; }
    if(chan->tail) chan->tail->next = request;
    else chan->requests = request;
    chan->tail = request;
    pthread_cond_signal(&chan->changed);
    pthread_mutex_unlock(&chan->mutex);
}

//#define WEBRTC_MAX_REQUEST_SIZE 65536
//
//static void myAvailableCallback(int id, void *user_ptr) {
//    webrtc_set_thread_name();
//
//    WEBRTC_DC *chan = user_ptr;
//    internal_fatal(chan->dc != id, "WEBRTC[%d],DC[%d]: dc mismatch, expected %d, got %d", chan->conn->pc, chan->dc, chan->dc, id);
//
//    internal_fatal(!chan->open, "WEBRTC[%d],DC[%d]: received message on closed channel", chan->conn->pc, chan->dc);
//
//    int size = WEBRTC_MAX_REQUEST_SIZE;
//    char buffer[WEBRTC_MAX_REQUEST_SIZE];
//    while(rtcReceiveMessage(id, buffer, &size) == RTC_ERR_SUCCESS) {
//        bool binary = (size >= 0);
//        if(size < 0)
//            size = -size;
//
//        webrtc_execute_api_request(chan, message, size, binary);
//    }
//}

static void myDataChannelCallback(int pc, int dc, void *user_ptr __maybe_unused) {
    webrtc_set_thread_name();

    WEBRTC_CONN *conn = connection_acquire(pc);
    if(!conn) return;
    WEBRTC_DC *chan = callocz(1, sizeof(WEBRTC_DC));
    chan->dc = dc;
    chan->conn = conn;
    pthread_mutex_init(&chan->mutex, NULL);
    pthread_cond_init(&chan->changed, NULL);
    char label[1025];
    if(rtcGetDataChannelLabel(dc, label, sizeof(label) - 1) < 0) label[0] = 0;
    label[sizeof(label) - 1] = 0;
    chan->label = strdupz(label);
    chan->stopped = __atomic_load_n(&conn->stopped, __ATOMIC_ACQUIRE);
    spinlock_lock(&conn->channels.spinlock);
    DOUBLE_LINKED_LIST_APPEND_ITEM_UNSAFE(conn->channels.head, chan, link.prev, link.next);
    spinlock_unlock(&conn->channels.spinlock);
    pthread_mutex_lock(&chan->mutex);
    bool start_worker = !chan->stopped;
    pthread_mutex_unlock(&chan->mutex);
    ND_THREAD *worker = start_worker ? nd_thread_create("WEBRTC_QUERY", NETDATA_THREAD_OPTION_DONT_LOG, response_worker, chan) : NULL;
    pthread_mutex_lock(&chan->mutex);
    chan->worker = worker;
    if(!worker) chan->stopped = true;
    pthread_mutex_unlock(&chan->mutex);
    rtcSetUserPointer(dc, chan);

    if(rtcSetOpenCallback(dc, myOpenCallback) != RTC_ERR_SUCCESS)
        netdata_log_error("WEBRTC[%d],DC[%d]: rtcSetOpenCallback() failed.", conn->pc, chan->dc);

    if(rtcSetClosedCallback(dc, myClosedCallback) != RTC_ERR_SUCCESS)
        netdata_log_error("WEBRTC[%d],DC[%d]: rtcSetClosedCallback() failed.", conn->pc, chan->dc);

    if(rtcSetErrorCallback(dc, myErrorCallback) != RTC_ERR_SUCCESS)
        netdata_log_error("WEBRTC[%d],DC[%d]: rtcSetErrorCallback() failed.", conn->pc, chan->dc);

    if(rtcSetMessageCallback(dc, myMessageCallback) != RTC_ERR_SUCCESS)
        netdata_log_error("WEBRTC[%d],DC[%d]: rtcSetMessageCallback() failed.", conn->pc, chan->dc);

    if(rtcSetBufferedAmountLowThreshold(dc, 0) != RTC_ERR_SUCCESS ||
       rtcSetBufferedAmountLowCallback(dc, myBufferedLowCallback) != RTC_ERR_SUCCESS) channel_stop(chan);

//    if(rtcSetAvailableCallback(dc, myAvailableCallback) != RTC_ERR_SUCCESS)
//        netdata_log_error("WEBRTC[%d],DC[%d]: rtcSetAvailableCallback() failed.", conn->pc, chan->dc);

    internal_error(true, "WEBRTC[%d],DC[%d]: new data channel with label '%s'", chan->conn->pc, chan->dc, chan->label);
    pthread_mutex_lock(&chan->mutex);
    chan->ready = true;
    if(__atomic_load_n(&conn->stopped, __ATOMIC_ACQUIRE)) chan->stopped = true;
    pthread_cond_broadcast(&chan->changed);
    pthread_mutex_unlock(&chan->mutex);
    connection_release(conn);
    cleanupConnections();
}

// ----------------------------------------------------------------------------
// webrtc connection

static void channel_delete(WEBRTC_DC *chan) {
    channel_stop(chan);
    rtcSetOpenCallback(chan->dc, NULL);
    rtcSetClosedCallback(chan->dc, NULL);
    rtcSetErrorCallback(chan->dc, NULL);
    rtcSetMessageCallback(chan->dc, NULL);
    rtcSetBufferedAmountLowCallback(chan->dc, NULL);
    if(chan->worker) nd_thread_join(chan->worker);
    rtcSetUserPointer(chan->dc, NULL);
    rtcDeleteDataChannel(chan->dc);
    spinlock_lock(&chan->conn->channels.spinlock);
    DOUBLE_LINKED_LIST_REMOVE_ITEM_UNSAFE(chan->conn->channels.head, chan, link.prev, link.next);
    spinlock_unlock(&chan->conn->channels.spinlock);
    pthread_cond_destroy(&chan->changed);
    pthread_mutex_destroy(&chan->mutex);
    freez(chan->label); freez(chan);
}

static void manager_wait_prepare(void) {
    pthread_mutex_lock(&webrtc_base.manager_mutex);
    webrtc_base.manager_wake = false;
    pthread_mutex_unlock(&webrtc_base.manager_mutex);
}

static void manager_wait(void) {
    pthread_mutex_lock(&webrtc_base.manager_mutex);
    while(!webrtc_base.manager_wake) pthread_cond_wait(&webrtc_base.manager_changed, &webrtc_base.manager_mutex);
    pthread_mutex_unlock(&webrtc_base.manager_mutex);
}

static void connection_delete(WEBRTC_CONN *conn) {
    connection_stop(conn);
    if(conn->pc >= 0) {
        rtcSetLocalDescriptionCallback(conn->pc, NULL);
        rtcSetLocalCandidateCallback(conn->pc, NULL);
        rtcSetStateChangeCallback(conn->pc, NULL);
        rtcSetGatheringStateChangeCallback(conn->pc, NULL);
        rtcSetDataChannelCallback(conn->pc, NULL);
    }
    for(;;) {
        manager_wait_prepare();
        spinlock_lock(&webrtc_base.unsafe.spinlock);
        bool pinned = conn->pins != 0;
        spinlock_unlock(&webrtc_base.unsafe.spinlock);
        if(!pinned) break;
        manager_wait();
    }
    while(conn->channels.head) channel_delete(conn->channels.head);
    if(conn->pc >= 0) { rtcSetUserPointer(conn->pc, NULL); rtcDeletePeerConnection(conn->pc); }
    freez(conn);
}

static void cleanup_manager(void *data __maybe_unused) {
    nd_thread_set_manual_join(nd_thread_self());
    for(;;) {
        manager_wait_prepare();
        WEBRTC_CONN *retired = NULL;
        WEBRTC_DC *closed = NULL;
        spinlock_lock(&webrtc_base.unsafe.spinlock);
        for(WEBRTC_CONN *conn = webrtc_base.unsafe.head; conn; conn = conn->link.next) {
            if(conn->setup_ready && __atomic_load_n(&conn->stopped, __ATOMIC_ACQUIRE)) {
                conn->retiring = true;
                DOUBLE_LINKED_LIST_REMOVE_ITEM_UNSAFE(webrtc_base.unsafe.head, conn, link.prev, link.next);
                retired = conn; break;
            }
            spinlock_lock(&conn->channels.spinlock);
            for(WEBRTC_DC *chan = conn->channels.head; chan; chan = chan->link.next) {
                pthread_mutex_lock(&chan->mutex);
                bool retire = chan->ready && chan->stopped && !chan->cleaning;
                if(retire) { chan->cleaning = true; closed = chan; }
                pthread_mutex_unlock(&chan->mutex);
                if(closed) break;
            }
            spinlock_unlock(&conn->channels.spinlock);
            if(closed) break;
        }
        bool done = webrtc_base.stopping && !webrtc_base.unsafe.head;
        spinlock_unlock(&webrtc_base.unsafe.spinlock);
        if(retired) { connection_delete(retired); continue; }
        if(closed) { channel_delete(closed); continue; }
        if(done) return;
        manager_wait();
    }
}

static WEBRTC_CONN * webrtc_create_connection(void) {
    WEBRTC_CONN *conn = callocz(1, sizeof(WEBRTC_CONN));
    conn->pc = -1;
    conn->pins = 1;

    spinlock_init(&conn->response.spinlock);
    spinlock_init(&conn->channels.spinlock);

    spinlock_lock(&webrtc_base.unsafe.spinlock);
    if(webrtc_base.stopping) { spinlock_unlock(&webrtc_base.unsafe.spinlock); freez(conn); return NULL; }
    DOUBLE_LINKED_LIST_APPEND_ITEM_UNSAFE(webrtc_base.unsafe.head, conn, link.prev, link.next);
    spinlock_unlock(&webrtc_base.unsafe.spinlock);
    return conn;
}

static void myDescriptionCallback(int pc, const char *sdp, const char *type, void *user_ptr __maybe_unused) {
    webrtc_set_thread_name();

    WEBRTC_CONN *conn = connection_acquire(pc);
    if(!conn) return;
    internal_fatal(conn->pc != pc, "WEBRTC[%d]: pc mismatch, expected %d, got %d", conn->pc, conn->pc, pc);

    internal_error(true, "WEBRTC[%d]: local description type '%s': %s", conn->pc, type, sdp);
    spinlock_lock(&conn->response.spinlock);
    if(conn->response.wb && !conn->response.candidates) {
        buffer_json_member_add_string(conn->response.wb, "sdp", sdp);
        buffer_json_member_add_string(conn->response.wb, "type", type);
        conn->response.sdp = true;
    }
    spinlock_unlock(&conn->response.spinlock);

    connection_release(conn);
}

static void myCandidateCallback(int pc, const char *cand, const char *mid __maybe_unused, void *user_ptr __maybe_unused) {
    webrtc_set_thread_name();

    WEBRTC_CONN *conn = connection_acquire(pc);
    if(!conn) return;
    internal_fatal(conn->pc != pc, "WEBRTC[%d]: pc mismatch, expected %d, got %d", conn->pc, conn->pc, pc);

    spinlock_lock(&conn->response.spinlock);
    internal_error(true, "WEBRTC[%d]: local candidate '%s', mid '%s'", conn->pc, cand, mid);
    if(conn->response.wb) {
        if(!conn->response.candidates) {
            buffer_json_member_add_array(conn->response.wb, "candidates");
            conn->response.candidates = true;
        }

        buffer_json_add_array_item_string(conn->response.wb, cand);
    }
    spinlock_unlock(&conn->response.spinlock);
    connection_release(conn);
}

static void myStateChangeCallback(int pc, rtcState state, void *user_ptr __maybe_unused) {
    webrtc_set_thread_name();

    WEBRTC_CONN *conn = connection_acquire(pc);
    if(!conn) return;
    internal_fatal(conn->pc != pc, "WEBRTC[%d]: pc mismatch, expected %d, got %d", conn->pc, conn->pc, pc);

    webrtc_conn_set_state(conn, state);

    switch(state) {
        case RTC_NEW:
            internal_error(true, "WEBRTC[%d]: new connection...", conn->pc);
            break;

        case RTC_CONNECTING:
            nd_log(NDLS_ACCESS, NDLP_DEBUG, "WEBRTC[%d]: %d CONNECTING", conn->pc, gettid_cached());
            internal_error(true, "WEBRTC[%d]: connecting...", conn->pc);
            break;

        case RTC_CONNECTED:
            nd_log(NDLS_ACCESS, NDLP_DEBUG, "WEBRTC[%d]: %d CONNECTED", conn->pc, gettid_cached());
            internal_error(true, "WEBRTC[%d]: connected!", conn->pc);
            break;

        case RTC_DISCONNECTED:
            nd_log(NDLS_ACCESS, NDLP_DEBUG, "WEBRTC[%d]: %d DISCONNECTED", conn->pc, gettid_cached());
            internal_error(true, "WEBRTC[%d]: disconnected.", conn->pc);
            spinlock_lock(&conn->channels.spinlock);
            for(WEBRTC_DC *chan = conn->channels.head; chan; chan = chan->link.next) channel_stop(chan);
            spinlock_unlock(&conn->channels.spinlock);
            break;

        case RTC_FAILED:
            nd_log(NDLS_ACCESS, NDLP_DEBUG, "WEBRTC[%d]: %d CONNECTION FAILED", conn->pc, gettid_cached());
            internal_error(true, "WEBRTC[%d]: failed.", conn->pc);
            connection_stop(conn);
            break;

        case RTC_CLOSED:
            nd_log(NDLS_ACCESS, NDLP_DEBUG, "WEBRTC[%d]: %d CONNECTION CLOSED", conn->pc, gettid_cached());
            internal_error(true, "WEBRTC[%d]: closed.", conn->pc);
            connection_stop(conn);
            break;
    }
    connection_release(conn);
}

static void myGatheringStateCallback(int pc, rtcGatheringState state, void *user_ptr __maybe_unused) {
    webrtc_set_thread_name();

    WEBRTC_CONN *conn = connection_acquire(pc);
    if(!conn) return;
    internal_fatal(conn->pc != pc, "WEBRTC[%d]: pc mismatch, expected %d, got %d", conn->pc, conn->pc, pc);

    webrtc_conn_set_gathering_state(conn, state);

    switch(state) {
        case RTC_GATHERING_NEW:
            internal_error(true, "WEBRTC[%d]: gathering...", conn->pc);
            break;

        case RTC_GATHERING_INPROGRESS:
            internal_error(true, "WEBRTC[%d]: gathering in progress...", conn->pc);
            break;

        case RTC_GATHERING_COMPLETE:
            internal_error(true, "WEBRTC[%d]: gathering complete!", conn->pc);
            break;
    }
    connection_release(conn);
}

int webrtc_new_connection(const char *sdp, BUFFER *wb) {
    if(unlikely(!webrtc_base.enabled)) {
        buffer_flush(wb);
        buffer_strcat(wb, "WebRTC is not enabled on this agent.");
        wb->content_type = CT_TEXT_PLAIN;
        return HTTP_RESP_BAD_REQUEST;
    }

    cleanupConnections();

    if(unlikely(!sdp || !*sdp)) {
        buffer_flush(wb);
        buffer_strcat(wb, "No SDP message posted with the request");
        wb->content_type = CT_TEXT_PLAIN;
        return HTTP_RESP_BAD_REQUEST;
    }

    buffer_flush(wb);
    buffer_json_initialize(wb, "\"", "\"", 0, true, BUFFER_JSON_OPTIONS_DEFAULT);
    wb->content_type = CT_APPLICATION_JSON;

    WEBRTC_CONN *conn = webrtc_create_connection();
    if(!conn) { buffer_flush(wb); buffer_strcat(wb, "WebRTC is stopping."); return HTTP_RESP_SERVICE_UNAVAILABLE; }
    conn->response.wb = wb;

    conn->config.iceServers = (const char **)webrtc_base.iceServers;
    conn->config.iceServersCount = webrtc_base.iceServersCount;
    conn->config.proxyServer = webrtc_base.proxyServer;
    conn->config.bindAddress = webrtc_base.bindAddress;
    conn->config.certificateType = RTC_CERTIFICATE_DEFAULT;
    conn->config.iceTransportPolicy = RTC_TRANSPORT_POLICY_ALL;
    conn->config.enableIceTcp = true; // libnice only
    conn->config.enableIceUdpMux = true; // libjuice only
    conn->config.disableAutoNegotiation = false;
    conn->config.forceMediaTransport = false;
    conn->config.portRangeBegin = 0; // 0 means automatic
    conn->config.portRangeEnd = 0; // 0 means automatic
    conn->config.mtu = 0; // <= 0 means automatic
    conn->config.maxMessageSize = WEBRTC_OUR_MAX_MESSAGE_SIZE; // <= 0 means default

    int pc = rtcCreatePeerConnection(&conn->config);
    spinlock_lock(&webrtc_base.unsafe.spinlock);
    conn->pc = pc;
    spinlock_unlock(&webrtc_base.unsafe.spinlock);
    if(pc >= 0) rtcSetUserPointer(pc, conn);
    bool setup = pc >= 0 &&
        rtcSetLocalDescriptionCallback(pc, myDescriptionCallback) == RTC_ERR_SUCCESS &&
        rtcSetLocalCandidateCallback(pc, myCandidateCallback) == RTC_ERR_SUCCESS &&
        rtcSetStateChangeCallback(pc, myStateChangeCallback) == RTC_ERR_SUCCESS &&
        rtcSetGatheringStateChangeCallback(pc, myGatheringStateCallback) == RTC_ERR_SUCCESS &&
        rtcSetDataChannelCallback(pc, myDataChannelCallback) == RTC_ERR_SUCCESS &&
        rtcSetRemoteDescription(pc, sdp, "offer") == RTC_ERR_SUCCESS &&
        (!conn->config.disableAutoNegotiation || rtcSetLocalDescription(pc, NULL) == RTC_ERR_SUCCESS);
    if(!setup) connection_stop(conn);
    spinlock_lock(&webrtc_base.unsafe.spinlock);
    conn->setup_ready = true;
    spinlock_unlock(&webrtc_base.unsafe.spinlock);
    cleanupConnections();

    bool logged = false;
    while(webrtc_conn_gathering_state(conn) != RTC_GATHERING_COMPLETE &&
          !__atomic_load_n(&conn->stopped, __ATOMIC_ACQUIRE)) {
        if(!logged) {
            logged = true;
            internal_error(true, "WEBRTC[%d]: Waiting for gathering to complete", conn->pc);
        }
        sleep_usec(1000);
    }

    if(logged)
        internal_error(true, "WEBRTC[%d]: Gathering finished, our answer is ready", conn->pc);

    spinlock_lock(&conn->response.spinlock);
    bool complete = !__atomic_load_n(&conn->stopped, __ATOMIC_ACQUIRE) && conn->response.sdp;
    if(complete) buffer_json_finalize(wb);
    else { buffer_flush(wb); buffer_strcat(wb, "WebRTC connection setup failed."); wb->content_type = CT_TEXT_PLAIN; }
    conn->response.wb = NULL;
    spinlock_unlock(&conn->response.spinlock);
    if(!complete) connection_stop(conn);
    connection_release(conn);
    return complete ? HTTP_RESP_OK : HTTP_RESP_BAD_REQUEST;
}

#if defined(ENABLE_PROMQL) && defined(NETDATA_INTERNAL_CHECKS)
WEBRTC_TEST_STATE webrtc_test_state(void) {
    WEBRTC_TEST_STATE result = {.channel = -1};
    spinlock_lock(&webrtc_base.unsafe.spinlock);
    for(WEBRTC_CONN *conn = webrtc_base.unsafe.head; conn; conn = conn->link.next) {
        result.connections++;
        spinlock_lock(&conn->channels.spinlock);
        for(WEBRTC_DC *chan = conn->channels.head; chan; chan = chan->link.next) {
            result.channels++;
            pthread_mutex_lock(&chan->mutex);
            result.channel = chan->dc; result.operation_id = chan->operation_id;
            pthread_mutex_unlock(&chan->mutex);
        }
        spinlock_unlock(&conn->channels.spinlock);
    }
    spinlock_unlock(&webrtc_base.unsafe.spinlock);
    if(result.channel >= 0) result.buffered = rtcGetBufferedAmount(result.channel);
    return result;
}
#endif

#else // ! HAVE_LIBDATACHANNEL

void webrtc_initialize() {
    ;
}

int webrtc_new_connection(const char *sdp __maybe_unused, BUFFER *wb) {
    buffer_flush(wb);
    buffer_strcat(wb, "WEBRTC is not available on this server");
    wb->content_type = CT_TEXT_PLAIN;
    return HTTP_RESP_BAD_REQUEST;
}

void webrtc_close_all_connections() {
    ;
}

#if defined(ENABLE_PROMQL) && defined(NETDATA_INTERNAL_CHECKS)
WEBRTC_TEST_STATE webrtc_test_state(void) { return (WEBRTC_TEST_STATE){.channel = -1}; }
#endif

#endif // ! HAVE_LIBDATACHANNEL
