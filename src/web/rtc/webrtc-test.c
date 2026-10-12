// SPDX-License-Identifier: GPL-3.0-or-later

#include "webrtc.h"
#include "web/api/promql/api-internal.h"
#include "web/api/promql/json.h"
#include "web/api/promql/rtc-body.h"

#if defined(HAVE_LIBDATACHANNEL) && defined(NETDATA_INTERNAL_CHECKS)
#include <rtc/rtc.h>
#include <json-c/json.h>

#define PEER_MAXIMUM 2049

typedef struct {
    pthread_mutex_t mutex;
    pthread_cond_t changed;
    bool gathered, open, done, hold, paused, stop;
    int errors;
    PROMQL_BODY *body;
    uint64_t offset, chunks, total;
    size_t maximum_frame;
} PEER_RESULT;

static void peer_gathering(int id __maybe_unused, rtcGatheringState state, void *data) {
    PEER_RESULT *result = data;
    pthread_mutex_lock(&result->mutex);
    result->gathered = state == RTC_GATHERING_COMPLETE;
    pthread_cond_broadcast(&result->changed);
    pthread_mutex_unlock(&result->mutex);
}

static void peer_open(int id __maybe_unused, void *data) {
    PEER_RESULT *result = data;
    pthread_mutex_lock(&result->mutex);
    result->open = true;
    pthread_cond_broadcast(&result->changed);
    pthread_mutex_unlock(&result->mutex);
}

static void peer_error(int id __maybe_unused, const char *message __maybe_unused, void *data) {
    PEER_RESULT *result = data;
    pthread_mutex_lock(&result->mutex);
    result->errors++; result->done = true;
    pthread_cond_broadcast(&result->changed);
    pthread_mutex_unlock(&result->mutex);
}

static void peer_message(int id __maybe_unused, const char *message, int size, void *data) {
    PEER_RESULT *result = data;
    pthread_mutex_lock(&result->mutex);
    if(result->stop) { pthread_mutex_unlock(&result->mutex); return; }
    size_t length = size < 0 && size != INT_MIN ? (size_t)(-size - 1) : 0;
    const char *separator = length ? strstr(message, "\r\n") : NULL;
    if(!separator || separator - message > PROMQL_RTC_HEADER_SIZE - 2 || length >= PEER_MAXIMUM || !result->body) {
        result->errors++; result->done = true;
        goto finished;
    }
    size_t header_length = (size_t)(separator - message) + 2;
    char header[PROMQL_RTC_HEADER_SIZE + 1], type[16], content_type[64];
    memcpy(header, message, header_length); header[header_length] = 0;
    size_t payload = 0;
    uint64_t chunk = 0, total = 0;
    int code = 0, io = 0;
    uint8_t expected[PROMQL_BODY_WINDOW];
    result->errors += sscanf(header, "%d %15s %zu %"SCNu64" %"SCNu64" %63s",
        &code, type, &payload, &chunk, &total, content_type) != 6 || code != 200 || strcmp(type, "PLAIN") ||
        strcmp(content_type, "application/json") || length != header_length + payload ||
        chunk != result->chunks + 1 || !total || (result->total && total != result->total);
    if(payload > sizeof(expected) || !promql_body_read(result->body, result->offset, expected, payload, NULL, &io))
        result->errors++;
    else result->errors += memcmp(expected, message + header_length, payload) != 0;
    for(size_t offset = 0; offset < payload && !result->errors;) {
        uint32_t rune;
        size_t width = promql_utf8_rune((const uint8_t *)message + header_length + offset, payload - offset, &rune);
        if(!width) { result->errors++; break; }
        offset += width;
    }
    result->offset += payload; result->chunks++; result->total = total;
    result->maximum_frame = MAX(result->maximum_frame, length);
    if(result->hold && result->chunks == 1) {
        result->paused = true;
        pthread_cond_broadcast(&result->changed);
        while(result->hold && !result->stop) pthread_cond_wait(&result->changed, &result->mutex);
    }
    result->done = result->errors || chunk == total;
    if(result->done && !result->stop) result->errors += result->offset != promql_body_length(result->body);
finished:
    pthread_cond_broadcast(&result->changed);
    pthread_mutex_unlock(&result->mutex);
}

static bool peer_wait(PEER_RESULT *result, bool *condition) {
    struct timespec until;
    clock_gettime(CLOCK_REALTIME, &until); until.tv_sec += 30;
    pthread_mutex_lock(&result->mutex);
    int status = 0;
    while(!*condition && !result->errors && !status) status = pthread_cond_timedwait(&result->changed, &result->mutex, &until);
    bool ok = *condition && !result->errors;
    pthread_mutex_unlock(&result->mutex);
    return ok;
}

