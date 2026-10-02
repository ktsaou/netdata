/* SPDX-License-Identifier: GPL-3.0-or-later */
#define _POSIX_C_SOURCE 200809L
#include "abi.h"
#include <assert.h>
#include <pthread.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

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
        char *query = strdup("owned_metric");
        PPRequest request = {&data, query, 3000, 300000, 100000, NULL, NULL, 0};
        PPResult *result = pp_eval(&request);
        assert(result && result->kind == PP_VECTOR && result->rows_len == 1);
        for (size_t i = 0; i < 2; i++) {
            memset((void *)labels[i].name, 'x', strlen(labels[i].name));
            memset((void *)labels[i].value, 'x', strlen(labels[i].value));
            free((void *)labels[i].name);
            free((void *)labels[i].value);
        }
        memset(points, 0, 3 * sizeof(*points));
        memset(query, 'x', strlen(query));
        free(query);
        free(points);
        free(labels);
        const PPSeries *row = result->rows;
        assert(row->points_len == 1 && row->points[0].t == 3000 && row->points[0].v == 3);
        assert(
            row->labels_len == 2 && !strcmp(row->labels[0].name, "__name__") &&
            !strcmp(row->labels[0].value, "owned_metric"));
        assert(!strcmp(row->labels[1].name, "node") && !strcmp(row->labels[1].value, "one"));
        pp_free(result);
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
    puts("owned result after input destruction; 4 concurrent callers: passed");
    return 0;
}
