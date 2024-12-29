// SPDX-License-Identifier: GPL-3.0-or-later

#include "nd-poll.h"

#ifndef POLLRDHUP
#define POLLRDHUP 0
#endif

#if defined(OS_LINUX)
#include <sys/epoll.h>

#define MAX_EVENTS_PER_CALL 100

// Event poll context
struct nd_poll_t {
    int epoll_fd;
    struct epoll_event ev[MAX_EVENTS_PER_CALL];
    size_t last_pos;
    size_t used;
    size_t nfds;
};

// Initialize the event poll context
nd_poll_t *nd_poll_create() {
    nd_poll_t *ndpl = callocz(1, sizeof(nd_poll_t));

    ndpl->epoll_fd = epoll_create1(0);
    if (ndpl->epoll_fd < 0) {
        freez(ndpl);
        return NULL;
    }

    return ndpl;
}

static inline void nd_poll_replace_data_on_loaded_events(nd_poll_t *ndpl, int fd __maybe_unused, void *old_data, void *new_data) {
    for(size_t i = ndpl->last_pos; i < ndpl->used; i++) {
        if(ndpl->ev[i].data.ptr == old_data)
            ndpl->ev[i].data.ptr = new_data;
    }
}

// Add a file descriptor to the event poll
bool nd_poll_add(nd_poll_t *ndpl, int fd, nd_poll_event_t events, void *data) {
    internal_fatal(!data, "nd_poll() does not support NULL data pointers");

    struct epoll_event ev = {
        .events = (events & ND_POLL_READ ? EPOLLIN : 0) | (events & ND_POLL_WRITE ? EPOLLOUT : 0),
        .data.ptr = data,
    };
    bool rc = epoll_ctl(ndpl->epoll_fd, EPOLL_CTL_ADD, fd, &ev) == 0;
    if(rc) ndpl->nfds++;
    internal_fatal(!rc, "epoll_ctl() failed");
    return rc;
}

// Remove a file descriptor from the event poll
bool nd_poll_del(nd_poll_t *ndpl, int fd, void *data) {
    internal_fatal(!data, "nd_poll() does not support NULL data pointers");

    ndpl->nfds--; // we can't check for success/failure here, because epoll() removes fds when they are closed
    bool rc = epoll_ctl(ndpl->epoll_fd, EPOLL_CTL_DEL, fd, NULL) == 0;
    internal_error(!rc, "epoll_ctl() failed (is the socket already closed)"); // this is ok if the socket is already closed

    // we may have an event pending for this fd.
    // but epoll() does not give us fd in the events,
    // so we use the data pointer to invalidate it
    nd_poll_replace_data_on_loaded_events(ndpl, fd, data, NULL);
    return rc;
}

// Update an existing file descriptor in the event poll
bool nd_poll_upd(nd_poll_t *ndpl, int fd, nd_poll_event_t events, void *data) {
    internal_fatal(!data, "nd_poll() does not support NULL data pointers - you should also NEVER change the pointer with an update");

    struct epoll_event ev = {
        .events = (events & ND_POLL_READ ? EPOLLIN : 0) | (events & ND_POLL_WRITE ? EPOLLOUT : 0),
        .data.ptr = data,
    };
    bool rc = epoll_ctl(ndpl->epoll_fd, EPOLL_CTL_MOD, fd, &ev) == 0;
    internal_fatal(!rc, "epoll_ctl() failed");
    return rc;
}

static inline nd_poll_event_t nd_poll_events_from_epoll_events(uint32_t events) {
    nd_poll_event_t nd_poll_events = ND_POLL_NONE;

    if (events & (EPOLLIN|EPOLLPRI|EPOLLRDNORM|EPOLLRDBAND))
        nd_poll_events |= ND_POLL_READ;

    if (events & (EPOLLOUT|EPOLLWRNORM|EPOLLWRBAND))
        nd_poll_events |= ND_POLL_WRITE;

    if (events & EPOLLERR)
        nd_poll_events |= ND_POLL_ERROR;

    if (events & (EPOLLHUP|EPOLLRDHUP))
        nd_poll_events |= ND_POLL_HUP;

    return nd_poll_events;
}

