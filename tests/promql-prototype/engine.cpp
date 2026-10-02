// SPDX-License-Identifier: GPL-3.0-or-later
// Experimental evaluator. Algorithms are compared with Prometheus v3.15.0.
#include "abi.h"
#include <algorithm>
#include <cmath>
#include <cstdlib>
#include <cstring>
#include <limits>
#include <map>
#include <memory>
#include <stdexcept>
#include <string>
#include <utility>
#include <vector>

namespace
{
using Labels = std::map<std::string, std::string>;
struct Row {
    Labels labels;
    std::vector<PPPoint> points;
};
struct Value {
    int kind = PP_VECTOR;
    double scalar = 0;
    std::string text;
    std::vector<Row> rows;
};
struct Match {
    std::string name, op, value;
};
struct Node {
    enum Kind { Number, String, Selector, Unary, Binary, Call, Aggregate, Subquery } kind;
    std::string text, grouping, matching, cardinality;
    double number = 0;
    int64_t range = 0, offset = 0, at = -1, step = 0;
    bool boolean = false;
    std::vector<std::string> labels, include;
    std::vector<Match> matchers;
    std::vector<std::unique_ptr<Node> > args;
    explicit Node(Kind k) : kind(k)
    {
    }
};
struct Token {
    std::string text;
    int kind;
    double number = 0;
};
class Parser {
    const char *p;
    Token tok;
    unsigned depth = 0;
    unsigned tokens = 0;
    static bool ident(char c)
    {
        return std::isalnum(static_cast<unsigned char>(c)) || c == '_';
    }
    void next()
    {
        if (++tokens > 1024)
            throw std::runtime_error("parse token limit");
        while (*p && std::isspace(static_cast<unsigned char>(*p)))
            ++p;
        if (!*p) {
            tok = {"", 0};
            return;
        }
        if (*p == '"') {
            ++p;
            std::string s;
            while (*p && *p != '"') {
                char c = *p++;
                if (c == '\\') {
                    if (!*p)
                        throw std::runtime_error("unfinished escape");
                    c = *p++;
                    if (c == 'n')
                        c = '\n';
                    else if (c == 't')
                        c = '\t';
                    else if (c != '"' && c != '\\')
                        throw std::runtime_error("unsupported string escape");
                }
                s += c;
            }
            if (*p != '"')
                throw std::runtime_error("unfinished string");
            ++p;
            tok = {s, 3};
            return;
        }
        if (std::isdigit(static_cast<unsigned char>(*p)) ||
            (*p == '.' && std::isdigit(static_cast<unsigned char>(p[1])))) {
            char *end;
            double n = std::strtod(p, &end);
            tok = {std::string(p, static_cast<size_t>(end - p)), 2, n};
            p = end;
            return;
        }
        if (std::isalpha(static_cast<unsigned char>(*p)) || *p == '_') {
            const char *b = p++;
            while (ident(*p))
                ++p;
            tok = {std::string(b, p), 1};
            return;
        }
        std::string s(1, *p++);
        if ((*p == '=' && (s == "=" || s == "!" || s == "<" || s == ">")) || (*p == '~' && (s == "=" || s == "!")))
            s += *p++;
        tok = {s, 4};
    }
    bool eat(const std::string &s)
    {
        if (tok.kind == 3 || tok.text != s)
            return false;
        next();
        return true;
    }
    void need(const std::string &s)
    {
        if (!eat(s))
            throw std::runtime_error("expected " + s);
    }
    std::vector<std::string> names()
    {
        need("(");
        std::vector<std::string> v;
        if (!eat(")")) {
            do {
                if (tok.kind != 1 && tok.kind != 3)
                    throw std::runtime_error("expected label");
                v.push_back(tok.text);
                next();
            } while (eat(","));
            need(")");
        }
        return v;
    }
    int64_t duration()
    {
        if (tok.kind != 2)
            throw std::runtime_error("expected duration");
        double n = tok.number;
        next();
        if (tok.kind != 1)
            throw std::runtime_error("duration unit required");
        std::string u = tok.text;
        next();
        double factor = u == "ms" ? 1 :
                        u == "s"  ? 1000 :
                        u == "m"  ? 60000 :
                        u == "h"  ? 3600000 :
                        u == "d"  ? 86400000 :
                        u == "w"  ? 604800000 :
                        u == "y"  ? 31536000000.0 :
                                    0;
        if (!factor || !std::isfinite(n) || n * factor < 1 || n * factor > 9e15)
            throw std::runtime_error("unsupported duration");
        return static_cast<int64_t>(n * factor);
    }
    static bool aggregate(const std::string &s)
    {
        return s == "sum" || s == "avg" || s == "min" || s == "max" || s == "count" || s == "topk" || s == "bottomk";
    }
    std::unique_ptr<Node> primary()
    {
        std::unique_ptr<Node> n;
        if (tok.kind != 3 && (tok.text == "+" || tok.text == "-")) {
            n = std::make_unique<Node>(Node::Unary);
            n->text = tok.text;
            next();
            n->args.push_back(expr(6));
        } else if (tok.kind == 2) {
            n = std::make_unique<Node>(Node::Number);
            n->number = tok.number;
            next();
        } else if (tok.kind == 3) {
            n = std::make_unique<Node>(Node::String);
            n->text = tok.text;
            next();
        } else if (eat("(")) {
            n = expr(0);
            need(")");
        } else if (tok.kind == 1 || tok.text == "{") {
            std::string name;
            if (tok.kind == 1) {
                name = tok.text;
                next();
            }
            if (tok.text == "(" || tok.text == "by" || tok.text == "without") {
                n = std::make_unique<Node>(aggregate(name) ? Node::Aggregate : Node::Call);
                n->text = name;
                if (tok.text == "by" || tok.text == "without") {
                    n->grouping = tok.text;
                    next();
                    n->labels = names();
                }
                need("(");
                if (!eat(")")) {
                    do {
                        n->args.push_back(expr(0));
                    } while (eat(","));
                    need(")");
                }
                if (n->kind == Node::Aggregate && (tok.text == "by" || tok.text == "without")) {
                    n->grouping = tok.text;
                    next();
                    n->labels = names();
                }
            } else {
                n = std::make_unique<Node>(Node::Selector);
                if (!name.empty())
                    n->matchers.push_back({"__name__", "=", name});
                if (eat("{")) {
                    if (!eat("}")) {
                        do {
                            if (tok.kind == 3) {
                                n->matchers.push_back({"__name__", "=", tok.text});
                                next();
                            } else {
                                if (tok.kind != 1)
                                    throw std::runtime_error("expected matcher label");
                                std::string key = tok.text;
                                next();
                                std::string op = tok.text;
                                if (op != "=" && op != "!=" && op != "=~" && op != "!~")
                                    throw std::runtime_error("expected matcher operator");
                                next();
                                if (tok.kind != 3)
                                    throw std::runtime_error("matcher needs string");
                                n->matchers.push_back({key, op, tok.text});
                                next();
                            }
                        } while (eat(",") && tok.text != "}");
                        need("}");
                    }
                }
                if (n->matchers.empty())
                    throw std::runtime_error("empty selector");
            }
        } else
            throw std::runtime_error("expected expression");
        if (eat("[")) {
            int64_t range = duration();
            if (eat(":")) {
                auto sub = std::make_unique<Node>(Node::Subquery);
                sub->range = range;
                sub->step = tok.text == "]" ? 60000 : duration();
                sub->args.push_back(std::move(n));
                n = std::move(sub);
            } else {
                if (n->kind != Node::Selector)
                    throw std::runtime_error("range requires selector");
                n->range = range;
            }
            need("]");
        }
        for (;;) {
            if (eat("offset"))
                n->offset = duration();
            else if (eat("@")) {
                if (tok.kind != 2 || !std::isfinite(tok.number) || tok.number < 0 || tok.number > 9e12)
                    throw std::runtime_error("numeric @ required");
                n->at = static_cast<int64_t>(tok.number * 1000);
                next();
            } else
                break;
        }
        return n;
    }
    static int precedence(const std::string &s)
    {
        if (s == "or")
            return 1;
        if (s == "and" || s == "unless")
            return 2;
        if (s == "==" || s == "!=" || s == "<" || s == ">" || s == "<=" || s == ">=")
            return 3;
        if (s == "+" || s == "-")
            return 4;
        if (s == "*" || s == "/" || s == "%")
            return 5;
        if (s == "^")
            return 7;
        return -1;
    }
    std::unique_ptr<Node> expr(int minimum)
    {
        if (++depth > 128)
            throw std::runtime_error("parse depth limit");
        auto lhs = primary();
        while (tok.kind != 3 && precedence(tok.text) >= minimum) {
            std::string op = tok.text;
            int prec = precedence(op);
            next();
            auto n = std::make_unique<Node>(Node::Binary);
            n->text = op;
            if (eat("bool"))
                n->boolean = true;
            if (tok.text == "on" || tok.text == "ignoring") {
                n->matching = tok.text;
                next();
                n->labels = names();
            }
            if (tok.text == "group_left" || tok.text == "group_right") {
                n->cardinality = tok.text;
                next();
                if (tok.text == "(")
                    n->include = names();
            }
            n->args.push_back(std::move(lhs));
            n->args.push_back(expr(prec + (op == "^" ? 0 : 1)));
            lhs = std::move(n);
        }
        --depth;
        return lhs;
    }

public:
    explicit Parser(const char *q) : p(q)
    {
        if (std::strlen(q) > 65536)
            throw std::runtime_error("query length limit");
        next();
    }
    std::unique_ptr<Node> parse()
    {
        auto n = expr(0);
        if (tok.kind)
            throw std::runtime_error("unexpected token " + tok.text);
        return n;
    }
};
bool contains(const std::vector<std::string> &v, const std::string &s)
{
    return std::find(v.begin(), v.end(), s) != v.end();
}
std::string label(const Labels &ls, const std::string &name)
{
    auto i = ls.find(name);
    return i == ls.end() ? "" : i->second;
}
Labels project(const Labels &ls, const std::vector<std::string> &names, bool keep)
{
    Labels out;
    for (const auto &l : ls)
        if (contains(names, l.first) == keep && (keep || l.first != "__name__"))
            out.insert(l);
    return out;
}
std::string key(const Labels &ls)
{
    std::string s;
    for (const auto &l : ls)
        s += std::to_string(l.first.size()) + ":" + l.first + std::to_string(l.second.size()) + ":" + l.second;
    return s;
}
void unique(const Value &v)
{
    std::map<std::string, bool> seen;
    for (const auto &r : v.rows)
        if (!seen.emplace(key(r.labels), true).second)
            throw std::runtime_error("duplicate output labelset");
}
bool comparison(const std::string &op)
{
    return op == "==" || op == "!=" || op == "<" || op == ">" || op == "<=" || op == ">=";
}
bool valueLess(double a, double b, bool desc)
{
    if (std::isnan(a))
        return false;
    if (std::isnan(b))
        return true;
    return desc ? a > b : a < b;
}
double operation(const std::string &op, double a, double b)
{
    if (op == "+")
        return a + b;
    if (op == "-")
        return a - b;
    if (op == "*")
        return a * b;
    if (op == "/")
        return a / b;
    if (op == "%")
        return std::fmod(a, b);
    if (op == "^")
        return std::pow(a, b);
    if (op == "==")
        return a == b;
    if (op == "!=")
        return a != b;
    if (op == "<")
        return a < b;
    if (op == ">")
        return a > b;
    if (op == "<=")
        return a <= b;
    if (op == ">=")
        return a >= b;
    throw std::runtime_error("unsupported binary operator");
}
class Evaluator {
    const PPRequest &req;
    uint64_t work = 0;
    unsigned depth = 0;
    void tick()
    {
        ++work;
        if (req.work_limit && work > req.work_limit)
            throw std::runtime_error("work budget exceeded");
        if (req.cancelled && req.cancelled(req.cancel_context))
            throw std::runtime_error("cancelled");
    }
    static void type(const Value &v, int kind)
    {
        if (v.kind != kind)
            throw std::runtime_error("wrong argument type");
    }
    static Value scalar(double x)
    {
        Value v;
        v.kind = PP_SCALAR;
        v.scalar = x;
        return v;
    }
    static Row sample(Labels labels, int64_t t, double x)
    {
        return {std::move(labels), {{t, x}}};
    }
    Value select(const Node &n, int64_t time)
    {
        Value out;
        out.kind = n.range ? PP_MATRIX : PP_VECTOR;
        int64_t t = (n.at >= 0 ? n.at : time) - n.offset;
        for (size_t i = 0; i < req.data->series_len; ++i) {
            tick();
            const auto &s = req.data->series[i];
            bool ok = true;
            for (const auto &m : n.matchers) {
                const char *x = "";
                for (size_t j = 0; j < s.labels_len; ++j)
                    if (m.name == s.labels[j].name) {
                        x = s.labels[j].value;
                        break;
                    }
                bool match;
                if (m.op == "=" || m.op == "!=")
                    match = m.value == x;
                else {
                    int r = pp_match(m.value.c_str(), x);
                    if (r < 0)
                        throw std::runtime_error("invalid regex");
                    match = r != 0;
                }
                if (m.op == "!=" || m.op == "!~")
                    match = !match;
                if (!match) {
                    ok = false;
                    break;
                }
            }
            if (!ok)
                continue;
            Labels ls;
            for (size_t j = 0; j < s.labels_len; ++j)
                ls[s.labels[j].name] = s.labels[j].value;
            Row row{ls, {}};
            for (size_t j = 0; j < s.points_len; ++j) {
                tick();
                const auto &p = s.points[j];
                if (p.t <= t && p.t > t - (n.range ? n.range : req.lookback_ms))
                    row.points.push_back(p);
            }
            if (!n.range && !row.points.empty())
                row.points = {{time, row.points.back().v}};
            if (!row.points.empty())
                out.rows.push_back(std::move(row));
        }
        return out;
    }
    Labels signature(const Row &r, const Node &n)
    {
        return project(
            r.labels, n.matching == "" ? std::vector<std::string>{"__name__"} : n.labels, n.matching == "on");
    }
    Value binary(const Node &n, Value a, Value b, int64_t t)
    {
        bool cmp = comparison(n.text);
        bool set = n.text == "or" || n.text == "and" || n.text == "unless";
        if (a.kind == PP_SCALAR && b.kind == PP_SCALAR) {
            if (set || (cmp && !n.boolean))
                throw std::runtime_error("scalar comparison requires bool");
            return scalar(operation(n.text, a.scalar, b.scalar));
        }
        Value out;
        if (a.kind == PP_SCALAR || b.kind == PP_SCALAR) {
            if (set || !n.matching.empty() || !n.cardinality.empty())
                throw std::runtime_error("vector matching requires vectors");
            bool left = a.kind == PP_VECTOR;
            Value v = left ? std::move(a) : std::move(b);
            type(v, PP_VECTOR);
            double s = left ? b.scalar : a.scalar;
            for (auto &r : v.rows) {
                tick();
                double old = r.points.back().v;
                double x = operation(n.text, left ? old : s, left ? s : old);
                if (cmp && !n.boolean && !x)
                    continue;
                if (!cmp || n.boolean)
                    r.labels.erase("__name__");
                r.points = {{t, cmp && !n.boolean ? old : x}};
                out.rows.push_back(std::move(r));
            }
            unique(out);
            return out;
        }
        type(a, PP_VECTOR);
        type(b, PP_VECTOR);
        if (set) {
            if (n.boolean || !n.cardinality.empty())
                throw std::runtime_error("invalid set modifier");
            std::map<std::string, bool> right, left;
            for (const auto &r : b.rows)
                right[key(signature(r, n))] = true;
            for (auto &r : a.rows) {
                tick();
                std::string k = key(signature(r, n));
                left[k] = true;
                if (n.text == "or" || ((n.text == "and") == (right.count(k) != 0)))
                    out.rows.push_back(std::move(r));
            }
            if (n.text == "or")
                for (auto &r : b.rows)
                    if (!left.count(key(signature(r, n))))
                        out.rows.push_back(std::move(r));
            return out;
        }
        if (a.rows.empty() || b.rows.empty())
            return out;
        bool reverse = n.cardinality == "group_right";
        const auto &many = reverse ? b.rows : a.rows;
        const auto &one = reverse ? a.rows : b.rows;
        std::map<std::string, const Row *> index;
        for (const auto &r : one)
            if (!index.emplace(key(signature(r, n)), &r).second)
                throw std::runtime_error("duplicate series on one side");
        std::map<std::string, bool> used;
        for (const auto &r : many) {
            tick();
            std::string k = key(signature(r, n));
            auto hit = index.find(k);
            if (hit == index.end())
                continue;
            if (n.cardinality.empty() && !used.emplace(k, true).second)
                throw std::runtime_error("many-to-one needs group modifier");
            const Row &other = *hit->second;
            double x = operation(
                n.text,
                reverse ? other.points.back().v : r.points.back().v,
                reverse ? r.points.back().v : other.points.back().v);
            if (cmp && !n.boolean && !x)
                continue;
            Labels ls = r.labels;
            if (!cmp || n.boolean)
                ls.erase("__name__");
            if (n.cardinality.empty()) {
                if (n.matching == "on")
                    ls = project(ls, n.labels, true);
                else if (n.matching == "ignoring")
                    ls = project(ls, n.labels, false);
            }
            for (const auto &name : n.include) {
                std::string v = label(other.labels, name);
                if (v.empty())
                    ls.erase(name);
                else
                    ls[name] = v;
            }
            out.rows.push_back(sample(
                std::move(ls), t, cmp && !n.boolean ? (reverse ? other.points.back().v : r.points.back().v) : x));
        }
        unique(out);
        return out;
    }
    Value aggregate(const Node &n, int64_t t)
    {
        bool top = n.text == "topk" || n.text == "bottomk";
        if (n.args.size() != (top ? 2u : 1u))
            throw std::runtime_error("aggregation arity");
        double k = 0;
        if (top) {
            Value v = eval(*n.args[0], t);
            type(v, PP_SCALAR);
            k = v.scalar;
        }
        Value a = eval(*n.args.back(), t);
        type(a, PP_VECTOR);
        Value out;
        std::map<std::string, std::vector<Row> > groups;
        for (auto &r : a.rows) {
            tick();
            Labels g = n.grouping.empty() ? Labels{} : project(r.labels, n.labels, n.grouping == "by");
            groups[key(g)].push_back(std::move(r));
        }
        for (auto &entry : groups) {
            auto &rows = entry.second;
            if (top) {
                std::stable_sort(rows.begin(), rows.end(), [&](const Row &a, const Row &b) {
                    return valueLess(a.points.back().v, b.points.back().v, n.text == "topk");
                });
                size_t count =
                    std::isfinite(k) && k > 0 ? static_cast<size_t>(std::min<double>(std::floor(k), rows.size())) : 0;
                for (size_t i = 0; i < count; ++i)
                    out.rows.push_back(std::move(rows[i]));
                continue;
            }
            double x = rows[0].points.back().v;
            if (n.text == "sum" || n.text == "avg")
                x = 0;
            for (const auto &r : rows) {
                double v = r.points.back().v;
                if (n.text == "sum" || n.text == "avg")
                    x += v;
                else if (n.text == "min")
                    x = std::isnan(x) ? v : std::min(x, v);
                else if (n.text == "max")
                    x = std::isnan(x) ? v : std::max(x, v);
            }
            if (n.text == "avg")
                x /= rows.size();
            if (n.text == "count")
                x = rows.size();
            Labels g = n.grouping.empty() ? Labels{} : project(rows[0].labels, n.labels, n.grouping == "by");
            out.rows.push_back(sample(std::move(g), t, x));
        }
        return out;
    }
    static Labels absentLabels(const Node &n)
    {
        Labels ls;
        if (n.kind == Node::Selector)
            for (const auto &m : n.matchers)
                if (m.name != "__name__" && m.op == "=")
                    ls[m.name] = m.value;
        return ls;
    }
    Value call(const Node &n, int64_t t)
    {
        const std::string &fn = n.text;
        if (n.args.empty())
            throw std::runtime_error("function needs arguments");
        std::vector<Value> args;
        for (const auto &a : n.args)
            args.push_back(eval(*a, t));
        Value out;
        if (fn == "vector") {
            if (args.size() != 1)
                throw std::runtime_error("vector arity");
            type(args[0], PP_SCALAR);
            out.rows.push_back(sample({}, t, args[0].scalar));
            return out;
        }
        if (fn == "scalar") {
            if (args.size() != 1)
                throw std::runtime_error("scalar arity");
            type(args[0], PP_VECTOR);
            return scalar(args[0].rows.size() == 1 ? args[0].rows[0].points.back().v : NAN);
        }
        if (fn == "absent" || fn == "absent_over_time") {
            if (args.size() != 1)
                throw std::runtime_error("absent arity");
            type(args[0], fn == "absent" ? PP_VECTOR : PP_MATRIX);
            if (args[0].rows.empty())
                out.rows.push_back(sample(absentLabels(*n.args[0]), t, 1));
            return out;
        }
        if (fn == "histogram_quantile") {
            if (args.size() != 2)
                throw std::runtime_error("histogram_quantile arity");
            type(args[0], PP_SCALAR);
            type(args[1], PP_VECTOR);
            std::map<std::string, std::pair<Labels, std::vector<std::pair<double, double> > > > groups;
            for (const auto &r : args[1].rows) {
                tick();
                std::string le = label(r.labels, "le");
                char *end;
                double upper = std::strtod(le.c_str(), &end);
                if (le.empty() || *end)
                    continue;
                Labels ls = r.labels;
                ls.erase("__name__");
                ls.erase("le");
                auto &g = groups[key(ls)];
                g.first = ls;
                g.second.push_back({upper, r.points.back().v});
            }
            for (auto &entry : groups) {
                auto &bs = entry.second.second;
                std::sort(bs.begin(), bs.end());
                std::vector<std::pair<double, double> > buckets;
                for (auto b : bs) {
                    if (!buckets.empty() && buckets.back().first == b.first)
                        buckets.back().second += b.second;
                    else
                        buckets.push_back(b);
                }
                double x = NAN, phi = args[0].scalar;
                if (phi < 0)
                    x = -INFINITY;
                else if (phi > 1)
                    x = INFINITY;
                else if (buckets.size() >= 2 && buckets.back().first == INFINITY && buckets.back().second > 0) {
                    double previous = 0;
                    for (auto &b : buckets) {
                        b.second = std::max(previous, b.second);
                        previous = b.second;
                    }
                    double rank = phi * buckets.back().second;
                    size_t i = 0;
                    while (i + 1 < buckets.size() && buckets[i].second < rank)
                        ++i;
                    if (i == buckets.size() - 1)
                        x = buckets[i - 1].first;
                    else if (i == 0 && buckets[0].first <= 0)
                        x = buckets[0].first;
                    else {
                        double lo = i ? buckets[i - 1].first : 0, base = i ? buckets[i - 1].second : 0;
                        x = lo + (buckets[i].first - lo) * (rank - base) / (buckets[i].second - base);
                    }
                }
                out.rows.push_back(sample(entry.second.first, t, x));
            }
            return out;
        }
        if (fn == "label_replace" || fn == "label_join") {
            if ((fn == "label_replace" && args.size() != 5) || (fn == "label_join" && args.size() < 4))
                throw std::runtime_error("label function arity");
            type(args[0], PP_VECTOR);
            for (size_t i = 1; i < args.size(); ++i)
                type(args[i], 0);
            if (fn == "label_replace" && pp_match(args[4].text.c_str(), "") < 0)
                throw std::runtime_error("invalid regex");
            out = std::move(args[0]);
            for (auto &r : out.rows) {
                tick();
                std::string value;
                if (fn == "label_replace") {
                    char *raw = nullptr;
                    int rc = pp_replace(
                        args[4].text.c_str(), label(r.labels, args[3].text).c_str(), args[2].text.c_str(), &raw);
                    std::unique_ptr<char, decltype(&pp_string_free)> s(raw, pp_string_free);
                    if (rc < 0)
                        throw std::runtime_error("regex replacement failure");
                    if (!rc)
                        continue;
                    value = s.get();
                } else {
                    for (size_t i = 3; i < args.size(); ++i) {
                        if (i > 3)
                            value += args[2].text;
                        value += label(r.labels, args[i].text);
                    }
                }
                if (value.empty())
                    r.labels.erase(args[1].text);
                else
                    r.labels[args[1].text] = value;
            }
            unique(out);
            return out;
        }
        if (args[0].kind == PP_MATRIX) {
            if (args.size() != 1)
                throw std::runtime_error("range function arity");
            const bool rate = fn == "rate" || fn == "increase" || fn == "delta",
                       instant = fn == "irate" || fn == "idelta";
            const bool plain = fn == "avg_over_time" || fn == "sum_over_time" || fn == "min_over_time" ||
                               fn == "max_over_time" || fn == "count_over_time" || fn == "last_over_time" ||
                               fn == "changes" || fn == "resets";
            if (!rate && !instant && !plain)
                throw std::runtime_error("unsupported range function " + fn);
            const Node &selector = *n.args[0];
            int64_t end = (selector.at >= 0 ? selector.at : t) - selector.offset;
            for (auto &r : args[0].rows) {
                tick();
                const auto &ps = r.points;
                if ((rate || instant) && ps.size() < 2)
                    continue;
                double x = ps.back().v;
                if (rate || instant) {
                    bool counter = fn == "rate" || fn == "increase" || fn == "irate";
                    size_t first = instant ? ps.size() - 2 : 0;
                    x = ps.back().v - ps[first].v;
                    if (counter) {
                        if (instant) {
                            if (ps.back().v < ps[first].v)
                                x = ps.back().v;
                        } else
                            for (size_t i = 1; i < ps.size(); ++i) {
                                tick();
                                if (ps[i].v < ps[i - 1].v)
                                    x += ps[i - 1].v;
                            }
                    }
                    double elapsed = (ps.back().t - ps[first].t) / 1000.0;
                    if (elapsed <= 0)
                        continue;
                    if (instant) {
                        if (fn == "irate")
                            x /= elapsed;
                    } else {
                        double avg = elapsed / (ps.size() - 1),
                               start = (ps.front().t - (end - selector.range)) / 1000.0,
                               finish = (end - ps.back().t) / 1000.0, threshold = avg * 1.1;
                        if (start >= threshold)
                            start = avg / 2;
                        if (counter && x > 0 && ps.front().v >= 0)
                            start = std::min(start, elapsed * ps.front().v / x);
                        if (finish >= threshold)
                            finish = avg / 2;
                        x *= (elapsed + start + finish) / elapsed;
                        if (fn == "rate")
                            x /= selector.range / 1000.0;
                    }
                } else if (fn == "count_over_time")
                    x = ps.size();
                else if (fn == "changes" || fn == "resets") {
                    x = 0;
                    for (size_t i = 1; i < ps.size(); ++i) {
                        tick();
                        if (fn == "resets" ?
                                ps[i].v < ps[i - 1].v :
                                ps[i].v != ps[i - 1].v && !(std::isnan(ps[i].v) && std::isnan(ps[i - 1].v)))
                            ++x;
                    }
                } else if (fn != "last_over_time") {
                    x = (fn == "min_over_time" || fn == "max_over_time") ? ps.front().v : 0;
                    for (const auto &p : ps) {
                        tick();
                        if (fn == "min_over_time")
                            x = std::isnan(x) ? p.v : std::min(x, p.v);
                        else if (fn == "max_over_time")
                            x = std::isnan(x) ? p.v : std::max(x, p.v);
                        else
                            x += p.v;
                    }
                    if (fn == "avg_over_time")
                        x /= ps.size();
                }
                if (fn != "last_over_time")
                    r.labels.erase("__name__");
                out.rows.push_back(sample(r.labels, t, x));
            }
            unique(out);
            return out;
        }
        type(args[0], PP_VECTOR);
        out = std::move(args[0]);
        if (fn == "sort" || fn == "sort_desc") {
            if (args.size() != 1)
                throw std::runtime_error("sort arity");
            std::stable_sort(out.rows.begin(), out.rows.end(), [&](const Row &a, const Row &b) {
                return valueLess(a.points.back().v, b.points.back().v, fn == "sort_desc");
            });
            return out;
        }
        if (fn != "abs" && fn != "clamp_min" && fn != "clamp_max" && fn != "round")
            throw std::runtime_error("unsupported function " + fn);
        if ((fn == "abs" && args.size() != 1) || ((fn == "clamp_min" || fn == "clamp_max") && args.size() != 2) ||
            (fn == "round" && (args.size() < 1 || args.size() > 2)))
            throw std::runtime_error("function arity");
        double parameter = 1;
        if (args.size() > 1) {
            type(args[1], PP_SCALAR);
            parameter = args[1].scalar;
        }
        for (auto &r : out.rows) {
            tick();
            r.labels.erase("__name__");
            double &x = r.points.back().v;
            if (fn == "abs")
                x = std::fabs(x);
            else if (fn == "clamp_min")
                x = std::isnan(x) || std::isnan(parameter) ? NAN : std::max(x, parameter);
            else if (fn == "clamp_max")
                x = std::isnan(x) || std::isnan(parameter) ? NAN : std::min(x, parameter);
            else
                x = std::floor(x / parameter + 0.5) * parameter;
        }
        unique(out);
        return out;
    }

public:
    explicit Evaluator(const PPRequest &r) : req(r)
    {
    }
    uint64_t count() const
    {
        return work;
    }
    Value eval(const Node &n, int64_t t)
    {
        if (depth >= 128)
            throw std::runtime_error("evaluation depth limit");
        struct DepthGuard {
            unsigned &depth;
            explicit DepthGuard(unsigned &d) : depth(d)
            {
                ++depth;
            }
            ~DepthGuard()
            {
                --depth;
            }
        } guard(depth);
        tick();
        switch (n.kind) {
            case Node::Number:
                return scalar(n.number);
            case Node::String: {
                Value v;
                v.kind = 0;
                v.text = n.text;
                return v;
            }
            case Node::Selector:
                return select(n, t);
            case Node::Unary: {
                Value v = eval(*n.args[0], t);
                if (v.kind == PP_SCALAR) {
                    if (n.text == "-")
                        v.scalar = -v.scalar;
                } else {
                    type(v, PP_VECTOR);
                    if (n.text == "-")
                        for (auto &r : v.rows) {
                            r.labels.erase("__name__");
                            r.points.back().v = -r.points.back().v;
                        }
                    unique(v);
                }
                return v;
            }
            case Node::Binary: {
                Value a = eval(*n.args[0], t);
                Value b = eval(*n.args[1], t);
                return binary(n, std::move(a), std::move(b), t);
            }
            case Node::Aggregate:
                return aggregate(n, t);
            case Node::Call:
                return call(n, t);
            case Node::Subquery: {
                Value out;
                out.kind = PP_MATRIX;
                int64_t end = (n.at >= 0 ? n.at : t) - n.offset;
                int64_t begin = (end - n.range) / n.step * n.step;
                if (begin <= end - n.range)
                    begin += n.step;
                std::map<std::string, size_t> index;
                for (int64_t s = begin; s <= end; s += n.step) {
                    Value v = eval(*n.args[0], s);
                    type(v, PP_VECTOR);
                    for (auto &r : v.rows) {
                        std::string k = key(r.labels);
                        auto hit = index.find(k);
                        if (hit == index.end()) {
                            index[k] = out.rows.size();
                            out.rows.push_back({r.labels, {}});
                            hit = index.find(k);
                        }
                        out.rows[hit->second].points.push_back({s, r.points.back().v});
                    }
                }
                return out;
            }
        }
        throw std::runtime_error("invalid node");
    }
};
struct Owner {
    PPResult result{};
    std::string error;
    std::vector<Row> rows;
    std::vector<std::vector<PPLabel> > labels;
    std::vector<PPSeries> view;
};
PPResult emergency{PP_ERROR, nullptr, 0, "allocation failure", 0, nullptr};
} // namespace
extern "C" PPResult *pp_eval(const PPRequest *request)
{
    std::unique_ptr<Owner> owner;
    try {
        owner = std::make_unique<Owner>();
        owner->result.owner = owner.get();
        if (!request || !request->data || !request->query || request->lookback_ms <= 0 ||
            request->lookback_ms > 9000000000000000LL || request->time_ms < -9000000000000000LL ||
            request->time_ms > 9000000000000000LL)
            throw std::runtime_error("invalid request");
        Parser parser(request->query);
        auto ast = parser.parse();
        Evaluator evaluator(*request);
        try {
            Value v = evaluator.eval(*ast, request->time_ms);
            owner->result.kind = v.kind;
            if (v.kind == 0)
                throw std::runtime_error("string result unsupported");
            if (v.kind == PP_SCALAR)
                v.rows.push_back({{}, {{request->time_ms, v.scalar}}});
            if (request->inject_failure == 2)
                throw std::runtime_error("injected evaluation exception");
            if (request->inject_failure)
                throw std::bad_alloc();
            owner->rows = std::move(v.rows);
            owner->result.work = evaluator.count();
        } catch (...) {
            owner->result.work = evaluator.count();
            throw;
        }
        owner->labels.resize(owner->rows.size());
        owner->view.reserve(owner->rows.size());
        for (size_t i = 0; i < owner->rows.size(); ++i) {
            auto &r = owner->rows[i];
            auto &ls = owner->labels[i];
            for (const auto &l : r.labels)
                ls.push_back({l.first.c_str(), l.second.c_str()});
            owner->view.push_back({ls.data(), ls.size(), r.points.data(), r.points.size()});
        }
        owner->result.rows = owner->view.data();
        owner->result.rows_len = owner->view.size();
    } catch (const std::bad_alloc &) {
        return &emergency;
    } catch (const std::exception &e) {
        try {
            if (!owner) {
                owner = std::make_unique<Owner>();
                owner->result.owner = owner.get();
            }
            owner->error = e.what();
            owner->result.kind = PP_ERROR;
            owner->result.error = owner->error.c_str();
            owner->result.rows = nullptr;
            owner->result.rows_len = 0;
        } catch (...) {
            return &emergency;
        }
    } catch (...) {
        return &emergency;
    }
    PPResult *result = &owner->result;
    owner.release();
    return result;
}
extern "C" void pp_free(PPResult *result)
{
    if (result && result->owner)
        delete static_cast<Owner *>(result->owner);
}
