/* SPDX-License-Identifier: GPL-3.0-or-later */
#define PCRE2_CODE_UNIT_WIDTH 8
#include <pcre2.h>
#include <stdlib.h>
#include <string.h>
#include <ctype.h>
#include <limits.h>
#include "abi.h"
static pcre2_code *compile(const char *pattern)
{
    int err;
    PCRE2_SIZE pos;
    size_t n = strlen(pattern);
    if (n > 4096)
        return NULL;
    char *anchored = malloc(n + 9);
    if (!anchored)
        return NULL;
    memcpy(anchored, "\\A(?:", 5);
    memcpy(anchored + 5, pattern, n);
    memcpy(anchored + 5 + n, ")\\z", 4);
    pcre2_code *code =
        pcre2_compile((PCRE2_SPTR)anchored, PCRE2_ZERO_TERMINATED, PCRE2_DOTALL | PCRE2_UTF, &err, &pos, NULL);
    free(anchored);
    return code;
}
static int execute(pcre2_code *code, const char *subject, pcre2_match_data *md)
{
    if (!md || strlen(subject) > 1048576)
        return -1;
    pcre2_match_context *ctx = pcre2_match_context_create(NULL);
    if (!ctx)
        return -1;
    pcre2_set_match_limit(ctx, 100000);
    pcre2_set_depth_limit(ctx, 1000);
    int rc = pcre2_match(code, (PCRE2_SPTR)subject, strlen(subject), 0, 0, md, ctx);
    pcre2_match_context_free(ctx);
    return rc == PCRE2_ERROR_NOMATCH ? 0 : rc < 0 ? -1 : rc;
}
int pp_match(const char *pattern, const char *subject)
{
    pcre2_code *code = compile(pattern);
    if (!code)
        return -1;
    pcre2_match_data *md = pcre2_match_data_create_from_pattern(code, NULL);
    int rc = execute(code, subject, md);
    pcre2_match_data_free(md);
    pcre2_code_free(code);
    return rc < 0 ? -1 : rc > 0;
}
static int append(char **out, size_t *used, size_t *capacity, const char *s, size_t n)
{
    if (n > 1048576 - *used)
        return 0;
    if (*used + n + 1 > *capacity) {
        size_t cap = *capacity;
        while (cap < *used + n + 1)
            cap *= 2;
        char *p = realloc(*out, cap);
        if (!p)
            return 0;
        *out = p;
        *capacity = cap;
    }
    memcpy(*out + *used, s, n);
    *used += n;
    return 1;
}
int pp_replace(const char *pattern, const char *subject, const char *replacement, char **output)
{
    *output = NULL;
    if (strlen(replacement) > 1048576)
        return -1;
    pcre2_code *code = compile(pattern);
    if (!code)
        return -1;
    pcre2_match_data *md = pcre2_match_data_create_from_pattern(code, NULL);
    if (!md) {
        pcre2_code_free(code);
        return -1;
    }
    int rc = execute(code, subject, md);
    if (rc <= 0) {
        pcre2_match_data_free(md);
        pcre2_code_free(code);
        return rc;
    }
    PCRE2_SIZE *ov = pcre2_get_ovector_pointer(md);
    size_t cap = 64, z = 0;
    char *out = malloc(cap);
    int ok = out != NULL;
    for (size_t i = 0; ok && replacement[i];) {
        if (replacement[i] != '$') {
            ok = append(&out, &z, &cap, replacement + i, 1);
            i++;
            continue;
        }
        size_t dollar = i++;
        if (replacement[i] == '$') {
            ok = append(&out, &z, &cap, "$", 1);
            i++;
            continue;
        }
        int brace = replacement[i] == '{';
        if (brace)
            i++;
        size_t first = i;
        while (isalnum((unsigned char)replacement[i]) || replacement[i] == '_')
            i++;
        size_t len = i - first;
        int capture = -1;
        if (!len || (brace && replacement[i] != '}')) {
            ok = append(&out, &z, &cap, "$", 1);
            i = dollar + 1;
            continue;
        }
        if (len < 128) {
            char name[128];
            memcpy(name, replacement + first, len);
            name[len] = 0;
            char *end;
            long k = strtol(name, &end, 10);
            if (!*end && !(len > 1 && name[0] == '0') && k >= 0 && k <= INT_MAX)
                capture = (int)k;
            else
                capture = pcre2_substring_number_from_name(code, (PCRE2_SPTR)name);
        }
        if (brace)
            i++;
        if (capture >= 0 && capture < rc && ov[2 * capture] != PCRE2_UNSET) {
            size_t count = ov[2 * capture + 1] - ov[2 * capture];
            ok = append(&out, &z, &cap, subject + ov[2 * capture], count);
        }
    }
    pcre2_match_data_free(md);
    pcre2_code_free(code);
    if (!ok) {
        free(out);
        return -1;
    }
    out[z] = 0;
    *output = out;
    return 1;
}
void pp_string_free(char *s)
{
    free(s);
}
