// SPDX-License-Identifier: GPL-3.0-or-later

#include "provider-internal.h"

#define PHYSICAL_REMOVED ((PROMQL_PHYSICAL *)(uintptr_t)1)

int promql_label_compare(netdata_promql_bytes_t a, netdata_promql_bytes_t b) {
    size_t common = (size_t)MIN(a.len, b.len);
    int comparison = common ? memcmp(a.data, b.data, common) : 0;
    return comparison ? comparison : (a.len > b.len) - (a.len < b.len);
}

static void label_heap(netdata_promql_label_t *labels, size_t root, size_t count) {
    while(root < count / 2) {
        size_t child = 2 * root + 1;
        if(child + 1 < count && promql_label_compare(labels[child].name, labels[child + 1].name) < 0) child++;
        if(promql_label_compare(labels[root].name, labels[child].name) >= 0) break;
        SWAP(labels[root], labels[child]);
        root = child;
    }
}

void promql_labels_sort(netdata_promql_label_t *labels, size_t count) {
    for(size_t root = count / 2; root; root--) label_heap(labels, root - 1, count);
    for(size_t remaining = count; remaining > 1; remaining--) {
        SWAP(labels[0], labels[remaining - 1]);
        label_heap(labels, 0, remaining - 1);
    }
}

uint32_t promql_provider_error(PROMQL_PROVIDER_CONTEXT *context, const char *message,
                               netdata_promql_bytes_t *error) {
    snprintfz(context->error, sizeof(context->error) - 1, "%s", message);
    *error = promql_bytes(context->error);
    return NETDATA_PROMQL_PROVIDER_ERROR;
}

PROMQL_HOST *promql_host_acquire(PROMQL_PROVIDER_QUERY *query, RRDHOST *host,
                                 const netdata_promql_accounting_context_t *account, bool *busy, bool *unavailable) {
    *busy = false;
    *unavailable = false;
    for(PROMQL_HOST *owner = query->hosts; owner; owner = owner->next) {
        if(owner->host != host) continue;
        owner->references++;
        return owner;
    }
    if(!rw_spinlock_tryread_lock(&host->metadata_lifetime_lock)) { *busy = true; return NULL; }
    RRD_CONTEXT_LOAD_STATE state = __atomic_load_n(&host->rrdctx.initial_load, __ATOMIC_ACQUIRE);
    if(state != RRD_CONTEXT_LOAD_COMPLETE) {
        *unavailable = state == RRD_CONTEXT_LOAD_UNAVAILABLE;
        *busy = !*unavailable;
        rw_spinlock_read_unlock(&host->metadata_lifetime_lock);
        return NULL;
    }
    PROMQL_HOST *owner = promql_malloc(account, NETDATA_PROMQL_MEMORY_PROVIDER_HELD, sizeof(*owner));
    if(!owner) { rw_spinlock_read_unlock(&host->metadata_lifetime_lock); return NULL; }
    owner->host = host;
    owner->references = 1;
    owner->next = query->hosts;
    query->hosts = owner;
    return owner;
}

void promql_host_release(PROMQL_PROVIDER_QUERY *query, PROMQL_HOST *host) {
    if(!host || --host->references) return;
    PROMQL_HOST **previous = &query->hosts;
    while(*previous != host) previous = &(*previous)->next;
    *previous = host->next;
    rw_spinlock_read_unlock(&host->host->metadata_lifetime_lock);
    promql_free(host);
}

static size_t physical_hash(RRDHOST *host, const nd_uuid_t uuid) {
    uint64_t hash = (uint64_t)(uintptr_t)host ^ UINT64_C(14695981039346656037);
    for(size_t i = 0; i < sizeof(nd_uuid_t); i++) hash = (hash ^ uuid[i]) * UINT64_C(1099511628211);
    return (size_t)(hash ^ (hash >> 32));
}

static size_t physical_slot(PROMQL_PROVIDER_QUERY *query, RRDHOST *host, const nd_uuid_t uuid) {
    size_t slot = physical_hash(host, uuid) & (query->capacity - 1), removed = SIZE_MAX;
    for(;;) {
        PROMQL_PHYSICAL *item = query->physical[slot];
        if(!item) return removed == SIZE_MAX ? slot : removed;
        if(item == PHYSICAL_REMOVED) { if(removed == SIZE_MAX) removed = slot; }
        else if(item->host->host == host && nd_uuid_eq(item->uuid, uuid)) return slot;
        slot = (slot + 1) & (query->capacity - 1);
    }
}

