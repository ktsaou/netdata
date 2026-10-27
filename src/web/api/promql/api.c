// SPDX-License-Identifier: GPL-3.0-or-later

#include "api-internal.h"
#include "daemon/config/netdata-conf-directories.h"

static pthread_mutex_t api_lock = PTHREAD_MUTEX_INITIALIZER;
static netdata_promql_engine_t *api_engine;
static size_t api_users;
static bool api_stopping;
static int64_t api_lookback;
static const char *api_directory;

static bool configured_duration(netdata_promql_engine_t *engine, const char *name, const char *fallback, int64_t *out) {
    const char *value = inicfg_get(&netdata_config, "promql", name, fallback);
    netdata_promql_report_t *report = NULL;
    uint32_t status = netdata_promql_api_parse_duration(engine, promql_bytes(value), NULL, out, &report);
    if(report) netdata_promql_report_release(report);
    if(status == NETDATA_PROMQL_OK && *out > 0) return true;
    netdata_log_error("PromQL configuration '%s' must be a positive duration", name);
    return false;
}

static uint32_t engine_acquire(PROMQL_API_REQUEST *request) {
    pthread_mutex_lock(&api_lock);
    uint32_t status = api_stopping ? NETDATA_PROMQL_CANCELLED : NETDATA_PROMQL_OK;
    if(status == NETDATA_PROMQL_OK && !api_engine) {
        netdata_promql_engine_options_t options;
        status = netdata_promql_engine_options_default(&options);
        const struct { const char *name; uint32_t flag; } switches[] = {
            {"experimental functions", NETDATA_PROMQL_EXPERIMENTAL_FUNCTIONS},
            {"binary fill modifiers", NETDATA_PROMQL_BINOP_FILL_MODIFIERS},
            {"extended range selectors", NETDATA_PROMQL_EXTENDED_RANGE_SELECTORS},
            {"start timestamps", NETDATA_PROMQL_START_TIMESTAMPS},
            {"delayed metric name removal", NETDATA_PROMQL_DELAYED_NAME_REMOVAL},
        };
        for(size_t i = 0; status == NETDATA_PROMQL_OK && i < sizeof(switches) / sizeof(switches[0]); i++)
            if(inicfg_get_boolean(&netdata_config, "promql", switches[i].name, CONFIG_BOOLEAN_NO)) options.flags |= switches[i].flag;
        if(status == NETDATA_PROMQL_OK) status = netdata_promql_engine_create(&options, &api_engine);
        int64_t subquery = 0;
        if(status == NETDATA_PROMQL_OK && (!configured_duration(api_engine, "lookback delta", "5m", &api_lookback) ||
            !configured_duration(api_engine, "default subquery step", "1m", &subquery))) status = NETDATA_PROMQL_INVALID_ARGUMENT;
        if(status == NETDATA_PROMQL_OK) {
            netdata_promql_engine_release(api_engine); api_engine = NULL;
            options.lookback_ms = MAX(INT64_C(1), api_lookback / INT64_C(1000000));
            options.no_step_subquery_interval_ms = MAX(INT64_C(1), subquery / INT64_C(1000000));
            status = netdata_promql_engine_create(&options, &api_engine);
            api_directory = inicfg_get(&netdata_config, "promql", "temporary directory", netdata_configured_cache_dir);
        }
        if(status != NETDATA_PROMQL_OK && api_engine) { netdata_promql_engine_release(api_engine); api_engine = NULL; }
    }
    if(status == NETDATA_PROMQL_OK) {
        api_users++;
        request->engine = api_engine; request->engine_acquired = true;
        request->directory = api_directory; request->request.lookback_ns = api_lookback;
    }
    pthread_mutex_unlock(&api_lock);
    return status;
}

static void engine_retire(PROMQL_API_REQUEST *request) {
    if(!request->engine_acquired) return;
    pthread_mutex_lock(&api_lock);
    internal_fatal(!api_users, "PromQL API engine reference underflow");
    api_users--;
    if(api_stopping && !api_users && api_engine) { netdata_promql_engine_release(api_engine); api_engine = NULL; }
    pthread_mutex_unlock(&api_lock);
    request->engine_acquired = false;
}

