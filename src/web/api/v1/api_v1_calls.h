// SPDX-License-Identifier: GPL-3.0-or-later

#ifndef NETDATA_API_V1_CALLS_H
#define NETDATA_API_V1_CALLS_H

#include "../web_api_v1.h"

int web_client_api_request_v1_info(RRDHOST *host, struct web_client *w, char *url);

int web_client_api_request_v1_registry(RRDHOST *host, struct web_client *w, char *url);

int web_client_api_request_v1_manage(RRDHOST *host, struct web_client *w, char *url);

int web_client_api_request_v1_data(RRDHOST *host, struct web_client *w, char *url);
int web_client_api_request_v1_chart(RRDHOST *host, struct web_client *w, char *url);
int web_client_api_request_v1_charts(RRDHOST *host, struct web_client *w, char *url);

int web_client_api_request_v1_context(RRDHOST *host, struct web_client *w, char *url);
int web_client_api_request_v1_contexts(RRDHOST *host, struct web_client *w, char *url);

int web_client_api_request_v1_alarms(RRDHOST *host, struct web_client *w, char *url);
int web_client_api_request_v1_alarms_values(RRDHOST *host, struct web_client *w, char *url);
int web_client_api_request_v1_alarm_count(RRDHOST *host, struct web_client *w, char *url);
int web_client_api_request_v1_alarm_log(RRDHOST *host, struct web_client *w, char *url);
int web_client_api_request_v1_variable(RRDHOST *host, struct web_client *w, char *url);
int web_client_api_request_v1_alarm_variables(RRDHOST *host, struct web_client *w, char *url);

int web_client_api_request_v1_dbengine_stats(RRDHOST *host, struct web_client *w, char *url);
int web_client_api_request_v1_ml_info(RRDHOST *host, struct web_client *w, char *url);
int web_client_api_request_v1_aclk(RRDHOST *host, struct web_client *w, char *url);

int web_client_api_request_v1_functions(RRDHOST *host, struct web_client *w, char *url);
int web_client_api_request_v1_metric_correlations(RRDHOST *host, struct web_client *w, char *url);
int web_client_api_request_v1_weights(RRDHOST *host, struct web_client *w, char *url);

// common library calls
int web_client_api_request_single_chart(RRDHOST *host, struct web_client *w, char *url, void callback(RRDSET *st, BUFFER *buf));

#endif //NETDATA_API_V1_CALLS_H