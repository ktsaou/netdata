
#include "../libnetdata.h"

#define YAML_MAX_LINE (1024 * 64)
#define YAML_MAX_NESTING 1024
#define YAML_MAX_STRING_LITERAL (64 * 1024)

// ----------------------------------------------------------------------------
// hashtable for CFG_KEY

// cleanup hashtable defines
#undef SIMPLE_HASHTABLE_SORT_FUNCTION
#undef SIMPLE_HASHTABLE_VALUE_TYPE
#undef SIMPLE_HASHTABLE_NAME
#undef NETDATA_SIMPLE_HASHTABLE_H

struct cfg_node;
static inline int compare_cfg_nodes(struct cfg_node *n1, struct cfg_node *n2);
#define SIMPLE_HASHTABLE_SORT_FUNCTION compare_cfg_nodes
#define SIMPLE_HASHTABLE_VALUE_TYPE struct cfg_node
#define SIMPLE_HASHTABLE_NAME _CFG_NODE
#define SIMPLE_HASHTABLE_SAMPLE_IMPLEMENTATION 1
#include "../simple_hashtable.h"

// ----------------------------------------------------------------------------

typedef struct cfg_node CFG_NODE;
typedef struct cfg_value CFG_VALUE;

static inline void cfg_value_cleanup(CFG_VALUE *v);
static inline XXH64_hash_t cfg_node_hash(CFG_NODE *n);

// ----------------------------------------------------------------------------

typedef struct cfg_key {
    char *name;
    XXH64_hash_t hash;
    size_t len;
} CFG_KEY;

static inline void cfg_key_cleanup(CFG_KEY *k) {
    assert(k);
    freez(k->name);

    memset(k, 0, sizeof(*k));
}

static inline void cfg_key_init(CFG_KEY *k, const char *key, size_t key_len) {
    assert(key);

    cfg_key_cleanup(k);

    if(key && *key && key_len) {
        k->name = strndupz(key, key_len);
        k->len = key_len;
        k->hash = XXH3_64bits(k->name, k->len);
    }
}

static inline XXH64_hash_t cfg_key_hash(CFG_KEY *k) {
    return k->hash;
}

// ----------------------------------------------------------------------------

typedef enum {
    CFG_NODE_ID_TYPE_NONE = 0,
    CFG_NODE_ID_TYPE_NAMED,
    CFG_NODE_ID_TYPE_NUMBERED,
} CFG_NODE_ID_TYPE;

typedef struct cfg_node_id {
    CFG_NODE_ID_TYPE type;
    union {
        CFG_KEY key;
        uint32_t number;
    };
} CFG_NODE_ID;

static inline void cfg_node_id_cleanup(CFG_NODE_ID *id) {
    assert(id);

    if(id->type == CFG_NODE_ID_TYPE_NAMED)
        cfg_key_cleanup(&id->key);

    memset(id, 0, sizeof(*id));
}

static inline bool cfg_node_id_set_named(CFG_NODE_ID *id, const char *key, size_t key_len) {
    assert(id);

    if(id->type != CFG_NODE_ID_TYPE_NONE)
        return false;

    cfg_node_id_cleanup(id);

    id->type = CFG_NODE_ID_TYPE_NAMED;
    cfg_key_init(&id->key, key, key_len);
    return true;
}

static inline bool cfg_node_id_set_numbered(CFG_NODE_ID *id, size_t number) {
    assert(id);

    if(id->type != CFG_NODE_ID_TYPE_NONE)
        return false;

    cfg_node_id_cleanup(id);

    id->type = CFG_NODE_ID_TYPE_NUMBERED;
    id->number = number;
    return true;
}

static inline XXH64_hash_t cfg_node_id_hash(CFG_NODE_ID *id) {
    return cfg_key_hash(&id->key);
}

// ----------------------------------------------------------------------------

typedef struct cfg_value_map {
    SIMPLE_HASHTABLE_CFG_NODE hashtable;
} CFG_VALUE_MAP;

static inline void cfg_value_map_cleanup(CFG_VALUE_MAP *map);
static inline void cfg_value_map_init(CFG_VALUE_MAP *map) {
    assert(map);
    cfg_value_map_cleanup(map);
    simple_hashtable_init_CFG_NODE(&map->hashtable, 1);
}

static inline void cfg_value_map_add_child(CFG_VALUE_MAP *map, CFG_NODE *child) {
    SIMPLE_HASHTABLE_SLOT_CFG_NODE *sl = simple_hashtable_get_slot_CFG_NODE(&map->hashtable, cfg_node_hash(child), true);
    simple_hashtable_set_slot_CFG_NODE(&map->hashtable, sl, cfg_node_hash(child), child);
}

// ----------------------------------------------------------------------------

typedef struct cfg_value_array {
    uint32_t size;
    uint32_t used;
    CFG_NODE **array;
} CFG_VALUE_ARRAY;

static inline void cfg_value_array_cleanup(CFG_VALUE_ARRAY *arr);
static inline void cfg_value_array_init(CFG_VALUE_ARRAY *arr) {
    assert(arr);
    cfg_value_array_cleanup(arr);
    // no need to initialize anything
}

static inline void cfg_value_array_add_child(CFG_VALUE_ARRAY *arr, CFG_NODE *child);

// ----------------------------------------------------------------------------

typedef enum {
    CFG_NON = 0,
    CFG_TXT,
    CFG_U64,
    CFG_I64,
    CFG_DBL,
    CFG_BLN,
    CFG_MAP,
    CFG_ARR,
    CFG_LNK,
} CFG_VALUE_TYPE;

