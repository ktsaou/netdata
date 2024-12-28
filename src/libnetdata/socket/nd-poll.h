// SPDX-License-Identifier: GPL-3.0-or-later

#ifndef NETDATA_ND_POLL_H
#define NETDATA_ND_POLL_H

#include "libnetdata/common.h"

typedef enum __attribute__((packed)) {
    ND_POLL_NONE            = 0,

    ND_POLL_READ            = 1 << 0, // same as EPOLLIN, POLLIN
    ND_POLL_WRITE           = 1 << 2, // same as EPOLLOUT, POLLOUT
    ND_POLL_ERROR           = 1 << 3, // same as EPOLLERR, POLLERR
    ND_POLL_HUP             = 1 << 4, // same as EPOLLHUP, POLLHUP
    ND_POLL_INVALID         = 1 << 5, // same as POLLNVAL

    ND_POLL_TIMEOUT         = 1 << 6,
    ND_POLL_POLL_FAILED     = 1 << 7,
} nd_poll_event_t;

typedef struct {
    nd_poll_event_t events;
    void *data;
} nd_poll_result_t;

typedef struct nd_poll_t nd_poll_t;

nd_poll_t *nd_poll_create();
void nd_poll_destroy(nd_poll_t *ndpl);

// the events can be updated with nd_poll_upd
// the data pointer SHOULD NEVER be changed and cannot be NULL
bool nd_poll_add(nd_poll_t *ndpl, int fd, nd_poll_event_t events, void *data);

// give the same data pointer used in nd_poll_add()
// otherwise, you may receive back invalid events
bool nd_poll_del(nd_poll_t *ndpl, int fd, void *data);

// this is for updating events
// the data pointer must be the same used in nd_poll_add()
// to change the data pointer, delete and add the same fd again
bool nd_poll_upd(nd_poll_t *ndpl, int fd, nd_poll_event_t events, void *data);

// returns -1 = error, 0 = timeout, 1 = event in result
int nd_poll_wait(nd_poll_t *ndpl, int timeout_ms, nd_poll_result_t *result);

#include "libnetdata/libnetdata.h"

#endif //NETDATA_ND_POLL_H
