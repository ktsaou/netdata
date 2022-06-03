// SPDX-License-Identifier: GPL-3.0-or-later

#define NETDATA_RRD_INTERNALS
#include "rrd.h"

// ----------------------------------------------------------------------------
// labels_create()

typedef struct label {
    const char *value;
    LABEL_SOURCE label_source;
} LABEL;

static void label_insert_callback(const char *name, void *value, void *data) {
    (void)name;
    RRDHOST *host = (RRDHOST *)data; (void)host;
    LABEL *lb = (LABEL *)value;

    // allocate our own memory for the value
    lb->value = strdupz(lb->value);
    lb->label_source |= LABEL_FLAG_NEW;
}

static void label_delete_callback(const char *name, void *value, void *data) {
    (void)name;
    RRDHOST *host = (RRDHOST *)data; (void)host;
    LABEL *lb = (LABEL *)value;

    freez((void *)lb->value);
    lb->value = NULL;
}

static void label_conflict_callback(const char *name, void *oldvalue, void *newvalue, void *data) {
    (void)name;
    (void)data;
    (void)newvalue;
    LABEL *lb = (LABEL *)oldvalue;
    lb->label_source |= LABEL_FLAG_OLD;
}

DICTIONARY *labels_create(void) {
    DICTIONARY *dict = dictionary_create(DICTIONARY_FLAG_DONT_OVERWRITE_VALUE);
    dictionary_register_insert_callback(dict, label_insert_callback, NULL);
    dictionary_register_delete_callback(dict, label_delete_callback, NULL);
    dictionary_register_conflict_callback(dict, label_conflict_callback, NULL);
    return dict;
}


// ----------------------------------------------------------------------------
// labels_empty()

static int delete_all_labels_callback(const char *name, void *value, void *data) {
    DICTIONARY *dict = (DICTIONARY *)data;
    (void)value;
    dictionary_del_having_write_lock(dict, name);
    return 1;
}

void labels_empty(DICTIONARY *labels_dict) {
    dictionary_walkthrough_write(labels_dict, delete_all_labels_callback, labels_dict);
}


// ----------------------------------------------------------------------------
// labels_destroy()

void labels_destroy(DICTIONARY *labels_dict) {
    dictionary_destroy(labels_dict);
}


// ----------------------------------------------------------------------------
// labels_add()

void labels_add(DICTIONARY *dict, const char *key, const char *value, LABEL_SOURCE label_source) {
    if(!dict) {
        error("%s(): called with NULL dictionary.", __FUNCTION__ );
        return;
    }

    LABEL lab = {
        .label_source = label_source,
        .value = value
    };
    dictionary_set(dict, key, &lab, sizeof(LABEL));
}

// TODO - no one should be using this ever!
// the returned string is not guaranteed to exist after the return of this call
#warning Someone is still using labels_add_the_really_bad_way()

DICTIONARY *labels_add_the_really_bad_way(DICTIONARY *labels, char *key, char *value, LABEL_SOURCE label_source) {
    if(!labels) {
        error("%s(): called with NULL dictionary. Creating one.", __FUNCTION__ );
        labels = labels_create();
    }

    labels_add(labels, key, value, label_source);
    return labels;
}

// ----------------------------------------------------------------------------
// labels_get()

// TODO - no one should be using this ever!
// the returned string is not guaranteed to exist after the return of this call
#warning Someone is still using labels_get()

const char *labels_get(DICTIONARY *head, const char *key) {
    LABEL *lb = dictionary_get(head, key);
    if(lb) return lb->value;
    return NULL;
}


// ----------------------------------------------------------------------------
// labels_unmark_all()
// remove labels LABEL_FLAG_OLD and LABEL_FLAG_NEW from all dictionary items

static int remove_flags_old_new(const char *name, void *value, void *data) {
    (void)name;
    (void)data;

    LABEL *lb = (LABEL *)value;

    if(lb->label_source & LABEL_FLAG_OLD)
        lb->label_source &= ~LABEL_FLAG_OLD;

    if(lb->label_source & LABEL_FLAG_NEW)
        lb->label_source &= ~LABEL_FLAG_NEW;

    return 1;
}

void labels_unmark_all(DICTIONARY *labels) {
    dictionary_walkthrough_read(labels, remove_flags_old_new, NULL);
}


