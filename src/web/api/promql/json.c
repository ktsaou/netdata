// SPDX-License-Identifier: GPL-3.0-or-later

#include "json.h"

bool promql_json_text(PROMQL_BODY *body, const char *text, PROMQL_READ_FRAME *frame, int *error) {
    return promql_body_append(body, text, strlen(text), frame, error);
}

size_t promql_utf8_rune(const uint8_t *bytes, size_t length, uint32_t *rune) {
    if(!length) return 0;
    unsigned leading = bytes[0];
    if(leading < 0x80) { *rune = leading; return 1; }
    size_t width = leading >= 0xf0 && leading <= 0xf4 ? 4 :
        leading >= 0xe0 && leading <= 0xef ? 3 : leading >= 0xc2 && leading <= 0xdf ? 2 : 0;
    if(!width || width > length) return 0;
    for(size_t i = 1; i < width; i++) if(bytes[i] < 0x80 || bytes[i] > 0xbf) return 0;
    if((leading == 0xe0 && bytes[1] < 0xa0) || (leading == 0xed && bytes[1] >= 0xa0) ||
       (leading == 0xf0 && bytes[1] < 0x90) || (leading == 0xf4 && bytes[1] >= 0x90)) return 0;
    uint32_t value = leading & (0x7fU >> width);
    for(size_t i = 1; i < width; i++) value = (value << 6) | (bytes[i] & 0x3fU);
    *rune = value;
    return width;
}

bool promql_json_escape(PROMQL_BODY *body, netdata_promql_bytes_t bytes, PROMQL_JSON_MODE mode,
                         PROMQL_READ_FRAME *frame, int *error) {
    if(bytes.len > SIZE_MAX || (bytes.len && !bytes.data)) { *error = EOVERFLOW; return false; }
    const uint8_t *source = bytes.data;
    size_t length = (size_t)bytes.len, position = 0, run = 0;
    while(position < length) {
        uint8_t byte = source[position];
        const char *escape = NULL;
        char control[7];
        size_t width = 1;
        if(byte == '"') escape = "\\\"";
        else if(byte == '\\') escape = "\\\\";
        else if(byte == '/' && mode == PROMQL_JSON_NATIVE) escape = "\\/";
        else if(byte == '\n') escape = "\\n";
        else if(byte == '\r') escape = "\\r";
        else if(byte == '\t') escape = "\\t";
        else if(byte == '\b' && (mode == PROMQL_JSON_SCALAR || mode == PROMQL_JSON_NATIVE)) escape = "\\b";
        else if(byte == '\f' && (mode == PROMQL_JSON_SCALAR || mode == PROMQL_JSON_NATIVE)) escape = "\\f";
        else if(byte < 0x20 || ((mode == PROMQL_JSON_ENVELOPE || mode == PROMQL_JSON_SCALAR) &&
                              (byte == '<' || byte == '>' || byte == '&'))) {
            static const char hex[] = "0123456789abcdef";
            memcpy(control, "\\u00", 4);
            control[4] = hex[byte >> 4];
            control[5] = hex[byte & 15];
            control[6] = 0;
            escape = control;
        }
        else if(byte >= 0x80 && (mode == PROMQL_JSON_ENVELOPE || mode == PROMQL_JSON_SCALAR)) {
            uint32_t rune;
            width = promql_utf8_rune(source + position, length - position, &rune);
            if(!width) { width = 1; escape = "\\ufffd"; }
            else if(width == 3 && byte == 0xe2 && source[position + 1] == 0x80 &&
                    (source[position + 2] == 0xa8 || source[position + 2] == 0xa9))
                escape = source[position + 2] == 0xa8 ? "\\u2028" : "\\u2029";
        }
        if(escape) {
            if(!promql_body_append(body, source + run, position - run, frame, error) ||
               !promql_json_text(body, escape, frame, error)) return false;
            position += width;
            run = position;
        }
        else position += width;
        // Long unescaped inputs must remain cooperative without allocating their escaped form.
        if(position - run >= PROMQL_BODY_WINDOW) {
            if(!promql_body_append(body, source + run, position - run, frame, error)) return false;
            run = position;
        }
    }
    return promql_body_append(body, length ? source + run : NULL, length - run, frame, error);
}

bool promql_json_string(PROMQL_BODY *body, netdata_promql_bytes_t bytes, PROMQL_JSON_MODE mode,
                         PROMQL_READ_FRAME *frame, int *error) {
    return promql_json_text(body, "\"", frame, error) && promql_json_escape(body, bytes, mode, frame, error) &&
        promql_json_text(body, "\"", frame, error);
}

