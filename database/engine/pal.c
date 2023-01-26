// SPDX-License-Identifier: GPL-3.0-or-later

#include "pal.h"

#define PAL_PAGES_PER_FILE 1000

typedef enum __attribute__ ((__packed__)) {
    PAL_OPCODE_NONE         = 0,

    PAL_OPCODE_PAGE_ADD     = 1,
    PAL_OPCODE_PAGE_DONE    = 2,

    PAL_OPCODE_POINT_T0_ADD = 10,
    PAL_OPCODE_POINT_T1_ADD = 11,

    PAL_OPCODE_MAX,
} PAL_OPCODES;

struct pal_page_add {
    uint16_t id;
    uint8_t type;
    time_t update_every_s:24;
    uuid_t uuid;
};

struct pal_point_t0 {
    uint16_t page_id;
    usec_t point_in_time_ut;
    storage_number tier0;
};

struct pal_point_t1 {
    uint16_t page_id;
    usec_t point_in_time_ut;
    storage_number_tier1_t tier1;
};

struct pal_page_done {
    uint16_t page_id;
};

struct {
    size_t size;
} pals[PAL_OPCODE_MAX] = {
        [PAL_OPCODE_PAGE_ADD] = { .size = sizeof(struct pal_page_add), },
        [PAL_OPCODE_PAGE_DONE] = { .size = sizeof(struct pal_page_done), },
        [PAL_OPCODE_POINT_T0_ADD] = { .size = sizeof(struct pal_point_t0), },
        [PAL_OPCODE_POINT_T1_ADD] = { .size = sizeof(struct pal_point_t1), },
};

typedef struct pal_page {
    PAL_PAGE_ID id;
    bool done;
    void *data_ptr;
} PAL_PAGE;

typedef struct pal_cmd {
    PAL_OPCODES opcode;

    union {
        struct pal_page_add page_add;
        struct pal_point_t0 point_t0;
        struct pal_point_t1 point_t1;
        struct pal_page_done page_done;
    };
} PAL_CMD;

typedef struct pal_file {
    int fd;
    size_t pos;

    struct {
        SPINLOCK spinlock;
        uint16_t used;
        uint16_t size;
        PAL_PAGE *array;
    } pages;

} PAL_FILE;

struct pal {
    struct {
        const char *path;
        uint16_t pages_per_file;
    } config;

    struct {
        SPINLOCK spinlock;
        uint16_t used;
        uint16_t size;
        PAL_FILE *array;
    } files;

    struct {
        SPINLOCK spinlock;

    } queue;

    struct {
        SPINLOCK spinlock;
        Pvoid_t pages_by_ptr_judyl;
    } page_index;
};


// ----------------------------------------------------------------------------
// public API

PAL *pal_create(const char *path, uint16_t pages_per_file) {

}

void pal_destroy(PAL *pal) {

}

PAL_PAGE_ID pal_page_add(PAL *pal, void *data, uuid_t *uuid, uint8_t type, time_t update_every_s) {

}

void pal_t0_point_add(PAL *pal, PAL_PAGE_ID id, usec_t point_in_time_ut, storage_number n) {

}

void pal_t1_point_add(PAL *pal, PAL_PAGE_ID id, usec_t point_in_time_ut, storage_number_tier1_t n) {

}

void pal_page_done(PAL *pal, PAL_PAGE_ID id) {

}

PAL_PAGE_ID pal_page_find_by_ptr(PAL *pal, void *page_data) {
    // lookup page_data in page_index to return its page id;

    return (PAL_PAGE_ID) { .page_id = PAL_PAGE_ID_INVALID, .file_id = PAL_PAGE_ID_INVALID };
}
