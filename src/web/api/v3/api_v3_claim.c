// SPDX-License-Identifier: GPL-3.0-or-later

#include "api_v3_calls.h"

int web_client_api_request_v3_claim(RRDHOST *host __maybe_unused, struct web_client *w, char *url) {
    return api_v3_claim(w, url);
}