static bool physical_capacity(PROMQL_PROVIDER_QUERY *query, const netdata_promql_accounting_context_t *account) {
    if(query->capacity && query->used + query->tombstones < query->capacity / 2) return true;
    size_t capacity = query->capacity ? query->capacity : 16;
    if(query->used >= capacity / 4) {
        if(capacity > SIZE_MAX / 2) return false;
        capacity *= 2;
    }
    if(capacity > SIZE_MAX / sizeof(*query->physical)) return false;
    PROMQL_PHYSICAL **slots = promql_malloc(account, NETDATA_PROMQL_MEMORY_PROVIDER_HELD,
                                           capacity * sizeof(*slots));
    if(!slots) return false;
    PROMQL_PHYSICAL **previous = query->physical;
    size_t old_capacity = query->capacity;
    query->physical = slots;
    query->capacity = capacity;
    query->tombstones = 0;
    for(size_t i = 0; i < old_capacity; i++) {
        PROMQL_PHYSICAL *item = previous[i];
        if(item && item != PHYSICAL_REMOVED)
            slots[physical_slot(query, item->host->host, item->uuid)] = item;
    }
    promql_free(previous);
    return true;
}

static void physical_destroy(PROMQL_PHYSICAL *physical) {
    for(uint32_t tier = 0; tier < RRD_STORAGE_TIERS; tier++) {
        rrdeng_read_unfreeze_metric(&physical->promises[tier]);
        if(physical->metrics[tier]) rrdeng_metric_release((STORAGE_METRIC_HANDLE *)physical->metrics[tier]);
    }
    for(size_t i = 0; i < physical->label_count; i++) {
        string_freez(physical->label_references[i].name);
        string_freez(physical->label_references[i].value);
        promql_free(physical->label_references[i].generated_name);
    }
    if(physical->metadata_view) netdata_promql_allocation_release(physical->metadata_view);
    promql_free(physical->label_references);
    promql_free(physical->labels);
    promql_free(physical->alias);
    promql_free(physical->checkpoints);
    promql_host_release(physical->query, physical->host);
    promql_free(physical);
}

void promql_physical_release(PROMQL_PHYSICAL *physical) {
    if(!physical || --physical->references) return;
    PROMQL_PROVIDER_QUERY *query = physical->query;
    size_t slot = physical_slot(query, physical->host->host, physical->uuid);
    internal_fatal(query->physical[slot] != physical, "PromQL physical descriptor is not indexed");
    query->physical[slot] = PHYSICAL_REMOVED;
    query->used--;
    query->tombstones++;
    physical_destroy(physical);
}

