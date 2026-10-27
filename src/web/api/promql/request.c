// SPDX-License-Identifier: GPL-3.0-or-later

#include "request.h"

static const char *parameter_names[PROMQL_PARAM_COUNT] = {
    "query", "time", "timeout", "lookback_delta", "limit", "stats", "start", "end", "step", "match[]", "options",
    "metric", "limit_per_metric"
};

int promql_hex_digit(uint8_t byte) {
    return byte >= '0' && byte <= '9' ? byte - '0' : byte >= 'a' && byte <= 'f' ? byte - 'a' + 10 :
        byte >= 'A' && byte <= 'F' ? byte - 'A' + 10 : -1;
}

static bool decode_field(netdata_promql_bytes_t input, uint8_t *output, size_t *length,
                          PROMQL_FORM_ERROR *error, PROMQL_READ_FRAME *frame) {
    *length = 0;
    for(size_t i = 0; i < (size_t)input.len; i++) {
        if(!(i % PROMQL_BODY_WINDOW) && !promql_read_poll(frame)) return false;
        uint8_t byte = input.data[i];
        if(byte == '%') {
            if(input.len - i < 3 || promql_hex_digit(input.data[i + 1]) < 0 || promql_hex_digit(input.data[i + 2]) < 0) {
                if(!error->message) {
                    error->message = "invalid URL escape ";
                    error->escape_length = (size_t)MIN(UINT64_C(3), input.len - i);
                    memcpy(error->escape, input.data + i, error->escape_length);
                }
                return false;
            }
            byte = (uint8_t)(promql_hex_digit(input.data[i + 1]) * 16 + promql_hex_digit(input.data[i + 2]));
            i += 2;
        }
        else if(byte == '+') byte = ' ';
        if(output) output[*length] = byte;
        (*length)++;
    }
    return true;
}

static bool parse_fields(PROMQL_FORM *form, netdata_promql_bytes_t input, PROMQL_FORM_ERROR *form_error,
                          PROMQL_READ_FRAME *frame, int *error) {
    size_t position = 0;
    while(position < (size_t)input.len) {
        if(!promql_read_poll(frame)) { *error = ECANCELED; return false; }
        size_t end = position, separator = SIZE_MAX;
        bool semicolon = false;
        while(end < (size_t)input.len && input.data[end] != '&') {
            if(!(end % PROMQL_BODY_WINDOW) && !promql_read_poll(frame)) { *error = ECANCELED; return false; }
            if(input.data[end] == ';') semicolon = true;
            if(separator == SIZE_MAX && input.data[end] == '=') separator = end;
            end++;
        }
        if(end == position) { position = end + 1; continue; }
        if(semicolon) {
            *form_error = (PROMQL_FORM_ERROR){.message = "invalid semicolon separator in query"};
            position = end + 1; continue;
        }
        if(separator == SIZE_MAX) separator = end;
        netdata_promql_bytes_t key = {input.data + position, separator - position};
        netdata_promql_bytes_t value = {input.data + separator + (separator < end), end - separator - (separator < end)};
        size_t key_length = 0, value_length = 0;
        if(!decode_field(key, NULL, &key_length, form_error, frame) ||
           !decode_field(value, NULL, &value_length, form_error, frame)) {
            if(frame->interrupted != NETDATA_PROMQL_OK) { *error = ECANCELED; return false; }
            position = end + 1; continue;
        }
        uint8_t name[32];
        PROMQL_PARAMETER parameter = PROMQL_PARAM_COUNT;
        if(key_length < sizeof(name)) {
            if(!decode_field(key, name, &key_length, form_error, frame)) { *error = ECANCELED; return false; }
            for(size_t i = 0; i < PROMQL_PARAM_COUNT; i++)
                if(strlen(parameter_names[i]) == key_length && !memcmp(name, parameter_names[i], key_length)) {
                    parameter = (PROMQL_PARAMETER)i; break;
                }
        }
        if(parameter != PROMQL_PARAM_COUNT) {
            if(form->count == form->capacity) {
                size_t capacity = form->capacity ? form->capacity * 2 : 16;
                if(capacity < form->capacity || capacity > SIZE_MAX / sizeof(*form->fields)) { *error = EOVERFLOW; return false; }
                void *fields = form->fields ? promql_resize(frame->account, form->fields, capacity * sizeof(*form->fields)) :
                    promql_malloc(frame->account, NETDATA_PROMQL_MEMORY_WORKSPACE, capacity * sizeof(*form->fields));
                if(!fields) { *error = ENOMEM; return false; }
                form->fields = fields; form->capacity = capacity;
            }
            uint8_t *decoded = promql_malloc(frame->account, NETDATA_PROMQL_MEMORY_WORKSPACE, value_length + 1);
            if(!decoded) { *error = ENOMEM; return false; }
            if(!decode_field(value, decoded, &value_length, form_error, frame)) {
                promql_free(decoded); *error = ECANCELED; return false;
            }
            form->fields[form->count++] = (PROMQL_FORM_FIELD){parameter, {decoded, value_length}};
        }
        position = end + 1;
    }
    return true;
}

