// SPDX-License-Identifier: GPL-3.0-or-later

#ifndef NETDATA_PROMQL_API_H
#define NETDATA_PROMQL_API_H

#include "database/rrd.h"
#include "web/server/web_client.h"

int api_v1_promql_query(RRDHOST *host, struct web_client *client, char *query_string);
int api_v1_promql_query_range(RRDHOST *host, struct web_client *client, char *query_string);
int api_v1_promql_labels(RRDHOST *host, struct web_client *client, char *query_string);
int api_v1_promql_label_values(RRDHOST *host, struct web_client *client, char *query_string);
int api_v1_promql_series(RRDHOST *host, struct web_client *client, char *query_string);
int api_v1_promql_metadata(RRDHOST *host, struct web_client *client, char *query_string);
void promql_api_cleanup(void);

#endif
