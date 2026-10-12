// SPDX-License-Identifier: GPL-3.0-or-later

#ifndef NETDATA_PROMQL_NOTES_H
#define NETDATA_PROMQL_NOTES_H

#include "json.h"

typedef struct { PROMQL_BODY *bodies[2]; uint64_t counts[2]; } PROMQL_NOTES;

void promql_notes_destroy(PROMQL_NOTES *notes);
bool promql_notes_append(PROMQL_NOTES *notes, unsigned severity, netdata_promql_bytes_t message,
                         const char *directory, PROMQL_READ_FRAME *frame, int *error);
bool promql_notes_stats(PROMQL_NOTES *notes, netdata_promql_bytes_t value, const char *directory,
                        PROMQL_READ_FRAME *frame, int *error);
bool promql_notes_write(PROMQL_NOTES *notes, PROMQL_BODY *body, uint8_t window[PROMQL_BODY_WINDOW],
                        PROMQL_READ_FRAME *frame, int *error);

#endif
