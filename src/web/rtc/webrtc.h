// SPDX-License-Identifier: GPL-3.0-or-later

#ifndef NETDATA_WEBRTC_H
#define NETDATA_WEBRTC_H

#include "libnetdata/libnetdata.h"

int webrtc_new_connection(const char *sdp, BUFFER *wb);
void webrtc_close_all_connections();
void webrtc_initialize();

#if defined(ENABLE_PROMQL) && defined(NETDATA_INTERNAL_CHECKS)
typedef struct {
    size_t connections, channels;
    int channel, buffered;
    uint64_t operation_id;
} WEBRTC_TEST_STATE;
WEBRTC_TEST_STATE webrtc_test_state(void);
int webrtc_promql_peer_fixture(void);
#endif

#endif //NETDATA_WEBRTC_H
