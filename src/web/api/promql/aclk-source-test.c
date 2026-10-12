// SPDX-License-Identifier: GPL-3.0-or-later

#include "aclk-source.h"
#include "daemon/config/netdata-conf-directories.h"
#include "web/server/web_client.h"
#include <json-c/json.h>
#include <zlib.h>

typedef struct {
    uint64_t length;
    bool gzip, unbacked;
    PROMQL_BODY *body;
    int error;
} BODY_PREPARATION;

static uint8_t pattern(uint64_t position) { return (uint8_t)('a' + position % 23); }

static void prepare_body(void *context, void *handle __maybe_unused,
                          const netdata_promql_accounting_context_t *account) {
    BODY_PREPARATION *preparation = context;
    PROMQL_READ_FRAME frame = {.account = account};
    PROMQL_BODY *body = promql_body_create(account, preparation->unbacked ? "" : netdata_configured_cache_dir,
                                           NETDATA_PROMQL_MEMORY_RESULT, &preparation->error);
    if(!body) return;
    uint8_t bytes[PROMQL_BODY_WINDOW];
    for(uint64_t offset = 0; offset < preparation->length && !preparation->error;) {
        size_t count = (size_t)MIN(preparation->length - offset, (uint64_t)sizeof(bytes));
        for(size_t i = 0; i < count; i++) bytes[i] = pattern(offset + i);
        if(!promql_body_append(body, bytes, count, &frame, &preparation->error)) break;
        offset += count;
    }
    if(!preparation->error) promql_body_seal(body, &frame, &preparation->error);
    if(!preparation->error && preparation->gzip) {
        PROMQL_BODY *encoded = promql_body_gzip(body, &frame, Z_DEFAULT_COMPRESSION, Z_DEFAULT_STRATEGY, &preparation->error);
        promql_body_release(body); body = encoded;
    }
    if(preparation->error) { promql_body_release(body); return; }
    preparation->body = body;
}

static json_object *native_envelope(const PROMQL_ACLK_ENVELOPE *envelope) {
    json_object *object = json_object_new_object();
    json_object_object_add(object, "type", json_object_new_string("http"));
    json_object_object_add(object, "msg-id", json_object_new_string(envelope->message_id));
    json_object_object_add(object, "timestamp", json_object_new_int64(envelope->timestamp / USEC_PER_SEC));
    json_object_object_add(object, "timestamp-offset-usec", json_object_new_int64(envelope->timestamp % USEC_PER_SEC));
    json_object_object_add(object, "connect", json_object_new_int64(envelope->session / USEC_PER_SEC));
    json_object_object_add(object, "connect-offset-usec", json_object_new_int64(envelope->session % USEC_PER_SEC));
    json_object_object_add(object, "version", json_object_new_int(envelope->version));
    json_object_object_add(object, "t-exec", json_object_new_int64(envelope->execution_time));
    json_object_object_add(object, "t-rx", json_object_new_int64(envelope->received_time));
    json_object_object_add(object, "http-code", json_object_new_int(envelope->code));
    return object;
}

static int compare_source(struct mqtt_payload_source *source, json_object *object, const char *headers,
                           size_t headers_length, uint64_t original_length, bool gzip) {
    size_t json_length;
    const char *json = json_object_to_json_string_length(object, JSON_C_TO_STRING_PLAIN, &json_length);
    const bool payload = original_length || headers_length;
    const uint64_t prefix = json_length + (payload ? 4 : 0) + headers_length;
    z_stream stream = {0};
    int errors = 0;
    bool finished = false;
    uint64_t decoded = 0;
    uint8_t output[PROMQL_BODY_WINDOW];
    if(gzip && inflateInit2(&stream, 15 + 16) != Z_OK) return 1;
    for(uint64_t offset = 0; !errors && offset < source->length;) {
        size_t count = (size_t)MIN(source->length - offset, (uint64_t)source->window_size);
        if(!source->read(source->context, offset, source->window, count)) { errors++; break; }
        size_t used = 0;
        while(used < count && offset + used < prefix) {
            uint64_t position = offset + used;
            uint8_t expected = position < json_length ? (uint8_t)json[position] :
                position < json_length + 4 ? (uint8_t)"\r\n\r\n"[position - json_length] :
                (uint8_t)headers[position - json_length - 4];
            errors += source->window[used++] != expected;
        }
        if(gzip && used < count) {
            if(finished) { errors++; break; }
            stream.next_in = source->window + used; stream.avail_in = (uInt)(count - used);
            do {
                stream.next_out = output; stream.avail_out = sizeof(output);
                int status = inflate(&stream, Z_NO_FLUSH);
                size_t produced = sizeof(output) - stream.avail_out;
                for(size_t i = 0; i < produced; i++) errors += output[i] != pattern(decoded++);
                finished = status == Z_STREAM_END;
                if(status != Z_OK && !finished) { errors++; break; }
                if(finished && stream.avail_in) { errors++; break; }
            } while(!finished && (stream.avail_in || !stream.avail_out));
        }
        else if(!gzip) while(used < count) errors += source->window[used++] != pattern(decoded++);
        offset += count;
    }
    if(gzip) { errors += !finished; inflateEnd(&stream); }
    errors += decoded != original_length;
    errors += source->read(source->context, UINT64_MAX, output, 1) ||
        source->read(source->context, source->length, output, 1) ||
        !source->read(source->context, source->length, NULL, 0);
    return errors;
}

