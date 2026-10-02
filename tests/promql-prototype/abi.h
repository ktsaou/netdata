/* SPDX-License-Identifier: GPL-3.0-or-later */
#ifndef PROMQL_PROTOTYPE_ABI_H
#define PROMQL_PROTOTYPE_ABI_H
#include <stddef.h>
#include <stdint.h>
#ifdef __cplusplus
extern "C" {
#endif
typedef struct {
    const char *name, *value;
} PPLabel;
typedef struct {
    int64_t t;
    double v;
} PPPoint;
typedef struct {
    const PPLabel *labels;
    size_t labels_len;
    const PPPoint *points;
    size_t points_len;
} PPSeries;
typedef struct {
    const PPSeries *series;
    size_t series_len;
} PPData;
typedef struct {
    const PPData *data;
    const char *query;
    int64_t time_ms, lookback_ms;
    uint64_t work_limit;
    int (*cancelled)(void *);
    void *cancel_context;
    int inject_failure;
} PPRequest;
enum { PP_SCALAR = 1, PP_VECTOR = 2, PP_MATRIX = 3, PP_ERROR = 4 };
typedef struct {
    int kind;
    const PPSeries *rows;
    size_t rows_len;
    const char *error;
    uint64_t work;
    void *owner;
} PPResult;
/* Result views are read-only and owned by the engine until pp_free().
 * Allocation fallback errors may be static; pp_free() handles both cases. */
PPResult *pp_eval(const PPRequest *);
void pp_free(PPResult *);
/* Shared regex facility: PCRE2 subset, an experimental compatibility limitation. */
int pp_match(const char *pattern, const char *subject);
/* 1: match with allocated output; 0: no match; -1: failure. */
int pp_replace(const char *pattern, const char *subject, const char *replacement, char **output);
void pp_string_free(char *);
#ifdef __cplusplus
}
#endif
#endif