struct cfg_value {
    CFG_VALUE_TYPE type;
    union {
        char           *txt;  // for strings
        uint64_t        u64;
        int64_t         i64;
        double          dbl;  // for doubles
        bool            bln;  // for booleans
        CFG_VALUE_MAP   map;
        CFG_VALUE_ARRAY arr;
    } value;
};

static inline const char *cfg_value_type(CFG_VALUE_TYPE type) {
    switch(type) {
        default:
        case CFG_NON:
            return "empty";

        case CFG_TXT:
            return "text";

        case CFG_U64:
            return "unsigned integer";

        case CFG_I64:
            return "signed integer";

        case CFG_DBL:
            return "double";

        case CFG_BLN:
            return "boolean";

        case CFG_MAP:
            return "map";

        case CFG_ARR:
            return "array";

        case CFG_LNK:
            return "link";
    }
}

static inline void cfg_value_cleanup(CFG_VALUE *v) {
    assert(v);

    switch(v->type) {
        case CFG_TXT:
            freez(v->value.txt);
            v->value.txt = NULL;
            break;

        case CFG_NON:
        case CFG_U64:
        case CFG_I64:
        case CFG_DBL:
        case CFG_BLN:
            break;

        case CFG_MAP:
            cfg_value_map_cleanup(&v->value.map);
            break;

        case CFG_ARR:
            cfg_value_array_cleanup(&v->value.arr);
            break;

        case CFG_LNK:
            // FIXME implement link cleanup
            break;
    }

    memset(v, 0, sizeof(*v));
}

static inline bool cfg_value_make_array(CFG_VALUE *v) {
    if(v->type != CFG_NON && v->type != CFG_ARR)
        return false;

    if(v->type == CFG_ARR)
        return true;

    v->type = CFG_ARR;
    cfg_value_array_init(&v->value.arr);
    return true;
}

static inline bool cfg_value_make_map(CFG_VALUE *v) {
    if(v->type != CFG_NON && v->type != CFG_MAP)
        return false;

    if(v->type == CFG_MAP)
        return true;

    v->type = CFG_MAP;
    cfg_value_map_init(&v->value.map);
    return true;
}

static inline bool cfg_value_done(CFG_VALUE *v) {
    return v->type != CFG_NON;
}

static inline bool cfg_value_add_child(CFG_VALUE *v, CFG_NODE *child);

static inline bool cfg_value_set_literal(CFG_VALUE *v, const char *s, size_t len) {
    assert(v || s || len);

    if(v->type != CFG_NON)
        return false;

    cfg_value_cleanup(v);

    v->type = CFG_TXT;
    v->value.txt = strndupz(s, len);

    return true;
}

// ----------------------------------------------------------------------------

struct cfg_node {
    CFG_NODE_ID id;
    CFG_VALUE value;       // Node value
};

static inline void cfg_node_cleanup(CFG_NODE *n) {
    assert(n);

    cfg_node_id_cleanup(&n->id);
    cfg_value_cleanup(&n->value);
    memset(n, 0, sizeof(*n));
}

static inline void cfg_node_init(CFG_NODE *n) {
    assert(n);
    cfg_node_cleanup(n);
}

static inline CFG_NODE *cfg_node_create(void) {
    return callocz(1, sizeof(CFG_NODE));
}

static inline void cfg_node_free(CFG_NODE *n) {
    if(!n) return;

    cfg_node_cleanup(n);
    freez(n);
}

#define CLEANUP_CFG_NODE _cleanup_(cfg_node_freep) CFG_NODE
static inline void cfg_node_freep(CFG_NODE **nptr) {
    if(nptr) cfg_node_free(*nptr);
}

static inline bool cfg_node_make_array(CFG_NODE *n) {
    return cfg_value_make_array(&n->value);
}

static inline bool cfg_node_make_map(CFG_NODE *n) {
    return cfg_value_make_map(&n->value);
}

static inline bool cfg_node_done(CFG_NODE *n) {
    return cfg_value_done(&n->value);
}

static inline bool cfg_node_add_child(CFG_NODE *n, CFG_NODE *child) {
    return cfg_value_add_child(&n->value, child);
}

static inline XXH64_hash_t cfg_node_hash(CFG_NODE *n) {
    return cfg_node_id_hash(&n->id);
}

static inline bool cfg_node_set_name(CFG_NODE *n, const char *key, size_t key_len) {
    return cfg_node_id_set_named(&n->id, key, key_len);
}

static inline bool cfg_node_set_literal(CFG_NODE *n, const char *s, size_t len) {
    return cfg_value_set_literal(&n->value, s, len);
}

static inline int compare_cfg_nodes(struct cfg_node *n1, struct cfg_node *n2) {
    assert(n1->id.type == n2->id.type);
    assert(n1->id.type == CFG_NODE_ID_TYPE_NAMED || n1->id.type == CFG_NODE_ID_TYPE_NUMBERED);

    if(n1->id.type == CFG_NODE_ID_TYPE_NAMED)
        return strcmp(n1->id.key.name, n2->id.key.name);

    int a = (int)n1->id.number;
    int b = (int)n2->id.number;

    return a - b;
}

// ----------------------------------------------------------------------------
// functions that need to have visibility on all structure definitions

static inline void cfg_value_array_cleanup(CFG_VALUE_ARRAY *arr) {
    assert(arr);
    for(size_t i = 0; i < arr->used ;i++) {
        cfg_node_cleanup(arr->array[i]);
        freez(arr->array[i]);
    }

    freez(arr->array);
    memset(arr, 0, sizeof(*arr));
}

