#ifndef NETDATA_RRDFILE_H
#define NETDATA_RRDFILE_H

typedef struct rrdfile_name_value {
    size_t length;                  // the length of this record, including the name
    size_t value;                   // the value associated with the name
    uint32_t hash;                  // the hash of the name
    char name[];                    // the name
} RRDMETRIC;

typedef struct rrdfile_slot_name_value {
    size_t size_used;
    size_t entries;
} RRDFILE_SLOT_NAMEVALUE;

typedef struct rrdfile_slot_rrdpage {
    size_t metric_id;
    size_t size;
    size_t update_every;
    size_t first_t;
    size_t last_t;
} RRDFILE_SLOT_RRDPAGE;






// ----------------------------------------------------------------------------
// a file slot

typedef enum rrdfile_slot_type {
    RRDSLOT_RESERVED        = -1,   // empty slot, but the data should not be used
    RRDSLOT_FREE            =  0,   // empty slot, available for use
    RRDSLOT_NAME_VALUE      =  1,   // a slot of name-value pairs
    RRDSLOT_RRDPAGE         =  2,   // a slot of metric data
    RRDSLOT_RRDPAGE_GZIP    =  3    // a slot of metric data, compressed with gzip
} RRDFILE_SLOT_TYPE;

typedef enum rrdfile_slot_flags {
} RRDSLOT_FLAGS;

// on disk
typedef struct rrdfile_slot_header {
    RRDFILE_SLOT_TYPE type;         // the type of data this slot holds
    size_t seq;                     // the sequence number of this slot for this file
    off_t offset;                   // the offset of this slot in this file
    size_t size;                    // the length of the slot, from the first byte to the last, including the data

    union {
        RRDFILE_SLOT_RRDPAGE rrd_page;
        RRDFILE_SLOT_NAMEVALUE name_value;
    };
} RRDFILE_SLOT_HEADER;

// at memory
typedef struct rrdfile_slot {
    RRDFILE_SLOT_HEADER header;

    void *data;                     // the data of this slot
    void *uncompressed_data;        // the uncompressed data of this slot

    time_t uncompressed_t;
    time_t saved_t;
    time_t loaded_t;
    time_t accessed_t;

    RRDSLOT_FLAGS flags;            // flags related to this slot

    struct rrdfile_slot *disk_prev;
    struct rrdfile_slot *disk_next;

} RRDFILE_SLOT;


// ----------------------------------------------------------------------------
// a file

// on disk
typedef struct rrdfile_header {
    char magic[20];
    size_t page_size;
    size_t size;

} RRDFILE_HEADER;

// at memory
typedef struct rrdfile {
    RRDFILE_HEADER header;

    const char *filename;
    int fd;

    size_t next_slot_seq;
    size_t next_metric_id;

    size_t slots_count;

    size_t reads;
    size_t writes;
    size_t seeks;
    size_t read_bytes;
    size_t write_bytes;

    RRDFILE_SLOT *slots;            // a sorted list of all slots in the file

} RRDFILE;


extern RRDFILE *rrdfile_open(const char *filename, size_t size);
extern RRDFILE *rrdfile_create(const char *filename, size_t size);
extern void rrdfile_close(RRDFILE *rf);

extern void *rrdfile_free_space(RRDFILE *rf, size_t size);

extern void rrdfile_del(RRDFILE *rf, RRDFILE_SLOT *rs);
extern RRDFILE_SLOT *rrdfile_add(RRDFILE *rf, RRDFILE_SLOT_TYPE type, void *data, size_t size);

#endif //NETDATA_RRDFILE_H
