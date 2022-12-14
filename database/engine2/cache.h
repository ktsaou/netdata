#ifndef DBENGINE_CACHE_H
#define DBENGINE_CACHE_H

#include "../rrd.h"

typedef struct pgc PGC;
typedef struct pgc_page PGC_PAGE;

typedef enum __attribute__ ((__packed__)) {
    PGC_OPTIONS_NONE       = 0,
    PGC_OPTIONS_EVICT_PAGES_INLINE = (1 << 0),
    PGC_OPTIONS_FLUSH_PAGES_INLINE = (1 << 1),
    PGC_OPTIONS_AUTOSCALE          = (1 << 2),
} PGC_OPTIONS;

#define PGC_OPTIONS_DEFAULT (PGC_OPTIONS_EVICT_PAGES_INLINE | PGC_OPTIONS_FLUSH_PAGES_INLINE | PGC_OPTIONS_AUTOSCALE)

typedef struct pgc_entry {
    Word_t section;             // the section this belongs to
    Word_t metric_id;           // the metric this belongs to
    time_t start_time_t;        // the start time of the page
    time_t end_time_t;          // the end time of the page
    size_t size;                // the size in bytes of the allocation, outside the cache
    void *data;                 // a pointer to data outside the cache
    uint32_t update_every;      // the update every of the page
    bool hot;                   // true if this entry is currently being collected
    uint8_t *custom_data;
} PGC_ENTRY;

struct pgc_queue_statistics {
    size_t entries;
    size_t size;
    size_t max_entries;
    size_t max_size;
    size_t added_entries;
    size_t added_size;
    size_t removed_entries;
    size_t removed_size;
};

struct pgc_statistics {
    size_t added_entries;
    size_t added_size;

    size_t removed_entries;
    size_t removed_size;

    size_t entries;                 // all the entries (includes clean, dirty, host)
    size_t size;                    // all the entries (includes clean, dirty, host)

    size_t referenced_entries;      // all the entries currently referenced
    size_t referenced_size;         // all the entries currently referenced

    size_t searches_exact;
    size_t searches_exact_hits;
    size_t searches_exact_misses;

    size_t searches_closest;
    size_t searches_closest_hits;
    size_t searches_closest_misses;

    size_t flushes_completed;
    size_t flushes_completed_size;
    size_t flushes_cancelled;
    size_t flushes_cancelled_size;

    size_t points_collected;

    size_t insert_spins;
    size_t evict_spins;
    size_t release_spins;
    size_t acquire_spins;
    size_t delete_spins;

    size_t evict_skipped;
    size_t hot_empty_pages_evicted_immediately;
    size_t hot_empty_pages_evicted_later;

    // events
    size_t events_cache_under_severe_pressure;
    size_t events_cache_needs_space_aggressively;
    size_t events_flush_critical;

    struct {
        struct pgc_queue_statistics hot;
        struct pgc_queue_statistics dirty;
        struct pgc_queue_statistics clean;
    } queues;
};


typedef void (*free_clean_page_callback)(PGC *cache, PGC_ENTRY entry);
typedef void (*save_dirty_page_callback)(PGC *cache, PGC_ENTRY *array, size_t entries);

// create a cache
PGC *pgc_create(size_t clean_size_bytes, free_clean_page_callback pgc_free_clean_cb,
                size_t max_dirty_pages_per_call, save_dirty_page_callback pgc_save_dirty_cb,
                size_t max_pages_per_inline_eviction, size_t max_skip_pages_per_inline_eviction,
                size_t max_flushes_inline,
                PGC_OPTIONS options, size_t partitions, size_t additional_bytes_per_page);

// destroy the cache
void pgc_destroy(PGC *cache);

// add a page to the cache and return a pointer to it
PGC_PAGE *pgc_page_add_and_acquire(PGC *cache, PGC_ENTRY entry, bool *added);

// release a page (all pointers to it are now invalid)
void pgc_page_release(PGC *cache, PGC_PAGE *page);

// mark a hot page dirty, and release it
void pgc_page_hot_to_dirty_and_release(PGC *cache, PGC_PAGE *page);

// find a page from the cache
PGC_PAGE *pgc_page_get_and_acquire(PGC *cache, Word_t section, Word_t metric_id, time_t start_time_t, bool exact);

// get information from an acquired page
Word_t pgc_page_section(PGC_PAGE *page);
Word_t pgc_page_metric(PGC_PAGE *page);
time_t pgc_page_start_time_t(PGC_PAGE *page);
time_t pgc_page_end_time_t(PGC_PAGE *page);
time_t pgc_page_update_every(PGC_PAGE *page);
void *pgc_page_data(PGC_PAGE *page);
void *pgc_page_custom_data(PGC *cache, PGC_PAGE *page);
size_t pgc_page_data_size(PGC *cache, PGC_PAGE *page);
bool pgc_is_page_hot(PGC_PAGE *page);
bool pgc_is_page_dirty(PGC_PAGE *page);
bool pgc_is_page_clean(PGC_PAGE *page);

// resetting the end time of a hot page
void pgc_page_hot_set_end_time_t(PGC *cache, PGC_PAGE *page, time_t end_time_t);
void pgc_page_hot_to_clean_empty_and_release(PGC *cache, PGC_PAGE *page);

typedef void (*migrate_to_v2_callback)(Word_t section, int datafile_fileno, uint8_t type, Pvoid_t JudyL_metrics, Pvoid_t JudyL_extents_pos, size_t count_of_unique_extents, size_t count_of_unique_metrics, size_t count_of_unique_pages, void *data);
void pgc_open_cache_to_journal_v2(PGC *cache, Word_t section, int datafile_fileno, uint8_t type, migrate_to_v2_callback cb, void *data);
// return true when there is more work to do
bool pgc_evict_pages(PGC *cache, size_t max_skip, size_t max_evict);
bool pgc_flush_pages(PGC *cache, size_t max_flushes);

struct pgc_statistics pgc_get_statistics(PGC *cache);

#endif // DBENGINE_CACHE_H