static bool label_add(PROMQL_PHYSICAL *physical, const netdata_promql_accounting_context_t *account,
                       const char *prefix, const char *name, STRING *name_reference,
                       STRING *value_reference, const char *raw_value) {
    const char *value = value_reference ? string2str(value_reference) : raw_value ? raw_value : "";
    size_t prefix_bytes = strlen(prefix), name_bytes = strlen(name);
    if(prefix_bytes >= UINT32_C(16777216) || name_bytes >= UINT32_C(16777216) - prefix_bytes ||
       strlen(value) >= UINT32_C(16777216)) return false;
    if(physical->label_count == physical->label_capacity) {
        size_t capacity = physical->label_capacity ? physical->label_capacity * 2 : 16;
        if(capacity < physical->label_capacity || capacity > SIZE_MAX / sizeof(*physical->labels) ||
           capacity > SIZE_MAX / sizeof(*physical->label_references)) return false;
        void *labels = physical->labels ? promql_resize(account, physical->labels, capacity * sizeof(*physical->labels)) :
            promql_malloc(account, NETDATA_PROMQL_MEMORY_PROVIDER_HELD, capacity * sizeof(*physical->labels));
        if(!labels) return false;
        physical->labels = labels;
        void *references = physical->label_references ?
            promql_resize(account, physical->label_references, capacity * sizeof(*physical->label_references)) :
            promql_malloc(account, NETDATA_PROMQL_MEMORY_PROVIDER_HELD, capacity * sizeof(*physical->label_references));
        if(!references) return false;
        physical->label_references = references;
        physical->label_capacity = capacity;
    }
    char *generated = NULL;
    if(prefix_bytes) {
        generated = promql_malloc(account, NETDATA_PROMQL_MEMORY_PROVIDER_HELD, prefix_bytes + name_bytes + 1);
        if(!generated) return false;
        memcpy(generated, prefix, prefix_bytes);
        memcpy(generated + prefix_bytes, name, name_bytes + 1);
    }
    uint64_t borrowed = (uint64_t)strlen(value) + 1 + (prefix_bytes ? 0 : (uint64_t)name_bytes + 1);
    if(!physical->metadata_view && netdata_promql_allocation_acquire(account, NETDATA_PROMQL_MEMORY_BORROWED_VIEW,
        0, &physical->metadata_view) != NETDATA_PROMQL_OK) { promql_free(generated); return false; }
    if(borrowed > UINT64_MAX - physical->metadata_view_bytes ||
       netdata_promql_allocation_resize(physical->metadata_view, physical->metadata_view_bytes + borrowed) != NETDATA_PROMQL_OK) {
        promql_free(generated); return false;
    }
    physical->metadata_view_bytes += borrowed;
    physical->label_references[physical->label_count] = (PROMQL_LABEL_REFERENCE){
        .name = prefix_bytes ? NULL : string_dup(name_reference), .value = string_dup(value_reference),
        .generated_name = generated};
    physical->labels[physical->label_count++] = (netdata_promql_label_t){
        promql_bytes(generated ? generated : name), promql_bytes(value)};
    return true;
}

typedef struct {
    PROMQL_PHYSICAL *physical;
    PROMQL_READ_FRAME *frame;
    bool host, failed;
} LABEL_COPY;

static bool label_reference(LABEL_COPY *copy, STRING *name_reference, STRING *value_reference) {
    const char *name = string2str(name_reference);
    if(copy->failed || !promql_read_poll(copy->frame)) { copy->failed = true; return false; }
    const char *prefix = copy->host ? "netdata_host_label_" :
        (!strncmp(name, "netdata_", sizeof("netdata_") - 1) || !strcmp(name, "__name__")) ? "netdata_chart_label_" : "";
    copy->failed = !label_add(copy->physical, copy->frame->account, prefix, name,
                              name_reference, value_reference, NULL);
    return !copy->failed;
}

static int labels_reference(STRING *name, STRING *value, RRDLABEL_SRC source __maybe_unused, void *data) {
    return label_reference(data, name, value) ? 0 : -1;
}

static bool bytes_equal(netdata_promql_bytes_t bytes, const char *text) {
    return !promql_label_compare(bytes, promql_bytes(text));
}

bool promql_selection_prefilters(PROMQL_SELECTION *selection, netdata_promql_matchers_t matchers,
                                 const netdata_promql_accounting_context_t *account) {
    size_t count = 0;
    for(uint64_t i = 0; i < matchers.len; i++) {
        const netdata_promql_matcher_t *matcher = &matchers.data[i];
        if(matcher->kind <= NETDATA_PROMQL_MATCH_NOT_EQUAL &&
           (bytes_equal(matcher->name, "__name__") || bytes_equal(matcher->name, "netdata_context") ||
            bytes_equal(matcher->name, "netdata_host_id") || bytes_equal(matcher->name, "netdata_dimension_id") ||
            bytes_equal(matcher->name, "netdata_dimension_name"))) count++;
    }
    if(count > SIZE_MAX / sizeof(*selection->prefilters)) return false;
    if(!count) return true;
    selection->prefilters = promql_malloc(account, NETDATA_PROMQL_MEMORY_PROVIDER_HELD, count * sizeof(*selection->prefilters));
    if(!selection->prefilters) return false;
    for(uint64_t i = 0; i < matchers.len; i++) {
        const netdata_promql_matcher_t *matcher = &matchers.data[i];
        if(matcher->kind > NETDATA_PROMQL_MATCH_NOT_EQUAL ||
           !(bytes_equal(matcher->name, "__name__") || bytes_equal(matcher->name, "netdata_context") ||
             bytes_equal(matcher->name, "netdata_host_id") || bytes_equal(matcher->name, "netdata_dimension_id") ||
             bytes_equal(matcher->name, "netdata_dimension_name"))) continue;
        netdata_promql_matcher_t *copy = &selection->prefilters[selection->prefilter_count++];
        copy->kind = matcher->kind;
        copy->name.data = (uint8_t *)promql_malloc(account, NETDATA_PROMQL_MEMORY_PROVIDER_HELD, (size_t)matcher->name.len);
        if(matcher->value.len > SIZE_MAX) return false;
        copy->value.data = (uint8_t *)promql_malloc(account, NETDATA_PROMQL_MEMORY_PROVIDER_HELD, (size_t)matcher->value.len);
        if(!copy->name.data || !copy->value.data) return false;
        memcpy((void *)copy->name.data, matcher->name.data, (size_t)matcher->name.len);
        if(matcher->value.len) memcpy((void *)copy->value.data, matcher->value.data, (size_t)matcher->value.len);
        copy->name.len = matcher->name.len;
        copy->value.len = matcher->value.len;
    }
    return true;
}