static inline void cfg_value_map_cleanup(CFG_VALUE_MAP *map) {
    assert(map);

    SIMPLE_HASHTABLE_FOREACH_READ_ONLY(&map->hashtable, sl, _CFG_NODE) {
        CFG_NODE *n = SIMPLE_HASHTABLE_FOREACH_READ_ONLY_VALUE(sl);
        // the order of these statements is important!
        simple_hashtable_del_slot_CFG_NODE(&map->hashtable, sl); // remove any references to n
        cfg_node_cleanup(n); // cleanup all the internals of n
        freez(n); // free n
    }

    simple_hashtable_destroy_CFG_NODE(&map->hashtable);
    memset(map, 0, sizeof(*map));
}

static inline void cfg_value_array_add_child(CFG_VALUE_ARRAY *arr, CFG_NODE *child) {
    if(arr->size <= arr->used) {
        size_t new_size = arr->size ? arr->size * 2 : 1;
        arr->array = reallocz(arr->array, new_size * sizeof(CFG_NODE *));
        arr->size = new_size;
    }

    arr->array[arr->used++] = child;
}

static inline bool cfg_value_add_child(CFG_VALUE *v, CFG_NODE *child) {
    switch(v->type) {
        case CFG_ARR:
            if(child->id.type != CFG_NODE_ID_TYPE_NUMBERED) return false;
            cfg_value_array_add_child(&v->value.arr, child);
            return true;

        case CFG_MAP:
            if(child->id.type != CFG_NODE_ID_TYPE_NAMED) return false;
            cfg_value_map_add_child(&v->value.map, child);
            return true;

        default:
            return false;
    }

    return false;
}

// ----------------------------------------------------------------------------

typedef struct cfg {
    CFG_NODE *root;
    SIMPLE_HASHTABLE_CFG_NODE hashtable;
} CFG;

static inline void cfg_cleanup(CFG *cfg) {
    assert(cfg);

    simple_hashtable_destroy_CFG_NODE(&cfg->hashtable);
    cfg_node_free(cfg->root);

    memset(cfg, 0, sizeof(*cfg));
}

static inline void cfg_init(CFG *cfg) {
    assert(cfg);

    cfg_cleanup(cfg);
    simple_hashtable_init_CFG_NODE(&cfg->hashtable, 1);
}

// ----------------------------------------------------------------------------

typedef struct yaml_parser_current_pos {
    size_t line;
    size_t indent;
    size_t pos;
    const char *line_start;
    const char *s;
} YAML_POS;

typedef struct yaml_parser_stack_entry {
    CFG_NODE *node;
    YAML_POS pos;
} YAML_STACK;

typedef struct yaml_parser_state {
    const char *txt;
    size_t txt_len;

    YAML_POS current;
    BUFFER *literal;

    struct {
        YAML_STACK *array;
        size_t depth;
        size_t size;
    } stack;

    CFG *cfg;

    char error[1024];
} YAML_PARSER;

static bool yaml_error(YAML_PARSER *yp, const char *fmt, ...) {
    va_list args;
    va_start(args, fmt);
    buffer_vsprintf(yp->error, fmt, args);
    va_end(args);
    return false;
}

static inline void yaml_push(YAML_PARSER *yp, CFG_NODE *node) {
    assert(node);

    // do not add the same node twice in the stack
    assert(!yp->stack.depth || yp->stack.array[yp->stack.depth - 1].node != node);

    if(yp->stack.depth >= yp->stack.size) {
        size_t size = yp->stack.size ? yp->stack.size * 2 : 2;
        yp->stack.array = reallocz(yp->stack.array, size * sizeof(*yp->stack.array));
        yp->stack.size = size;
    }

    yp->stack.array[yp->stack.depth++] = (struct yaml_parser_stack_entry) {
        .node = node,
        .pos = yp->current,
    };
}

static inline void yaml_pop(YAML_PARSER *yp) {
    assert(yp->stack.depth);

    // find the first node that has indentation more than the current
    for(size_t i = 0; i < yp->stack.depth ;i++) {
        if(yp->stack.array[i].pos.indent > yp->current.indent) {
            yp->stack.array[i] = (struct yaml_parser_stack_entry){
                .node = NULL,
                .pos = { 0 },
            };
            yp->stack.depth = i;
            return;
        }
    }
}

static inline CFG_NODE *yaml_parent_node(YAML_PARSER *yp) {
    assert(yp->stack.depth);
    return yp->stack.array[yp->stack.depth - 1].node;
}

static inline size_t yaml_parent_indent(YAML_PARSER *yp) {
    assert(yp->stack.depth);
    return yp->stack.array[yp->stack.depth - 1].pos.indent;
}

const char *yaml_current_pos(YAML_PARSER *yp) {
    assert(yp->current.pos <= yp->txt_len);
    return &yp->txt[yp->current.pos];
}

static inline void yaml_update_pos(YAML_PARSER *yp, const char *next) {
    assert(next >= yp->txt && next <= &yp->txt[yp->txt_len]);
    yp->current.pos += next - yaml_current_pos(yp);
}

static inline void yaml_newline(YAML_PARSER *yp, const char *s) {
    assert(*s == '\n');

    yp->current.line++;
    yp->current.line_start = s;
    yp->current.indent = 0;

    while(*++s == ' ')
        yp->current.indent++;
}

