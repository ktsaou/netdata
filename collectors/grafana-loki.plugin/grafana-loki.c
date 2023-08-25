// SPDX-License-Identifier: GPL-3.0-or-later
/*
 *  netdata grafana-loki.plugin
 *  Copyright (C) 2023 Netdata Inc.
 *  GPL v3+
 */

#include "collectors/all.h"
#include "libnetdata/libnetdata.h"
#include "libnetdata/required_dummies.h"

#include <curl/curl.h>

static size_t write_callback(void* contents, size_t size, size_t nmemb, void* userp) {
    size_t realsize = size * nmemb;
    fwrite(contents, size, nmemb, stdout);
    return realsize;
}

void grafana_loki_query(usec_t after_ut, usec_t before_ut, usec_t stop_monotonic_ut) {
    CURL* curl;
    CURLcode res;

    // Initialize curl session
    curl_global_init(CURL_GLOBAL_DEFAULT);
    curl = curl_easy_init();

    if (curl) {
        // Compose the Loki query URL
        char query_url[256];
        snprintf(query_url, sizeof(query_url), "http://localhost:3100/loki/api/v1/query_range?query=&limit=1000000&start=%llu&end=%llu&direction=backward", after_ut, before_ut);

        fprintf(stderr, "URL: %s\n", query_url);

        // Set curl options
        curl_easy_setopt(curl, CURLOPT_URL, query_url);
        curl_easy_setopt(curl, CURLOPT_WRITEFUNCTION, write_callback);

        // Perform the request and check for errors
        res = curl_easy_perform(curl);
        if (res != CURLE_OK) {
            fprintf(stderr, "curl_easy_perform() failed: %s\n", curl_easy_strerror(res));
        }

        // Cleanup
        curl_easy_cleanup(curl);
    }

    curl_global_cleanup();
}

int main() {
    usec_t now_ut = now_realtime_usec();

    usec_t after_ut = now_ut - 86400 * USEC_PER_SEC;
    usec_t before_ut = now_ut;
    usec_t stop_monotonic_ut = now_monotonic_usec() + 10 * USEC_PER_SEC;

    grafana_loki_query(after_ut, before_ut, stop_monotonic_ut);
    return 0;
}
