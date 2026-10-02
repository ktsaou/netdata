/* SPDX-License-Identifier: GPL-3.0-or-later */
#define _POSIX_C_SOURCE 200809L
#include "abi.h"
#include <assert.h>
#include <pthread.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

static const char *label_value(const PPSeries *row, const char *name)
{
    for (size_t i = 0; i < row->labels_len; i++)
        if (!strcmp(row->labels[i].name, name))
            return row->labels[i].value;
    return NULL;
}
static void *reader(void *unused)
{
    (void)unused;
    for (int iteration = 0; iteration < 50; iteration++) {
        PPLabel *labels = calloc(2, sizeof(*labels));
        PPPoint *points = calloc(3, sizeof(*points));
        assert(labels && points);
        labels[0] = (PPLabel){strdup("__name__"), strdup("owned_metric")};
        labels[1] = (PPLabel){strdup("node"), strdup("one")};
        points[0] = (PPPoint){1000, 1};
        points[1] = (PPPoint){2000, 2};
        points[2] = (PPPoint){3000, 3};
        PPSeries series = {labels, 2, points, 3};
        PPData data = {&series, 1};
        const char *queries[] = {"owned_metric", "owned_metric[2s]",
                                 "label_join(owned_metric,\"joined\",\"/\",\"node\",\"node\")",
                                 "sum by(node)(owned_metric)", "vector(1) or owned_metric"};
        int shape = iteration % 5;
        char *query = strdup(queries[shape]);
        PPRequest request = {&data, query, 3000, 300000, 100000, NULL, NULL, 0};
        PPResult *result = pp_eval(&request);
        assert(result && result->kind == (shape == 1 ? PP_MATRIX : PP_VECTOR));
        assert(result->rows_len == (shape == 4 ? 2 : 1));
        assert(result->parse_ns && result->evaluation_ns);
        char *other_query = strdup("owned_metric * 2");
        request.query = other_query;
        PPResult *other = pp_eval(&request);
        assert(other && other->kind == PP_VECTOR && other->rows_len == 1);
        for (size_t i = 0; i < 2; i++) {
            memset((void *)labels[i].name, 'x', strlen(labels[i].name));
            memset((void *)labels[i].value, 'x', strlen(labels[i].value));
            free((void *)labels[i].name);
            free((void *)labels[i].value);
        }
        memset(points, 0, 3 * sizeof(*points));
        memset(query, 'x', strlen(query));
        memset(other_query, 'x', strlen(other_query));
        free(query);
        free(other_query);
        free(points);
        free(labels);
        const PPSeries *row = result->rows + (shape == 4 ? 1 : 0);
        if (shape == 4) {
            assert(!result->rows[0].labels_len && result->rows[0].points_len == 1);
            assert(result->rows[0].points[0].t == 3000 && result->rows[0].points[0].v == 1);
        }
        assert(row->points_len == (shape == 1 ? 2 : 1));
        assert(row->points[row->points_len - 1].t == 3000 && row->points[row->points_len - 1].v == 3);
        if (shape == 1)
            assert(row->points[0].t == 2000 && row->points[0].v == 2);
        assert(row->labels_len == (shape == 2 ? 3 : shape == 3 ? 1 : 2));
        assert(!strcmp(label_value(row, "node"), "one"));
        if (shape != 3)
            assert(!strcmp(label_value(row, "__name__"), "owned_metric"));
        else
            assert(!label_value(row, "__name__"));
        if (shape == 2)
            assert(!strcmp(label_value(row, "joined"), "one/one"));
        assert(other->rows[0].points_len == 1 && other->rows[0].points[0].v == 6);
        pp_free(result);
        assert(other->rows[0].points[0].v == 6 && !strcmp(label_value(other->rows, "node"), "one"));
        pp_free(other);
    }
    return NULL;
}
int main(void)
{
    pthread_t threads[4];
    for (size_t i = 0; i < 4; i++)
        assert(!pthread_create(threads + i, NULL, reader, NULL));
    for (size_t i = 0; i < 4; i++)
        assert(!pthread_join(threads[i], NULL));
    pp_free(NULL);
    puts("owned selector/matrix/generated/aggregate/mixed-label results after input destruction; independent held results; 4 concurrent callers: passed");
    return 0;
}
