// SPDX-License-Identifier: GPL-3.0-or-later

#include "database/rrd.h"
#include "database/rrddim-collection.h"
#include "daemon/config/netdata-conf-directories.h"

#define LIVE_BASE ((time_t)1700002800)
#define LIVE_CHILD_GUID "4d3fbf52-5b77-4f04-9781-9a1b3f15b902"

extern LISTEN_SOCKETS api_sockets;
int promql_host_readiness_fixture(void);

static int seed_descriptions(RRDHOST *host, bool local) {
    char long_title[9050];
    memset(long_title, 'Z', 9000);
    strcpy(long_title + 9000, " quote\" slash\\ line\n");
    const char *titles[] = {"Common description", "Common description", local ? "Local description" : "Child description",
        "Common description", long_title};
    for(size_t i = 0; i < sizeof(titles) / sizeof(titles[0]); i++) {
        char id[64];
        snprintfz(id, sizeof(id) - 1, "description_%zu", i);
        RRDSET *chart = rrdset_create(host, "promql_fixture", id, NULL, "fixture", "fixture.descriptions",
            titles[i], i == 3 ? "MiB" : "KiB", "fixture", "fixture", 1, 1, RRDSET_TYPE_LINE);
        if(!chart || !rrddim_add(chart, "value", NULL, 1, 1, RRD_ALGORITHM_ABSOLUTE)) return 1;
    }
    RRDSET *collision = rrdset_create(host, "promql_fixture", "description_collision", NULL, "fixture", "fixture_descriptions",
        "Alias collision", "KiB", "fixture", "fixture", 1, 1, RRDSET_TYPE_LINE);
    return !collision || !rrddim_add(collision, "value", NULL, 1, 1, RRD_ALGORITHM_ABSOLUTE);
}

static int seed_host(RRDHOST *host, collected_number gauge_value, collected_number increment) {
    if(!host || host->rrd_memory_mode != RRD_DB_MODE_DBENGINE || !host->db[0].si) return 1;
    RRDSET *chart = rrdset_create(host, "promql_fixture", "http", NULL, "fixture", "fixture.http",
        "HTTP collection fixture", "KiB", "fixture", "fixture", 1, 1, RRDSET_TYPE_LINE);
    if(!chart) return 1;
    RRDDIM *counter = rrddim_add(chart, "counter", NULL, 1, 1, RRD_ALGORITHM_INCREMENTAL);
    RRDDIM *gauge = rrddim_add(chart, "gauge", NULL, 1, 1, RRD_ALGORITHM_ABSOLUTE);
    if(!counter || !gauge) return 1;
    rrdlabels_add(chart->rrdlabels, "fixture_role", gauge_value == 5 ? "local" : "child", RRDLABEL_SRC_AUTO);
    chart->last_collected_time = chart->last_updated = counter->collector.last_collected_time =
        gauge->collector.last_collected_time = (struct timeval){.tv_sec = LIVE_BASE - 1};
    for(unsigned i = 0; i <= 10; i++) {
        if(i == 9) continue;
        if(i == 10) rrdset_set_update_every_s(chart, 2);
        collected_number value = i < 6 ? 100 + increment * i : 10 + increment * (i - 6);
        RRDDIM *dimensions[] = {counter, gauge};
        collected_number values[] = {value, gauge_value};
        for(size_t d = 0; d < 2; d++) {
            dimensions[d]->collector.last_collected_time = (struct timeval){.tv_sec = LIVE_BASE + i};
            rrddim_set_collected_int(dimensions[d], values[d]);
            rrddim_set_updated(dimensions[d]);
            dimensions[d]->collector.counter++;
            rrddim_update_collected_max_from_int(dimensions[d], values[d]);
        }
        chart->usec_since_last_update = (usec_t)(i == 10 ? 2 : 1) * USEC_PER_SEC;
        rrdset_timed_done(chart, (struct timeval){.tv_sec = LIVE_BASE + i}, false);
    }
    rrddim_finalize_collection_and_check_retention(counter);
    rrddim_finalize_collection_and_check_retention(gauge);
    // The real chart and contexts stay owned by the Agent until its normal shutdown.
    return seed_descriptions(host, gauge_value == 5);
}

int promql_http_fixture_seed(void) {
    if(promql_host_readiness_fixture()) return 1;
    RRDHOST *child = rrdhost_find_or_create("promql-fixture-child", "promql-fixture-child", LIVE_CHILD_GUID,
        "linux", "UTC", "UTC", 0, "fixture", "fixture", 1, default_rrd_history_entries, RRD_DB_MODE_DBENGINE,
        false, false, NULL, NULL, NULL, false, 0, 0, NULL, false);
    return seed_host(localhost, 5, 2) + seed_host(child, 7, 4);
}

int promql_http_fixture_ready(void) {
    unsigned port = 0;
    for(size_t i = 0; i < api_sockets.opened; i++) {
        struct sockaddr_in address;
        socklen_t size = sizeof(address);
        if(api_sockets.fds_families[i] == AF_INET && !getsockname(api_sockets.fds[i], (struct sockaddr *)&address, &size) &&
           address.sin_addr.s_addr == htonl(INADDR_LOOPBACK)) port = ntohs(address.sin_port);
    }
    if(!port) return 1;
    char path[PATH_MAX];
    int length = snprintf(path, sizeof(path), "%s/promql-live-ready.json", netdata_configured_cache_dir);
    if(length < 0 || (size_t)length >= sizeof(path)) return 1;
    FILE *file = fopen(path, "w");
    if(!file) return 1;
    bool failed = fprintf(file, "{\"port\":%u,\"pid\":%ld,\"base\":%ld,\"host\":\"%s\",\"child\":\"%s\"}\n",
        port, (long)getpid(), (long)LIVE_BASE, localhost->machine_guid, LIVE_CHILD_GUID) < 0;
    failed |= fclose(file) != 0;
    return failed;
}
