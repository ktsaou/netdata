// SPDX-License-Identifier: GPL-3.0-or-later

#include "output.h"
#include "provider.h"
#include "database/rrd.h"
#include "daemon/config/netdata-conf-directories.h"
#include "web/server/web_client.h"
#include <zlib.h>

#define OUTPUT_BASE INT64_C(1700002800)

typedef struct { PROMQL_OUTPUT *output; const char *directory; bool gzip; int error; } OUTPUT_PREPARATION;

static void prepare_output(void *context, void *handle __maybe_unused,
                             const netdata_promql_accounting_context_t *account) {
    OUTPUT_PREPARATION *preparation = context;
    preparation->output = promql_output_create(account, preparation->directory, 0, false,
                                               preparation->gzip, &preparation->error);
}

typedef struct {
    const char *literal;
    unsigned stage;
    uint64_t points, index;
    char fragment[192];
    size_t length, consumed;
} EXPECTED_BODY;

static bool expected_next(EXPECTED_BODY *expected) {
    expected->consumed = 0;
    if(expected->literal) {
        if(expected->stage++) return false;
        expected->length = strlen(expected->literal);
        return true;
    }
    if(!expected->stage) {
        expected->stage = 1;
        snprintfz(expected->fragment, sizeof(expected->fragment) - 1,
            "{\"status\":\"success\",\"data\":{\"resultType\":\"matrix\",\"result\":[{\"metric\":{},\"values\":[");
    }
    else if(expected->stage == 1 && expected->index < expected->points) {
        snprintfz(expected->fragment, sizeof(expected->fragment) - 1, "%s[%"PRIu64",\"1\"]",
                  expected->index ? "," : "", (uint64_t)OUTPUT_BASE + expected->index);
        expected->index++;
    }
    else if(expected->stage == 1) {
        expected->stage = 2;
        snprintfz(expected->fragment, sizeof(expected->fragment) - 1, "]}]}}");
    }
    else return false;
    expected->length = strlen(expected->fragment);
    return true;
}

static bool expected_consume(EXPECTED_BODY *expected, const uint8_t *bytes, size_t length) {
    while(length) {
        if(expected->consumed == expected->length && !expected_next(expected)) return false;
        size_t count = MIN(length, expected->length - expected->consumed);
        const char *fragment = expected->literal ? expected->literal : expected->fragment;
        if(memcmp(bytes, fragment + expected->consumed, count)) return false;
        bytes += count; length -= count; expected->consumed += count;
    }
    return true;
}

static bool expected_end(EXPECTED_BODY *expected) {
    return expected->consumed == expected->length && !expected_next(expected);
}

static int compare_body(PROMQL_BODY *body, EXPECTED_BODY *expected) {
    uint8_t input[PROMQL_BODY_WINDOW], decoded[PROMQL_BODY_WINDOW];
    uint64_t offset = 0, length = promql_body_length(body);
    int error = 0;
    if(!promql_body_is_gzip(body)) {
        while(offset < length) {
            size_t count = (size_t)MIN(length - offset, (uint64_t)sizeof(input));
            if(!promql_body_read(body, offset, input, count, NULL, &error) || !expected_consume(expected, input, count)) return 1;
            offset += count;
        }
        return !expected_end(expected);
    }
    z_stream stream = {0};
    if(inflateInit2(&stream, 15 + 16) != Z_OK) return 1;
    bool valid = true, finished = false;
    while(valid && !finished) {
        if(!stream.avail_in && offset < length) {
            size_t count = (size_t)MIN(length - offset, (uint64_t)sizeof(input));
            if(!promql_body_read(body, offset, input, count, NULL, &error)) { valid = false; break; }
            offset += count;
            stream.next_in = input; stream.avail_in = (uInt)count;
        }
        stream.next_out = decoded; stream.avail_out = sizeof(decoded);
        int status = inflate(&stream, Z_NO_FLUSH);
        valid = (status == Z_OK || status == Z_STREAM_END) &&
            expected_consume(expected, decoded, sizeof(decoded) - stream.avail_out);
        finished = status == Z_STREAM_END;
        if(!finished && !stream.avail_in && offset == length && stream.avail_out == sizeof(decoded)) valid = false;
    }
    valid = valid && finished && offset == length && !stream.avail_in && expected_end(expected);
    inflateEnd(&stream);
    return !valid;
}