static bool metadata_prefilter(PROMQL_SELECTION *selection, RRDCONTEXT *context, RRDMETRIC *metric,
                                const netdata_promql_accounting_context_t *account, bool *failed) {
    unsigned names = 3;
    char *alias = NULL;
    for(size_t i = 0; i < selection->prefilter_count; i++) {
        netdata_promql_matcher_t matcher = selection->prefilters[i];
        if(bytes_equal(matcher.name, "__name__")) {
            const char *name = string2str(context->id);
            if(!alias) {
                size_t bytes = strlen(name) + 1;
                alias = promql_malloc(account, NETDATA_PROMQL_MEMORY_PROVIDER_HELD, bytes);
                if(!alias) { *failed = true; return false; }
                prometheus_rrdlabels_sanitize_name(alias, name, bytes);
            }
            bool exact = bytes_equal(matcher.value, name), alternate = bytes_equal(matcher.value, alias);
            if(matcher.kind == NETDATA_PROMQL_MATCH_NOT_EQUAL) { exact = !exact; alternate = !alternate; }
            names &= (exact ? 1U : 0U) | (alternate ? 2U : 0U);
        }
        else {
            const char *value = bytes_equal(matcher.name, "netdata_context") ? string2str(context->id) :
                bytes_equal(matcher.name, "netdata_host_id") ? selection->host->host->machine_guid :
                bytes_equal(matcher.name, "netdata_dimension_id") ? string2str(metric->id) : string2str(metric->name);
            bool equal = bytes_equal(matcher.value, value);
            if(equal != (matcher.kind == NETDATA_PROMQL_MATCH_EQUAL)) names = 0;
        }
        if(!names) break;
    }
    promql_free(alias);
    return names != 0;
}