void promql_api_cleanup(void) {
    pthread_mutex_lock(&api_lock);
    api_stopping = true;
    if(!api_users && api_engine) { netdata_promql_engine_release(api_engine); api_engine = NULL; }
    pthread_mutex_unlock(&api_lock);
}

uint32_t promql_api_observations(netdata_promql_engine_observations_t *out) {
    pthread_mutex_lock(&api_lock);
    uint32_t status = api_engine ? netdata_promql_engine_observations(api_engine, out) : NETDATA_PROMQL_INVALID_ARGUMENT;
    pthread_mutex_unlock(&api_lock);
    return status;
}

void promql_api_replace_report(PROMQL_API_REQUEST *request, netdata_promql_report_t *report) {
    if(request->report) netdata_promql_report_release(request->report);
    request->report = report;
}

void promql_api_bad_data(PROMQL_API_REQUEST *request, const char *parameter, const char *message) {
    request->failure = (PROMQL_API_ERROR){.code = HTTP_RESP_BAD_REQUEST, .type = "bad_data",
        .parameter = parameter, .message = promql_bytes(message)};
}

void promql_api_report_failure(PROMQL_API_REQUEST *request, uint32_t status, const char *parameter,
                                netdata_promql_bytes_t source) {
    const char *message = "PromQL internal error", *type = "internal";
    int code = HTTP_RESP_INTERNAL_SERVER_ERROR;
    switch(status) {
        case NETDATA_PROMQL_PARSE_ERROR: type = "bad_data"; code = HTTP_RESP_BAD_REQUEST; message = "invalid expression"; break;
        case NETDATA_PROMQL_CANCELLED: type = "canceled"; code = HTTP_RESP_CLIENT_CLOSED_REQUEST; message = "query canceled"; break;
        case NETDATA_PROMQL_DEADLINE: type = "timeout"; code = HTTP_RESP_SERVICE_UNAVAILABLE; message = "query timed out"; break;
        case NETDATA_PROMQL_EVALUATION_ERROR:
        case NETDATA_PROMQL_UNSUPPORTED: type = "execution"; code = 422; message = "query evaluation failed"; break;
        case NETDATA_PROMQL_PROVIDER_ERROR: message = "PromQL storage provider failed"; break;
        case NETDATA_PROMQL_OUTPUT_ERROR: message = "PromQL response backing failed"; break;
        default: break;
    }
    request->failure = (PROMQL_API_ERROR){.code = code, .type = type,
        .parameter = status == NETDATA_PROMQL_PARSE_ERROR ? parameter : NULL, .message = promql_bytes(message), .source = source};
    netdata_promql_error_t error;
    if(request->report && netdata_promql_report_error(request->report, 0, &error) == NETDATA_PROMQL_OK) {
        request->failure.message = error.message;
        request->failure.position_present = error.kind == NETDATA_PROMQL_ERROR_PARSE && error.position_present;
        request->failure.position = error.position.start;
    }
}

int64_t promql_api_milliseconds(netdata_promql_timestamp_t value) {
    uint64_t bits = (uint64_t)value.unix_seconds * UINT64_C(1000) + value.nanoseconds / UINT32_C(1000000);
    int64_t result; memcpy(&result, &bits, sizeof(result));
    return result;
}

bool promql_api_before(netdata_promql_timestamp_t a, netdata_promql_timestamp_t b) {
    // Go compares its internal signed seconds, including the wrap at extreme Unix values.
    uint64_t a_bits = (uint64_t)a.unix_seconds + UINT64_C(62135596800);
    uint64_t b_bits = (uint64_t)b.unix_seconds + UINT64_C(62135596800);
    int64_t a_seconds, b_seconds;
    memcpy(&a_seconds, &a_bits, sizeof(a_seconds)); memcpy(&b_seconds, &b_bits, sizeof(b_seconds));
    return a_seconds < b_seconds || (a_seconds == b_seconds && a.nanoseconds < b.nanoseconds);
}