bool promql_json_go_quote(PROMQL_BODY *body, netdata_promql_bytes_t bytes, PROMQL_READ_FRAME *frame, int *error) {
    if(bytes.len > SIZE_MAX || (bytes.len && !bytes.data)) { *error = EINVAL; return false; }
    if(!promql_json_escape(body, promql_bytes("\""), PROMQL_JSON_ENVELOPE, frame, error)) return false;
    for(size_t position = 0; position < (size_t)bytes.len;) {
        uint32_t rune, printable = 0;
        size_t width = promql_utf8_rune(bytes.data + position, (size_t)bytes.len - position, &rune);
        char escaped[12];
        const char *text = escaped;
        size_t length;
        if(!width) {
            width = 1;
            length = (size_t)snprintf(escaped, sizeof(escaped), "\\x%02x", bytes.data[position]);
        }
        else if(rune == '"' || rune == '\\') {
            escaped[0] = '\\'; escaped[1] = (char)rune; length = 2;
        }
        else if(netdata_promql_api_rune_is_print(rune, &printable) != NETDATA_PROMQL_OK) { *error = EINVAL; return false; }
        else if(printable) { text = (const char *)bytes.data + position; length = width; }
        else {
            const char *special = rune == 7 ? "\\a" : rune == 8 ? "\\b" : rune == 12 ? "\\f" :
                rune == 10 ? "\\n" : rune == 13 ? "\\r" : rune == 9 ? "\\t" : rune == 11 ? "\\v" : NULL;
            if(special) { text = special; length = 2; }
            else length = (size_t)snprintf(escaped, sizeof(escaped), rune < 32 || rune == 127 ? "\\x%02x" :
                rune <= 0xffff ? "\\u%04x" : "\\U%08x", rune);
        }
        if(!promql_json_escape(body, (netdata_promql_bytes_t){(const uint8_t *)text, length},
                                PROMQL_JSON_ENVELOPE, frame, error)) return false;
        position += width;
    }
    return promql_json_escape(body, promql_bytes("\""), PROMQL_JSON_ENVELOPE, frame, error);
}

bool promql_json_float(PROMQL_BODY *body, uint64_t bits, bool scalar, PROMQL_READ_FRAME *frame, int *error) {
    netdata_promql_numeric_text_t text;
    uint32_t status = scalar ? netdata_promql_api_scalar_float_text(bits, &text) : netdata_promql_api_float_text(bits, &text);
    if(status != NETDATA_PROMQL_OK) { *error = EINVAL; return false; }
    return promql_json_text(body, "\"", frame, error) &&
        promql_body_append(body, text.bytes, text.len, frame, error) && promql_json_text(body, "\"", frame, error);
}

bool promql_json_time(PROMQL_BODY *body, int64_t milliseconds, bool scalar, PROMQL_READ_FRAME *frame, int *error) {
    netdata_promql_numeric_text_t text;
    uint32_t status = scalar ? netdata_promql_api_scalar_timestamp_text(milliseconds, &text) :
        netdata_promql_api_timestamp_text(milliseconds, &text);
    if(status != NETDATA_PROMQL_OK) { *error = EINVAL; return false; }
    return promql_body_append(body, text.bytes, text.len, frame, error);
}

bool promql_json_unsigned(PROMQL_BODY *body, uint64_t number, PROMQL_READ_FRAME *frame, int *error) {
    char text[21];
    int count = snprintf(text, sizeof(text), "%"PRIu64, number);
    if(count <= 0 || (size_t)count >= sizeof(text)) { *error = EOVERFLOW; return false; }
    return promql_body_append(body, text, (size_t)count, frame, error);
}

bool promql_json_signed(PROMQL_BODY *body, int64_t number, PROMQL_READ_FRAME *frame, int *error) {
    char text[21];
    int count = snprintf(text, sizeof(text), "%"PRId64, number);
    if(count <= 0 || (size_t)count >= sizeof(text)) { *error = EOVERFLOW; return false; }
    return promql_body_append(body, text, (size_t)count, frame, error);
}

bool promql_json_labels(PROMQL_BODY *body, netdata_promql_labels_t labels, PROMQL_JSON_MODE mode,
                         PROMQL_READ_FRAME *frame, int *error) {
    if(labels.len > SIZE_MAX / sizeof(*labels.data) || (labels.len && !labels.data)) { *error = EOVERFLOW; return false; }
    if(!promql_json_text(body, "{", frame, error)) return false;
    for(uint64_t i = 0; i < labels.len; i++) {
        if((i && !promql_json_text(body, ",", frame, error)) ||
           !promql_json_string(body, labels.data[i].name, mode, frame, error) ||
           !promql_json_text(body, ":", frame, error) ||
           !promql_json_string(body, labels.data[i].value, mode, frame, error)) return false;
    }
    return promql_json_text(body, "}", frame, error);
}