typedef enum {
    TOKEN_FINISHED = 0,
    TOKEN_SAME_LINE,
    TOKEN_OTHER_LINE,
} NEXT_TOKEN;

static inline NEXT_TOKEN yaml_find_token_start(YAML_PARSER *yp) {
    const char *s = yaml_current_pos(yp);
    bool at_line_start = (s == yp->current.line_start);
    NEXT_TOKEN ret = token;

    while(*s) {
        if(*s == '\n') {
            // line changed
            yp->current.line++;
            yp->current.line_start = ++s;
            yp->current.indent = 0;
            at_line_start = true;
            ret = TOKEN_KEYWORD;
        }

        else if(*s == ' ') {
            // a space
            if(at_line_start)
                yp->current.indent++;

            s++;
        }

        else if(*s == '#') {
            // skip the comment
            while(*s && *s != '\n')
                s++;
        }

        else
            break;
    }

    yp->current.pos += s - yaml_current_pos(yp);

    return (!*s) ? TOKEN_FINISHED : ret;
}

static inline void yaml_skip_spaces(YAML_PARSER *yp) {
    const char *s = yaml_current_pos(yp);
    while(*s == ' ' || *s == '\t') s++;
    yaml_update_pos(yp, s);
}

static inline const bool *yaml_unquoted_value_stop_map(void) {
    static const bool map[256] = {
            ['#'] = true,
            ['\n'] = true,
    };

    return map;
}

static inline bool yaml_set_literal(YAML_PARSER *yp, const char *s, size_t len, CFG_NODE *node, bool keyword) {
    if(keyword) {
        // set it as keyword

        if(!*s || !len) {
            snprintf(yp->error, sizeof(yp->error), "empty keyword");
            return false;
        }

        if(!cfg_node_set_name(node, s, len)) {
            snprintf(yp->error, sizeof(yp->error), "cannot set node keyword to '%.*s'",
                     (int)len, s);
            return false;
        }
    }
    else {
        // set is as value

        if(*s && len) {
            if (!cfg_node_set_literal(node, s, len)) {
                snprintf(yp->error, sizeof(yp->error), "cannot set node string value to '%.*s'",
                         (int)len, s);
                return false;
            }
        }
    }
    return true;
}

static inline char *yaml_static_thread_buffer(size_t *size) {
    static __thread char buffer[YAML_MAX_STRING_LITERAL + 1];
    *size = sizeof(buffer);
    return buffer;
}

static inline bool yaml_parse_single_quoted_string(YAML_PARSER *yp, CFG_NODE *node, bool keyword) {
    const char *s = yaml_current_pos(yp) + 1; // Skip the initial single quote

    size_t buffer_size;
    char *buffer = yaml_static_thread_buffer(&buffer_size);
    size_t buffer_len = 0;
    bool single_quote_escaped = false;

    while (*s && buffer_len < buffer_size - 1) {
        if (*s == '\'') {
            if (*(s + 1) == '\'') {
                // Handle escaped single quote ('')
                if (!single_quote_escaped && buffer_len < buffer_size - 1) {
                    buffer[buffer_len++] = *s;
                    single_quote_escaped = true;
                } else {
                    single_quote_escaped = false;
                }
                s++;
            } else {
                // End of single-quoted string
                buffer[buffer_len] = '\0';
                yaml_update_pos(yp, s + 1);
                return yaml_set_literal(yp, buffer, buffer_len, node, keyword);
            }
        } else {
            // Regular character
            buffer[buffer_len++] = *s;
            single_quote_escaped = false;

            if (*s == '\n') {
                // Update line counter and line start
                yp->current.line++;
                yp->current.line_start = s + 1;
            }
        }
        s++;
    }

    // Check for buffer overflow or unterminated string
    if (buffer_len >= buffer_size - 1) {
        snprintf(yp->error, sizeof(yp->error), "string literal too long");
        return false;
    }
    if (*s != '\'') {
        snprintf(yp->error, sizeof(yp->error), "unterminated single-quoted string");
        return false;
    }

    return true;
}

size_t parse_utf8_surrogate(const char *s, char *d, size_t *remaining);

static inline bool yaml_parse_double_quoted_string(YAML_PARSER *yp, CFG_NODE *node, bool keyword) {
    const char *s = yaml_current_pos(yp) + 1; // Skip the initial double quote

    size_t buffer_size;
    char *buffer = yaml_static_thread_buffer(&buffer_size);
    size_t buffer_len = 0;

    while (*s && buffer_len < buffer_size - 1) {
        if (*s == '\\') {
            s++; // Move past the backslash
            if (*s == 'u' || *s == 'U') {
                size_t old_remaining = buffer_size - buffer_len;
                size_t consumed = parse_utf8_surrogate(s - 1, buffer + buffer_len, &old_remaining);
                if (consumed > 0) {
                    s += consumed - 1; // Adjust s based on characters consumed
                    buffer_len = buffer_size - old_remaining; // Update buffer_len
                    continue;
                } else {
                    // Handle error or unrecognized sequence
                    snprintf(yp->error, sizeof(yp->error), "invalid Unicode escape sequence");
                    return false;
                }
            } else {
                // Handle other escape sequences
                switch (*s) {
                    case 'n':
                        if (buffer_len < buffer_size - 1) {
                            buffer[buffer_len++] = '\n';
                        }
                        break;
                    case 't':
                        if (buffer_len < buffer_size - 1) {
                            buffer[buffer_len++] = '\t';
                        }
                        break;
                    case '\n':
                        yp->current.line++;
                        yp->current.line_start = s + 1;
                        break;
                    default:
                        if (buffer_len < buffer_size - 1) {
                            buffer[buffer_len++] = *s;
                        }
                }
            }
            s++;
        } else if (*s == '"') {
            // End of double-quoted string
            buffer[buffer_len] = '\0'; // Null-terminate the string
            yaml_update_pos(yp, s + 1); // Update position to after the closing quote
            return yaml_set_literal(yp, buffer, buffer_len, node, keyword);
        } else {
            // Regular character
            if (buffer_len < buffer_size - 1) {
                buffer[buffer_len++] = *s;
            }
            s++;
        }
    }

    // Check for buffer overflow or unterminated string
    if (buffer_len >= buffer_size - 1) {
        snprintf(yp->error, sizeof(yp->error), "string literal too long");
        return false;
    }
    if (*s != '"') {
        snprintf(yp->error, sizeof(yp->error), "unterminated double-quoted string");
        return false;
    }

    return true;
}