bool promql_api_time(PROMQL_API_REQUEST *request, PROMQL_PARAMETER parameter, const char *name,
                     bool optional, netdata_promql_timestamp_t fallback, netdata_promql_timestamp_t *out) {
    netdata_promql_bytes_t value = promql_form_value(request->form, parameter);
    if(optional && !value.len) { *out = fallback; return true; }
    netdata_promql_report_t *report = NULL;
    uint32_t status = netdata_promql_api_parse_time(request->engine, value, request->operation.control, out, &report);
    promql_api_replace_report(request, report);
    if(status == NETDATA_PROMQL_OK) return true;
    promql_api_report_failure(request, status, name, (netdata_promql_bytes_t){0});
    if(optional && status == NETDATA_PROMQL_PARSE_ERROR) request->failure.time_parameter = name;
    return false;
}

bool promql_api_limit(PROMQL_API_REQUEST *request) {
    netdata_promql_report_t *report = NULL;
    int64_t limit;
    uint32_t status = netdata_promql_api_parse_limit(request->engine, promql_form_value(request->form, PROMQL_PARAM_LIMIT),
        request->operation.control, &limit, &report);
    promql_api_replace_report(request, report);
    if(status != NETDATA_PROMQL_OK) { promql_api_report_failure(request, status, "limit", (netdata_promql_bytes_t){0}); return false; }
    request->limit = (uint64_t)limit;
    return true;
}

static bool parse_duration(PROMQL_API_REQUEST *request, PROMQL_PARAMETER parameter, const char *name, int64_t *out) {
    netdata_promql_report_t *report = NULL;
    uint32_t status = netdata_promql_api_parse_duration(request->engine, promql_form_value(request->form, parameter),
        request->operation.control, out, &report);
    promql_api_replace_report(request, report);
    if(status == NETDATA_PROMQL_OK) return true;
    promql_api_report_failure(request, status, name, (netdata_promql_bytes_t){0});
    return false;
}

static void prepare_form(void *context, void *handle __maybe_unused, const netdata_promql_accounting_context_t *account) {
    PROMQL_API_REQUEST *request = context;
    struct web_client *client = request->client;
    PROMQL_READ_FRAME frame = {.account = account, .call_control = request->operation.control};
    netdata_promql_bytes_t target = {.data = (const uint8_t *)buffer_tostring(client->url_as_received),
        .len = buffer_strlen(client->url_as_received)};
    netdata_promql_bytes_t payload = client->payload ? (netdata_promql_bytes_t){
        .data = (const uint8_t *)buffer_tostring(client->payload), .len = buffer_strlen(client->payload)} : (netdata_promql_bytes_t){0};
    request->form = promql_form_create(target, payload, client->request_content_type,
        client->mode == HTTP_REQUEST_MODE_POST, &frame, &request->error);
}

static size_t encode_rune(uint32_t rune, uint8_t *out) {
    if(rune < 0x80) { if(out) out[0] = (uint8_t)rune; return 1; }
    size_t width = rune < 0x800 ? 2 : rune < 0x10000 ? 3 : 4;
    if(out) {
        uint32_t tail = rune;
        for(size_t i = width - 1; i; i--) { out[i] = 0x80 | (tail & 0x3f); tail >>= 6; }
        out[0] = (uint8_t)((width == 2 ? 0xc0 : width == 3 ? 0xe0 : 0xf0) | tail);
    }
    return width;
}

static bool unescape_name(uint8_t *bytes, size_t length, bool write, size_t *decoded) {
    size_t output = 0;
    for(size_t i = 3; i < length; i++) {
        if(bytes[i] != '_') { if(write) bytes[output] = bytes[i]; output++; continue; }
        if(++i == length) return false;
        if(bytes[i] == '_') { if(write) bytes[output] = '_'; output++; continue; }
        uint32_t rune = 0;
        size_t digits = 0;
        while(i < length && bytes[i] != '_') {
            int digit = promql_hex_digit(bytes[i]);
            if(digit < 0 || digits++ >= 5) return false;
            rune = rune * 16 + (uint32_t)digit; i++;
        }
        if(i == length || !digits || rune > 0x10ffff || (rune >= 0xd800 && rune <= 0xdfff)) return false;
        output += encode_rune(rune, write ? bytes + output : NULL);
    }
    *decoded = output;
    return true;
}