static int source_case(netdata_promql_engine_t *engine, const char *name, uint64_t length,
                        bool gzip, size_t id_length, bool unbacked) {
    netdata_promql_report_t *report = NULL;
    int64_t limit;
    int errors = netdata_promql_api_parse_limit(engine, (netdata_promql_bytes_t){0}, NULL, &limit, &report) != NETDATA_PROMQL_OK;
    BODY_PREPARATION preparation = {.length = length, .gzip = gzip, .unbacked = unbacked};
    if(!errors) errors += netdata_promql_report_accounted(report, prepare_body, &preparation, NULL) != NETDATA_PROMQL_OK || !preparation.body;
    char *id = mallocz(id_length + 1);
    const char special[] = "a/<> &\b\f\n\r\t\\\"\x01\x7f\xff\xe2\x80\xa8\xe2\x80\xa9";
    for(size_t i = 0; i < id_length; i++) id[i] = special[i % (sizeof(special) - 1)];
    id[id_length] = 0;
    char headers[160];
    snprintfz(headers, sizeof(headers) - 1, "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\n%s\r\n",
               gzip ? "Content-Encoding: gzip\r\n" : "");
    PROMQL_ACLK_ENVELOPE envelope = {.message_id = id, .headers = headers, .headers_length = strlen(headers),
        .timestamp = UINT64_C(1700002800123456), .session = UINT64_C(1700000000765432),
        .execution_time = UINT64_MAX, .received_time = UINT64_C(9223372036854775808), .version = 2, .code = 200};
    json_object *expected = native_envelope(&envelope);
    struct mqtt_payload_source *source = NULL;
    struct web_client *client = NULL;
    size_t native_memory = 0;
    uint64_t bytes = 0, current = 0;
    if(preparation.body && report && promql_body_attach_report(preparation.body, report)) {
        report = NULL;
        client = web_client_create(&native_memory);
        errors += !web_client_attach_promql_body(client, preparation.body);
        if(client->response.promql_body) preparation.body = NULL;
        int error = 0;
        source = promql_aclk_source(client->response.promql_body, &envelope, &error);
        if(unbacked) errors += source != NULL || error != ENOENT;
        else errors += !source || error;
        web_client_reuse_from_cache(client);
        web_client_free(client); client = NULL;
        errors += native_memory != 0;
    }
    if(source) {
        bytes = source->length;
        errors += source->window_size != PROMQL_BODY_WINDOW || !source->view(source->context, 777);
        memset(id, 'z', id_length);
        char expected_headers[sizeof(headers)];
        memcpy(expected_headers, headers, sizeof(headers)); memset(headers, 'x', sizeof(headers));
        errors += compare_source(source, expected, expected_headers, envelope.headers_length, length, gzip);
        netdata_promql_engine_observations_t observed = {0};
        errors += netdata_promql_engine_observations(engine, &observed) != NETDATA_PROMQL_OK;
        current = observed.memory[NETDATA_PROMQL_MEMORY_RESULT].current;
        // Prefix/body backing and a single send window stay fixed as payload and message ID grow.
        errors += current > 65536;
        source->release(source->context);
    }
    promql_body_release(preparation.body);
    if(report) netdata_promql_report_release(report);
    if(client) web_client_free(client);
    json_object_put(expected); freez(id);
    netdata_promql_engine_observations_t after = {0};
    errors += netdata_promql_engine_observations(engine, &after) != NETDATA_PROMQL_OK;
    for(size_t i = 0; i < NETDATA_PROMQL_MEMORY_KINDS; i++)
        if(i != NETDATA_PROMQL_MEMORY_ACCOUNTING) errors += after.memory[i].current != 0;
    printf("{\"aclk_source_case\":\"%s\",\"errors\":%d,\"bytes\":%"PRIu64",\"current_result\":%"PRIu64"}\n",
           name, errors, bytes, current);
    return errors;
}

int promql_aclk_source_fixture(netdata_promql_engine_t *engine) {
    int errors = source_case(engine, "plain-8193", 8193, false, 64, false);
    errors += source_case(engine, "plain-1000000", 1000000, false, 64, false);
    errors += source_case(engine, "gzip-1000000", 1000000, true, 64, false);
    errors += source_case(engine, "long-prefix", 8193, false, 20000, false);
    errors += source_case(engine, "no-prefix-backing", 3, false, 20000, true);
    return errors;
}