static inline bool yaml_parse_block_string(YAML_PARSER *yp, CFG_NODE *node, bool keyword) {
    const char *s = yaml_current_pos(yp) + 1; // Skip the initial '|'

    // Handle '+' or '-' following the '|'
    bool keep_newlines = false;
    if (*s == '+' || *s == '-') {
        keep_newlines = (*s == '+');
        s++;
    }

    // Handle explicit indentation level (if present)
    size_t explicit_indent = 0;
    while (*s >= '0' && *s <= '9') {
        explicit_indent = explicit_indent * 10 + (*s - '0');
        s++;
    }

    size_t remaining;
    char *d = yaml_static_thread_buffer(&remaining);
    char *b = d;

    // Determine the effective indentation level
    size_t block_indent = explicit_indent > 0 ? explicit_indent : yp->current.indent;

    // Skip the current line (up to the next newline)
    while (*s != '\n' && *s != '\0') s++; // FIXME there may be invalid characters in the remaining line
    if (*s == '\n') {
        yp->current.line++;
        yp->current.line_start = ++s;
    }

    while (*s && remaining) {
        // Check the line's indentation
        size_t current_indent = 0;
        while (*s == ' ') {
            current_indent++;
            s++;
        }

        // Check for the end of the block based on indentation
        if (*s && *s != '\n' && current_indent <= block_indent)
            break; // This line is not part of the block

        // Add the line to the buffer
        while (*s != '\n' && *s != '\0' && remaining) {
            *d++ = *s++;
            remaining--;
        }

        // Correctly handle the newline character
        if (*s == '\n') {
            if (keep_newlines && remaining) {
                *d++ = '\n';
                remaining--;
            }

            yp->current.line++;
            yp->current.line_start = ++s;
        }
    }

    // Check for buffer overflow
    if (!remaining) {
        snprintf(yp->error, sizeof(yp->error), "block string too long");
        return false;
    }

    // Remove trailing newlines if `|-` is used
    if (!keep_newlines) {
        while (d > b && *(d - 1) == '\n') {
            d--;
            remaining++;
        }
    }

    *d = '\0'; // Null-terminate the string
    yaml_update_pos(yp, s); // Update position to the end of the block string
    return yaml_set_literal(yp, b, d - b, node, keyword);
}

static inline bool yaml_parse_folded_block_string(YAML_PARSER *yp, CFG_NODE *node, bool keyword) {
    const char *s = yaml_current_pos(yp) + 1; // Skip the initial '>'

    // Handle '+' or '-' following the '>'
    bool keep_newlines = false;
    if (*s == '+' || *s == '-') {
        keep_newlines = (*s == '+');
        s++;
    }

    // Handle explicit indentation level (if present)
    size_t explicit_indent = 0;
    while (*s >= '0' && *s <= '9') {
        explicit_indent = explicit_indent * 10 + (*s - '0');
        s++;
    }

    size_t buffer_size;
    char *buffer = yaml_static_thread_buffer(&buffer_size);
    size_t buffer_len = 0;

    // Determine the effective indentation level
    size_t block_indent = explicit_indent > 0 ? explicit_indent : yp->current.indent;

    // Skip the current line (up to the next newline)
    while (*s != '\n' && *s != '\0') s++;
    if (*s == '\n') s++;

    bool previous_was_newline = false;
    while (*s) {
        // Check the line's indentation
        size_t current_indent = 0;
        while (*s == ' ') {
            current_indent++;
            s++;
        }

        // Check for the end of the block based on indentation
        if (*s != '\n' && *s != '\0' && current_indent < block_indent) {
            break; // This line is not part of the block
        }

        if (*s == '\n') {
            // Handle newlines
            if (previous_was_newline || current_indent > block_indent) {
                // Preserve newline
                if (buffer_len < buffer_size - 1) {
                    buffer[buffer_len++] = '\n';
                }
            } else if (buffer_len > 0 && buffer[buffer_len - 1] != ' ') {
                // Convert single newline to space
                buffer[buffer_len++] = ' ';
            }
            previous_was_newline = true;
        } else {
            // Add regular character to buffer
            if (buffer_len < buffer_size - 1) {
                buffer[buffer_len++] = *s;
            }
            previous_was_newline = false;
        }
        s++;

        // Update line counter and line start
        if (*s == '\n' || *s == '\0') {
            yp->current.line++;
            yp->current.line_start = s;
        }
    }

    // Remove trailing newlines if `|-` is used
    if (!keep_newlines) {
        while (buffer_len > 0 && buffer[buffer_len - 1] == '\n') {
            buffer_len--;
        }
    }

    // Check for buffer overflow
    if (buffer_len >= buffer_size - 1) {
        snprintf(yp->error, sizeof(yp->error), "folded block string too long");
        return false;
    }

    buffer[buffer_len] = '\0'; // Null-terminate the string
    yaml_update_pos(yp, s); // Update position to the end of the block string
    return yaml_set_literal(yp, buffer, buffer_len, node, keyword);
}