// ----------------------------------------------------------------------------
// labels_remove_all_unmarked()
// remove dictionary items that are neither old, nor new

static int remove_not_old_not_new_callback(const char *name, void *value, void *data) {
    DICTIONARY *dict = (DICTIONARY *)data;
    LABEL *lb = (LABEL *)value;

    if(!(lb->label_source & LABEL_FLAG_OLD) && !(lb->label_source & LABEL_FLAG_NEW)) {
        dictionary_del_having_write_lock(dict, name);
        return 1;
    }

    return 0;
}

void labels_remove_all_unmarked(DICTIONARY *labels) {
    dictionary_walkthrough_write(labels, remove_not_old_not_new_callback, labels);
}


// ----------------------------------------------------------------------------
// labels_walkthrough_read()

struct labels_walkthrough {
    int (*callback)(const char *name, const char *value, LABEL_SOURCE ls, void *data);
    void *data;
};

static int labels_walkthrough_callback(const char *name, void *value, void *data) {
    struct labels_walkthrough *d = (struct labels_walkthrough *)data;
    LABEL *lb = (LABEL *)value;
    return d->callback(name, lb->value, lb->label_source, d->data);
}

int labels_walkthrough_read(DICTIONARY *labels, int (*callback)(const char *name, const char *value, LABEL_SOURCE ls, void *data), void *data) {
    if(!labels) return 0;

    struct labels_walkthrough d = {
        .callback = callback,
        .data = data
    };
    return dictionary_walkthrough_read(labels, labels_walkthrough_callback, &d);
}


// ----------------------------------------------------------------------------
// labels_copy_and_replace_existing()
// migrate an existing label list to a new list, INPLACE

static int copy_label_to_dictionary_callback(const char *name, void *value, void *data) {
    DICTIONARY *dst = (DICTIONARY *)data;
    LABEL *lb = (LABEL *)value;
    labels_add(dst, name, lb->value, lb->label_source);
    return 1;
}

void labels_copy_and_replace_existing(DICTIONARY *dst, DICTIONARY *src) {
    if(!src) {
        error("%s(): called with NULL src dictionary", __FUNCTION__ );
        return;
    }
    if(!dst) {
        error("%s(): called with NULL dst dictionary", __FUNCTION__ );
        return;
    }

    // remove the LABEL_FLAG_OLD and LABEL_FLAG_NEW from all items
    labels_unmark_all(dst);

    // Mark the existing ones as LABEL_FLAG_OLD,
    // or the newly added ones as LABEL_FLAG_NEW
    dictionary_walkthrough_read(src, copy_label_to_dictionary_callback, dst);

    // remove the unmarked dst
    labels_remove_all_unmarked(dst);
}

void labels_copy(DICTIONARY *dst, DICTIONARY *src) {
    dictionary_walkthrough_read(src, copy_label_to_dictionary_callback, dst);
}


// ----------------------------------------------------------------------------
// labels_match_simple_pattern()
// returns true when there are keys in the dictionary matching a simple pattern

static int simple_pattern_match_callback(const char *name, void *value, void *data) {
    (void)value;
    SIMPLE_PATTERN *pattern = (SIMPLE_PATTERN *)data;

    // we return -1 to stop the walkthrough on first match
    if(simple_pattern_matches(pattern, name)) return -1;

    return 0;
}

bool labels_match_simple_pattern(DICTIONARY *labels, char *simple_pattern_txt) {
    SIMPLE_PATTERN *pattern = simple_pattern_create(simple_pattern_txt, ",|\t\r\n\f\v", SIMPLE_PATTERN_EXACT);

    int ret = dictionary_walkthrough_read(labels, simple_pattern_match_callback, pattern);

    simple_pattern_free(pattern);

    return (ret == -1);
}


// ----------------------------------------------------------------------------
// labels_match_key_and_value()
// return true if there is an item in the dictionary matching both
// the key and its value

bool labels_match_key_and_value(DICTIONARY *labels, const char *name, const char *value) {
    LABEL *lb = dictionary_get(labels, name);

    if(strcmp(lb->value, value) == 0)
        return true;

    return false;
}


// ----------------------------------------------------------------------------
// Log all labels