static void prepare_label_name(void *context, void *handle __maybe_unused,
                                const netdata_promql_accounting_context_t *account) {
    PROMQL_API_REQUEST *request = context;
    PROMQL_READ_FRAME frame = {.account = account, .call_control = request->operation.control};
    const uint8_t *target = (const uint8_t *)buffer_tostring(request->client->url_as_received);
    size_t length = buffer_strlen(request->client->url_as_received);
    const uint8_t *question = memchr(target, '?', length);
    if(question) length = (size_t)(question - target);
    static const char marker[] = "/api/v1/label/", suffix[] = "/values";
    size_t begin = SIZE_MAX;
    for(size_t i = 0; i + sizeof(marker) - 1 <= length; i++)
        if(!memcmp(target + i, marker, sizeof(marker) - 1)) { begin = i + sizeof(marker) - 1; break; }
    if(begin == SIZE_MAX || length - begin < sizeof(suffix) - 1 ||
       memcmp(target + length - sizeof(suffix) + 1, suffix, sizeof(suffix) - 1)) {
        request->failure = (PROMQL_API_ERROR){.code = HTTP_RESP_NOT_FOUND, .type = "not_found", .message = promql_bytes("not found")};
        return;
    }
    length -= begin + sizeof(suffix) - 1;
    if(memchr(target + begin, '/', length)) {
        request->failure = (PROMQL_API_ERROR){.code = HTTP_RESP_NOT_FOUND, .type = "not_found", .message = promql_bytes("not found")};
        return;
    }
    uint8_t *decoded = promql_malloc(account, NETDATA_PROMQL_MEMORY_WORKSPACE, length + 1);
    if(!decoded) { request->error = ENOMEM; return; }
    request->label_name_owned = true; request->label_name.data = decoded;
    size_t output = 0;
    for(size_t i = 0; i < length; i++) {
        if(!(i % PROMQL_BODY_WINDOW) && !promql_read_poll(&frame)) { request->error = ECANCELED; break; }
        uint8_t byte = target[begin + i];
        if(byte == '%') {
            if(length - i < 3 || promql_hex_digit(target[begin + i + 1]) < 0 || promql_hex_digit(target[begin + i + 2]) < 0) {
                request->failure = (PROMQL_API_ERROR){.code = HTTP_RESP_BAD_REQUEST, .type = "bad_data", .message = promql_bytes("invalid label path escape")};
                break;
            }
            byte = (uint8_t)(promql_hex_digit(target[begin + i + 1]) * 16 + promql_hex_digit(target[begin + i + 2])); i += 2;
        }
        decoded[output++] = byte;
    }
    if(request->error || request->failure.code) return;
    size_t unescaped;
    if(output >= 3 && !memcmp(decoded, "U__", 3) && unescape_name(decoded, output, false, &unescaped)) {
        unescape_name(decoded, output, true, &unescaped); output = unescaped;
    }
    request->label_name.len = output;
    bool valid = output != 0;
    for(size_t i = 0; valid && i < output;) {
        uint32_t rune;
        size_t width = promql_utf8_rune(decoded + i, output - i, &rune);
        valid = width != 0; i += width;
    }
    if(!valid) request->failure = (PROMQL_API_ERROR){.code = HTTP_RESP_BAD_REQUEST, .type = "bad_data",
        .prefix = "invalid label name: ", .message = request->label_name, .quote = true};
}

static PROMQL_OUTPUT_STORAGE output_storage(PROMQL_API_REQUEST *request) {
    return (PROMQL_OUTPUT_STORAGE){.pages = request->context.storage_pages,
        .points = request->context.storage_points, .tiers = RRD_STORAGE_TIERS,
        .reconstruction_points = &request->context.reconstruction_points,
        .overflowed = &request->context.counters_overflowed};
}