static inline bool yaml_parse_unquoted_string(YAML_PARSER  *yp, const bool *stop_map, CFG_NODE *node, bool keyword) {
    const char *s = yaml_current_pos(yp);
    const char *start = s;

    // jump to a stop char
    while(*s && !stop_map[(uint8_t)*s])
        s++;

    // remove trailing spaces
    while(s > start && *(s - 1) == ' ')
        s--;

    yaml_update_pos(yp, s);
    return yaml_set_literal(yp, start, s - start, node, keyword);
}

static inline bool yaml_parse_string_literal(YAML_PARSER *yp, const bool *stop_map, CFG_NODE *node, bool keyword) {
    yaml_skip_spaces(yp);
    const char *s = yaml_current_pos(yp);

    switch(*s) {
        case '\'':
            return yaml_parse_single_quoted_string(yp, node, keyword);

        case '"':
            return yaml_parse_double_quoted_string(yp, node, keyword);

        case '|':
            return yaml_parse_block_string(yp, node, keyword);

        case '>':
            return yaml_parse_folded_block_string(yp, node, keyword);

        default:
            return yaml_parse_unquoted_string(yp, stop_map, node, keyword);
    }
}

static inline bool yaml_parse_inline_array(YAML_PARSER  *yp, CFG_NODE *node) {
    return false;
}

static inline bool yaml_parse_inline_map(YAML_PARSER  *yp, CFG_NODE *node) {
    return false;
}

static inline bool yaml_parse_value(YAML_PARSER  *yp, CFG_NODE *node) {
    assert(node);

    const char *s = yaml_current_pos(yp);

    switch(*s) {
        case '[':
            return yaml_parse_inline_array(yp, node);

        case '{':
            return yaml_parse_inline_map(yp, node);

//        case '&':
//            return yaml_parse_label(yp);
//
//        case '*':
//            return yaml_parse_pointer(yp);

        default:
            return yaml_parse_string_literal(yp, yaml_unquoted_value_stop_map(), node, false);
    }
}

static inline bool yaml_parse_child(YAML_PARSER *yp) {
    if(*yaml_current_pos(yp) == '-') {
        CFG_NODE *parent = yaml_parent_node(yp);
        if(!cfg_node_make_array(parent))
            return yaml_error(yp, "parent object is a %s; cannot switch it to %s",
                              cfg_value_type(parent->value.type), cfg_value_type(CFG_ARR));

        yaml_update_pos(yp, yaml_current_pos(yp) + 1);

        if(yaml_find_token_start(yp) == TOKEN_FINISHED)
            return yaml_error(yp, "there is a hanging - at the end");

        yp->current.indent = yaml_current_pos(yp) - yp->current.line_start;
    }
    else {
        // we need the parent to become a map
        CFG_NODE *parent = yaml_parent_node(yp);
        if(!cfg_node_make_map(parent)) {
            return yaml_error(yp, "parent object is a %s; cannot switch it to %s",
                              cfg_value_type(parent->value.type), cfg_value_type(CFG_MAP));
        }
    }
}

static inline bool yaml_parse_indented_array_values(YAML_PARSER *yp) {
    YAML_POS pos = yp->current;

    while(yaml_find_token_start(yp) != TOKEN_FINISHED && yp->current.indent >= pos.indent) {
        if(yp->current.indent > pos.indent) {
            if(!yaml_parse_child(yp))
                return false;
            continue;
        }
    }

    assert(yp->current.indent < pos.indent);

}

static inline bool yaml_parse_indented_key(YAML_PARSER *yp) {
    static const bool stop_map[256] = {
            ['#'] = true,
            [':'] = true,
            ['\n'] = true,
    };

    if(yaml_find_token_start(yp) == TOKEN_FINISHED)
        return true;

    yaml_pop(yp); // pop the parent of the current indentation

    bool with_dash = false;
    YAML_POS before_dash = yp->current;
    YAML_POS key_start = yp->current;

    if(*yaml_current_pos(yp) == '-') {
        with_dash = true;

        // we need the parent to become an array
        CFG_NODE *parent = yaml_parent_node(yp);
        if(!cfg_node_make_array(parent)) {
            snprintf(yp->error, sizeof(yp->error), "parent object is a %s; cannot switch it to %s",
                     cfg_value_type(parent->value.type), cfg_value_type(CFG_ARR));
            return false;
        }

        yaml_update_pos(yp, yaml_current_pos(yp) + 1);

        if(yaml_find_token_start(yp) == TOKEN_FINISHED) {
            snprintf(yp->error, sizeof(yp->error), "there is a hanging - at the end");
            return false;
        }

        yp->current.indent = yaml_current_pos(yp) - yp->current.line_start;
        key_start = yp->current;
    }
    else {
        // we need the parent to become a map
        CFG_NODE *parent = yaml_parent_node(yp);
        if(!cfg_node_make_map(parent)) {
            if(!yp->error[0])
                snprintf(yp->error, sizeof(yp->error), "parent object is a %s; cannot switch it to %s",
                         cfg_value_type(parent->value.type), cfg_value_type(CFG_MAP));
            return false;
        }
    }

    CLEANUP_CFG_NODE *node = cfg_node_create();

    if(!yaml_parse_string_literal(yp, stop_map, node, true))
        return false;

    // skip spaces
    yaml_find_token_start(yp);

    if(yaml_parent_node(yp)->value.type == CFG_ARR) {
        if(*yaml_current_pos(yp) == ':') {

        }
    }

    yaml_skip_spaces(yp);
    const char *s = yaml_current_pos(yp);
    if(*s != ':') {
        snprintf(yp->error, sizeof(yp->error), "missing colon");
        return false;
    }
    yaml_update_pos(yp, ++s);

    return true;
}

