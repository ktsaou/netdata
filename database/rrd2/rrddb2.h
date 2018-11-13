//
// Created by costa on 04/11/18.
//

#ifndef NETDATA_RRDDB2_H
#define NETDATA_RRDDB2_H

#include "libnetdata/libnetdata.h"

#define RRDDB2_PAGE_TYPE_BITMAP 'B'
#define RRDDB2_PAGE_TYPE_DATA   'D'
#define RRDDB2_PAGE_TYPE_EMPTY  'E'

#define RDDDB2_PAGE_COMPRESSION_NONE 'N'
#define RDDDB2_PAGE_COMPRESSION_GZIP 'G'

struct rrddb2_file {
    char version;
    uuid_t host;
    off_t first_bitmap_offset;
    uint8_t data[];
} __attribute__((packed));

struct rrddb2_page {
    char type;
    char version;
    char compression;
    uint8_t flags;
    uint32_t size;

    uint32_t compressed_hash;
    uint32_t uncompressed_hash;

    uint8_t data[];
} __attribute__((packed));



#endif //NETDATA_RRDDB2_H