static bool go_space(uint32_t rune) {
    return (rune >= 9 && rune <= 13) || rune == 32 || rune == 0x85 || rune == 0xa0 || rune == 0x1680 ||
        (rune >= 0x2000 && rune <= 0x200a) || rune == 0x2028 || rune == 0x2029 || rune == 0x202f ||
        rune == 0x205f || rune == 0x3000;
}

static netdata_promql_bytes_t trim_left(netdata_promql_bytes_t value) {
    while(value.len) {
        uint32_t rune;
        size_t width = promql_utf8_rune(value.data, (size_t)value.len, &rune);
        if(!width || !go_space(rune)) break;
        value.data += width; value.len -= width;
    }
    return value;
}

static netdata_promql_bytes_t trim_space(netdata_promql_bytes_t value) {
    value = trim_left(value);
    size_t keep = 0;
    for(size_t i = 0; i < (size_t)value.len;) {
        uint32_t rune;
        size_t width = promql_utf8_rune(value.data + i, (size_t)value.len - i, &rune);
        if(!width) { width = 1; rune = 0xfffd; }
        i += width;
        if(!go_space(rune)) keep = i;
    }
    value.len = keep;
    return value;
}

static bool tspecial(uint8_t byte) { return byte && byte < 0x80 && strchr("()<>@,;:\\\"/[]?=", byte) != NULL; }
static bool token_byte(uint8_t byte) { return byte > 32 && byte < 127 && !tspecial(byte); }
static size_t token_length(netdata_promql_bytes_t value) {
    size_t length = 0;
    while(length < (size_t)value.len && token_byte(value.data[length])) length++;
    return length;
}

static size_t media_byte(netdata_promql_bytes_t value, uint8_t *byte) {
    uint32_t rune;
    size_t width = promql_utf8_rune(value.data, (size_t)value.len, &rune);
    if(!width) return 0;
    // These are the pinned Unicode lowercase mappings whose non-ASCII inputs become ASCII.
    if(rune == 0x130) rune = 'i';
    else if(rune == 0x212a) rune = 'k';
    if(rune >= 0x80) return 0;
    *byte = (uint8_t)(rune >= 'A' && rune <= 'Z' ? rune + 'a' - 'A' : rune);
    return width;
}

static size_t media_token_length(netdata_promql_bytes_t value) {
    size_t length = 0;
    while(length < (size_t)value.len) {
        uint8_t byte;
        size_t width = media_byte((netdata_promql_bytes_t){value.data + length, value.len - length}, &byte);
        if(!width || !token_byte(byte)) break;
        length += width;
    }
    return length;
}

static bool media_equals(netdata_promql_bytes_t value, const char *expected) {
    size_t position = 0, index = 0;
    while(position < (size_t)value.len && expected[index]) {
        uint8_t byte;
        size_t width = media_byte((netdata_promql_bytes_t){value.data + position, value.len - position}, &byte);
        if(!width || byte != (uint8_t)expected[index++]) return false;
        position += width;
    }
    return position == value.len && !expected[index];
}

