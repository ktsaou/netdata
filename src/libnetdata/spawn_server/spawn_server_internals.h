// SPDX-License-Identifier: GPL-3.0-or-later

#ifndef NETDATA_SPAWN_SERVER_INTERNALS_H
#define NETDATA_SPAWN_SERVER_INTERNALS_H

#include "../libnetdata.h"
#include "spawn_server.h"

#if defined(OS_WINDOWS)
#include <windows.h>
#include <io.h>
#include <fcntl.h>
#include <process.h>
#include <tlhelp32.h>
#include <tchar.h>
#include <sys/cygwin.h>
#endif

struct spawn_server {
    size_t id;
    size_t request_id;
    const char *name;

#if !defined(OS_WINDOWS)
    SPAWN_SERVER_OPTIONS options;

    ND_UUID magic;          // for authorizing requests, the client needs to know our random UUID
                            // it is ignored for PING requests

    int pipe[2];
    int sock;               // the listening socket of the server
    pid_t server_pid;
    char *path;
    spawn_request_callback_t cb;

    int argc;
    const char **argv;
#endif
};

struct spawm_instance {
    size_t request_id;
    int sock;
    int write_fd;
    int read_fd;
    pid_t child_pid;

#if defined(OS_WINDOWS)
    HANDLE process_handle;
    DWORD dwProcessId;
#endif
};

#endif //NETDATA_SPAWN_SERVER_INTERNALS_H