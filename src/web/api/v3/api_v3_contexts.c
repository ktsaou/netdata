// SPDX-License-Identifier: GPL-3.0-or-later

#include "api_v3_calls.h"

int web_client_api_request_v3_contexts_internal(RRDHOST *host __maybe_unused, struct web_client *w, char *url, CONTEXTS_V2_MODE mode) {
    struct api_v2_contexts_request req = { 0 };

    while(url) {
        char *value = strsep_skip_consecutive_separators(&url, "&");
        if(!value || !*value) continue;

        char *name = strsep_skip_consecutive_separators(&value, "=");
        if(!name || !*name) continue;
        if(!value || !*value) continue;

        // name and value are now the parameters
        // they are not null and not empty

        if(!strcmp(name, "scope_nodes"))
            req.scope_nodes = value;
        else if(!strcmp(name, "nodes"))
            req.nodes = value;
        else if((mode & (CONTEXTS_V2_CONTEXTS | CONTEXTS_V2_SEARCH | CONTEXTS_V2_ALERTS | CONTEXTS_V2_ALERT_TRANSITIONS)) && !strcmp(name, "scope_contexts"))
            req.scope_contexts = value;
        else if((mode & (CONTEXTS_V2_CONTEXTS | CONTEXTS_V2_SEARCH | CONTEXTS_V2_ALERTS | CONTEXTS_V2_ALERT_TRANSITIONS)) && !strcmp(name, "contexts"))
            req.contexts = value;
        else if((mode & CONTEXTS_V2_SEARCH) && !strcmp(name, "q"))
            req.q = value;
        else if(!strcmp(name, "options"))
            req.options = web_client_api_request_v2_context_options(value);
        else if(!strcmp(name, "after"))
            req.after = str2l(value);
        else if(!strcmp(name, "before"))
            req.before = str2l(value);
        else if(!strcmp(name, "timeout"))
            req.timeout_ms = str2l(value);
        else if(mode & (CONTEXTS_V2_ALERTS | CONTEXTS_V2_ALERT_TRANSITIONS)) {
            if (!strcmp(name, "alert"))
                req.alerts.alert = value;
            else if (!strcmp(name, "transition"))
                req.alerts.transition = value;
            else if(mode & CONTEXTS_V2_ALERTS) {
                if (!strcmp(name, "status"))
                    req.alerts.status = web_client_api_request_v2_alert_status(value);
            }
            else if(mode & CONTEXTS_V2_ALERT_TRANSITIONS) {
                if (!strcmp(name, "last"))
                    req.alerts.last = strtoul(value, NULL, 0);
                else if(!strcmp(name, "context"))
                    req.contexts = value;
                else if (!strcmp(name, "anchor_gi")) {
                    req.alerts.global_id_anchor = str2ull(value, NULL);
                }
                else {
                    for(int i = 0; i < ATF_TOTAL_ENTRIES ;i++) {
                        if(!strcmp(name, alert_transition_facets[i].query_param))
                            req.alerts.facets[i] = value;
                    }
                }
            }
        }
    }

    if ((mode & CONTEXTS_V2_ALERT_TRANSITIONS) && !req.alerts.last)
        req.alerts.last = 1;

    buffer_flush(w->response.data);
    buffer_no_cacheable(w->response.data);
    return rrdcontext_to_json_v2(w->response.data, &req, mode);
}

int web_client_api_request_v3_contexts(RRDHOST *host __maybe_unused, struct web_client *w, char *url) {
    return web_client_api_request_v3_contexts_internal(
        host, w, url,
        CONTEXTS_V2_CONTEXTS | CONTEXTS_V2_NODES | CONTEXTS_V2_AGENTS | CONTEXTS_V2_VERSIONS);
}

int web_client_api_request_v3_alert_transitions(RRDHOST *host __maybe_unused, struct web_client *w, char *url) {
    return web_client_api_request_v3_contexts_internal(
        host, w, url,
        CONTEXTS_V2_ALERT_TRANSITIONS | CONTEXTS_V2_NODES);
}

int web_client_api_request_v3_alerts(RRDHOST *host __maybe_unused, struct web_client *w, char *url) {
    return web_client_api_request_v3_contexts_internal(
        host, w, url,
        CONTEXTS_V2_ALERTS | CONTEXTS_V2_NODES);
}

int web_client_api_request_v3_functions(RRDHOST *host __maybe_unused, struct web_client *w, char *url) {
    return web_client_api_request_v3_contexts_internal(
        host, w, url,
        CONTEXTS_V2_FUNCTIONS | CONTEXTS_V2_NODES | CONTEXTS_V2_AGENTS | CONTEXTS_V2_VERSIONS);
}

int web_client_api_request_v3_versions(RRDHOST *host __maybe_unused, struct web_client *w, char *url) {
    return web_client_api_request_v3_contexts_internal(
        host, w, url,
        CONTEXTS_V2_VERSIONS);
}

int web_client_api_request_v3_q(RRDHOST *host __maybe_unused, struct web_client *w, char *url) {
    return web_client_api_request_v3_contexts_internal(
        host, w, url,
        CONTEXTS_V2_SEARCH | CONTEXTS_V2_CONTEXTS | CONTEXTS_V2_NODES | CONTEXTS_V2_AGENTS | CONTEXTS_V2_VERSIONS);
}

int web_client_api_request_v3_nodes(RRDHOST *host __maybe_unused, struct web_client *w, char *url) {
    return web_client_api_request_v3_contexts_internal(
        host, w, url,
        CONTEXTS_V2_NODES | CONTEXTS_V2_NODES_INFO);
}

int web_client_api_request_v3_info(RRDHOST *host __maybe_unused, struct web_client *w, char *url) {
    return web_client_api_request_v3_contexts_internal(
        host, w, url,
        CONTEXTS_V2_AGENTS | CONTEXTS_V2_AGENTS_INFO);
}

int web_client_api_request_v3_node_instances(RRDHOST *host __maybe_unused, struct web_client *w, char *url) {
    return web_client_api_request_v3_contexts_internal(
        host, w, url,
        CONTEXTS_V2_NODES | CONTEXTS_V2_NODE_INSTANCES | CONTEXTS_V2_AGENTS | CONTEXTS_V2_AGENTS_INFO | CONTEXTS_V2_VERSIONS);
}