static PROMQL_PHYSICAL *physical_create(PROMQL_SELECTION *selection, RRDCONTEXT *context, RRDINSTANCE *instance,
                                        RRDMETRIC *metric, PROMQL_READ_FRAME *frame, uint32_t *status,
                                        netdata_promql_bytes_t *error) {
    PROMQL_PROVIDER_QUERY *query = selection->query;
    PROMQL_PHYSICAL *physical = NULL;
    for(;;) {
        if(!promql_read_poll(frame)) { *status = frame->interrupted; return NULL; }
        rrdinstance_metadata_lock(instance);
        nd_uuid_t uuid;
        bool identity = metric->ri == instance && instance->rc == context && uuidmap_uuid(metric->uuid, uuid);
        if(!identity) {
            rrdinstance_metadata_unlock(instance);
            *status = promql_provider_error(query->context, "metric metadata changed during selection", error);
            return NULL;
        }
        if(query->capacity) {
            physical = query->physical[physical_slot(query, selection->host->host, uuid)];
            if(physical && physical != PHYSICAL_REMOVED) {
                physical->references++;
                rrdinstance_metadata_unlock(instance);
                return physical;
            }
        }
        bool prefilter_failed = false;
        if(!metadata_prefilter(selection, context, metric, frame->account, &prefilter_failed)) {
            rrdinstance_metadata_unlock(instance);
            if(prefilter_failed) *status = promql_provider_error(query->context, "cannot account metric-name prefilter", error);
            return NULL;
        }
        if(!instance->rrdset && rrd_flag_check_all(instance, RRD_FLAG_OWN_LABELS | RRD_FLAG_DEMAND_LABELS)) {
            rrdinstance_metadata_unlock(instance);
            if(!load_instance_labels_on_demand(instance, promql_read_poll, frame)) {
                *status = frame->interrupted != NETDATA_PROMQL_OK ? frame->interrupted :
                    promql_provider_error(query->context, "cannot load archived chart labels into contexts", error);
                return NULL;
            }
            continue;
        }
        physical = promql_malloc(frame->account, NETDATA_PROMQL_MEMORY_PROVIDER_HELD, sizeof(*physical));
        if(!physical) {
            rrdinstance_metadata_unlock(instance);
            *status = promql_provider_error(query->context, "cannot account metric metadata", error);
            return NULL;
        }
        physical->query = query;
        physical->host = selection->host;
        physical->host->references++;
        physical->references = 1;
        memcpy(physical->uuid, uuid, sizeof(uuid));
        physical->incremental = rrdmetric_algorithm_atomic_load(metric) == RRD_ALGORITHM_INCREMENTAL;
        physical->name = string2str(context->id);
        if(physical->name) {
            size_t bytes = strlen(physical->name) + 1;
            physical->alias = promql_malloc(frame->account, NETDATA_PROMQL_MEMORY_PROVIDER_HELD, bytes);
            if(physical->alias) prometheus_rrdlabels_sanitize_name(physical->alias, physical->name, bytes);
        }
        LABEL_COPY copy = {.physical = physical, .frame = frame};
        bool copied = physical->name && physical->alias &&
            label_add(physical, frame->account, "", "netdata_context", NULL, context->id, NULL) &&
            label_add(physical, frame->account, "", "netdata_chart_id", NULL, instance->id, NULL) &&
            label_add(physical, frame->account, "", "netdata_chart_name", NULL, instance->name, NULL) &&
            label_add(physical, frame->account, "", "netdata_dimension_id", NULL, metric->id, NULL) &&
            label_add(physical, frame->account, "", "netdata_dimension_name", NULL, metric->name, NULL) &&
            label_add(physical, frame->account, "", "netdata_units", NULL, instance->units, NULL);
        if(copied) {
            RRDLABELS *labels = instance->rrdset ? instance->rrdset->rrdlabels : instance->rrdlabels;
            copied = rrdlabels_walkthrough_read_string(labels, labels_reference, &copy) >= 0;
        }
        rrdinstance_metadata_unlock(instance);
        if(!copied || copy.failed) {
            physical_destroy(physical);
            *status = frame->interrupted != NETDATA_PROMQL_OK ? frame->interrupted :
                promql_provider_error(query->context, "cannot copy metric metadata", error);
            return NULL;
        }
        RRDHOST *host = selection->host->host;
        RRDHOST_IDENTITY host_identity = rrdhost_identity_acquire(host);
        copied = label_add(physical, frame->account, "", "netdata_host_id", NULL, NULL, host->machine_guid) &&
            label_add(physical, frame->account, "", "netdata_host_name", NULL, host_identity.hostname, NULL);
        rrdhost_identity_release(&host_identity);
        copy.host = true;
        if(copied) copied = rrdlabels_walkthrough_read_string(host->rrdlabels, labels_reference, &copy) >= 0;
        if(!copied || copy.failed) {
            physical_destroy(physical);
            *status = frame->interrupted != NETDATA_PROMQL_OK ? frame->interrupted :
                promql_provider_error(query->context, "cannot copy host labels", error);
            return NULL;
        }
        promql_labels_sort(physical->labels, physical->label_count);
        for(size_t i = 1; i < physical->label_count; i++) {
            if(promql_label_compare(physical->labels[i - 1].name, physical->labels[i].name)) continue;
            physical_destroy(physical);
            *status = promql_provider_error(query->context, "duplicate mapped label name", error);
            return NULL;
        }
        for(uint32_t tier = 0; tier < RRD_STORAGE_TIERS; tier++)
            if(host->db[tier].mode == RRD_DB_MODE_DBENGINE && host->db[tier].si)
                physical->metrics[tier] = (METRIC *)rrdeng_metric_get_by_uuid(host->db[tier].si, &physical->uuid);
        if(!rrdeng_read_freeze_tiers(physical->metrics, RRD_STORAGE_TIERS, physical->promises,
                                   query->first, query->last, physical->spans, &physical->span_count)) {
            physical_destroy(physical);
            *status = promql_provider_error(query->context, "cannot freeze metric retention", error);
            return NULL;
        }
        if(!physical->span_count) { physical_destroy(physical); return NULL; }
        if(!physical_capacity(query, frame->account)) {
            physical_destroy(physical);
            *status = promql_provider_error(query->context, "cannot account metric index", error);
            return NULL;
        }
        size_t slot = physical_slot(query, host, physical->uuid);
        if(query->physical[slot] == PHYSICAL_REMOVED) query->tombstones--;
        query->physical[slot] = physical;
        query->used++;
        return physical;
    }
}

