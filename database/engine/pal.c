// SPDX-License-Identifier: GPL-3.0-or-later

#include "pal.h"

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
    uuid_t metric_uuid;
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

typedef struct pal {
    FILE *fp;

} PAL;


struct {
    struct {

    } config;

    Pvoid_t pal_by_id_judyhs;
} pal_globals;