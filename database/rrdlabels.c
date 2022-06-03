// SPDX-License-Identifier: GPL-3.0-or-later

#define NETDATA_RRD_INTERNALS
#include "rrd.h"

// ----------------------------------------------------------------------------
// labels sanitization
/*
 * Tags must start with a letter or an underscore and after that may contain:
 *
 * Alphanumerics
 * Underscores
 * Minuses
 * Colons
 * Periods
 * Slashes
 *
 * Other special characters are converted to underscores.
 *
 * Note: A tag cannot end with a colon, for example tag:
 *
 * Tags can be up to 200 characters long. Tags are converted to lowercase.
 */

#define LABELS_MAX_LENGTH 200

static unsigned char labels_char_map[256] = {
    [0] = '\0', //
    [1] = '_', //
    [2] = '_', //
    [3] = '_', //
    [4] = '_', //
    [5] = '_', //
    [6] = '_', //
    [7] = '_', //
    [8] = '_', //
    [9] = '_', //
    [10] = '_', //
    [11] = '_', //
    [12] = '_', //
    [13] = '_', //
    [14] = '_', //
    [15] = '_', //
    [16] = '_', //
    [17] = '_', //
    [18] = '_', //
    [19] = '_', //
    [20] = '_', //
    [21] = '_', //
    [22] = '_', //
    [23] = '_', //
    [24] = '_', //
    [25] = '_', //
    [26] = '_', //
    [27] = '_', //
    [28] = '_', //
    [29] = '_', //
    [30] = '_', //
    [31] = '_', //
    [32] = '_', // SPACE    - convert SPACE to underscore
    [33] = '_', // !
    [34] = '_', // "
    [35] = '_', // #
    [36] = '_', // $
    [37] = '_', // %
    [38] = '_', // &
    [39] = '_', // '
    [40] = '_', // (
    [41] = '_', // )
    [42] = '_', // *
    [43] = '_', // +
    [44] = '.', // ,        - convert , to .
    [45] = '-', // -
    [46] = '.', // .
    [47] = '/', // /
    [48] = '0', // 0
    [49] = '1', // 1
    [50] = '2', // 2
    [51] = '3', // 3
    [52] = '4', // 4
    [53] = '5', // 5
    [54] = '6', // 6
    [55] = '7', // 7
    [56] = '8', // 8
    [57] = '9', // 9
    [58] = ':', // :
    [59] = ':', // ;        - convert ; to :
    [60] = '_', // <
    [61] = ':', // =        - convert = to :
    [62] = '_', // >
    [63] = '_', // ?
    [64] = '_', // @
    [65] = 'a', // A        - convert capitals to lowercase
    [66] = 'b', // B
    [67] = 'c', // C
    [68] = 'd', // D
    [69] = 'e', // E
    [70] = 'f', // F
    [71] = 'g', // G
    [72] = 'h', // H
    [73] = 'i', // I
    [74] = 'j', // J
    [75] = 'k', // K
    [76] = 'l', // L
    [77] = 'm', // M
    [78] = 'n', // N
    [79] = 'o', // O
    [80] = 'p', // P
    [81] = 'q', // Q
    [82] = 'r', // R
    [83] = 's', // S
    [84] = 't', // T
    [85] = 'u', // U
    [86] = 'v', // V
    [87] = 'w', // W
    [88] = 'x', // X
    [89] = 'y', // Y
    [90] = 'z', // Z
    [91] = '_', // [
    [92] = '/', // backslash    - convert \ to /
    [93] = '_', // ]
    [94] = '_', // ^
    [95] = '_', // _
    [96] = '_', // `
    [97] = 'a', // a
    [98] = 'b', // b
    [99] = 'c', // c
    [100] = 'd', // d
    [101] = 'e', // e
    [102] = 'f', // f
    [103] = 'g', // g
    [104] = 'h', // h
    [105] = 'i', // i
    [106] = 'j', // j
    [107] = 'k', // k
    [108] = 'l', // l
    [109] = 'm', // m
    [110] = 'n', // n
    [111] = 'o', // o
    [112] = 'p', // p
    [113] = 'q', // q
    [114] = 'r', // r
    [115] = 's', // s
    [116] = 't', // t
    [117] = 'u', // u
    [118] = 'v', // v
    [119] = 'w', // w
    [120] = 'x', // x
    [121] = 'y', // y
    [122] = 'z', // z
    [123] = '_', // {
    [124] = '_', // |
    [125] = '_', // }
    [126] = '_', // ~
    [127] = '_', //
    [128] = '_', //
    [129] = '_', //
    [130] = '_', //
    [131] = '_', //
    [132] = '_', //
    [133] = '_', //
    [134] = '_', //
    [135] = '_', //
    [136] = '_', //
    [137] = '_', //
    [138] = '_', //
    [139] = '_', //
    [140] = '_', //
    [141] = '_', //
    [142] = '_', //
    [143] = '_', //
    [144] = '_', //
    [145] = '_', //
    [146] = '_', //
    [147] = '_', //
    [148] = '_', //
    [149] = '_', //
    [150] = '_', //
    [151] = '_', //
    [152] = '_', //
    [153] = '_', //
    [154] = '_', //
    [155] = '_', //
    [156] = '_', //
    [157] = '_', //
    [158] = '_', //
    [159] = '_', //
    [160] = '_', //
    [161] = '_', //
    [162] = '_', //
    [163] = '_', //
    [164] = '_', //
    [165] = '_', //
    [166] = '_', //
    [167] = '_', //
    [168] = '_', //
    [169] = '_', //
    [170] = '_', //
    [171] = '_', //
    [172] = '_', //
    [173] = '_', //
    [174] = '_', //
    [175] = '_', //
    [176] = '_', //
    [177] = '_', //
    [178] = '_', //
    [179] = '_', //
    [180] = '_', //
    [181] = '_', //
    [182] = '_', //
    [183] = '_', //
    [184] = '_', //
    [185] = '_', //
    [186] = '_', //
    [187] = '_', //
    [188] = '_', //
    [189] = '_', //
    [190] = '_', //
    [191] = '_', //
    [192] = '_', //
    [193] = '_', //
    [194] = '_', //
    [195] = '_', //
    [196] = '_', //
    [197] = '_', //
    [198] = '_', //
    [199] = '_', //
    [200] = '_', //
    [201] = '_', //
    [202] = '_', //
    [203] = '_', //
    [204] = '_', //
    [205] = '_', //
    [206] = '_', //
    [207] = '_', //
    [208] = '_', //
    [209] = '_', //
    [210] = '_', //
    [211] = '_', //
    [212] = '_', //
    [213] = '_', //
    [214] = '_', //
    [215] = '_', //
    [216] = '_', //
    [217] = '_', //
    [218] = '_', //
    [219] = '_', //
    [220] = '_', //
    [221] = '_', //
    [222] = '_', //
    [223] = '_', //
    [224] = '_', //
    [225] = '_', //
    [226] = '_', //
    [227] = '_', //
    [228] = '_', //
    [229] = '_', //
    [230] = '_', //
    [231] = '_', //
    [232] = '_', //
    [233] = '_', //
    [234] = '_', //
    [235] = '_', //
    [236] = '_', //
    [237] = '_', //
    [238] = '_', //
    [239] = '_', //
    [240] = '_', //
    [241] = '_', //
    [242] = '_', //
    [243] = '_', //
    [244] = '_', //
    [245] = '_', //
    [246] = '_', //
    [247] = '_', //
    [248] = '_', //
    [249] = '_', //
    [250] = '_', //
    [251] = '_', //
    [252] = '_', //
    [253] = '_', //
    [254] = '_', //
    [255] = '_'  //
};