static int native_delivery(PROMQL_BODY *body, uint64_t points) {
    int pair[2] = {-1, -1}, errors = 0, size = 1024;
    size_t native_memory = 0;
    bool short_write = false, keepalive = points == 2049;
    uint64_t offset = 0, length = promql_body_length(body), iterations = 0, sent = 0;
    uint8_t received[997], expected[997];
    struct web_client *client = NULL;
    if(socketpair(AF_UNIX, SOCK_STREAM, 0, pair)) return 1;
    if(fcntl(pair[0], F_SETFL, O_NONBLOCK) || fcntl(pair[1], F_SETFL, O_NONBLOCK) ||
       setsockopt(pair[0], SOL_SOCKET, SO_SNDBUF, &size, sizeof(size))) { errors++; goto done; }
    client = web_client_create(&native_memory);
    client->fd = pair[0];
    web_client_set_conn_unix(client);
    if(keepalive) web_client_enable_keepalive(client);
    client->response.code = HTTP_RESP_OK;
    client->response.data->content_type = CT_APPLICATION_JSON;
    client->response.zinitialized = deflateInit(&client->response.zstream, Z_DEFAULT_COMPRESSION) == Z_OK;
    client->response.zoutput = client->response.zinitialized;
    web_client_flag_set(client, WEB_CLIENT_CHUNKED_TRANSFER);
    if(!promql_body_acquire(body)) { errors++; goto done; }
    if(!web_client_attach_promql_body(client, body)) {
        promql_body_release(body); errors++; goto done;
    }
    errors += client->response.zinitialized || client->response.zoutput ||
        web_client_flag_check(client, WEB_CLIENT_CHUNKED_TRANSFER);
    char expected_header[80];
    snprintfz(expected_header, sizeof(expected_header) - 1, "Content-Length: %"PRIu64"\r\n", length);
    web_client_build_http_header(client);
    const char *header = buffer_tostring(client->response.header_output);
    errors += !strstr(header, expected_header) || strstr(header, "Transfer-Encoding:") != NULL ||
        (strstr(header, "Content-Encoding: gzip\r\n") != NULL) != promql_body_is_gzip(body);
    while(!errors && offset < length) {
        if(++iterations > length + 1024) { errors++; break; }
        if(client->response.promql_body) {
            ssize_t written = web_client_send(client);
            if(written < 0) { errors++; break; }
            sent += (uint64_t)written;
        }
        short_write |= client->response.sent > 0 && client->response.sent < client->response.data->len;
        ssize_t bytes = recv(pair[1], received, sizeof(received), MSG_DONTWAIT);
        if(bytes < 0 && (errno == EAGAIN || errno == EWOULDBLOCK)) continue;
        int error = 0;
        if(bytes <= 0 || !promql_body_read(body, offset, expected, (size_t)bytes, NULL, &error) ||
           memcmp(received, expected, (size_t)bytes)) { errors++; break; }
        offset += (uint64_t)bytes;
        errors += client->response.data->len > PROMQL_BODY_WINDOW || client->response.promql_sent > length;
    }
    errors += !short_write || offset != length || sent != length;
    if(!errors) {
        if(client->response.promql_body) errors += web_client_send(client) != 0;
        if(keepalive) errors += client->response.promql_body != NULL || client->response.promql_window != NULL;
        else errors += !web_client_flag_check(client, WEB_CLIENT_FLAG_DEAD);
    }
done:
    if(client) web_client_free(client);
    if(pair[0] >= 0) close(pair[0]);
    if(pair[1] >= 0) close(pair[1]);
    errors += native_memory != 0;
    // An unfinished response must also release its body and transfer-window lease.
    client = web_client_create(&native_memory);
    if(promql_body_acquire(body)) {
        if(!web_client_attach_promql_body(client, body)) { promql_body_release(body); errors++; }
    }
    else errors++;
    web_client_free(client);
    errors += native_memory != 0;
    printf("{\"native_case\":\"%s-%"PRIu64"\",\"errors\":%d,\"bytes\":%"PRIu64
           ",\"received\":%"PRIu64",\"short_write\":%s,\"keepalive\":%s}\n",
           promql_body_is_gzip(body) ? "gzip" : "plain", points, errors, length, offset,
           short_write ? "true" : "false", keepalive ? "true" : "false");
    return errors;
}

