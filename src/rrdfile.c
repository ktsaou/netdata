#include "common.h"

#define RRDFILE_MAGIC "NETDATA_RRDFILE v1"

// ----------------------------------------------------------------------------
// rrdfile basic file I/O operations

static off_t rrdfile_seek(RRDFILE *rf, off_t offset) {
    rf->seeks++;

    off_t ret = lseek(rf->fd, offset, SEEK_SET);

    if(ret == -1)
        error("RRDFILE '%s': failed to seek at offset %zu", rf->filename, (size_t)offset);

    return ret;
}

static ssize_t rrdfile_write(RRDFILE *rf, off_t offset, void *data, size_t size) {
    if(rrdfile_seek(rf, offset) == -1)
        return -1;

    ssize_t ret = write(rf->fd, data, size);
    rf->writes++;

    if(ret == -1)
        error("RRDFILE '%s': failed to write %zu bytes at %zu", rf->filename, size, (size_t)offset);
    else {
        if(ret != size)
            error("RRDFILE '%s': attempted to write %zu bytes at %zu, but wrote %zu", rf->filename, size, (size_t)offset, ret);

        rf->write_bytes += ret;
    }

    return ret;
}

static ssize_t rrdfile_read(RRDFILE *rf, off_t offset, void *data, size_t size) {
    if(rrdfile_seek(rf, offset) == -1)
        return -1;

    ssize_t ret = read(rf->fd, data, size);
    rf->reads++;

    if(ret == -1)
        error("RRDFILE '%s': failed to read %zu bytes at %zu", rf->filename, size, (size_t)offset);
    else {
        if(ret != size)
            error("RRDFILE '%s': attempted to read %zu bytes at %zu, but got %zu", rf->filename, size, (size_t)offset, ret);

        rf->read_bytes += ret;
    }

    return ret;
}

// ----------------------------------------------------------------------------
// rrdfile basic slot operations

static ssize_t rrdfile_write_slot_header(RRDFILE *rf, RRDFILE_SLOT *slot) {
    ssize_t ret = rrdfile_write(rf, slot->header.offset, &slot->header, sizeof(slot->header));
    if(ret == -1)
        error("RRDFILE '%s', failed to write header of slot seq %zu", rf->filename, slot->header.seq);

    return ret;
}

static void rrdfile_unlink_and_delete_slot(RRDFILE *rf, RRDFILE_SLOT *slot) {
    if(slot->disk_prev)
        slot->disk_prev->disk_next = slot->disk_next;

    if(slot->disk_next)
        slot->disk_next->disk_prev = slot->disk_prev;

    if(rf->slots == slot)
        rf->slots = slot->disk_next;

    rf->slots_count--;

    freez(slot);
}

static RRDFILE_SLOT *rrdfile_free_slot(RRDFILE *rf, RRDFILE_SLOT *slot) {
    slot->header.type = RRDSLOT_FREE;

    // find the first free, before this one
    while(slot->disk_prev && slot->disk_prev->header.type == RRDSLOT_FREE)
        slot = slot->disk_prev;

    // merge it with the next, if it is free too
    RRDFILE_SLOT *t = slot->disk_next;
    while(t && t->header.type == RRDSLOT_FREE) {
        slot->header.size += t->header.size;
        rrdfile_unlink_and_delete_slot(rf, t);
        t = slot->disk_next;
    }

    slot->header.seq = (size_t)rf->next_slot_seq++;
    rrdfile_write_slot_header(rf, slot);

    return slot;
}

static RRDFILE_SLOT *rrdfile_find_space(RRDFILE *rf, size_t size) {
    RRDFILE_SLOT *slot, *first_free = NULL;

    for(slot = rf->slots; slot ; slot = slot->disk_next) {
        if(slot->header.type == RRDSLOT_FREE) {
            // FIXME
        }
    }

    // FIXME
}

static RRDFILE_SLOT *rrdfile_create_free_slot(RRDFILE *rf, off_t offset, size_t size) {
    RRDFILE_SLOT *slot = callocz(1, size);
    slot->header.type = RRDSLOT_FREE;
    slot->header.seq = (size_t)rf->next_slot_seq++;
    slot->header.offset = offset;
    slot->header.size = size;

    rrdfile_write_slot_header(rf, slot);

    rf->slots_count++;
    return slot;
}


// ----------------------------------------------------------------------------
// rrdfile header read/write

ssize_t rrdfile_header_read(RRDFILE *rf) {
    ssize_t ret = rrdfile_read(rf, 0, &rf->header, sizeof(rf->header));

    if(ret == -1)
        error("RRDFILE '%s': failed to read header (%zu bytes)", rf->filename, sizeof(rf->header));

    return ret;
}