static void prepare_output(void *context, void *handle __maybe_unused, const netdata_promql_accounting_context_t *account) {
    PROMQL_API_REQUEST *request = context;
    netdata_promql_bytes_t stats = promql_form_value(request->form, PROMQL_PARAM_STATS);
    request->output = promql_output_create(account, request->directory, request->limit, stats.len != 0, request->gzip, &request->error);
    if(request->output) {
        promql_output_stats_parameter(request->output, stats);
        promql_output_storage(request->output, output_storage(request));
    }
}

static bool query(PROMQL_API_REQUEST *request, bool range) {
    if(!promql_api_limit(request)) return false;
    struct timespec now = {0};
    if(clock_gettime(CLOCK_REALTIME, &now)) {
        promql_api_report_failure(request, NETDATA_PROMQL_INTERNAL_ERROR, NULL, (netdata_promql_bytes_t){0}); return false;
    }
    netdata_promql_timestamp_t current = {.unix_seconds = now.tv_sec, .nanoseconds = (uint32_t)now.tv_nsec};
    if(!promql_api_time(request, range ? PROMQL_PARAM_START : PROMQL_PARAM_TIME, range ? "start" : "time",
        !range, current, &request->request.start)) return false;
    if(range) {
        if(!promql_api_time(request, PROMQL_PARAM_END, "end", false, current, &request->request.end)) return false;
        if(promql_api_before(request->request.end, request->request.start)) {
            promql_api_bad_data(request, "end", "end timestamp must not be before start time"); return false;
        }
        if(!parse_duration(request, PROMQL_PARAM_STEP, "step", &request->request.interval_ns)) return false;
        if(request->request.interval_ns <= 0) {
            promql_api_bad_data(request, "step", "zero or negative query resolution step widths are not accepted. Try a positive integer"); return false;
        }
    }
    if(promql_form_value(request->form, PROMQL_PARAM_TIMEOUT).len) {
        int64_t timeout;
        if(!parse_duration(request, PROMQL_PARAM_TIMEOUT, "timeout", &timeout)) return false;
        uint32_t status = promql_operation_timeout(&request->operation, request->engine, timeout);
        if(status != NETDATA_PROMQL_OK) { promql_api_report_failure(request, status, NULL, (netdata_promql_bytes_t){0}); return false; }
    }
    if(promql_form_value(request->form, PROMQL_PARAM_LOOKBACK).len) {
        int64_t lookback;
        if(!parse_duration(request, PROMQL_PARAM_LOOKBACK, NULL, &lookback)) {
            if(request->failure.code == HTTP_RESP_BAD_REQUEST) request->failure.prefix = "error parsing lookback delta duration: ";
            return false;
        }
        if(lookback > 0) request->request.lookback_ns = lookback;
    }
    netdata_promql_bytes_t source = promql_form_value(request->form, PROMQL_PARAM_QUERY);
    netdata_promql_report_t *report = NULL;
    uint32_t status = netdata_promql_compile(request->engine, source, request->operation.control, &request->query, &report);
    promql_api_replace_report(request, report);
    if(status != NETDATA_PROMQL_OK) { promql_api_report_failure(request, status, "query", source); return false; }
    if(range) {
        uint32_t type;
        status = netdata_promql_query_type(request->query, &type);
        if(status != NETDATA_PROMQL_OK) { promql_api_report_failure(request, status, NULL, source); return false; }
        if(type != NETDATA_PROMQL_TYPE_SCALAR && type != NETDATA_PROMQL_TYPE_VECTOR) {
            promql_api_bad_data(request, "query", type == NETDATA_PROMQL_TYPE_MATRIX ?
                "invalid expression type \"range vector\" for range query, must be Scalar or instant Vector" :
                "invalid expression type \"string\" for range query, must be Scalar or instant Vector");
            return false;
        }
    }
    netdata_promql_bytes_t options = promql_form_value(request->form, PROMQL_PARAM_OPTIONS);
    if(options.len && (options.len != 8 || memcmp(options.data, "absolute", 8))) {
        promql_api_bad_data(request, "options", "unsupported PromQL option; supported value is absolute"); return false;
    }
    request->context.absolute = options.len != 0;
    promql_operation_activate(&request->operation);
    status = netdata_promql_report_accounted(request->report, prepare_output, request, NULL);
    if(status != NETDATA_PROMQL_OK || !request->output) {
        promql_api_report_failure(request, NETDATA_PROMQL_OUTPUT_ERROR, NULL, source); return false;
    }
    netdata_promql_provider_t provider = promql_provider(&request->context);
    netdata_promql_output_t output = promql_output_callbacks(request->output);
    request->request.kind = range ? NETDATA_PROMQL_REQUEST_RANGE : NETDATA_PROMQL_REQUEST_INSTANT;
    report = NULL;
    request->execution_started = true;
    status = netdata_promql_execute_precise(request->query, &provider, &request->request, request->operation.control, &output, &report);
    promql_api_replace_report(request, report);
    if(status != NETDATA_PROMQL_OK) { promql_api_report_failure(request, status, NULL, source); return false; }
    request->body = promql_output_take_body(request->output);
    return request->body != NULL;
}