static inline bool nd_poll_get_next_event(nd_poll_t *ndpl, nd_poll_result_t *result) {
    while(ndpl->last_pos < ndpl->used) {
        void *data = ndpl->ev[ndpl->last_pos].data.ptr;

        // Skip events that have been invalidated by nd_poll_del()
        if(!data) {
            ndpl->last_pos++;
            continue;
        }

        *result = (nd_poll_result_t){
            .events = nd_poll_events_from_epoll_events(ndpl->ev[ndpl->last_pos].events),
            .data = data,
        };

        ndpl->last_pos++;
        return true;
    }

    return false;
}

// Wait for events
int nd_poll_wait(nd_poll_t *ndpl, int timeout_ms, nd_poll_result_t *result) {
    if(nd_poll_get_next_event(ndpl, result))
        return 1;

    do {
        errno_clear();
        ndpl->last_pos = 0;
        ndpl->used = 0;

        int maxevents = ndpl->nfds / 2;
        if(maxevents > (int)_countof(ndpl->ev)) maxevents = (int)_countof(ndpl->ev);
        if(maxevents < 2) maxevents = 2;

        int n = epoll_wait(ndpl->epoll_fd, &ndpl->ev[0], maxevents, timeout_ms);

        if(unlikely(n <= 0)) {
            if(n == 0) {
                result->events = ND_POLL_TIMEOUT;
                result->data = NULL;
                return 0;
            }

            if(errno == EINTR || errno == EAGAIN)
                continue;

            result->events = ND_POLL_POLL_FAILED;
            result->data = NULL;
            return -1;
        }

        ndpl->used = n;
        ndpl->last_pos = 0;
        if (nd_poll_get_next_event(ndpl, result))
            return 1;

        internal_fatal(true, "nd_poll_get_next_event() should have 1 event!");
    } while(true);
}

// Destroy the event poll context
void nd_poll_destroy(nd_poll_t *ndpl) {
    if (ndpl) {
        close(ndpl->epoll_fd);
        freez(ndpl);
    }
}
#else

DEFINE_JUDYL_TYPED(POINTERS, void *);

struct nd_poll_t {
    struct pollfd *fds;         // Array of file descriptors
    nfds_t nfds;                // Number of active file descriptors
    nfds_t capacity;            // Allocated capacity for `fds` array
    nfds_t last_pos;
    POINTERS_JudyLSet pointers; // Judy array to store user data
};

#define INITIAL_CAPACITY 4

// Initialize the event poll context
nd_poll_t *nd_poll_create() {
    nd_poll_t *ndpl = callocz(1, sizeof(nd_poll_t));
    ndpl->fds = mallocz(INITIAL_CAPACITY * sizeof(struct pollfd));
    ndpl->nfds = 0;
    ndpl->capacity = INITIAL_CAPACITY;

    POINTERS_INIT(&ndpl->pointers);

    return ndpl;
}

// Ensure capacity for adding new file descriptors
static void ensure_capacity(nd_poll_t *ndpl) {
    if (ndpl->nfds < ndpl->capacity) return;

    nfds_t new_capacity = ndpl->capacity * 2;
    struct pollfd *new_fds = reallocz(ndpl->fds, new_capacity * sizeof(struct pollfd));

    ndpl->fds = new_fds;
    ndpl->capacity = new_capacity;
}

bool nd_poll_add(nd_poll_t *ndpl, int fd, nd_poll_event_t events, void *data) {
    internal_fatal(POINTERS_GET(&ndpl->pointers, fd) != NULL, "File descriptor %d is already served - cannot add", fd);

    ensure_capacity(ndpl);
    struct pollfd *pfd = &ndpl->fds[ndpl->nfds++];
    pfd->fd = fd;
    pfd->events = 0;
    if (events & ND_POLL_READ) pfd->events |= POLLIN;
    if (events & ND_POLL_WRITE) pfd->events |= POLLOUT;
    pfd->revents = 0;

    POINTERS_SET(&ndpl->pointers, fd, data);

    return true;
}

// Remove a file descriptor from the event poll
bool nd_poll_del(nd_poll_t *ndpl, int fd, void *data __maybe_unused) {
    for (nfds_t i = 0; i < ndpl->nfds; i++) {
        if (ndpl->fds[i].fd == fd) {

            // Remove the file descriptor by shifting the array
            memmove(&ndpl->fds[i], &ndpl->fds[i + 1], (ndpl->nfds - i - 1) * sizeof(struct pollfd));
            ndpl->nfds--;
            POINTERS_DEL(&ndpl->pointers, fd);

            if(i < ndpl->last_pos)
                ndpl->last_pos--;

            return true;
        }
    }

    return false; // File descriptor not found
}