static PROMQL_BODY *peer_baseline(const char *target, bool range, const char *literal) {
    size_t memory = 0;
    struct web_client *client = web_client_create(&memory);
    client->mode = HTTP_REQUEST_MODE_GET;
    buffer_strcat(client->url_as_received, target);
    int code = range ? api_v1_promql_query_range(localhost, client, NULL) : api_v1_promql_query(localhost, client, NULL);
    PROMQL_BODY *body = code == 200 ? client->response.promql_body : NULL;
    if(body && literal) {
        char bytes[1024];
        uint64_t length = promql_body_length(body);
        int error = 0;
        if(length >= sizeof(bytes) || !promql_body_read(body, 0, bytes, (size_t)length, NULL, &error)) body = NULL;
        else { bytes[length] = 0; if(strcmp(bytes, literal)) body = NULL; }
    }
    if(body && !promql_body_acquire(body)) body = NULL;
    web_client_free(client);
    return memory ? (promql_body_release(body), (PROMQL_BODY *)NULL) : body;
}

static void peer_prepare(PEER_RESULT *result, PROMQL_BODY *body, bool hold) {
    pthread_mutex_lock(&result->mutex);
    result->body = body; result->offset = result->chunks = result->total = 0;
    result->maximum_frame = 0; result->done = result->paused = result->stop = false; result->hold = hold;
    pthread_mutex_unlock(&result->mutex);
}

static int peer_record(const char *name, PEER_RESULT *result, int errors, int buffered) {
    pthread_mutex_lock(&result->mutex);
    errors += result->errors;
    printf("{\"rtc_peer_case\":\"%s\",\"errors\":%d,\"bytes\":%"PRIu64",\"chunks\":%"PRIu64
        ",\"max_frame\":%zu,\"buffered\":%d}\n", name, errors, result->offset, result->chunks, result->maximum_frame, buffered);
    pthread_mutex_unlock(&result->mutex);
    return errors;
}

static int peer_pressure(PEER_RESULT *result) {
    if(!peer_wait(result, &result->paused)) return 0;
    usec_t until = now_monotonic_usec() + 30 * USEC_PER_SEC;
    int buffered = 0;
    while(now_monotonic_usec() < until) {
        WEBRTC_TEST_STATE state = webrtc_test_state();
        buffered = state.buffered;
        if(buffered > 0 || !state.channels) break;
        sleep_usec(1000);
    }
    return buffered;
}

