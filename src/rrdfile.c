#include "common.h"

#define RRDFILE_MAGIC "NETDATA_RRDFILE 1.0"

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
        error("RRDFILE '%s': failed to write %zu bytes at %zu", rf->filename, size, offset);
    else {
        if(ret != size)
            error("RRDFILE '%s': attempted to write %zu bytes at %zu, but wrote %zu", rf->filename, size, offset, ret);

        rf->write_bytes += ret;
    }

    return ret;
}

static RRDFILE_SLOT *rrdfile_find_space(RRDFILE *rf, size_t size) {
    RRDFILE_SLOT *slot, *first_free = NULL;

    for(slot = rf->slots; slot ; slot = slot->next) {
        if(slot->header.type == RRDSLOT_FREE) {

        }
    }
}

static RRDFILE_SLOT *rrdfile_write_free_slot(RRDFILE *rf, off_t offset, size_t size) {
    RRDFILE_SLOT *slot = callocz(1, size);
    slot->header.type = RRDSLOT_FREE;
    slot->header.seq = rf->next_slot_seq++;
    slot->header.offset = offset;
    slot->header.size = size;

    if(rrdfile_write(rf, offset, &slot->header, sizeof(slot->header)) == -1) {
        error("RRDFILE '%s': failed to write free slot %zu", rf->filename, slot->header.seq);
        rf->next_slot_seq--;
        return NULL;
    }

    rf->slots_count++;
    return slot;
}

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
    rf->size = size;
    rf->filename = strdupz(filename);

    // ------------------------------------------------------------------------
    // write the file header

    strncpy(rf->header->magic, RRDFILE_MAGIC, sizeof(rf->header->magic));
    rf->header->magic[sizeof(rf->header->magic) - 1] = '\0';
    rf->header->page_size = (size_t)sysconf(_SC_PAGESIZE);

    if(write(rf->fd, &rf->header, sizeof(rf->header)) == -1) {
        error("RRDFILE '%s': failed to write header", rf->filename);
        rrdfile_close(rf);
        return NULL;
    }

    // ------------------------------------------------------------------------
    // write an empty slot

    rf->slots = rrdfile_write_free_slot(rf, sizeof(rf->header), size - sizeof(rf->header));
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

    // read the size

    // read the file header

    // read all all slots

}