// Update an existing file descriptor in the event poll
bool nd_poll_upd(nd_poll_t *ndpl, int fd, nd_poll_event_t events, void *data) {
    internal_fatal(POINTERS_GET(&ndpl->pointers, fd) == NULL, "File descriptor %d is not found - cannot modify", fd);

    for (nfds_t i = 0; i < ndpl->nfds; i++) {
        if (ndpl->fds[i].fd == fd) {
            struct pollfd *pfd = &ndpl->fds[i];
            pfd->events = 0;
            if (events & ND_POLL_READ) pfd->events |= POLLIN;
            if (events & ND_POLL_WRITE) pfd->events |= POLLOUT;
            POINTERS_SET(&ndpl->pointers, fd, data);
            return true;
        }
    }

    // File descriptor not found
    return false;
}

static inline nd_poll_event_t nd_poll_events_from_poll_revents(short int events) {
    nd_poll_event_t nd_poll_events = ND_POLL_NONE;

    if (events & (POLLIN|POLLPRI|POLLRDNORM|POLLRDBAND))
        nd_poll_events |= ND_POLL_READ;

    if (events & (POLLOUT|POLLWRNORM|POLLWRBAND))
        nd_poll_events |= ND_POLL_WRITE;

    if (events & POLLERR)
        nd_poll_events |= ND_POLL_ERROR;

    if (events & (POLLHUP|POLLRDHUP))
        nd_poll_events |= ND_POLL_HUP;

    if (events & (POLLNVAL))
        nd_poll_events |= ND_POLL_INVALID;

    return nd_poll_events;
}

static inline bool nd_poll_get_next_event(nd_poll_t *ndpl, nd_poll_result_t *result) {
    for (nfds_t i = ndpl->last_pos; i < ndpl->nfds; i++) {
        if (ndpl->fds[i].revents != 0) {

            result->data = POINTERS_GET(&ndpl->pointers, ndpl->fds[i].fd);
            result->events = nd_poll_events_from_poll_revents(ndpl->fds[i].revents);
            ndpl->fds[i].revents = 0;

            ndpl->last_pos = i + 1;
            return true;
        }
    }

    ndpl->last_pos = ndpl->nfds;
    return false;
}

// Rotate the fds array to prevent starvation
static inline void rotate_fds(nd_poll_t *ndpl) {
    if (ndpl->nfds == 0 || ndpl->nfds == 1)
        return; // No rotation needed for empty or single-entry arrays

    struct pollfd first = ndpl->fds[0];
    memmove(&ndpl->fds[0], &ndpl->fds[1], (ndpl->nfds - 1) * sizeof(struct pollfd));
    ndpl->fds[ndpl->nfds - 1] = first;
}

// Wait for events
int nd_poll_wait(nd_poll_t *ndpl, int timeout_ms, nd_poll_result_t *result) {
    if (nd_poll_get_next_event(ndpl, result))
        return 1; // Return immediately if there's a pending event

    do {
        errno_clear();
        ndpl->last_pos = 0;
        rotate_fds(ndpl); // Rotate the array on every wait
        int ret = poll(ndpl->fds, ndpl->nfds, timeout_ms);

        if(unlikely(ret <= 0)) {
            if(ret == 0) {
                result->events = ND_POLL_TIMEOUT;
                result->data = NULL;
                return 0;
            }

            if(errno == EAGAIN || errno == EINTR)
                continue;

            result->events = ND_POLL_POLL_FAILED;
            result->data = NULL;
            return -1;
        }

        // Process the next event
        if (nd_poll_get_next_event(ndpl, result))
            return 1;

        internal_fatal(true, "nd_poll_get_next_event() should have 1 event!");
    } while (true);
}

// Destroy the event poll context
void nd_poll_destroy(nd_poll_t *ndpl) {
    if (ndpl) {
        free(ndpl->fds);
        POINTERS_FREE(&ndpl->pointers, NULL);
        freez(ndpl);
    }
}

#endif