int webrtc_promql_peer_fixture(void) {
    inicfg_set_boolean(&netdata_config, CONFIG_SECTION_WEBRTC, "enabled", true);
    inicfg_set(&netdata_config, CONFIG_SECTION_WEBRTC, "ice servers", "");
    inicfg_set(&netdata_config, CONFIG_SECTION_WEBRTC, "bind address", "127.0.0.1");
    webrtc_initialize();
    rtcInitLogger(RTC_LOG_WARNING, NULL);
    PEER_RESULT result = {.mutex = PTHREAD_MUTEX_INITIALIZER, .changed = PTHREAD_COND_INITIALIZER};
    rtcConfiguration config = {.bindAddress = "127.0.0.1", .disableAutoNegotiation = true, .maxMessageSize = PEER_MAXIMUM};
    int pc = rtcCreatePeerConnection(&config), dc = -1, errors = pc < 0;
    BUFFER *answer = buffer_create(4096, NULL);
    if(pc >= 0) {
        rtcSetUserPointer(pc, &result);
        errors += rtcSetGatheringStateChangeCallback(pc, peer_gathering) != RTC_ERR_SUCCESS;
        dc = rtcCreateDataChannel(pc, "promql-fixture");
        errors += dc < 0;
        if(dc >= 0) {
            rtcSetUserPointer(dc, &result);
            errors += rtcSetOpenCallback(dc, peer_open) != RTC_ERR_SUCCESS ||
                rtcSetMessageCallback(dc, peer_message) != RTC_ERR_SUCCESS || rtcSetErrorCallback(dc, peer_error) != RTC_ERR_SUCCESS;
        }
        errors += rtcSetLocalDescription(pc, "offer") != RTC_ERR_SUCCESS || !peer_wait(&result, &result.gathered);
    }
    if(!errors) {
        int length = rtcGetLocalDescription(pc, NULL, 0);
        char *offer = length > 0 ? mallocz((size_t)length) : NULL;
        if(!offer || rtcGetLocalDescription(pc, offer, length) != length || webrtc_new_connection(offer, answer) != 200) errors++;
        freez(offer);
        json_object *object = errors ? NULL : json_tokener_parse(buffer_tostring(answer));
        json_object *sdp = NULL, *type = NULL, *candidates = NULL;
        if(!object || !json_object_object_get_ex(object, "sdp", &sdp) || !json_object_object_get_ex(object, "type", &type) ||
           rtcSetRemoteDescription(pc, json_object_get_string(sdp), json_object_get_string(type)) != RTC_ERR_SUCCESS) errors++;
        if(object && json_object_object_get_ex(object, "candidates", &candidates))
            for(size_t i = 0; i < json_object_array_length(candidates); i++)
                errors += rtcAddRemoteCandidate(pc, json_object_get_string(json_object_array_get_idx(candidates, i)), "0") != RTC_ERR_SUCCESS;
        if(object) json_object_put(object);
        if(!errors) errors += !peer_wait(&result, &result.open);
    }
    errors += peer_record("connect", &result, errors, 0);
    PROMQL_BODY *body = NULL;
    if(!errors) {
        const char *target = "/api/v1/query?query=scalar(fixture_provider{netdata_dimension_id=\"gauge\"})&time=1700002804";
        body = peer_baseline(target, false, "{\"status\":\"success\",\"data\":{\"resultType\":\"scalar\",\"result\":[1700002804,\"5\"]}}");
        peer_prepare(&result, body, false);
        char request[256]; snprintf(request, sizeof(request), "GET %s", target);
        int case_errors = !body || rtcSendMessage(dc, request, -(int)strlen(request)) != RTC_ERR_SUCCESS || !peer_wait(&result, &result.done);
        errors += peer_record("live-gauge", &result, case_errors, 0);
        promql_body_release(body); body = NULL;
    }
    if(!errors) {
        const char *target = "/api/v1/query?query=1&time=1700002804";
        body = peer_baseline(target, false, "{\"status\":\"success\",\"data\":{\"resultType\":\"scalar\",\"result\":[1700002804,\"1\"]}}");
        peer_prepare(&result, body, false);
        const char *request = "POST /api/v1/query?query=2&time=1700002804 HTTP/1.1\r\nHost: fixture\r\nContent-Type: application/x-www-form-urlencoded\r\nContent-Length: 7\r\n\r\nquery=1";
        int case_errors = !body || rtcSendMessage(dc, request, -(int)strlen(request)) != RTC_ERR_SUCCESS || !peer_wait(&result, &result.done);
        errors += peer_record("form-post", &result, case_errors, 0);
        promql_body_release(body); body = NULL;
    }
    const char *range = "/api/v1/query_range?query=vector(1)&start=1700002800&end=1701002799&step=1";
    char request[256]; snprintf(request, sizeof(request), "GET %s", range);
    if(!errors) {
        body = peer_baseline(range, true, NULL);
        peer_prepare(&result, body, true);
        int case_errors = !body || rtcSendMessage(dc, request, -(int)strlen(request)) != RTC_ERR_SUCCESS;
        int buffered = case_errors ? 0 : peer_pressure(&result);
        case_errors += buffered <= 0 || buffered >= PEER_MAXIMUM;
        pthread_mutex_lock(&result.mutex); result.hold = false; pthread_cond_broadcast(&result.changed); pthread_mutex_unlock(&result.mutex);
        case_errors += !peer_wait(&result, &result.done);
        errors += peer_record("paced-range", &result, case_errors, buffered);
    }
    if(!errors) {
        peer_prepare(&result, body, true);
        int case_errors = rtcSendMessage(dc, request, -(int)strlen(request)) != RTC_ERR_SUCCESS;
        int buffered = case_errors ? 0 : peer_pressure(&result);
        case_errors += buffered <= 0 || buffered >= PEER_MAXIMUM;
        pthread_mutex_lock(&result.mutex); result.stop = true; result.hold = false; pthread_cond_broadcast(&result.changed); pthread_mutex_unlock(&result.mutex);
        case_errors += rtcClose(dc) != RTC_ERR_SUCCESS;
        usec_t until = now_monotonic_usec() + 30 * USEC_PER_SEC;
        while(webrtc_test_state().channels && now_monotonic_usec() < until) sleep_usec(1000);
        case_errors += webrtc_test_state().channels != 0 || promql_operations_active();
        errors += peer_record("remote-close", &result, case_errors, buffered);
    }
    pthread_mutex_lock(&result.mutex); result.stop = true; result.hold = false; pthread_cond_broadcast(&result.changed); pthread_mutex_unlock(&result.mutex);
    if(dc >= 0) {
        rtcSetOpenCallback(dc, NULL); rtcSetMessageCallback(dc, NULL); rtcSetErrorCallback(dc, NULL);
        rtcDeleteDataChannel(dc);
    }
    if(pc >= 0) { rtcSetGatheringStateChangeCallback(pc, NULL); rtcDeletePeerConnection(pc); }
    webrtc_close_all_connections();
    promql_body_release(body);
    int final_errors = webrtc_test_state().connections != 0 || webrtc_test_state().channels != 0 ||
        webrtc_new_connection("offer", answer) != HTTP_RESP_SERVICE_UNAVAILABLE || promql_operations_active();
    netdata_promql_engine_observations_t observed = {0};
    final_errors += promql_api_observations(&observed) != NETDATA_PROMQL_OK;
    for(size_t i = 0; i < NETDATA_PROMQL_MEMORY_KINDS; i++)
        if(i != NETDATA_PROMQL_MEMORY_ACCOUNTING) final_errors += observed.memory[i].current != 0;
    errors += peer_record("shutdown", &result, final_errors, 0);
    buffer_free(answer);
    pthread_cond_destroy(&result.changed); pthread_mutex_destroy(&result.mutex);
    return errors;
}
#elif defined(NETDATA_INTERNAL_CHECKS)
int webrtc_promql_peer_fixture(void) { return 1; }
#endif