static void *traverse_start(struct dictionary_foreach *iterator, DICTIONARY *dictionary) {
    *iterator = (struct dictionary_foreach){.dict = dictionary, .rw = DICTIONARY_LOCK_READ};
    void *value = dictionary_foreach_start_rw(iterator);
    dictionary_foreach_unlock(iterator);
    return value;
}

static void *traverse_next(struct dictionary_foreach *iterator) {
    void *value = dictionary_foreach_next(iterator);
    dictionary_foreach_unlock(iterator);
    return value;
}

static RRDHOST *scoped_host(PROMQL_PROVIDER_CONTEXT *context) {
    switch(context->scope) {
        case PROMQL_HOST_GUID: return rrdhost_find_by_guid(context->host);
        case PROMQL_HOST_NODE: return rrdhost_find_by_node_id(context->host);
        case PROMQL_HOST_NAME: return rrdhost_find_by_hostname(context->host);
        default: return NULL;
    }
}

static uint32_t selection_host(PROMQL_SELECTION *selection, PROMQL_READ_FRAME *frame, netdata_promql_bytes_t *error) {
    PROMQL_PROVIDER_QUERY *query = selection->query;
    for(;;) {
        if(!promql_read_poll(frame)) return frame->interrupted;
        if(unlikely(exit_initiated_get()))
            return promql_provider_error(query->context, "host metadata selection interrupted by shutdown", error);
        rrd_rdlock();
        RRDHOST *host;
        if(query->context->scope != PROMQL_FLEET) {
            host = selection->scoped_done ? NULL : scoped_host(query->context);
        }
        else if(!selection->hosts_started) {
            host = traverse_start(&selection->hosts, rrdhost_root_index);
            selection->hosts_started = true;
        }
        else host = traverse_next(&selection->hosts);
        for(;;) {
            if(!host) { rrd_rdunlock(); return NETDATA_PROMQL_END; }
            // A dictionary item retains its name, but the global lookup retains the host until its guard is acquired.
            if(query->context->scope == PROMQL_FLEET && rrdhost_find_by_guid(selection->hosts.name) != host) {
                host = traverse_next(&selection->hosts);
                continue;
            }
            bool busy, unavailable;
            selection->host = promql_host_acquire(query, host, frame->account, &busy, &unavailable);
            if(!busy) {
                selection->scoped_done = true;
                rrd_rdunlock();
                if(!selection->host) return promql_provider_error(query->context, unavailable ?
                    "initial host metadata load is unavailable" : "cannot account host reference", error);
                return NETDATA_PROMQL_OK;
            }
            rrd_rdunlock();
            if(!promql_read_poll(frame)) return frame->interrupted;
            sleep_usec(1000);
            rrd_rdlock();
            host = query->context->scope == PROMQL_FLEET ? rrdhost_find_by_guid(selection->hosts.name) : scoped_host(query->context);
        }
    }
}