static inline CFG *cfg_parse_yaml(const char *txt, size_t len) {
    YAML_PARSER yaml_parser = {
            .cfg = callocz(1, sizeof(CFG)),
    };
    YAML_PARSER *yp = &yaml_parser;

    cfg_init(yp->cfg);

    // push the parent node into the stack
    yp->cfg->root = cfg_node_create();
    yaml_push(yp, yp->cfg->root);

    yp->txt = txt;
    yp->txt_len = len;
    yp->current.line_start = txt;
    yp->current.line = 1;
    yp->current.literal = buffer_create(0, NULL);
    size_t iteration = 0;

    while(yaml_find_token_start(yp) != TOKEN_FINISHED) {
        CFG_VALUE *yaml_parse_key(yp);




    }

    while(yaml_find_token_start(yp, TOKEN_KEYWORD) == TOKEN_KEYWORD) {
        // expect a keyword or -

        iteration++;

        if (yp->current.indent == yaml_parent_indent(yp)) {
            if(iteration > 1)
                yaml_pop(yp);
        }
        else if (yp->current.indent < yaml_parent_indent(yp)) {
            yaml_pop(yp);
        }

        const char *s = yaml_current_pos(yp);
        if(*s == '-') {
            // the parent node must be an array
            CFG_NODE *parent = yaml_parent_node(yp);
            if(!cfg_node_make_array(parent)) {
                if(!yp->error[0])
                    snprintf(yp->error, sizeof(yp->error), "parent object is a %s; cannot switch it to %s",
                             cfg_value_type(parent->value.type), cfg_value_type(CFG_ARR));
                goto cleanup;
            }

            yp->current.pos++;

            if(yaml_find_token_start(yp, TOKEN_KEYWORD) != TOKEN_KEYWORD) {
                if(!yp->error[0])
                    snprintf(yp->error, sizeof(yp->error), "a - is handing around");
                goto cleanup;
            }
        }
        else {
            // the parent node must be a map
            CFG_NODE *parent = yaml_parent_node(yp);
            if(!cfg_node_make_map(parent)) {
                if(!yp->error[0])
                    snprintf(yp->error, sizeof(yp->error), "parent object is a %s; cannot switch it to %s",
                            cfg_value_type(parent->value.type), cfg_value_type(CFG_MAP));
                goto cleanup;
            }
        }

        CLEANUP_CFG_NODE *node = cfg_node_create();
        if(!yaml_parse_indented_key(yp, node)) {
            // no, this is not a key

            CFG_NODE *parent = yaml_parent_node(yp);
            if(parent->value.type == CFG_ARR) {
                // the parent is an array, and this is a value for the array
                if(!yaml_parse_value(yp, node)) {
                    if (!yp->error[0])
                        snprintf(yp->error, sizeof(yp->error), "failed to parse value");
                    goto cleanup;
                }
            }
            else {
                if (!yp->error[0])
                    snprintf(yp->error, sizeof(yp->error), "no keyword");
                goto cleanup;
            }
        }
        else {
            // the key has been parsed and a colon has been found

            CFG_NODE *parent = yaml_parent_node(yp);
            if (parent->value.type == CFG_ARR) {
                // the parent is an array, but this is a named value
                // 1. inject an anonymous map
                // 2. add this to the map

                CFG_NODE *map = cfg_node_create();

                if (!cfg_node_make_map(map) || !cfg_node_add_child(parent, map)) {
                    if (!yp->error[0])
                        snprintf(yp->error, sizeof(yp->error), "cannot add anonymous map under array");
                    goto cleanup;
                }
                yaml_push(yp, map);
            }

            size_t last_indent = yp->current.indent;
            switch (yaml_find_token_start(yp, TOKEN_VALUE)) {
                case TOKEN_FINISHED:
                    if (!yp->error[0])
                        snprintf(yp->error, sizeof(yp->error), "document ended without assigning value");
                    goto cleanup;
                    break;

                case TOKEN_KEYWORD:
                    // no value on this key and the next is a token
                    if(last_indent == yp->current.indent)
                        // the new token is at the same level as mine
                        break;
                    else
                        continue;

                case TOKEN_VALUE:
                    if (!yaml_parse_value(yp, node)) {
                        if (!yp->error[0])
                            snprintf(yp->error, sizeof(yp->error), "cannot parse value");
                        goto cleanup;
                    }
                    break;
            }
        }

        CFG_NODE *parent = yaml_parent_node(yp);
        if(!cfg_node_add_child(parent, node)) {
            if(!yp->error[0])
                snprintf(yp->error, sizeof(yp->error), "parent node is %s and cannot accept children nodes.",
                         cfg_value_type(parent->value.type));

            goto cleanup;
        }

        yaml_push(yp, node);
        node = NULL;
    }

    return yp->cfg;

cleanup:
    nd_log(NDLS_DAEMON, NDLP_ERR, "YAML PARSER: at line %zu: %s", yp->current.line, yp->error);
    cfg_cleanup(yp->cfg);
    freez(yp->cfg);
    buffer_free(yp->current.literal);
    return NULL;
}