static int labels_log_label_to_buffer_callback(const char *name, void *value, void *data) {
    BUFFER *wb = (BUFFER *)data;
    LABEL *lb = (LABEL *)value;

    buffer_sprintf(wb, "label: \"%s\" = \"%s\" ", name, lb->value);

    if(lb->label_source & LABEL_SOURCE_AUTO)
        buffer_strcat(wb, "(auto)");

    if(lb->label_source & LABEL_SOURCE_NETDATA_CONF)
        buffer_strcat(wb, "(netdata.conf)");

    if(lb->label_source & LABEL_SOURCE_DOCKER)
        buffer_strcat(wb, "(docker)");

    if(lb->label_source & LABEL_SOURCE_ENVIRONMENT)
        buffer_strcat(wb, "(env)");

    if(lb->label_source & LABEL_SOURCE_KUBERNETES)
        buffer_strcat(wb, "(k8s)");

    return 1;
}

static int log_label_callback(const char *name, void *value, void *data) {
    const char *prefix = (const char *)data;
    if(!prefix) prefix = "";

    LABEL *lb = (LABEL *)value;
    BUFFER *wb = buffer_create(200);
    labels_log_label_to_buffer_callback(name, lb, wb);
    info("%s %s", prefix, buffer_tostring(wb));
    buffer_free(wb);

    return 1;
}

void labels_log(DICTIONARY *labels, const char *prefix) {
    dictionary_walkthrough_read(labels, log_label_callback, (void *)prefix);
}

void labels_log_buffer(DICTIONARY *labels, BUFFER *wb) {
    dictionary_walkthrough_read(labels, labels_log_label_to_buffer_callback, wb);
}


// ----------------------------------------------------------------------------
// labels_to_json()

struct labels_to_json {
    BUFFER *wb;
    const char *prefix;
    const char *quote;
    const char *equal;
    const char *comma;
    size_t count;
};

static int label_to_json(const char *name, void *value, void *data) {
    struct labels_to_json *t = (struct labels_to_json *)data;
    LABEL *lb = (LABEL *)value;

    size_t value_len = strlen(lb->value);
    char v[value_len + 1];

    sanitize_json_string(v, (char *)lb->value, value_len);
    buffer_sprintf(t->wb, "%s%s%s%s%s%s%s%s%s", t->count++?t->comma:"", t->prefix, t->quote, name, t->quote, t->equal, t->quote, v, t->quote);
    return 1;
}

void labels_to_json(DICTIONARY *labels, BUFFER *wb, const char *prefix, const char *equal, const char *quote, const char *comma) {
    struct labels_to_json tmp = {
        .wb = wb,
        .prefix = prefix,
        .equal = equal,
        .quote = quote,
        .comma = comma,
        .count = 0
    };
    dictionary_walkthrough_read(labels, label_to_json, (void *)&tmp);
}


// ----------------------------------------------------------------------------
// LABEL INDEX management

void labelsindex_set_to_new_labels(LABEL_INDEX *label_index, DICTIONARY *new_labels) {
    labels_copy_and_replace_existing(label_index->head, new_labels);
    labels_destroy(new_labels);
}


// ----------------------------------------------------------------------------
// string operations related to labels

void strip_last_symbol(char *str, char symbol, SKIP_ESCAPED_CHARACTERS_OPTION skip_escaped_characters) {
    char *end = str;

    while (*end && *end != symbol) {
        if (unlikely(skip_escaped_characters && *end == '\\')) {
            end++;
            if (unlikely(!*end))
                break;
        }
        end++;
    }
    if (likely(*end == symbol))
        *end = '\0';
}

char *strip_double_quotes(char *str, SKIP_ESCAPED_CHARACTERS_OPTION skip_escaped_characters) {
    if (*str == '"') {
        str++;
        strip_last_symbol(str, '"', skip_escaped_characters);
    }

    return str;
}

int labels_is_valid_key(const char *key) {
    // Prometheus exporter
    if(!strcmp(key, "chart") || !strcmp(key, "family")  || !strcmp(key, "dimension")) return 0;

    // Netdata and Prometheus  internal
    if(*key == '_') return 0;

    while(*key) {
        if(!(isdigit(*key) || isalpha(*key) || *key == '.' || *key == '_' || *key == '-')) return 0;
        key++;
    }

    return 1;
}

int labels_is_valid_value(const char *value) {
    while(*value) {
        if(*value == '"' || *value == '\'' || *value == '*' || *value == '!') return 0;
        value++;
    }
    return 1;
}

