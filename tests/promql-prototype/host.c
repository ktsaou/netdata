/* SPDX-License-Identifier: GPL-3.0-or-later */
#define _POSIX_C_SOURCE 200809L
#include "abi.h"
#include <json-c/json.h>
#include <inttypes.h>
#include <math.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <time.h>
#include <sys/resource.h>
static void fail(const char *s)
{
    fprintf(stderr, "prototype host: %s\n", s);
    exit(2);
}
static json_object *field(json_object *o, const char *name)
{
    json_object *v = NULL;
    if (!json_object_object_get_ex(o, name, &v))
        fail(name);
    return v;
}
static int64_t integer(json_object *o)
{
    if (!json_object_is_type(o, json_type_int))
        fail("integer required");
    return json_object_get_int64(o);
}
static double number(json_object *o)
{
    if (json_object_is_type(o, json_type_double) || json_object_is_type(o, json_type_int))
        return json_object_get_double(o);
    if (json_object_is_type(o, json_type_string)) {
        const char *s = json_object_get_string(o);
        if (!strcmp(s, "NaN"))
            return NAN;
        if (!strcmp(s, "+Inf"))
            return INFINITY;
        if (!strcmp(s, "-Inf"))
            return -INFINITY;
    }
    fail("number required");
    return 0;
}
static json_object *value(double x)
{
    char b[64];
    if (isnan(x))
        strcpy(b, "NaN");
    else if (isinf(x))
        strcpy(b, signbit(x) ? "-Inf" : "+Inf");
    else
        snprintf(b, sizeof b, "%.17g", x);
    return json_object_new_string(b);
}
static json_object *value_bits(double x)
{
    uint64_t bits;
    char text[17];
    memcpy(&bits, &x, sizeof bits);
    snprintf(text, sizeof text, "%016" PRIx64, bits);
    return json_object_new_string(text);
}
static json_object *encode(const PPResult *r)
{
    json_object *out = json_object_new_object();
    json_object_object_add(out, "kind", json_object_new_int(r->kind));
    json_object_object_add(out, "work", json_object_new_uint64(r->work));
    if (r->error)
        json_object_object_add(out, "error", json_object_new_string(r->error));
    json_object *rows = json_object_new_array();
    for (size_t i = 0; i < r->rows_len; i++) {
        const PPSeries *s = r->rows + i;
        json_object *row = json_object_new_object(), *labels = json_object_new_object(),
                    *points = json_object_new_array(), *bits = json_object_new_array();
        for (size_t j = 0; j < s->labels_len; j++)
            json_object_object_add(labels, s->labels[j].name, json_object_new_string(s->labels[j].value));
        for (size_t j = 0; j < s->points_len; j++) {
            json_object *p = json_object_new_array();
            json_object_array_add(p, json_object_new_int64(s->points[j].t));
            json_object_array_add(p, value(s->points[j].v));
            json_object_array_add(points, p);
            json_object_array_add(bits, value_bits(s->points[j].v));
        }
        json_object_object_add(row, "labels", labels);
        json_object_object_add(row, "points", points);
        json_object_object_add(row, "point_bits", bits);
        json_object_array_add(rows, row);
    }
    json_object_object_add(out, "rows", rows);
    return out;
}
typedef struct {
    uint64_t calls, at;
} Cancel;
static int cancelled(void *ctx)
{
    Cancel *c = ctx;
    return c->at && ++c->calls >= c->at;
}
static double monotime(void)
{
    struct timespec t;
    clock_gettime(CLOCK_MONOTONIC, &t);
    return t.tv_sec + t.tv_nsec / 1e9;
}
static json_object *execute(PPRequest *req, int64_t start, int64_t end, int64_t step)
{
    if (!step) {
        PPResult *out = pp_eval(req);
        if (!out)
            fail("null result");
        json_object *v = encode(out);
        pp_free(out);
        return v;
    }
    if (step < 1 || end < start || (end - start) / step > 10000)
        fail("range bounds");
    json_object *matrix = json_object_new_object(), *rows = json_object_new_array();
    uint64_t work = 0;
    json_object_object_add(matrix, "kind", json_object_new_int(PP_MATRIX));
    json_object_object_add(matrix, "rows", rows);
    for (int64_t t = start; t <= end; t += step) {
        req->time_ms = t;
        PPResult *out = pp_eval(req);
        if (!out)
            fail("null result");
        work += out->work;
        if (out->kind == PP_ERROR) {
            json_object_put(matrix);
            matrix = encode(out);
            pp_free(out);
            return matrix;
        }
        if (out->kind != PP_VECTOR && out->kind != PP_SCALAR)
            fail("range expression must be vector or scalar");
        json_object *encoded = encode(out);
        pp_free(out);
        json_object *incoming = field(encoded, "rows");
        for (size_t i = 0; i < json_object_array_length(incoming); i++) {
            json_object *row = json_object_array_get_idx(incoming, i), *labs = field(row, "labels"), *target = NULL;
            for (size_t j = 0; j < json_object_array_length(rows); j++) {
                json_object *existing = json_object_array_get_idx(rows, j);
                if (json_object_equal(labs, field(existing, "labels"))) {
                    target = existing;
                    break;
                }
            }
            if (!target) {
                target = json_object_new_object();
                json_object_object_add(target, "labels", json_object_get(labs));
                json_object_object_add(target, "points", json_object_new_array());
                json_object_object_add(target, "point_bits", json_object_new_array());
                json_object_array_add(rows, target);
            }
            json_object *pts = field(row, "points"), *dest = field(target, "points");
            for (size_t j = 0; j < json_object_array_length(pts); j++)
                json_object_array_add(dest, json_object_get(json_object_array_get_idx(pts, j)));
            json_object *bits = field(row, "point_bits"), *dest_bits = field(target, "point_bits");
            for (size_t j = 0; j < json_object_array_length(bits); j++)
                json_object_array_add(dest_bits, json_object_get(json_object_array_get_idx(bits, j)));
        }
        json_object_put(encoded);
    }
    json_object_object_add(matrix, "work", json_object_new_uint64(work));
    return matrix;
}
int main(int argc, char **argv)
{
    if (argc != 3)
        fail("usage: host dataset.json request.json");
    json_object *root = json_object_from_file(argv[1]), *requests = json_object_from_file(argv[2]);
    if (!root || !requests)
        fail("invalid JSON file");
    json_object *ss = field(root, "series");
    if (!json_object_is_type(ss, json_type_array))
        fail("series array required");
    size_t n = json_object_array_length(ss);
    PPSeries *series = calloc(n, sizeof *series);
    if (n && !series)
        fail("allocation");
    for (size_t i = 0; i < n; i++) {
        json_object *s = json_object_array_get_idx(ss, i), *labs = field(s, "labels"), *pts = field(s, "points");
        if (!json_object_is_type(labs, json_type_object) || !json_object_is_type(pts, json_type_array))
            fail("series schema");
        size_t ln = json_object_object_length(labs), pn = json_object_array_length(pts);
        PPLabel *l = calloc(ln, sizeof *l);
        PPPoint *p = calloc(pn, sizeof *p);
        if ((ln && !l) || (pn && !p))
            fail("allocation");
        size_t j = 0;
        json_object_object_foreach(labs, k, v)
        {
            if (!json_object_is_type(v, json_type_string))
                fail("label string required");
            l[j++] = (PPLabel){k, json_object_get_string(v)};
        }
        for (j = 0; j < pn; j++) {
            json_object *point = json_object_array_get_idx(pts, j);
            if (!json_object_is_type(point, json_type_array) || json_object_array_length(point) != 2)
                fail("point pair");
            p[j] = (PPPoint){integer(json_object_array_get_idx(point, 0)), number(json_object_array_get_idx(point, 1))};
            if (j && p[j].t <= p[j - 1].t)
                fail("timestamps must increase");
        }
        series[i] = (PPSeries){l, ln, p, pn};
    }
    PPData data = {series, n};
    json_object *results = json_object_new_array();
    if (!json_object_is_type(requests, json_type_array))
        fail("requests array required");
    for (size_t i = 0; i < json_object_array_length(requests); i++) {
        json_object *r = json_object_array_get_idx(requests, i), *v = NULL;
        json_object *q = field(r, "query");
        if (!json_object_is_type(q, json_type_string))
            fail("query string");
        Cancel cancel = {0, 0};
        if (json_object_object_get_ex(r, "cancel_at", &v))
            cancel.at = integer(v);
        PPRequest req = {
            &data, json_object_get_string(q), integer(field(r, "time_ms")), 300000, 100000000, cancelled, &cancel, 0};
        if (json_object_object_get_ex(r, "lookback_ms", &v))
            req.lookback_ms = integer(v);
        if (json_object_object_get_ex(r, "work_limit", &v))
            req.work_limit = integer(v);
        if (json_object_object_get_ex(r, "inject_failure", &v))
            req.inject_failure = integer(v);
        int repetitions = 1;
        if (json_object_object_get_ex(r, "repetitions", &v))
            repetitions = (int)integer(v);
        if (repetitions < 1 || repetitions > 10000)
            fail("repetition bound");
        int64_t start = 0, end = 0, step = 0;
        if (json_object_object_get_ex(r, "step_ms", &v)) {
            step = integer(v);
            start = integer(field(r, "start_ms"));
            end = integer(field(r, "end_ms"));
        }
        /* Benchmark independent evaluations and releases, excluding JSON serialization. */
        double seconds = 0;
        double parse_seconds = 0, evaluation_seconds = 0, first_seconds = 0;
        double first_parse_seconds = 0, first_evaluation_seconds = 0;
        json_object *encoded = NULL;
        if (!step) {
            double began = monotime();
            for (int j = 0; j < repetitions; j++) {
                cancel.calls = 0;
                double first_began = j == 0 ? monotime() : 0;
                PPResult *out = pp_eval(&req);
                if (!out)
                    fail("null result");
                parse_seconds += out->parse_ns / 1e9;
                evaluation_seconds += out->evaluation_ns / 1e9;
                if (j == 0) {
                    first_parse_seconds = out->parse_ns / 1e9;
                    first_evaluation_seconds = out->evaluation_ns / 1e9;
                }
                pp_free(out);
                if (j == 0)
                    first_seconds = monotime() - first_began;
            }
            seconds = monotime() - began;
        }
        cancel.calls = 0;
        encoded = execute(&req, start, end, step);
        json_object_object_add(encoded, "seconds", json_object_new_double(seconds));
        json_object_object_add(encoded, "parse_seconds", json_object_new_double(parse_seconds));
        json_object_object_add(encoded, "evaluation_seconds", json_object_new_double(evaluation_seconds));
        json_object_object_add(encoded, "first_seconds", json_object_new_double(first_seconds));
        json_object_object_add(encoded, "first_parse_seconds", json_object_new_double(first_parse_seconds));
        json_object_object_add(encoded, "first_evaluation_seconds", json_object_new_double(first_evaluation_seconds));
        json_object_object_add(encoded, "query", json_object_get(q));
        json_object_object_add(encoded, "id", json_object_get(field(r, "id")));
        json_object_array_add(results, encoded);
    }
    struct rusage usage;
    getrusage(RUSAGE_SELF, &usage);
    json_object *out = json_object_new_object();
    json_object_object_add(out, "results", results);
    json_object_object_add(out, "maxrss_kb", json_object_new_int64(usage.ru_maxrss));
    puts(json_object_to_json_string_ext(out, JSON_C_TO_STRING_PLAIN));
    json_object_put(out);
    for (size_t i = 0; i < n; i++) {
        free((void *)series[i].labels);
        free((void *)series[i].points);
    }
    free(series);
    json_object_put(requests);
    json_object_put(root);
    return 0;
}