static char *cfg_load_file(const char *filename, size_t *len) {
    FILE *file = fopen(filename, "rb");
    char *buffer;
    long length;

    if (file == NULL) {
        nd_log(NDLS_DAEMON, NDLP_ERR, "YAML: cannot open file '%s'", filename);
        return NULL;
    }

    // Seek to the end of the file to determine its size
    fseek(file, 0, SEEK_END);
    length = ftell(file);
    fseek(file, 0, SEEK_SET);

    // Allocate memory for the entire content
    buffer = mallocz(length + 1);

    // Read the file into the buffer
    if (fread(buffer, 1, length, file) != (size_t)length) {
        nd_log(NDLS_DAEMON, NDLP_ERR, "YAML: cannot read file '%s'", filename);
        free(buffer);
        fclose(file);
        return NULL;
    }

    // Null-terminate the buffer
    buffer[length] = '\0';
    *len = length;

    fclose(file);
    return buffer;
}

static inline CFG *cfg_parse_yaml_file(const char *filename) {
    size_t len;
    const char *s = cfg_load_file(filename, &len);
    CFG *cfg = cfg_parse_yaml(s, len);
    freez((void *)s);
    return cfg;
}


// ----------------------------------------------------------------------------
// printing yaml

static void yaml_print_multiline_value(const char *s, size_t depth) {
    if (!s)
        s = "";

    do {
        const char* next = strchr(s, '\n');
        if(next) next++;

        size_t len = next ? (size_t)(next - s) : strlen(s);
        CLEAN_BUFFER *b = buffer_create(0, NULL);
        buffer_contents_replace(b, s, len);

        fprintf(stderr, "%.*s%s%s",
                (int)(depth * 2), "                    ",
                buffer_tostring(b), next ? "" : "\n");

        s = next;
    } while(s && *s);
}

static bool needs_quotes_in_yaml(const char *str) {
    // Lookup table for special YAML characters
    static bool special_chars[256] = { false };
    static bool table_initialized = false;

    if (!table_initialized) {
        // Initialize the lookup table
        const char *special_chars_str = ":{}[],&*!|>'\"%@`^";
        for (const char *c = special_chars_str; *c; ++c) {
            special_chars[(unsigned char)*c] = true;
        }
        table_initialized = true;
    }

    while (*str) {
        if (special_chars[(unsigned char)*str]) {
            return true;
        }
        str++;
    }
    return false;
}

static void yaml_print_node(const char *key, const char *value, size_t depth, bool dash) {
    if(depth > 10) depth = 10;
    const char *quote = "'";

    const char *second_line = NULL;
    if(value && strchr(value, '\n')) {
        second_line = value;
        value = "|";
        quote = "";
    }
    else if(!value || !needs_quotes_in_yaml(value))
        quote = "";

    fprintf(stderr, "%.*s%s%s%s%s%s%s\n",
            (int)(depth * 2), "                    ", dash ? "- ": "",
            key ? key : "", key ? ": " : "",
            quote, value ? value : "", quote);

    if(second_line) {
        yaml_print_multiline_value(second_line, depth + 1);
    }
}

static inline void cfg_print_yaml_node(CFG_NODE *n, size_t indent, bool dash) {
    assert(n);

    switch(n->value.type) {
        default:
        case CFG_NON:
        case CFG_U64:
        case CFG_I64:
        case CFG_DBL:
        case CFG_BLN:
        case CFG_LNK:
            yaml_print_node(n->id.type == CFG_NODE_ID_TYPE_NAMED ? n->id.key.name : NULL, cfg_value_type(n->value.type), indent, dash);
            break;

        case CFG_TXT:
            yaml_print_node(n->id.type == CFG_NODE_ID_TYPE_NAMED ? n->id.key.name : NULL, n->value.value.txt, indent, dash);
            break;

        case CFG_ARR:
            yaml_print_node(n->id.type == CFG_NODE_ID_TYPE_NAMED ? n->id.key.name : NULL, NULL, indent, dash);
            for(size_t i = 0; i < n->value.value.arr.used ;i++)
                cfg_print_yaml_node(n->value.value.arr.array[i], indent + 2, true);
            break;

        case CFG_MAP:
            yaml_print_node(n->id.type == CFG_NODE_ID_TYPE_NAMED ? n->id.key.name : NULL, NULL, indent, dash);
            SIMPLE_HASHTABLE_FOREACH_READ_ONLY(&n->value.value.map.hashtable, sl, _CFG_NODE) {
                cfg_print_yaml_node(SIMPLE_HASHTABLE_FOREACH_READ_ONLY_VALUE(sl), indent + 2, false);
            }
            break;
    }
}

static inline void cfg_print_yaml(CFG *cfg) {
    assert(cfg);
    cfg_print_yaml_node(cfg->root, 0, false);
}

int unittest_yaml_config(void) {
    const char *s = "array1:\n"
                    "  - a1_v1\n"
                    "  - a1_v2\n"
                    "map1:\n"
                    "  m1_v1: 1\n"
                    "  m1_v2: 2\n"
                    "";

    cfg_print_yaml(cfg_parse_yaml(s, strlen(s)));

    return 0;
}