static void labels_sanitization(char *dst, const char *src, size_t dst_size) {
    unsigned char *s = (unsigned char *)src, *d = (unsigned char *)dst;
    dst_size--;

    size_t i = 0;
    while(*s && i++ < dst_size)
        *d++ = labels_char_map[*s++];

    *d = '\0';
}

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

static void labels_add_already_sanitized(DICTIONARY *dict, const char *key, const char *value, LABEL_SOURCE label_source) {
    LABEL tmp = {
        .label_source = label_source,
        .value = value
    };
    dictionary_set(dict, key, &tmp, sizeof(LABEL));
}


void labels_add(DICTIONARY *dict, const char *key, const char *value, LABEL_SOURCE label_source) {
    if(!dict) {
        error("%s(): called with NULL dictionary.", __FUNCTION__ );
        return;
    }

    char k[LABELS_MAX_LENGTH + 1], v[LABELS_MAX_LENGTH + 1];
    labels_sanitization(k, key, LABELS_MAX_LENGTH);
    labels_sanitization(v, value, LABELS_MAX_LENGTH);

    labels_add_already_sanitized(dict, k, v, label_source);
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

int labels_sorted_walkthrough_read(DICTIONARY *labels, int (*callback)(const char *name, const char *value, LABEL_SOURCE ls, void *data), void *data) {
    if(!labels) return 0;

    struct labels_walkthrough d = {
        .callback = callback,
        .data = data
    };
    return dictionary_sorted_walkthrough_read(labels, labels_walkthrough_callback, &d);
}


// ----------------------------------------------------------------------------
// labels_copy_and_replace_existing()
// migrate an existing label list to a new list, INPLACE

static int copy_label_to_dictionary_callback(const char *name, void *value, void *data) {
    DICTIONARY *dst = (DICTIONARY *)data;
    LABEL *lb = (LABEL *)value;
    labels_add_already_sanitized(dst, name, lb->value, lb->label_source);
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

    buffer_sprintf(t->wb, "%s%s%s%s%s%s%s%s%s", t->count++?t->comma:"", t->prefix, t->quote, name, t->quote, t->equal, t->quote, lb->value, t->quote);
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