typedef struct mime_parameter {
    struct mime_parameter *next;
    netdata_promql_bytes_t name, value;
    uint8_t *owned_value;
} MIME_PARAMETER;

static bool mime_form(const char *content_type, PROMQL_FORM_ERROR *form_error, PROMQL_READ_FRAME *frame, int *error) {
    if(!content_type || !*content_type) return false;
    netdata_promql_bytes_t value = promql_bytes(content_type);
    const uint8_t *semicolon = memchr(value.data, ';', (size_t)value.len);
    size_t base_length = semicolon ? (size_t)(semicolon - value.data) : (size_t)value.len;
    netdata_promql_bytes_t base = trim_space((netdata_promql_bytes_t){value.data, base_length});
    size_t type = media_token_length(base);
    const char *failure = !type ? "mime: no media type" : NULL;
    if(!failure && type < base.len) {
        if(base.data[type] != '/') failure = "mime: expected slash after first token";
        else {
            netdata_promql_bytes_t subtype = {base.data + type + 1, base.len - type - 1};
            size_t length = media_token_length(subtype);
            failure = !length ? "mime: expected token after slash" : length < subtype.len ?
                "mime: unexpected content after media subtype" : NULL;
        }
    }
    if(failure) { form_error->message = failure; return false; }
    const char *expected = "application/x-www-form-urlencoded";
    bool form = media_equals(base, expected);
    value.data += base_length; value.len -= base_length;
    MIME_PARAMETER *parameters = NULL;
    while(value.len && !failure && !*error) {
        if(!promql_read_poll(frame)) { *error = ECANCELED; break; }
        value = trim_left(value);
        if(!value.len) break;
        if(value.data[0] != ';') { failure = "mime: invalid media parameter"; break; }
        value.data++; value.len--; value = trim_left(value);
        if(!value.len) break;
        size_t name_length = token_length(value);
        if(!name_length) { failure = "mime: invalid media parameter"; break; }
        netdata_promql_bytes_t name = {value.data, name_length};
        value.data += name_length; value.len -= name_length; value = trim_left(value);
        if(!value.len || value.data[0] != '=') { failure = "mime: invalid media parameter"; break; }
        value.data++; value.len--; value = trim_left(value);
        size_t length = value.len && value.data[0] == '"' ? 0 : token_length(value), consumed = length;
        uint8_t *decoded = NULL;
        if(value.len && value.data[0] == '"') {
            size_t i = 1;
            for(; i < (size_t)value.len && value.data[i] != '"'; i++) {
                if(!(i % PROMQL_BODY_WINDOW) && !promql_read_poll(frame)) { *error = ECANCELED; break; }
                uint8_t byte = value.data[i];
                if(byte == '\\' && i + 1 < value.len && tspecial(value.data[i + 1])) i++;
                else if(byte == '\r' || byte == '\n') break;
                length++;
            }
            if(*error || i == value.len || value.data[i] != '"') {
                failure = "mime: invalid media parameter"; break;
            }
            consumed = i + 1;
            decoded = promql_malloc(frame->account, NETDATA_PROMQL_MEMORY_WORKSPACE, length + 1);
            if(!decoded) { *error = ENOMEM; break; }
            size_t copied = 0;
            for(size_t j = 1; j + 1 < consumed; j++) {
                if(!(j % PROMQL_BODY_WINDOW) && !promql_read_poll(frame)) { *error = ECANCELED; break; }
                uint8_t byte = value.data[j];
                if(byte == '\\' && j + 2 < consumed && tspecial(value.data[j + 1])) byte = value.data[++j];
                decoded[copied++] = byte;
            }
            if(*error) { promql_free(decoded); break; }
        }
        else if(!length) { failure = "mime: invalid media parameter"; break; }
        netdata_promql_bytes_t parameter_value = {decoded ? decoded : value.data, length};
        for(MIME_PARAMETER *parameter = parameters; parameter; parameter = parameter->next) {
            if(!promql_read_poll(frame)) { *error = ECANCELED; break; }
            if(parameter->name.len == name.len && !strncasecmp((const char *)parameter->name.data,
                (const char *)name.data, (size_t)name.len) && (parameter->value.len != parameter_value.len ||
                memcmp(parameter->value.data, parameter_value.data, (size_t)parameter_value.len))) {
                failure = "mime: duplicate parameter name"; form = false; break;
            }
        }
        MIME_PARAMETER *parameter = failure || *error ? NULL :
            promql_malloc(frame->account, NETDATA_PROMQL_MEMORY_WORKSPACE, sizeof(*parameter));
        if(!parameter) { promql_free(decoded); if(!failure && !*error) *error = ENOMEM; break; }
        // Quoted values own their decoded buffer; token values borrow the native header.
        parameter->name = name; parameter->value = parameter_value; parameter->owned_value = decoded;
        parameter->next = parameters;
        parameters = parameter;
        value.data += consumed; value.len -= consumed;
    }
    while(parameters) {
        MIME_PARAMETER *next = parameters->next;
        promql_free(parameters->owned_value);
        promql_free(parameters); parameters = next;
    }
    if(failure) form_error->message = failure;
    return form;
}