static int output_case(netdata_promql_engine_t *engine, const char *name, const char *expression,
                         uint64_t points, bool gzip, bool backing, uint32_t wanted, const char *literal,
                         uint64_t *result_peak, uint64_t *workspace_peak) {
    netdata_promql_query_t *query = NULL;
    netdata_promql_report_t *report = NULL;
    uint32_t status = netdata_promql_compile(engine, promql_bytes(expression), NULL, &query, &report);
    int errors = status != NETDATA_PROMQL_OK || !report;
    OUTPUT_PREPARATION preparation = {.directory = backing ? netdata_configured_cache_dir : "", .gzip = gzip};
    if(!errors) errors += netdata_promql_report_accounted(report, prepare_output, &preparation, NULL) != NETDATA_PROMQL_OK;
    if(report) { netdata_promql_report_release(report); report = NULL; }
    errors += !preparation.output;
    PROMQL_PROVIDER_CONTEXT context = {.scope = PROMQL_HOST_GUID, .host = localhost->machine_guid,
        .backing_directory = netdata_configured_cache_dir};
    netdata_promql_provider_t provider = promql_provider(&context);
    netdata_promql_precise_request_t request = {.header = {NETDATA_PROMQL_ABI_VERSION, sizeof(request)},
        .kind = points ? NETDATA_PROMQL_REQUEST_RANGE : NETDATA_PROMQL_REQUEST_INSTANT,
        .start = {.unix_seconds = OUTPUT_BASE}, .end = {.unix_seconds = OUTPUT_BASE + (points ? (int64_t)points - 1 : 0)},
        .interval_ns = INT64_C(1000000000), .lookback_ns = INT64_C(300000000000)};
    if(!errors) {
        netdata_promql_output_t output = promql_output_callbacks(preparation.output);
        status = netdata_promql_execute_precise(query, &provider, &request, NULL, &output, &report);
    }
    errors += status != wanted || !report;
    netdata_promql_query_observations_t observations = {0};
    if(report) netdata_promql_report_observations(report, &observations);
    if(result_peak) *result_peak = observations.memory[NETDATA_PROMQL_MEMORY_RESULT].peak;
    if(workspace_peak) *workspace_peak = observations.memory[NETDATA_PROMQL_MEMORY_WORKSPACE].peak;
    PROMQL_BODY *body = promql_output_take_body(preparation.output);
    uint64_t body_bytes = promql_body_length(body);
    if(wanted == NETDATA_PROMQL_OK) {
        errors += !body || !promql_body_is_sealed(body) || promql_body_is_gzip(body) != gzip;
        if(body) {
            EXPECTED_BODY expected = {.literal = literal, .points = points};
            errors += compare_body(body, &expected);
            if(report && promql_body_attach_report(body, report)) report = NULL;
            else errors++;
            if(points && !errors) errors += native_delivery(body, points);
        }
    }
    else errors += body != NULL;
    if(errors && report) {
        netdata_promql_error_t failure;
        if(netdata_promql_report_error(report, 0, &failure) == NETDATA_PROMQL_OK)
            fprintf(stderr, "output failure: %s status%u %.*s\n", name, status, (int)failure.message.len, (const char *)failure.message.data);
    }
    promql_body_release(body);
    promql_output_destroy(preparation.output);
    if(report) netdata_promql_report_release(report);
    if(query) netdata_promql_query_release(query);
    printf("{\"output_case\":\"%s\",\"status\":%u,\"errors\":%d,\"points\":%"PRIu64
           ",\"body_bytes\":%"PRIu64",\"result_peak\":%"PRIu64",\"workspace_peak\":%"PRIu64"}\n",
           name, status, errors, points, body_bytes,
           observations.memory[NETDATA_PROMQL_MEMORY_RESULT].peak, observations.memory[NETDATA_PROMQL_MEMORY_WORKSPACE].peak);
    return errors;
}

int promql_output_fixture(netdata_promql_engine_t *engine) {
    int errors = 0;
    errors += output_case(engine, "scalar-no-backing", "1", 0, false, false, NETDATA_PROMQL_OK,
        "{\"status\":\"success\",\"data\":{\"resultType\":\"scalar\",\"result\":[1700002800,\"1\"]}}", NULL, NULL);
    errors += output_case(engine, "string-escaping", "\"\\x08\\x0c<&\\xff\"", 0, false, false, NETDATA_PROMQL_OK,
        "{\"status\":\"success\",\"data\":{\"resultType\":\"string\",\"result\":[1700002800,\"\\b\\f\\u003c\\u0026\\ufffd\"]}}", NULL, NULL);
    errors += output_case(engine, "vector-no-backing", "vector(1)", 0, false, false, NETDATA_PROMQL_OK,
        "{\"status\":\"success\",\"data\":{\"resultType\":\"vector\",\"result\":[{\"metric\":{},\"value\":[1700002800,\"1\"]}]}}", NULL, NULL);
    errors += output_case(engine, "large-no-backing", "vector(1)", 2049, false, false, NETDATA_PROMQL_OUTPUT_ERROR, NULL, NULL, NULL);
    for(unsigned gzip = 0; gzip < 2; gzip++) {
        uint64_t small_result = 0, large_result = 0, small_workspace = 0, large_workspace = 0;
        errors += output_case(engine, gzip ? "gzip-2049" : "plain-2049", "vector(1)", 2049, gzip, true,
                              NETDATA_PROMQL_OK, NULL, &small_result, &small_workspace);
        errors += output_case(engine, gzip ? "gzip-100000" : "plain-100000", "vector(1)", 100000, gzip, true,
                              NETDATA_PROMQL_OK, NULL, &large_result, &large_workspace);
        // The small gzip body stays inline; only the large one adds a fixed file owner.
        uint64_t extra_file = gzip ? sizeof(PROMQL_FILE) + sizeof(PROMQL_MEMORY) : 0;
        if(large_result != small_result + extra_file || small_workspace != large_workspace) {
            fprintf(stderr, "output history memory changed: gzip%u result%"PRIu64"/%"PRIu64" workspace%"PRIu64"/%"PRIu64"\n",
                    gzip, small_result, large_result, small_workspace, large_workspace);
            errors++;
        }
    }
    return errors;
}