static PROMQL_SERIES *series_create(PROMQL_PHYSICAL *physical, bool alias,
                                    const netdata_promql_accounting_context_t *account) {
    if(physical->label_count > (SIZE_MAX - sizeof(PROMQL_SERIES)) / sizeof(netdata_promql_label_t) - 1) return NULL;
    PROMQL_SERIES *series = promql_malloc(account, NETDATA_PROMQL_MEMORY_PROVIDER_HELD,
        sizeof(*series) + (physical->label_count + 1) * sizeof(*series->labels));
    if(!series) return NULL;
    series->physical = physical;
    memcpy(series->labels, physical->labels, physical->label_count * sizeof(*series->labels));
    series->labels[physical->label_count] = (netdata_promql_label_t){promql_bytes("__name__"),
                                                                    promql_bytes(alias ? physical->alias : physical->name)};
    promql_labels_sort(series->labels, physical->label_count + 1);
    return series;
}

uint32_t promql_metadata_next(PROMQL_SELECTION *selection, PROMQL_READ_FRAME *frame,
                              netdata_promql_series_open_t *out, netdata_promql_bytes_t *error) {
    if(selection->query->empty) return NETDATA_PROMQL_END;
    if(selection->pending_alias) {
        PROMQL_PHYSICAL *physical = selection->pending_alias;
        selection->pending_alias = NULL;
        PROMQL_SERIES *series = series_create(physical, true, frame->account);
        if(!series) {
            promql_physical_release(physical);
            return promql_provider_error(selection->query->context, "cannot account series alias", error);
        }
        *out = (netdata_promql_series_open_t){series, {series->labels, physical->label_count + 1}};
        return NETDATA_PROMQL_OK;
    }
    for(;;) {
        if(!promql_read_poll(frame)) return frame->interrupted;
        if(!selection->host) {
            uint32_t status = selection_host(selection, frame, error);
            if(status != NETDATA_PROMQL_OK) return status;
        }
        RRDCONTEXT *context;
        if(!selection->contexts_started) {
            context = traverse_start(&selection->contexts, selection->host->host->rrdctx.contexts);
            selection->contexts_started = true;
        }
        else context = selection->contexts.value;
        if(!context) {
            dictionary_foreach_done(&selection->contexts);
            selection->contexts_started = false;
            promql_host_release(selection->query, selection->host);
            selection->host = NULL;
            continue;
        }
        RRDINSTANCE *instance;
        if(!selection->instances_started) {
            instance = traverse_start(&selection->instances, context->rrdinstances);
            selection->instances_started = true;
        }
        else instance = selection->instances.value;
        if(!instance) {
            dictionary_foreach_done(&selection->instances);
            selection->instances_started = false;
            traverse_next(&selection->contexts);
            continue;
        }
        RRDMETRIC *metric;
        if(!selection->metrics_started) {
            metric = traverse_start(&selection->metrics, instance->rrdmetrics);
            selection->metrics_started = true;
        }
        else metric = selection->metrics.value;
        if(!metric) {
            dictionary_foreach_done(&selection->metrics);
            selection->metrics_started = false;
            traverse_next(&selection->instances);
            continue;
        }
        uint32_t status = NETDATA_PROMQL_OK;
        PROMQL_PHYSICAL *physical = physical_create(selection, context, instance, metric, frame, &status, error);
        traverse_next(&selection->metrics);
        if(status != NETDATA_PROMQL_OK) return status;
        if(!physical) continue;
        PROMQL_SERIES *series = series_create(physical, false, frame->account);
        if(!series) {
            promql_physical_release(physical);
            return promql_provider_error(selection->query->context, "cannot account series labels", error);
        }
        if(strcmp(physical->name, physical->alias)) {
            physical->references++;
            selection->pending_alias = physical;
        }
        *out = (netdata_promql_series_open_t){series, {series->labels, physical->label_count + 1}};
        return NETDATA_PROMQL_OK;
    }
}

void promql_selection_destroy(PROMQL_SELECTION *selection) {
    if(selection->pending_alias) promql_physical_release(selection->pending_alias);
    if(selection->metrics_started) dictionary_foreach_done(&selection->metrics);
    if(selection->instances_started) dictionary_foreach_done(&selection->instances);
    if(selection->contexts_started) dictionary_foreach_done(&selection->contexts);
    if(selection->hosts_started) dictionary_foreach_done(&selection->hosts);
    promql_host_release(selection->query, selection->host);
    for(size_t i = 0; i < selection->prefilter_count; i++) {
        promql_free((void *)selection->prefilters[i].name.data);
        promql_free((void *)selection->prefilters[i].value.data);
    }
    promql_free(selection->prefilters);
    promql_free(selection);
}