ssize_t rrdfile_header_write(RRDFILE *rf) {
    ssize_t ret = rrdfile_write(rf, 0, &rf->header, sizeof(rf->header));

    if(ret == -1)
        error("RRDFILE '%s': failed to write header", rf->filename);

    return ret;
}

// ----------------------------------------------------------------------------
// rrdfile open/create/close

void rrdfile_close(RRDFILE *rf) {
    close(rf->fd);
    freez((void *)rf->filename);
    freez((void *)rf);
}

RRDFILE *rrdfile_create(const char *filename, size_t size) {
    int fd = open(filename, O_CREAT|O_DIRECT|O_DSYNC);
    if(fd == -1) {
        error("RRDFILE '%s': cannot create file.", filename);
        return NULL;
    }

    RRDFILE *rf = callocz(1, sizeof(RRDFILE));
    rf->fd = fd;
    rf->filename = strdupz(filename);

    // ------------------------------------------------------------------------
    // write the file header

    strncpy(rf->header.magic, RRDFILE_MAGIC, sizeof(rf->header.magic));
    rf->header.magic[sizeof(rf->header.magic) - 1] = '\0';
    rf->header.page_size = (size_t)sysconf(_SC_PAGESIZE);
    rf->header.size = size;

    if(rrdfile_header_write(rf) == -1) {
        rrdfile_close(rf);
        return NULL;
    }

    // ------------------------------------------------------------------------
    // write an empty slot

    rf->slots = rrdfile_create_free_slot(rf, sizeof(rf->header), size - sizeof(rf->header));
    if(!rf->slots) {
        error("RRDFILE '%s': failed to create slots. Closing file.", rf->filename);
        rrdfile_close(rf);
        return NULL;
    }

    return rf;
}

RRDFILE *rrdfile_open(const char *filename, size_t size) {
    int fd = open(filename, O_DIRECT|O_DSYNC);
    if(fd == -1)
        return rrdfile_create(filename, size);

    RRDFILE *rf = callocz(1, sizeof(RRDFILE));
    rf->fd = fd;
    rf->filename = strdupz(filename);

    // ------------------------------------------------------------------------
    // read the header

    if(rrdfile_header_read(rf) == -1) {
        rrdfile_close(rf);
        return NULL;
    }

    // ------------------------------------------------------------------------
    // validate the header

    if(strcmp(rf->header.magic, RRDFILE_MAGIC) != 0) {
        error("RRDFILE '%s': file does not seem a netdata RRD file.", rf->filename);
        rrdfile_close(rf);
        return NULL;
    }

    // ------------------------------------------------------------------------
    // read all slots

    // FIXME: read all slots from disk
    // FIXME: make sure there are no gaps - if there are, resize the slots to fill the gaps
    // FIXME: make sure slots do not overlap - if they do, delete the overlapping slots - replace them with a free one
    // FIXME: sort slots, older to newer
    // FIXME: assign slots to metrics
    // FIXME: create name-value index

    // hm... this should not happen, but anyway, make sure we have at least a slot
    if(!rf->slots) {
        error("RRDFILE '%s': file found without any slots in it. Creating a free slot for the entire space.", rf->filename);
        rf->slots = rrdfile_create_free_slot(rf, sizeof(rf->header), rf->header.size - sizeof(rf->header));
    }

    // ------------------------------------------------------------------------
    // check if the wanted size is different

    if(rf->header.size + sizeof(RRDFILE_SLOT_HEADER) < size) {
        info("RRDFILE '%s': increasing file size from %zu to %zu", rf->filename, rf->header.size, size);

        // find the last slot
        RRDFILE_SLOT *slot;
        for(slot = rf->slots; slot->disk_next; slot = slot->disk_next) ;

        // append a new free slot;
        off_t offset = rf->header.size;
        size_t len = size - rf->header.size;
        rf->header.size = size;
        slot->disk_next = rrdfile_create_free_slot(rf, offset, len);

        // merge it - possibly
        rrdfile_free_slot(rf, slot->disk_next);

        // save the header to the file (new size)
        rrdfile_header_write(rf);
    }
    else if(rf->header.size < size) {
        error("RRDFILE '%s': new size ignored - the increment is too small to be used", rf->filename);
    }
    else if(rf->header.size > size) {
        error("RRDFILE '%s': cannot shrink files yet. Using the size found on disk (%zu).", rf->filename, rf->header.size);
    }

    return rf;
}