static bool error_text(PROMQL_API_REQUEST *request, PROMQL_BODY *body, const char *text, PROMQL_READ_FRAME *frame) {
    return promql_json_escape(body, promql_bytes(text), PROMQL_JSON_ENVELOPE, frame, &request->error);
}

static void prepare_error(void *context, void *handle __maybe_unused, const netdata_promql_accounting_context_t *account) {
    PROMQL_API_REQUEST *request = context;
    PROMQL_API_ERROR *failure = &request->failure;
    PROMQL_READ_FRAME frame = {.account = account};
    request->error = 0;
    PROMQL_BODY *body = promql_body_create(account, request->directory, NETDATA_PROMQL_MEMORY_RESULT, &request->error);
    request->body = body;
    if(!body || !promql_json_text(body, "{\"status\":\"error\",\"errorType\":", &frame, &request->error) ||
       !promql_json_string(body, promql_bytes(failure->type), PROMQL_JSON_ENVELOPE, &frame, &request->error) ||
       !promql_json_text(body, ",\"error\":\"", &frame, &request->error)) return;
    if(failure->parameter && (!error_text(request, body, "invalid parameter \"", &frame) ||
       !error_text(request, body, failure->parameter, &frame) || !error_text(request, body, "\": ", &frame))) return;
    if(failure->time_parameter && (!error_text(request, body, "invalid time value for '", &frame) ||
       !error_text(request, body, failure->time_parameter, &frame) || !error_text(request, body, "': ", &frame))) return;
    if(failure->prefix && !error_text(request, body, failure->prefix, &frame)) return;
    if(failure->position_present) {
        if(!failure->source.len) { if(!error_text(request, body, "unknown position", &frame)) return; }
        else if(failure->position > failure->source.len) { if(!error_text(request, body, "invalid position", &frame)) return; }
        else {
            uint64_t line = 1, column = 1;
            for(uint64_t i = 0; i < failure->position; i++) {
                if(failure->source.data[i] == '\n') { line++; column = 1; }
                else column++;
            }
            if(!promql_json_unsigned(body, line, &frame, &request->error) || !error_text(request, body, ":", &frame) ||
               !promql_json_unsigned(body, column, &frame, &request->error)) return;
        }
        if(!error_text(request, body, ": parse error: ", &frame)) return;
    }
    if(!(failure->quote ? promql_json_go_quote(body, failure->message, &frame, &request->error) :
         promql_json_escape(body, failure->message, PROMQL_JSON_ENVELOPE, &frame, &request->error))) return;
    if(failure->suffix && !error_text(request, body, failure->suffix, &frame)) return;
    if(!promql_json_text(body, "\"", &frame, &request->error)) return;
    netdata_promql_bytes_t stats = request->form ? promql_form_value(request->form, PROMQL_PARAM_STATS) : (netdata_promql_bytes_t){0};
    if(request->execution_started && stats.len == 3 && !memcmp(stats.data, "all", 3)) {
        netdata_promql_query_observations_t observations;
        if(netdata_promql_report_observations(request->report, &observations) == NETDATA_PROMQL_OK &&
           (!promql_json_text(body, ",\"stats\":{\"available\":true", &frame, &request->error) ||
            !promql_output_resources(body, &observations, output_storage(request), "execution-report", &frame, &request->error) ||
            !promql_json_text(body, "}", &frame, &request->error))) return;
    }
    if(!promql_json_text(body, "}", &frame, &request->error) || !promql_body_seal(body, &frame, &request->error)) return;
    if(request->gzip) {
        PROMQL_BODY *encoded = promql_body_gzip(body, &frame, web_gzip_level, web_gzip_strategy, &request->error);
        if(encoded) { promql_body_release(body); request->body = encoded; }
    }
}

