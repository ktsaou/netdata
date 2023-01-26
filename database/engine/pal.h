// SPDX-License-Identifier: GPL-3.0-or-later

#ifndef DBENGINE_PAL_H
#define DBENGINE_PAL_H

#include "rrdengine.h"

typedef struct pal_page_id {
    uint16_t file_id;
    uint16_t page_id;
} PAL_PAGE_ID;

#define PAL_PAGE_ID_INVALID 65535
#define pal_page_id_check(id) ((id).file_id != PAL_PAGE_ID_INVALID && (id).page_id != PAL_PAGE_ID_INVALID)

typedef struct pal PAL;

#endif //DBENGINE_PAL_H
