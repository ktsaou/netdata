// SPDX-License-Identifier: GPL-3.0-or-later

#include "notes.h"

void promql_notes_destroy(PROMQL_NOTES *notes) {
    for(size_t i = 0; i < 2; i++) { promql_body_release(notes->bodies[i]); notes->bodies[i] = NULL; }
}

static PROMQL_BODY *note_begin(PROMQL_NOTES *notes, unsigned severity, const char *directory,
                                PROMQL_READ_FRAME *frame, int *error) {
    if(severity > 1 || notes->counts[severity] == UINT64_MAX) { *error = EOVERFLOW; return NULL; }
    uint64_t count = notes->counts[severity]++;
    if(count >= 10) return NULL;
    if(!notes->bodies[severity])
        notes->bodies[severity] = promql_body_create(frame->account, directory, NETDATA_PROMQL_MEMORY_RESULT, error);
    PROMQL_BODY *body = notes->bodies[severity];
    if(!body || (count && !promql_json_text(body, ",", frame, error))) return NULL;
    return body;
}

bool promql_notes_append(PROMQL_NOTES *notes, unsigned severity, netdata_promql_bytes_t message,
                         const char *directory, PROMQL_READ_FRAME *frame, int *error) {
    PROMQL_BODY *body = note_begin(notes, severity, directory, frame, error);
    return body ? promql_json_string(body, message, PROMQL_JSON_ENVELOPE, frame, error) : !*error;
}

bool promql_notes_stats(PROMQL_NOTES *notes, netdata_promql_bytes_t value, const char *directory,
                        PROMQL_READ_FRAME *frame, int *error) {
    if(!value.len || (value.len == 4 && !memcmp(value.data, "true", 4)) ||
       (value.len == 3 && !memcmp(value.data, "all", 3))) return true;
    PROMQL_BODY *body = note_begin(notes, NETDATA_PROMQL_WARNING, directory, frame, error);
    if(!body) return !*error;
    return promql_json_text(body, "\"value ", frame, error) && promql_json_go_quote(body, value, frame, error) &&
        promql_json_escape(body, promql_bytes(" for parameter \"stats\" is deprecated and will be rejected in the next major release, use \"true\" or \"all\""),
                            PROMQL_JSON_ENVELOPE, frame, error) && promql_json_text(body, "\"", frame, error);
}

bool promql_notes_write(PROMQL_NOTES *notes, PROMQL_BODY *body, uint8_t window[PROMQL_BODY_WINDOW],
                        PROMQL_READ_FRAME *frame, int *error) {
    for(unsigned severity = 0; severity < 2; severity++) {
        uint64_t count = notes->counts[severity];
        if(!count) continue;
        if(!promql_json_text(body, severity ? ",\"warnings\":[" : ",\"infos\":[", frame, error) ||
           !promql_body_copy(notes->bodies[severity], 0, promql_body_length(notes->bodies[severity]), body, window, frame, error)) return false;
        if(count > 10) {
            char message[96];
            snprintfz(message, sizeof(message) - 1, "%"PRIu64" more %s annotations omitted", count - 10, severity ? "warning" : "info");
            if(!promql_json_text(body, ",", frame, error) ||
               !promql_json_string(body, promql_bytes(message), PROMQL_JSON_ENVELOPE, frame, error)) return false;
        }
        if(!promql_json_text(body, "]", frame, error)) return false;
    }
    return true;
}