void promql_form_destroy(PROMQL_FORM *form) {
    if(!form) return;
    for(size_t i = 0; i < form->count; i++) promql_free((void *)form->fields[i].value.data);
    promql_free(form->fields);
    if(form->input_view) netdata_promql_allocation_release(form->input_view);
    promql_free(form);
}

PROMQL_FORM *promql_form_create(netdata_promql_bytes_t target, netdata_promql_bytes_t payload,
                                const char *content_type, bool post, PROMQL_READ_FRAME *frame, int *error) {
    *error = 0;
    if(target.len > SIZE_MAX || payload.len > SIZE_MAX || (target.len && !target.data) || (payload.len && !payload.data) ||
       payload.len > UINT64_MAX - target.len) { *error = EINVAL; return NULL; }
    PROMQL_FORM *form = promql_malloc(frame->account, NETDATA_PROMQL_MEMORY_WORKSPACE, sizeof(*form));
    if(!form) { *error = ENOMEM; return NULL; }
    uint64_t bytes = target.len + payload.len, header = content_type ? strlen(content_type) : 0;
    if(header > UINT64_MAX - bytes || netdata_promql_allocation_acquire(frame->account,
        NETDATA_PROMQL_MEMORY_BORROWED_VIEW, bytes + header, &form->input_view) != NETDATA_PROMQL_OK) {
        *error = ENOMEM; promql_form_destroy(form); return NULL;
    }
    PROMQL_FORM_ERROR mime_error = {0}, body_error = {0}, query_error = {0};
    bool parse_body = post && mime_form(content_type, &mime_error, frame, error);
    const uint8_t *question = target.len ? memchr(target.data, '?', (size_t)target.len) : NULL;
    netdata_promql_bytes_t query = question ? (netdata_promql_bytes_t){question + 1,
        target.len - (uint64_t)(question + 1 - target.data)} : (netdata_promql_bytes_t){0};
    if(*error || (parse_body && !parse_fields(form, payload, &body_error, frame, error)) ||
       !parse_fields(form, query, &query_error, frame, error)) { promql_form_destroy(form); return NULL; }
    // Go keeps the MIME error even when decoding the accepted form body also fails.
    form->error = mime_error.message ? mime_error : body_error.message ? body_error : query_error;
    return form;
}

netdata_promql_bytes_t promql_form_value(const PROMQL_FORM *form, PROMQL_PARAMETER parameter) {
    for(size_t i = 0; i < form->count; i++) if(form->fields[i].parameter == parameter) return form->fields[i].value;
    return (netdata_promql_bytes_t){0};
}

size_t promql_form_count(const PROMQL_FORM *form, PROMQL_PARAMETER parameter) {
    size_t count = 0;
    for(size_t i = 0; i < form->count; i++) count += form->fields[i].parameter == parameter;
    return count;
}