static void cleanup(PROMQL_API_REQUEST *request) {
    promql_body_release(request->body);
    promql_output_destroy(request->output);
    if(request->selectors) for(size_t i = 0; i < request->selector_count; i++)
        if(request->selectors[i]) netdata_promql_selector_release(request->selectors[i]);
    promql_free(request->selectors);
    if(request->query) netdata_promql_query_release(request->query);
    if(request->label_name_owned) promql_free((void *)request->label_name.data);
    promql_form_destroy(request->form);
    if(request->report) netdata_promql_report_release(request->report);
    if(request->client->promql_operation_hook)
        request->client->promql_operation_hook(request->client->promql_operation_hook_data, 0);
    request->client->promql_operation_id = 0;
    promql_operation_end(&request->operation);
    engine_retire(request);
}

static int dispatch(struct web_client *client, PROMQL_API_KIND kind) {
    PROMQL_API_REQUEST request = {.client = client,
        .request = {.header = {NETDATA_PROMQL_ABI_VERSION, sizeof(netdata_promql_precise_request_t)}}};
    uint32_t status = engine_acquire(&request);
    netdata_promql_control_options_t caller;
    if(status == NETDATA_PROMQL_OK) status = netdata_promql_control_options_default(&caller);
    if(status == NETDATA_PROMQL_OK && client->timings.timeout_ut) {
        struct timeval now;
        now_monotonic_high_precision_timeval(&now);
        usec_t elapsed = dt_usec(&client->timings.tv_in, &now);
        usec_t remaining = elapsed < client->timings.timeout_ut ? client->timings.timeout_ut - elapsed : 0;
        caller.deadline_present = 1;
        caller.seconds = remaining / USEC_PER_SEC;
        caller.nanoseconds = (uint32_t)(remaining % USEC_PER_SEC) * 1000U;
    }
    if(status == NETDATA_PROMQL_OK) status = promql_operation_begin(&request.operation, request.engine,
        web_client_check_conn_tcp(client) || web_client_check_conn_unix(client) ? client->fd : -1, &caller);
    if(status != NETDATA_PROMQL_OK) promql_api_report_failure(&request, status, NULL, (netdata_promql_bytes_t){0});
    else {
        client->promql_operation_id = request.operation.id;
        if(client->promql_operation_hook)
            client->promql_operation_hook(client->promql_operation_hook_data, request.operation.id);
        request.gzip = client->response.zoutput && !web_client_check_conn_webrtc(client);
        request.context = (PROMQL_PROVIDER_CONTEXT){.scope = client->promql_scoped ? PROMQL_HOST_GUID : PROMQL_FLEET,
            .host = client->promql_scoped ? client->promql_host_guid : NULL, .backing_directory = request.directory};
        int64_t initial_limit;
        status = netdata_promql_api_parse_limit(request.engine, (netdata_promql_bytes_t){0}, request.operation.control,
            &initial_limit, &request.report);
        if(status != NETDATA_PROMQL_OK) promql_api_report_failure(&request, status, NULL, (netdata_promql_bytes_t){0});
        else if(client->mode != HTTP_REQUEST_MODE_GET &&
                (client->mode != HTTP_REQUEST_MODE_POST || kind == PROMQL_API_VALUES || kind == PROMQL_API_METADATA))
            request.failure = (PROMQL_API_ERROR){.code = HTTP_RESP_METHOD_NOT_ALLOWED, .type = "bad_data", .message = promql_bytes("method not allowed")};
        else {
            status = netdata_promql_report_accounted(request.report, prepare_form, &request, NULL);
            if(status != NETDATA_PROMQL_OK || !request.form) promql_api_report_failure(&request,
                request.error == ECANCELED ? NETDATA_PROMQL_CANCELLED : NETDATA_PROMQL_INTERNAL_ERROR, NULL, (netdata_promql_bytes_t){0});
            else if(kind == PROMQL_API_VALUES) {
                status = netdata_promql_report_accounted(request.report, prepare_label_name, &request, NULL);
                if(status != NETDATA_PROMQL_OK || request.error) promql_api_report_failure(&request,
                    request.error == ECANCELED ? NETDATA_PROMQL_CANCELLED : NETDATA_PROMQL_INTERNAL_ERROR, NULL, (netdata_promql_bytes_t){0});
            }
            if(!request.failure.code) {
                bool success = kind == PROMQL_API_QUERY || kind == PROMQL_API_RANGE ? query(&request, kind == PROMQL_API_RANGE) :
                    kind == PROMQL_API_METADATA ? promql_api_metadata(&request) : promql_api_discovery(&request, kind);
                if(!success && !request.failure.code) promql_api_report_failure(&request, NETDATA_PROMQL_INTERNAL_ERROR, NULL, (netdata_promql_bytes_t){0});
            }
        }
    }
    int code = request.failure.code ? request.failure.code : HTTP_RESP_OK;
    if(request.failure.code && request.report) {
        promql_body_release(request.body); request.body = NULL;
        netdata_promql_report_accounted(request.report, prepare_error, &request, NULL);
    }
    buffer_flush(client->response.data);
    client->response.data->content_type = CT_APPLICATION_JSON;
    buffer_no_cacheable(client->response.data);
    bool attached = request.body && !request.error && promql_body_is_sealed(request.body) && request.report &&
        promql_body_attach_report(request.body, request.report);
    if(attached) {
        request.report = NULL;
        attached = web_client_attach_promql_body(client, request.body);
        if(attached) request.body = NULL;
    }
    if(!attached) {
        code = request.error || request.body ? HTTP_RESP_INTERNAL_SERVER_ERROR :
            request.failure.code ? request.failure.code : HTTP_RESP_INTERNAL_SERVER_ERROR;
        // A bounded terminal response still works when response backing is unavailable.
        buffer_sprintf(client->response.data, "{\"status\":\"error\",\"errorType\":\"%s\",\"error\":\"%s\"}",
            code == HTTP_RESP_CLIENT_CLOSED_REQUEST ? "canceled" : "internal",
            request.error ? "PromQL response backing unavailable" : "PromQL response construction failed");
    }
    cleanup(&request);
    client->response.code = code;
    return code;
}

int api_v1_promql_query(RRDHOST *host __maybe_unused, struct web_client *client, char *query_string __maybe_unused) {
    return dispatch(client, PROMQL_API_QUERY);
}
int api_v1_promql_query_range(RRDHOST *host __maybe_unused, struct web_client *client, char *query_string __maybe_unused) {
    return dispatch(client, PROMQL_API_RANGE);
}
int api_v1_promql_labels(RRDHOST *host __maybe_unused, struct web_client *client, char *query_string __maybe_unused) {
    return dispatch(client, PROMQL_API_LABELS);
}
int api_v1_promql_label_values(RRDHOST *host __maybe_unused, struct web_client *client, char *query_string __maybe_unused) {
    return dispatch(client, PROMQL_API_VALUES);
}
int api_v1_promql_series(RRDHOST *host __maybe_unused, struct web_client *client, char *query_string __maybe_unused) {
    return dispatch(client, PROMQL_API_SERIES);
}
int api_v1_promql_metadata(RRDHOST *host __maybe_unused, struct web_client *client, char *query_string __maybe_unused) {
    return dispatch(client, PROMQL_API_METADATA);
}
