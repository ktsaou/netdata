// SPDX-License-Identifier: GPL-3.0-or-later

#include "rtc-body.h"
#include "api.h"
#include "json.h"

typedef struct {
    PROMQL_BODY *body;
    uint64_t offset, chunks, total;
    size_t max_frame, max_payload, maximum;
    unsigned ready_calls, fail_send, stop_ready;
    bool unicode;
    int errors;
} RTC_EXPECTED;

static bool fixture_ready(void *context) {
    RTC_EXPECTED *expected = context;
    expected->ready_calls++;
    return !expected->stop_ready || expected->ready_calls < expected->stop_ready;
}

static bool fixture_send(void *context, const char *text, size_t length) {
    RTC_EXPECTED *expected = context;
    if(expected->fail_send && expected->chunks + 1 == expected->fail_send) return false;
    const char *separator = strstr(text, "\r\n");
    if(!separator || separator - text >= PROMQL_RTC_HEADER_SIZE) { expected->errors++; return false; }
    size_t header_length = (size_t)(separator - text) + 2;
    char header[PROMQL_RTC_HEADER_SIZE], type[16], content_type[64];
    memcpy(header, text, header_length); header[header_length] = 0;
    uint64_t chunk = 0, total = 0;
    int code = 0;
    size_t payload = 0;
    expected->errors += sscanf(header, "%d %15s %zu %"SCNu64" %"SCNu64" %63s",
                                 &code, type, &payload, &chunk, &total, content_type) != 6 ||
        code != 200 || strcmp(type, "PLAIN") || strcmp(content_type, "application/json") ||
        chunk != expected->chunks + 1 || !total || (expected->total && expected->total != total) ||
        length != header_length + payload || length >= expected->maximum || payload > PROMQL_BODY_WINDOW ||
        text[length] || strlen(text) != length;
    uint8_t original[PROMQL_BODY_WINDOW];
    int error = 0;
    if(payload > sizeof(original) || !promql_body_read(expected->body, expected->offset, original, payload, NULL, &error)) {
        expected->errors++; return false;
    }
    expected->errors += memcmp(original, text + header_length, payload) != 0;
    if(expected->unicode) {
        for(size_t offset = 0; offset < payload;) {
            uint32_t rune;
            size_t width = promql_utf8_rune((const uint8_t *)text + header_length + offset, payload - offset, &rune);
            if(!width) { expected->errors++; break; }
            offset += width;
        }
    }
    expected->offset += payload; expected->chunks++; expected->total = total;
    expected->max_frame = MAX(expected->max_frame, length);
    expected->max_payload = MAX(expected->max_payload, payload);
    return expected->errors == 0;
}

static int frame_case(const char *name, const char *target, size_t maximum, bool unicode,
                       unsigned fail_send, unsigned stop_ready, int wanted) {
    size_t native_memory = 0;
    struct web_client *client = web_client_create(&native_memory);
    buffer_strcat(client->url_as_received, target);
    client->mode = HTTP_REQUEST_MODE_GET;
    bool range = strstr(target, "/query_range?") != NULL;
    int code = range ? api_v1_promql_query_range(localhost, client, NULL) : api_v1_promql_query(localhost, client, NULL);
    int errors = code != 200 || !client->response.promql_body;
    RTC_EXPECTED expected = {.body = client->response.promql_body, .maximum = maximum, .unicode = unicode,
        .fail_send = fail_send, .stop_ready = stop_ready};
    PROMQL_RTC_WRITER writer = {.context = &expected, .ready = fixture_ready, .send = fixture_send};
    uint64_t sent = 0;
    int error = 0;
    bool complete = promql_rtc_body_send(expected.body, 200, "application/json", maximum, &writer, &sent, &error);
    errors += complete != !wanted || error != wanted || expected.errors;
    if(!wanted) errors += expected.offset != promql_body_length(expected.body) || expected.chunks != expected.total;
    if(wanted == EMSGSIZE || wanted == ECANCELED) errors += sent != 0;
    uint64_t received = expected.offset;
    web_client_free(client);
    errors += native_memory != 0;
    printf("{\"rtc_frame_case\":\"%s\",\"errors\":%d,\"bytes\":%"PRIu64",\"chunks\":%"PRIu64",\"max_frame\":%zu,\"max_payload\":%zu}\n",
           name, errors, received, expected.chunks, expected.max_frame, expected.max_payload);
    return errors;
}

int promql_rtc_frame_fixture(void) {
    const char *range = "/api/v1/query_range?query=vector(1)&start=1700002800&end=1700102799&step=1";
    int errors = frame_case("plain-100000", range, 65536, false, 0, 0, 0);
    errors += frame_case("negotiated-2049", range, 2049, false, 0, 0, 0);
    errors += frame_case("unicode-205", "/api/v1/query?query=%22a%F0%9F%98%80b%CE%B1c%22&time=1700002804", 205, true, 0, 0, 0);
    errors += frame_case("unicode-too-small", "/api/v1/query?query=%22%F0%9F%98%80%22&time=1700002804", 202, true, 0, 0, EMSGSIZE);
    errors += frame_case("stopped-before-send", range, 65536, false, 0, 1, ECANCELED);
    errors += frame_case("send-failure", range, 65536, false, 2, 0, EIO);
    return errors;
}
