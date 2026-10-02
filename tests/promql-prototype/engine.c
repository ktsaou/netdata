/* SPDX-License-Identifier: GPL-3.0-or-later */
/* Experimental evaluator; retained float samples only. */
#include "abi.h"
#include <ctype.h>
#include <math.h>
#include <setjmp.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <limits.h>

typedef struct Block {
    struct Block *next;
    max_align_t alignment;
    unsigned char bytes[];
} Block;
typedef struct Context {
    PPResult result;
    Block *blocks;
    jmp_buf trap;
    const PPRequest *req;
    const char *error;
    char *pending_string;
    uint64_t work;
    unsigned depth;
} Context;
static void error(Context *c, const char *reason)
{
    c->error = reason;
    longjmp(c->trap, 1);
}
static void *allocate(Context *c, size_t n, size_t size)
{
    if (size && n > (SIZE_MAX - sizeof(Block)) / size)
        error(c, "allocation overflow");
    Block *b = calloc(1, sizeof(Block) + n * size);
    if (!b)
        error(c, "allocation failure");
    b->next = c->blocks;
    c->blocks = b;
    return b->bytes;
}
static char *copy_n(Context *c, const char *s, size_t n)
{
    char *p = allocate(c, n + 1, 1);
    memcpy(p, s, n);
    return p;
}
static char *copy(Context *c, const char *s)
{
    return copy_n(c, s, strlen(s));
}
static void tick(Context *c)
{
    ++c->work;
    if (c->req->work_limit && c->work > c->req->work_limit)
        error(c, "work budget exceeded");
    if (c->req->cancelled && c->req->cancelled(c->req->cancel_context))
        error(c, "cancelled");
}
static void grow(Context *c, void **p, size_t *cap, size_t used, size_t size)
{
    if (used < *cap)
        return;
    size_t n = *cap ? *cap * 2 : 4;
    if (n < *cap)
        error(c, "capacity overflow");
    void *q = allocate(c, n, size);
    if (used)
        memcpy(q, *p, used * size);
    *p = q;
    *cap = n;
}
typedef struct {
    char **p;
    size_t n, cap;
} Names;
typedef struct {
    PPLabel *p;
    size_t n;
} Labels;
typedef struct {
    Labels labels;
    PPPoint *points;
    size_t n;
} Row;
typedef struct {
    Row *p;
    size_t n, cap;
} Rows;
typedef struct {
    int kind;
    double scalar;
    char *text;
    Rows rows;
} Value;
typedef struct {
    char *name, *op, *value;
} Matcher;
typedef struct Node Node;
struct Node {
    int kind;
    char *text;
    double number;
    int64_t range, offset, at, step;
    int boolean, grouping, matching, cardinality;
    Names labels, include;
    Matcher *matchers;
    size_t matchers_n, matchers_cap;
    Node **args;
    size_t args_n, args_cap;
};
enum { N_NUMBER, N_STRING, N_SELECTOR, N_UNARY, N_BINARY, N_CALL, N_AGGREGATE, N_SUBQUERY };
typedef struct {
    char *text;
    int kind;
    double number;
} Token;
typedef struct {
    Context *c;
    const char *p;
    Token tok;
    unsigned depth;
    unsigned tokens;
} Parser;
static int equal(const char *a, const char *b)
{
    return strcmp(a, b) == 0;
}
static void next(Parser *p)
{
    if (++p->tokens > 1024)
        error(p->c, "parse token limit");
    while (isspace((unsigned char)*p->p))
        ++p->p;
    if (!*p->p) {
        p->tok = (Token){"", 0, 0};
        return;
    }
    if (*p->p == '"') {
        ++p->p;
        char *s = allocate(p->c, strlen(p->p) + 1, 1);
        size_t n = 0;
        while (*p->p && *p->p != '"') {
            char x = *p->p++;
            if (x == '\\') {
                if (!*p->p)
                    error(p->c, "unfinished escape");
                x = *p->p++;
                if (x == 'n')
                    x = '\n';
                else if (x == 't')
                    x = '\t';
                else if (x != '"' && x != '\\')
                    error(p->c, "unsupported string escape");
            }
            s[n++] = x;
        }
        if (*p->p != '"')
            error(p->c, "unfinished string");
        ++p->p;
        p->tok = (Token){s, 3, 0};
        return;
    }
    if (isdigit((unsigned char)*p->p) || (*p->p == '.' && isdigit((unsigned char)p->p[1]))) {
        char *end;
        double n = strtod(p->p, &end);
        p->tok = (Token){copy_n(p->c, p->p, end - p->p), 2, n};
        p->p = end;
        return;
    }
    if (isalpha((unsigned char)*p->p) || *p->p == '_') {
        const char *b = p->p++;
        while (isalnum((unsigned char)*p->p) || *p->p == '_')
            ++p->p;
        p->tok = (Token){copy_n(p->c, b, p->p - b), 1, 0};
        return;
    }
    const char *b = p->p++;
    if ((*p->p == '=' && (*b == '=' || *b == '!' || *b == '<' || *b == '>')) ||
        (*p->p == '~' && (*b == '=' || *b == '!')))
        ++p->p;
    p->tok = (Token){copy_n(p->c, b, p->p - b), 4, 0};
}
static int eat(Parser *p, const char *s)
{
    if (p->tok.kind == 3 || !equal(p->tok.text, s))
        return 0;
    next(p);
    return 1;
}
static void need(Parser *p, const char *s)
{
    if (!eat(p, s))
        error(p->c, "unexpected token");
}
static void name_push(Context *c, Names *v, char *s)
{
    grow(c, (void **)&v->p, &v->cap, v->n, sizeof *v->p);
    v->p[v->n++] = s;
}
static Names names(Parser *p)
{
    Names v = {0};
    need(p, "(");
    if (!eat(p, ")")) {
        do {
            if (p->tok.kind != 1 && p->tok.kind != 3)
                error(p->c, "expected label");
            name_push(p->c, &v, p->tok.text);
            next(p);
        } while (eat(p, ","));
        need(p, ")");
    }
    return v;
}
static int64_t duration(Parser *p)
{
    if (p->tok.kind != 2)
        error(p->c, "expected duration");
    double n = p->tok.number;
    next(p);
    char *u = p->tok.text;
    double f = equal(u, "ms") ? 1 :
               equal(u, "s")  ? 1000 :
               equal(u, "m")  ? 60000 :
               equal(u, "h")  ? 3600000 :
               equal(u, "d")  ? 86400000 :
               equal(u, "w")  ? 604800000 :
               equal(u, "y")  ? 31536000000.0 :
                                0;
    if (p->tok.kind != 1 || !f || !isfinite(n) || n * f < 1 || n * f > 9e15)
        error(p->c, "unsupported duration");
    next(p);
    return (int64_t)(n * f);
}
static Node *node(Context *c, int kind)
{
    Node *n = allocate(c, 1, sizeof *n);
    n->kind = kind;
    n->at = -1;
    n->text = "";
    return n;
}
static void arg(Context *c, Node *n, Node *a)
{
    grow(c, (void **)&n->args, &n->args_cap, n->args_n, sizeof *n->args);
    n->args[n->args_n++] = a;
}
static void matcher(Context *c, Node *n, char *label, char *op, char *value)
{
    grow(c, (void **)&n->matchers, &n->matchers_cap, n->matchers_n, sizeof *n->matchers);
    n->matchers[n->matchers_n++] = (Matcher){label, op, value};
}
static int is_agg(const char *s)
{
    return equal(s, "sum") || equal(s, "avg") || equal(s, "min") || equal(s, "max") || equal(s, "count") ||
           equal(s, "topk") || equal(s, "bottomk");
}
static Node *expr(Parser *, int);
static Node *primary(Parser *p)
{
    Node *n;
    if (p->tok.kind != 3 && (equal(p->tok.text, "+") || equal(p->tok.text, "-"))) {
        n = node(p->c, N_UNARY);
        n->text = p->tok.text;
        next(p);
        arg(p->c, n, expr(p, 6));
    } else if (p->tok.kind == 2) {
        n = node(p->c, N_NUMBER);
        n->number = p->tok.number;
        next(p);
    } else if (p->tok.kind == 3) {
        n = node(p->c, N_STRING);
        n->text = p->tok.text;
        next(p);
    } else if (eat(p, "(")) {
        n = expr(p, 0);
        need(p, ")");
    } else if (p->tok.kind == 1 || equal(p->tok.text, "{")) {
        char *name = "";
        if (p->tok.kind == 1) {
            name = p->tok.text;
            next(p);
        }
        if (equal(p->tok.text, "(") || equal(p->tok.text, "by") || equal(p->tok.text, "without")) {
            n = node(p->c, is_agg(name) ? N_AGGREGATE : N_CALL);
            n->text = name;
            if (equal(p->tok.text, "by") || equal(p->tok.text, "without")) {
                n->grouping = equal(p->tok.text, "by") ? 1 : 2;
                next(p);
                n->labels = names(p);
            }
            need(p, "(");
            if (!eat(p, ")")) {
                do {
                    arg(p->c, n, expr(p, 0));
                } while (eat(p, ","));
                need(p, ")");
            }
            if (n->kind == N_AGGREGATE && (equal(p->tok.text, "by") || equal(p->tok.text, "without"))) {
                n->grouping = equal(p->tok.text, "by") ? 1 : 2;
                next(p);
                n->labels = names(p);
            }
        } else {
            n = node(p->c, N_SELECTOR);
            if (*name)
                matcher(p->c, n, "__name__", "=", name);
            if (eat(p, "{")) {
                if (!eat(p, "}")) {
                    do {
                        if (p->tok.kind == 3) {
                            matcher(p->c, n, "__name__", "=", p->tok.text);
                            next(p);
                        } else {
                            if (p->tok.kind != 1)
                                error(p->c, "expected matcher label");
                            char *label = p->tok.text;
                            next(p);
                            char *op = p->tok.text;
                            if (!equal(op, "=") && !equal(op, "!=") && !equal(op, "=~") && !equal(op, "!~"))
                                error(p->c, "expected matcher operator");
                            next(p);
                            if (p->tok.kind != 3)
                                error(p->c, "matcher needs string");
                            matcher(p->c, n, label, op, p->tok.text);
                            next(p);
                        }
                    } while (eat(p, ",") && !equal(p->tok.text, "}"));
                    need(p, "}");
                }
            }
            if (!n->matchers_n)
                error(p->c, "empty selector");
        }
    } else
        error(p->c, "expected expression");
    if (eat(p, "[")) {
        int64_t range = duration(p);
        if (eat(p, ":")) {
            Node *sub = node(p->c, N_SUBQUERY);
            sub->range = range;
            sub->step = equal(p->tok.text, "]") ? 60000 : duration(p);
            arg(p->c, sub, n);
            n = sub;
        } else {
            if (n->kind != N_SELECTOR)
                error(p->c, "range requires selector");
            n->range = range;
        }
        need(p, "]");
    }
    for (;;) {
        if (eat(p, "offset"))
            n->offset = duration(p);
        else if (eat(p, "@")) {
            if (p->tok.kind != 2 || !isfinite(p->tok.number) || p->tok.number < 0 || p->tok.number > 9e12)
                error(p->c, "numeric @ required");
            n->at = (int64_t)(p->tok.number * 1000);
            next(p);
        } else
            break;
    }
    return n;
}
static int precedence(const char *s)
{
    if (equal(s, "or"))
        return 1;
    if (equal(s, "and") || equal(s, "unless"))
        return 2;
    if (equal(s, "==") || equal(s, "!=") || equal(s, "<") || equal(s, ">") || equal(s, "<=") || equal(s, ">="))
        return 3;
    if (equal(s, "+") || equal(s, "-"))
        return 4;
    if (equal(s, "*") || equal(s, "/") || equal(s, "%"))
        return 5;
    if (equal(s, "^"))
        return 7;
    return -1;
}
static Node *expr(Parser *p, int minimum)
{
    if (++p->depth > 128)
        error(p->c, "parse depth limit");
    Node *lhs = primary(p);
    while (p->tok.kind != 3 && precedence(p->tok.text) >= minimum) {
        char *op = p->tok.text;
        int prec = precedence(op);
        next(p);
        Node *n = node(p->c, N_BINARY);
        n->text = op;
        n->boolean = eat(p, "bool");
        if (equal(p->tok.text, "on") || equal(p->tok.text, "ignoring")) {
            n->matching = equal(p->tok.text, "on") ? 1 : 2;
            next(p);
            n->labels = names(p);
        }
        if (equal(p->tok.text, "group_left") || equal(p->tok.text, "group_right")) {
            n->cardinality = equal(p->tok.text, "group_left") ? 1 : 2;
            next(p);
            if (equal(p->tok.text, "("))
                n->include = names(p);
        }
        arg(p->c, n, lhs);
        arg(p->c, n, expr(p, prec + (equal(op, "^") ? 0 : 1)));
        lhs = n;
    }
    --p->depth;
    return lhs;
}
static int label_order(const void *a, const void *b)
{
    return strcmp(((const PPLabel *)a)->name, ((const PPLabel *)b)->name);
}
static const char *label(Labels ls, const char *name)
{
    for (size_t i = 0; i < ls.n; i++)
        if (equal(ls.p[i].name, name))
            return ls.p[i].value;
    return "";
}
static int contains(Names n, const char *name)
{
    for (size_t i = 0; i < n.n; i++)
        if (equal(n.p[i], name))
            return 1;
    return 0;
}
static Labels project(Context *c, Labels ls, Names ns, int keep)
{
    Labels out = {allocate(c, ls.n, sizeof(PPLabel)), 0};
    for (size_t i = 0; i < ls.n; i++)
        if (contains(ns, ls.p[i].name) == keep && (keep || !equal(ls.p[i].name, "__name__")))
            out.p[out.n++] = ls.p[i];
    return out;
}
static Labels put(Context *c, Labels ls, const char *name, const char *value)
{
    Labels out = {allocate(c, ls.n + 1, sizeof(PPLabel)), 0};
    for (size_t i = 0; i < ls.n; i++)
        if (!equal(ls.p[i].name, name))
            out.p[out.n++] = ls.p[i];
    if (*value)
        out.p[out.n++] = (PPLabel){copy(c, name), copy(c, value)};
    qsort(out.p, out.n, sizeof *out.p, label_order);
    return out;
}
static Labels drop_name(Context *c, Labels ls)
{
    return put(c, ls, "__name__", "");
}
static char *key(Context *c, Labels ls)
{
    size_t n = 1;
    for (size_t i = 0; i < ls.n; i++) {
        size_t a = strlen(ls.p[i].name), b = strlen(ls.p[i].value);
        if (a > SIZE_MAX - n - 64 || b > SIZE_MAX - n - a - 64)
            error(c, "label size overflow");
        n += a + b + 64;
    }
    char *s = allocate(c, n, 1);
    size_t used = 0;
    for (size_t i = 0; i < ls.n; i++)
        used += (size_t)snprintf(
            s + used,
            n - used,
            "%zu:%s%zu:%s",
            strlen(ls.p[i].name),
            ls.p[i].name,
            strlen(ls.p[i].value),
            ls.p[i].value);
    return s;
}
typedef struct {
    char *key;
    void *value;
} Entry;
typedef struct {
    Entry *p;
    size_t n, cap;
} Map;
static uint64_t hash(const char *s)
{
    uint64_t h = 1469598103934665603ULL;
    while (*s) {
        h ^= (unsigned char)*s++;
        h *= 1099511628211ULL;
    }
    return h;
}
static Entry *slot(Map *m, const char *s)
{
    size_t i = hash(s) & (m->cap - 1);
    while (m->p[i].key && !equal(m->p[i].key, s))
        i = (i + 1) & (m->cap - 1);
    return m->p + i;
}
static void map_grow(Context *c, Map *m)
{
    if (m->cap && m->n * 2 < m->cap)
        return;
    size_t old = m->cap;
    Entry *p = m->p;
    m->cap = old ? old * 2 : 16;
    if (m->cap < old)
        error(c, "map capacity overflow");
    m->p = allocate(c, m->cap, sizeof(Entry));
    for (size_t i = 0; i < old; i++)
        if (p[i].key)
            *slot(m, p[i].key) = p[i];
}
static void *lookup(Map *m, const char *s)
{
    return m->cap ? slot(m, s)->value : NULL;
}
static int insert(Context *c, Map *m, char *s, void *v)
{
    map_grow(c, m);
    Entry *e = slot(m, s);
    if (e->key)
        return 0;
    *e = (Entry){s, v};
    ++m->n;
    return 1;
}
static void row_push(Context *c, Rows *rs, Row r)
{
    grow(c, (void **)&rs->p, &rs->cap, rs->n, sizeof(Row));
    rs->p[rs->n++] = r;
}
static Row sample(Context *c, Labels ls, int64_t t, double x)
{
    PPPoint *p = allocate(c, 1, sizeof *p);
    *p = (PPPoint){t, x};
    return (Row){ls, p, 1};
}
static Value scalar(double x)
{
    Value v = {0};
    v.kind = PP_SCALAR;
    v.scalar = x;
    return v;
}
static Value vector(void)
{
    Value v = {0};
    v.kind = PP_VECTOR;
    return v;
}
static void type(Context *c, Value v, int k)
{
    if (v.kind != k)
        error(c, "wrong argument type");
}
static void unique(Context *c, Value v)
{
    Map m = {0};
    for (size_t i = 0; i < v.rows.n; i++)
        if (!insert(c, &m, key(c, v.rows.p[i].labels), (void *)1))
            error(c, "duplicate output labelset");
}
static double point(Row r)
{
    return r.points[r.n - 1].v;
}
static Labels signature(Context *c, Row r, Node *n)
{
    Names ns = n->labels;
    if (!n->matching) {
        char **p = allocate(c, 1, sizeof(char *));
        *p = "__name__";
        ns = (Names){p, 1, 1};
    }
    return project(c, r.labels, ns, n->matching == 1);
}
static Value eval(Context *, Node *, int64_t);
static Value select_rows(Context *c, Node *n, int64_t time)
{
    Value out = vector();
    if (n->range)
        out.kind = PP_MATRIX;
    int64_t t = (n->at >= 0 ? n->at : time) - n->offset;
    for (size_t i = 0; i < c->req->data->series_len; i++) {
        tick(c);
        const PPSeries *s = c->req->data->series + i;
        Labels ls = {(PPLabel *)s->labels, s->labels_len};
        int ok = 1;
        for (size_t j = 0; j < n->matchers_n; j++) {
            Matcher m = n->matchers[j];
            const char *v = label(ls, m.name);
            int match;
            if (equal(m.op, "=") || equal(m.op, "!="))
                match = equal(v, m.value);
            else {
                match = pp_match(m.value, v);
                if (match < 0)
                    error(c, "invalid regex");
            }
            if (equal(m.op, "!=") || equal(m.op, "!~"))
                match = !match;
            if (!match) {
                ok = 0;
                break;
            }
        }
        if (!ok)
            continue;
        PPPoint *points = allocate(c, s->points_len, sizeof(PPPoint));
        size_t len = 0;
        for (size_t j = 0; j < s->points_len; j++) {
            tick(c);
            PPPoint p = s->points[j];
            if (p.t <= t && p.t > t - (n->range ? n->range : c->req->lookback_ms))
                points[len++] = p;
        }
        if (!len)
            continue;
        Labels owned = {allocate(c, ls.n, sizeof(PPLabel)), ls.n};
        for (size_t j = 0; j < ls.n; j++)
            owned.p[j] = (PPLabel){copy(c, ls.p[j].name), copy(c, ls.p[j].value)};
        qsort(owned.p, owned.n, sizeof(PPLabel), label_order);
        if (n->range)
            row_push(c, &out.rows, (Row){owned, points, len});
        else
            row_push(c, &out.rows, sample(c, owned, time, points[len - 1].v));
    }
    return out;
}
static int comparison(const char *op)
{
    return equal(op, "==") || equal(op, "!=") || equal(op, "<") || equal(op, ">") || equal(op, "<=") || equal(op, ">=");
}
static double operation(Context *c, const char *op, double a, double b)
{
    if (equal(op, "+"))
        return a + b;
    if (equal(op, "-"))
        return a - b;
    if (equal(op, "*"))
        return a * b;
    if (equal(op, "/"))
        return a / b;
    if (equal(op, "%"))
        return fmod(a, b);
    if (equal(op, "^"))
        return pow(a, b);
    if (equal(op, "=="))
        return a == b;
    if (equal(op, "!="))
        return a != b;
    if (equal(op, "<"))
        return a < b;
    if (equal(op, ">"))
        return a > b;
    if (equal(op, "<="))
        return a <= b;
    if (equal(op, ">="))
        return a >= b;
    error(c, "unsupported operator");
    return 0;
}
static Value binary(Context *c, Node *n, Value a, Value b, int64_t t)
{
    Value out = vector();
    int cmp = comparison(n->text), set = equal(n->text, "or") || equal(n->text, "and") || equal(n->text, "unless");
    if (a.kind == PP_SCALAR && b.kind == PP_SCALAR) {
        if (set || (cmp && !n->boolean))
            error(c, "scalar comparison requires bool");
        return scalar(operation(c, n->text, a.scalar, b.scalar));
    }
    if (a.kind == PP_SCALAR || b.kind == PP_SCALAR) {
        if (set || n->matching || n->cardinality)
            error(c, "vector matching requires vectors");
        int left = a.kind == PP_VECTOR;
        Value v = left ? a : b;
        type(c, v, PP_VECTOR);
        double s = left ? b.scalar : a.scalar;
        for (size_t i = 0; i < v.rows.n; i++) {
            tick(c);
            Row r = v.rows.p[i];
            double old = point(r), x = operation(c, n->text, left ? old : s, left ? s : old);
            if (cmp && !n->boolean && !x)
                continue;
            if (!cmp || n->boolean)
                r.labels = drop_name(c, r.labels);
            row_push(c, &out.rows, sample(c, r.labels, t, cmp && !n->boolean ? old : x));
        }
        unique(c, out);
        return out;
    }
    type(c, a, PP_VECTOR);
    type(c, b, PP_VECTOR);
    if (set) {
        if (n->boolean || n->cardinality)
            error(c, "invalid set modifier");
        Map right = {0}, left = {0};
        for (size_t i = 0; i < b.rows.n; i++)
            insert(c, &right, key(c, signature(c, b.rows.p[i], n)), (void *)1);
        for (size_t i = 0; i < a.rows.n; i++) {
            tick(c);
            Row r = a.rows.p[i];
            char *k = key(c, signature(c, r, n));
            insert(c, &left, k, (void *)1);
            if (equal(n->text, "or") || (equal(n->text, "and") == !!lookup(&right, k)))
                row_push(c, &out.rows, r);
        }
        if (equal(n->text, "or"))
            for (size_t i = 0; i < b.rows.n; i++)
                if (!lookup(&left, key(c, signature(c, b.rows.p[i], n))))
                    row_push(c, &out.rows, b.rows.p[i]);
        return out;
    }
    if (!a.rows.n || !b.rows.n)
        return out;
    int reverse = n->cardinality == 2;
    Rows many = reverse ? b.rows : a.rows, one = reverse ? a.rows : b.rows;
    Map index = {0}, used = {0};
    for (size_t i = 0; i < one.n; i++)
        if (!insert(c, &index, key(c, signature(c, one.p[i], n)), one.p + i))
            error(c, "duplicate series on one side");
    for (size_t i = 0; i < many.n; i++) {
        tick(c);
        Row r = many.p[i];
        char *k = key(c, signature(c, r, n));
        Row *other = lookup(&index, k);
        if (!other)
            continue;
        if (!n->cardinality && !insert(c, &used, k, (void *)1))
            error(c, "many-to-one needs group modifier");
        double x = operation(c, n->text, reverse ? point(*other) : point(r), reverse ? point(r) : point(*other));
        if (cmp && !n->boolean && !x)
            continue;
        Labels ls = r.labels;
        if (!cmp || n->boolean)
            ls = drop_name(c, ls);
        if (!n->cardinality && n->matching)
            ls = project(c, ls, n->labels, n->matching == 1);
        for (size_t j = 0; j < n->include.n; j++)
            ls = put(c, ls, n->include.p[j], label(other->labels, n->include.p[j]));
        row_push(c, &out.rows, sample(c, ls, t, cmp && !n->boolean ? (reverse ? point(*other) : point(r)) : x));
    }
    unique(c, out);
    return out;
}
static int ascending(const void *a, const void *b)
{
    double x = point(*(const Row *)a), y = point(*(const Row *)b);
    return isnan(x) ? (isnan(y) ? 0 : 1) : isnan(y) ? -1 : x < y ? -1 : x > y ? 1 : 0;
}
static int descending(const void *a, const void *b)
{
    double x = point(*(const Row *)a), y = point(*(const Row *)b);
    return isnan(x) ? (isnan(y) ? 0 : 1) : isnan(y) ? -1 : x > y ? -1 : x < y ? 1 : 0;
}
typedef struct {
    Labels labels;
    Rows rows;
} Group;
static Value aggregate(Context *c, Node *n, int64_t t)
{
    int top = equal(n->text, "topk") || equal(n->text, "bottomk");
    if (n->args_n != (size_t)(top ? 2 : 1))
        error(c, "aggregation arity");
    double k = 0;
    if (top) {
        Value v = eval(c, n->args[0], t);
        type(c, v, PP_SCALAR);
        k = v.scalar;
    }
    Value a = eval(c, n->args[n->args_n - 1], t);
    type(c, a, PP_VECTOR);
    Value out = vector();
    Map index = {0};
    Group **groups = NULL;
    size_t len = 0, cap = 0;
    for (size_t i = 0; i < a.rows.n; i++) {
        tick(c);
        Row r = a.rows.p[i];
        Labels ls = n->grouping ? project(c, r.labels, n->labels, n->grouping == 1) : (Labels){0};
        char *s = key(c, ls);
        Group *g = lookup(&index, s);
        if (!g) {
            g = allocate(c, 1, sizeof *g);
            g->labels = ls;
            insert(c, &index, s, g);
            grow(c, (void **)&groups, &cap, len, sizeof *groups);
            groups[len++] = g;
        }
        row_push(c, &g->rows, r);
    }
    for (size_t i = 0; i < len; i++) {
        Group *g = groups[i];
        if (top) {
            qsort(g->rows.p, g->rows.n, sizeof(Row), equal(n->text, "topk") ? descending : ascending);
            size_t count = isfinite(k) && k > 0 ? (size_t)fmin(floor(k), g->rows.n) : 0;
            for (size_t j = 0; j < count; j++)
                row_push(c, &out.rows, g->rows.p[j]);
            continue;
        }
        double x = point(g->rows.p[0]);
        if (equal(n->text, "sum") || equal(n->text, "avg"))
            x = 0;
        for (size_t j = 0; j < g->rows.n; j++) {
            tick(c);
            double v = point(g->rows.p[j]);
            if (equal(n->text, "sum") || equal(n->text, "avg"))
                x += v;
            else if (equal(n->text, "min"))
                x = isnan(x) ? v : fmin(x, v);
            else if (equal(n->text, "max"))
                x = isnan(x) ? v : fmax(x, v);
        }
        if (equal(n->text, "avg"))
            x /= g->rows.n;
        if (equal(n->text, "count"))
            x = g->rows.n;
        row_push(c, &out.rows, sample(c, g->labels, t, x));
    }
    return out;
}
static Labels absent_labels(Context *c, Node *n)
{
    Labels ls = {0};
    if (n->kind == N_SELECTOR)
        for (size_t i = 0; i < n->matchers_n; i++) {
            Matcher m = n->matchers[i];
            if (!equal(m.name, "__name__") && equal(m.op, "="))
                ls = put(c, ls, m.name, m.value);
        }
    return ls;
}
typedef struct {
    double upper, count;
} Bucket;
typedef struct {
    Labels labels;
    Bucket *p;
    size_t n, cap;
} Histogram;
static int bucket_order(const void *a, const void *b)
{
    double x = ((const Bucket *)a)->upper, y = ((const Bucket *)b)->upper;
    return x < y ? -1 : x > y ? 1 : 0;
}
static Value histogram_quantile(Context *c, Value phi, Value vector, int64_t t)
{
    type(c, phi, PP_SCALAR);
    type(c, vector, PP_VECTOR);
    Map index = {0};
    Histogram **groups = NULL;
    size_t n = 0, cap = 0;
    Value out = {0};
    out.kind = PP_VECTOR;
    for (size_t i = 0; i < vector.rows.n; i++) {
        tick(c);
        Row r = vector.rows.p[i];
        const char *le = label(r.labels, "le");
        char *end;
        double upper = strtod(le, &end);
        if (!*le || *end)
            continue;
        Labels ls = put(c, drop_name(c, r.labels), "le", "");
        char *s = key(c, ls);
        Histogram *g = lookup(&index, s);
        if (!g) {
            g = allocate(c, 1, sizeof *g);
            g->labels = ls;
            insert(c, &index, s, g);
            grow(c, (void **)&groups, &cap, n, sizeof *groups);
            groups[n++] = g;
        }
        grow(c, (void **)&g->p, &g->cap, g->n, sizeof(Bucket));
        g->p[g->n++] = (Bucket){upper, point(r)};
    }
    for (size_t i = 0; i < n; i++) {
        Histogram *g = groups[i];
        qsort(g->p, g->n, sizeof(Bucket), bucket_order);
        size_t used = 0;
        for (size_t j = 0; j < g->n; j++) {
            if (used && g->p[used - 1].upper == g->p[j].upper)
                g->p[used - 1].count += g->p[j].count;
            else
                g->p[used++] = g->p[j];
        }
        g->n = used;
        double x = NAN;
        if (phi.scalar < 0)
            x = -INFINITY;
        else if (phi.scalar > 1)
            x = INFINITY;
        else if (g->n >= 2 && g->p[g->n - 1].upper == INFINITY && g->p[g->n - 1].count > 0) {
            double prev = 0;
            for (size_t j = 0; j < g->n; j++) {
                g->p[j].count = fmax(prev, g->p[j].count);
                prev = g->p[j].count;
            }
            double rank = phi.scalar * g->p[g->n - 1].count;
            size_t j = 0;
            while (j + 1 < g->n && g->p[j].count < rank)
                j++;
            if (j == g->n - 1)
                x = g->p[j - 1].upper;
            else if (j == 0 && g->p[0].upper <= 0)
                x = g->p[0].upper;
            else {
                double lo = j ? g->p[j - 1].upper : 0, base = j ? g->p[j - 1].count : 0;
                x = lo + (g->p[j].upper - lo) * (rank - base) / (g->p[j].count - base);
            }
        }
        row_push(c, &out.rows, sample(c, g->labels, t, x));
    }
    return out;
}
static Value call(Context *c, Node *n, int64_t t)
{
    if (!n->args_n)
        error(c, "function needs arguments");
    Value *args = allocate(c, n->args_n, sizeof(Value));
    for (size_t i = 0; i < n->args_n; i++)
        args[i] = eval(c, n->args[i], t);
    char *fn = n->text;
    Value out = vector();
    if (equal(fn, "vector")) {
        if (n->args_n != 1)
            error(c, "vector arity");
        type(c, args[0], PP_SCALAR);
        row_push(c, &out.rows, sample(c, (Labels){0}, t, args[0].scalar));
        return out;
    }
    if (equal(fn, "scalar")) {
        if (n->args_n != 1)
            error(c, "scalar arity");
        type(c, args[0], PP_VECTOR);
        return scalar(args[0].rows.n == 1 ? point(args[0].rows.p[0]) : NAN);
    }
    if (equal(fn, "absent") || equal(fn, "absent_over_time")) {
        if (n->args_n != 1)
            error(c, "absent arity");
        type(c, args[0], equal(fn, "absent") ? PP_VECTOR : PP_MATRIX);
        if (!args[0].rows.n)
            row_push(c, &out.rows, sample(c, absent_labels(c, n->args[0]), t, 1));
        return out;
    }
    if (equal(fn, "histogram_quantile")) {
        if (n->args_n != 2)
            error(c, "histogram_quantile arity");
        return histogram_quantile(c, args[0], args[1], t);
    }
    if (equal(fn, "label_replace") || equal(fn, "label_join")) {
        if ((equal(fn, "label_replace") && n->args_n != 5) || (equal(fn, "label_join") && n->args_n < 4))
            error(c, "label function arity");
        type(c, args[0], PP_VECTOR);
        for (size_t i = 1; i < n->args_n; i++)
            type(c, args[i], 0);
        if (equal(fn, "label_replace") && pp_match(args[4].text, "") < 0)
            error(c, "invalid regex");
        out = args[0];
        for (size_t i = 0; i < out.rows.n; i++) {
            tick(c);
            Row *r = out.rows.p + i;
            char *value;
            if (equal(fn, "label_replace")) {
                char *s = NULL;
                int rc = pp_replace(args[4].text, label(r->labels, args[3].text), args[2].text, &s);
                c->pending_string = s;
                if (rc < 0)
                    error(c, "regex replacement failure");
                if (!rc)
                    continue;
                value = copy(c, s);
                pp_string_free(s);
                c->pending_string = NULL;
            } else {
                size_t len = 1;
                for (size_t j = 3; j < n->args_n; j++)
                    len += strlen(label(r->labels, args[j].text)) + strlen(args[2].text);
                value = allocate(c, len, 1);
                for (size_t j = 3; j < n->args_n; j++) {
                    if (j > 3)
                        strcat(value, args[2].text);
                    strcat(value, label(r->labels, args[j].text));
                }
            }
            r->labels = put(c, r->labels, args[1].text, value);
        }
        unique(c, out);
        return out;
    }
    if (args[0].kind == PP_MATRIX) {
        if (n->args_n != 1)
            error(c, "range function arity");
        int rate = equal(fn, "rate") || equal(fn, "increase") || equal(fn, "delta"),
            instant = equal(fn, "irate") || equal(fn, "idelta"),
            plain = equal(fn, "avg_over_time") || equal(fn, "sum_over_time") || equal(fn, "min_over_time") ||
                    equal(fn, "max_over_time") || equal(fn, "count_over_time") || equal(fn, "last_over_time") ||
                    equal(fn, "changes") || equal(fn, "resets");
        if (!rate && !instant && !plain)
            error(c, "unsupported range function");
        Node *selector = n->args[0];
        int64_t end = (selector->at >= 0 ? selector->at : t) - selector->offset;
        for (size_t j = 0; j < args[0].rows.n; j++) {
            tick(c);
            Row r = args[0].rows.p[j];
            PPPoint *ps = r.points;
            if ((rate || instant) && r.n < 2)
                continue;
            double x = point(r);
            if (rate || instant) {
                int counter = equal(fn, "rate") || equal(fn, "increase") || equal(fn, "irate");
                size_t first = instant ? r.n - 2 : 0;
                x = ps[r.n - 1].v - ps[first].v;
                if (counter) {
                    if (instant) {
                        if (ps[r.n - 1].v < ps[first].v)
                            x = ps[r.n - 1].v;
                    } else
                        for (size_t i = 1; i < r.n; i++) {
                            tick(c);
                            if (ps[i].v < ps[i - 1].v)
                                x += ps[i - 1].v;
                        }
                }
                double elapsed = (ps[r.n - 1].t - ps[first].t) / 1000.0;
                if (elapsed <= 0)
                    continue;
                if (instant) {
                    if (equal(fn, "irate"))
                        x /= elapsed;
                } else {
                    double avg = elapsed / (r.n - 1), start = (ps[0].t - (end - selector->range)) / 1000.0,
                           finish = (end - ps[r.n - 1].t) / 1000.0, threshold = avg * 1.1;
                    if (start >= threshold)
                        start = avg / 2;
                    if (counter && x > 0 && ps[0].v >= 0)
                        start = fmin(start, elapsed * ps[0].v / x);
                    if (finish >= threshold)
                        finish = avg / 2;
                    x *= (elapsed + start + finish) / elapsed;
                    if (equal(fn, "rate"))
                        x /= selector->range / 1000.0;
                }
            } else if (equal(fn, "count_over_time"))
                x = r.n;
            else if (equal(fn, "changes") || equal(fn, "resets")) {
                x = 0;
                for (size_t i = 1; i < r.n; i++) {
                    tick(c);
                    if (equal(fn, "resets") ? ps[i].v < ps[i - 1].v :
                                              ps[i].v != ps[i - 1].v && !(isnan(ps[i].v) && isnan(ps[i - 1].v)))
                        x++;
                }
            } else if (!equal(fn, "last_over_time")) {
                x = equal(fn, "min_over_time") || equal(fn, "max_over_time") ? ps[0].v : 0;
                for (size_t i = 0; i < r.n; i++) {
                    tick(c);
                    if (equal(fn, "min_over_time"))
                        x = isnan(x) ? ps[i].v : fmin(x, ps[i].v);
                    else if (equal(fn, "max_over_time"))
                        x = isnan(x) ? ps[i].v : fmax(x, ps[i].v);
                    else
                        x += ps[i].v;
                }
                if (equal(fn, "avg_over_time"))
                    x /= r.n;
            }
            if (!equal(fn, "last_over_time"))
                r.labels = drop_name(c, r.labels);
            row_push(c, &out.rows, sample(c, r.labels, t, x));
        }
        unique(c, out);
        return out;
    }
    type(c, args[0], PP_VECTOR);
    out = args[0];
    if (equal(fn, "sort") || equal(fn, "sort_desc")) {
        if (n->args_n != 1)
            error(c, "sort arity");
        qsort(out.rows.p, out.rows.n, sizeof(Row), equal(fn, "sort") ? ascending : descending);
        return out;
    }
    if (!equal(fn, "abs") && !equal(fn, "clamp_min") && !equal(fn, "clamp_max") && !equal(fn, "round"))
        error(c, "unsupported function");
    if ((equal(fn, "abs") && n->args_n != 1) ||
        ((equal(fn, "clamp_min") || equal(fn, "clamp_max")) && n->args_n != 2) ||
        (equal(fn, "round") && (n->args_n < 1 || n->args_n > 2)))
        error(c, "function arity");
    double parameter = 1;
    if (n->args_n > 1) {
        type(c, args[1], PP_SCALAR);
        parameter = args[1].scalar;
    }
    for (size_t i = 0; i < out.rows.n; i++) {
        tick(c);
        Row *r = out.rows.p + i;
        r->labels = drop_name(c, r->labels);
        double *x = &r->points[r->n - 1].v;
        if (equal(fn, "abs"))
            *x = fabs(*x);
        else if (equal(fn, "clamp_min"))
            *x = isnan(*x) || isnan(parameter) ? NAN : fmax(*x, parameter);
        else if (equal(fn, "clamp_max"))
            *x = isnan(*x) || isnan(parameter) ? NAN : fmin(*x, parameter);
        else
            *x = floor(*x / parameter + 0.5) * parameter;
    }
    unique(c, out);
    return out;
}
static Value eval_inner(Context *c, Node *n, int64_t t)
{
    tick(c);
    switch (n->kind) {
        case N_NUMBER:
            return scalar(n->number);
        case N_STRING: {
            Value v = {0};
            v.text = n->text;
            return v;
        }
        case N_SELECTOR:
            return select_rows(c, n, t);
        case N_UNARY: {
            Value v = eval(c, n->args[0], t);
            if (v.kind == PP_SCALAR) {
                if (equal(n->text, "-"))
                    v.scalar = -v.scalar;
            } else {
                type(c, v, PP_VECTOR);
                if (equal(n->text, "-"))
                    for (size_t i = 0; i < v.rows.n; i++) {
                        v.rows.p[i].labels = drop_name(c, v.rows.p[i].labels);
                        v.rows.p[i].points[v.rows.p[i].n - 1].v = -point(v.rows.p[i]);
                    }
                unique(c, v);
            }
            return v;
        }
        case N_BINARY: {
            Value a = eval(c, n->args[0], t), b = eval(c, n->args[1], t);
            return binary(c, n, a, b, t);
        }
        case N_AGGREGATE:
            return aggregate(c, n, t);
        case N_CALL:
            return call(c, n, t);
        case N_SUBQUERY: {
            Value out = vector();
            out.kind = PP_MATRIX;
            int64_t end = (n->at >= 0 ? n->at : t) - n->offset, begin = (end - n->range) / n->step * n->step;
            if (begin <= end - n->range)
                begin += n->step;
            Map index = {0};
            typedef struct {
                Labels labels;
                PPPoint *points;
                size_t n, cap;
            } Accumulator;
            Accumulator **rows = NULL;
            size_t len = 0, cap = 0;
            for (int64_t s = begin; s <= end; s += n->step) {
                Value v = eval(c, n->args[0], s);
                type(c, v, PP_VECTOR);
                for (size_t i = 0; i < v.rows.n; i++) {
                    Row r = v.rows.p[i];
                    char *k = key(c, r.labels);
                    Accumulator *a = lookup(&index, k);
                    if (!a) {
                        a = allocate(c, 1, sizeof *a);
                        a->labels = r.labels;
                        insert(c, &index, k, a);
                        grow(c, (void **)&rows, &cap, len, sizeof *rows);
                        rows[len++] = a;
                    }
                    grow(c, (void **)&a->points, &a->cap, a->n, sizeof(PPPoint));
                    a->points[a->n++] = (PPPoint){s, point(r)};
                }
            }
            for (size_t i = 0; i < len; i++)
                row_push(c, &out.rows, (Row){rows[i]->labels, rows[i]->points, rows[i]->n});
            return out;
        }
    }
    error(c, "invalid node");
    return vector();
}
static PPResult emergency = {PP_ERROR, NULL, 0, "allocation failure", 0, NULL};
static Value eval(Context *c, Node *n, int64_t t)
{
    if (c->depth >= 128)
        error(c, "evaluation depth limit");
    c->depth++;
    Value v = eval_inner(c, n, t);
    c->depth--;
    return v;
}
PPResult *pp_eval(const PPRequest *request)
{
    Context *c = calloc(1, sizeof *c);
    if (!c)
        return &emergency;
    c->req = request;
    c->result.owner = c;
    if (setjmp(c->trap)) {
        c->result.kind = PP_ERROR;
        c->result.error = c->error;
        c->result.rows = NULL;
        c->result.rows_len = 0;
        c->result.work = c->work;
        return &c->result;
    }
    if (!request || !request->data || !request->query || request->lookback_ms <= 0 ||
        request->lookback_ms > 9000000000000000LL || request->time_ms < -9000000000000000LL ||
        request->time_ms > 9000000000000000LL)
        error(c, "invalid request");
    if (strlen(request->query) > 65536)
        error(c, "query length limit");
    Parser p = {.c = c, .p = request->query};
    next(&p);
    Node *ast = expr(&p, 0);
    if (p.tok.kind)
        error(c, "unexpected trailing token");
    Value v = eval(c, ast, request->time_ms);
    if (!v.kind)
        error(c, "string result unsupported");
    if (request->inject_failure == 2)
        error(c, "injected evaluation failure");
    if (request->inject_failure)
        error(c, "allocation failure (injected)");
    if (v.kind == PP_SCALAR)
        row_push(c, &v.rows, sample(c, (Labels){0}, request->time_ms, v.scalar));
    PPSeries *rows = allocate(c, v.rows.n, sizeof(PPSeries));
    for (size_t i = 0; i < v.rows.n; i++) {
        Row r = v.rows.p[i];
        rows[i] = (PPSeries){r.labels.p, r.labels.n, r.points, r.n};
    }
    c->result.kind = v.kind;
    c->result.rows = rows;
    c->result.rows_len = v.rows.n;
    c->result.work = c->work;
    return &c->result;
}
void pp_free(PPResult *result)
{
    if (!result || !result->owner)
        return;
    Context *c = result->owner;
    free(c->pending_string);
    Block *b = c->blocks;
    while (b) {
        Block *next = b->next;
        free(b);
        b = next;
    }
    free(c);
}
