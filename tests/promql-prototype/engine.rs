// SPDX-License-Identifier: GPL-3.0-or-later
// Experimental float evaluator. No async runtime or external Rust dependencies.
use std::collections::{BTreeMap, BTreeSet};
use std::ffi::{c_char, c_void, CStr, CString};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::ptr;
use std::slice;

#[repr(C)]
pub struct PPLabel {
    name: *const c_char,
    value: *const c_char,
}
#[repr(C)]
#[derive(Clone, Copy)]
pub struct PPPoint {
    t: i64,
    v: f64,
}
#[repr(C)]
pub struct PPSeries {
    labels: *const PPLabel,
    labels_len: usize,
    points: *const PPPoint,
    points_len: usize,
}
#[repr(C)]
pub struct PPData {
    series: *const PPSeries,
    series_len: usize,
}
#[repr(C)]
pub struct PPRequest {
    data: *const PPData,
    query: *const c_char,
    time_ms: i64,
    lookback_ms: i64,
    work_limit: u64,
    cancelled: Option<unsafe extern "C" fn(*mut c_void) -> i32>,
    cancel_context: *mut c_void,
    inject_failure: i32,
}
#[repr(C)]
pub struct PPResult {
    kind: i32,
    rows: *const PPSeries,
    rows_len: usize,
    error: *const c_char,
    work: u64,
    owner: *mut c_void,
    parse_ns: u64,
    evaluation_ns: u64,
}
unsafe extern "C" {
    fn pp_match(pattern: *const c_char, subject: *const c_char) -> i32;
    fn pp_replace(
        pattern: *const c_char,
        subject: *const c_char,
        replacement: *const c_char,
        output: *mut *mut c_char,
    ) -> i32;
    fn pp_string_free(s: *mut c_char);
    fn pp_monotonic_ns() -> u64;
}
type R<T> = Result<T, &'static str>;
type Labels = BTreeMap<String, String>;
#[derive(Clone)]
struct Row {
    labels: Labels,
    points: Vec<PPPoint>,
}
struct Value {
    kind: i32,
    scalar: f64,
    text: String,
    rows: Vec<Row>,
}
impl Value {
    fn vector() -> Self {
        Self {
            kind: 2,
            scalar: 0.0,
            text: String::new(),
            rows: Vec::new(),
        }
    }
    fn scalar(x: f64) -> Self {
        Self {
            kind: 1,
            scalar: x,
            ..Self::vector()
        }
    }
}
#[derive(Clone, Copy, PartialEq)]
enum Kind {
    Number,
    String,
    Selector,
    Unary,
    Binary,
    Call,
    Aggregate,
    Subquery,
}
struct Matcher {
    name: String,
    op: String,
    value: String,
}
struct Node {
    kind: Kind,
    text: String,
    number: f64,
    range: i64,
    offset: i64,
    at: Option<i64>,
    step: i64,
    boolean: bool,
    grouping: i32,
    matching: i32,
    cardinality: i32,
    labels: Vec<String>,
    include: Vec<String>,
    matchers: Vec<Matcher>,
    args: Vec<Node>,
}
impl Node {
    fn new(kind: Kind) -> Self {
        Self {
            kind,
            text: String::new(),
            number: 0.0,
            range: 0,
            offset: 0,
            at: None,
            step: 0,
            boolean: false,
            grouping: 0,
            matching: 0,
            cardinality: 0,
            labels: Vec::new(),
            include: Vec::new(),
            matchers: Vec::new(),
            args: Vec::new(),
        }
    }
}
struct Token {
    text: String,
    kind: i32,
    number: f64,
}
struct Parser<'a> {
    bytes: &'a [u8],
    i: usize,
    tok: Token,
    depth: usize,
    tokens: usize,
}
impl<'a> Parser<'a> {
    fn new(s: &'a str) -> R<Self> {
        if s.len() > 65536 {
            return Err("query length limit");
        };
        let mut p = Self {
            bytes: s.as_bytes(),
            i: 0,
            tok: Token {
                text: String::new(),
                kind: 0,
                number: 0.0,
            },
            depth: 0,
            tokens: 0,
        };
        p.next()?;
        Ok(p)
    }
    fn next(&mut self) -> R<()> {
        self.tokens += 1;
        if self.tokens > 1024 {
            return Err("parse token limit");
        };
        while self.i < self.bytes.len() && self.bytes[self.i].is_ascii_whitespace() {
            self.i += 1;
        }
        if self.i == self.bytes.len() {
            self.tok = Token {
                text: String::new(),
                kind: 0,
                number: 0.0,
            };
            return Ok(());
        }
        let b = self.bytes[self.i];
        let start = self.i;
        if b == b'"' {
            self.i += 1;
            let mut s = Vec::new();
            while self.i < self.bytes.len() && self.bytes[self.i] != b'"' {
                let mut x = self.bytes[self.i];
                self.i += 1;
                if x == b'\\' {
                    if self.i == self.bytes.len() {
                        return Err("unfinished escape");
                    };
                    x = self.bytes[self.i];
                    self.i += 1;
                    x = match x {
                        b'n' => b'\n',
                        b't' => b'\t',
                        b'"' => b'"',
                        b'\\' => b'\\',
                        _ => return Err("unsupported string escape"),
                    };
                }
                s.push(x);
            }
            if self.i == self.bytes.len() {
                return Err("unfinished string");
            };
            self.i += 1;
            self.tok = Token {
                text: String::from_utf8(s).map_err(|_| "invalid UTF-8")?,
                kind: 3,
                number: 0.0,
            };
            return Ok(());
        }
        if b.is_ascii_digit()
            || (b == b'.' && self.bytes.get(self.i + 1).is_some_and(u8::is_ascii_digit))
        {
            self.i += 1;
            while self.i < self.bytes.len()
                && (self.bytes[self.i].is_ascii_digit() || self.bytes[self.i] == b'.')
            {
                self.i += 1;
            }
            if self
                .bytes
                .get(self.i)
                .is_some_and(|x| *x == b'e' || *x == b'E')
            {
                self.i += 1;
                if self
                    .bytes
                    .get(self.i)
                    .is_some_and(|x| *x == b'+' || *x == b'-')
                {
                    self.i += 1;
                }
                while self.bytes.get(self.i).is_some_and(u8::is_ascii_digit) {
                    self.i += 1;
                }
            }
            let s =
                std::str::from_utf8(&self.bytes[start..self.i]).map_err(|_| "invalid number")?;
            let n = s.parse().map_err(|_| "invalid number")?;
            self.tok = Token {
                text: s.to_owned(),
                kind: 2,
                number: n,
            };
            return Ok(());
        }
        if b.is_ascii_alphabetic() || b == b'_' {
            self.i += 1;
            while self.i < self.bytes.len()
                && (self.bytes[self.i].is_ascii_alphanumeric() || self.bytes[self.i] == b'_')
            {
                self.i += 1;
            }
            self.tok = Token {
                text: std::str::from_utf8(&self.bytes[start..self.i])
                    .map_err(|_| "invalid identifier")?
                    .to_owned(),
                kind: 1,
                number: 0.0,
            };
            return Ok(());
        }
        self.i += 1;
        if self.i < self.bytes.len()
            && ((self.bytes[self.i] == b'=' && matches!(b, b'=' | b'!' | b'<' | b'>'))
                || (self.bytes[self.i] == b'~' && matches!(b, b'=' | b'!')))
        {
            self.i += 1;
        }
        self.tok = Token {
            text: std::str::from_utf8(&self.bytes[start..self.i])
                .map_err(|_| "unexpected character")?
                .to_owned(),
            kind: 4,
            number: 0.0,
        };
        Ok(())
    }
    fn eat(&mut self, s: &str) -> R<bool> {
        if self.tok.kind == 3 || self.tok.text != s {
            return Ok(false);
        }
        self.next()?;
        Ok(true)
    }
    fn need(&mut self, s: &str) -> R<()> {
        if self.eat(s)? {
            Ok(())
        } else {
            Err("unexpected token")
        }
    }
    fn names(&mut self) -> R<Vec<String>> {
        self.need("(")?;
        let mut v = Vec::new();
        if !self.eat(")")? {
            loop {
                if self.tok.kind != 1 && self.tok.kind != 3 {
                    return Err("expected label");
                };
                v.push(self.tok.text.clone());
                self.next()?;
                if !self.eat(",")? {
                    break;
                }
            }
            self.need(")")?;
        }
        Ok(v)
    }
    fn duration(&mut self) -> R<i64> {
        if self.tok.kind != 2 {
            return Err("expected duration");
        };
        let n = self.tok.number;
        self.next()?;
        if self.tok.kind != 1 {
            return Err("duration unit required");
        };
        let f = match self.tok.text.as_str() {
            "ms" => 1.0,
            "s" => 1000.0,
            "m" => 60000.0,
            "h" => 3600000.0,
            "d" => 86400000.0,
            "w" => 604800000.0,
            "y" => 31536000000.0,
            _ => return Err("unsupported duration"),
        };
        if !n.is_finite() || n * f < 1.0 || n * f > 9e15 {
            return Err("unsupported duration");
        };
        self.next()?;
        Ok((n * f) as i64)
    }
    fn primary(&mut self) -> R<Node> {
        let mut n;
        if self.tok.kind != 3 && (self.tok.text == "+" || self.tok.text == "-") {
            n = Node::new(Kind::Unary);
            n.text = self.tok.text.clone();
            self.next()?;
            n.args.push(self.expr(6)?);
        } else if self.tok.kind == 2 {
            n = Node::new(Kind::Number);
            n.number = self.tok.number;
            self.next()?;
        } else if self.tok.kind == 3 {
            n = Node::new(Kind::String);
            n.text = self.tok.text.clone();
            self.next()?;
        } else if self.eat("(")? {
            n = self.expr(0)?;
            self.need(")")?;
        } else if self.tok.kind == 1 || self.tok.text == "{" {
            let mut name = String::new();
            if self.tok.kind == 1 {
                name = self.tok.text.clone();
                self.next()?;
            }
            if matches!(self.tok.text.as_str(), "(" | "by" | "without") {
                let agg = matches!(
                    name.as_str(),
                    "sum" | "avg" | "min" | "max" | "count" | "topk" | "bottomk"
                );
                n = Node::new(if agg { Kind::Aggregate } else { Kind::Call });
                n.text = name;
                if self.tok.text == "by" || self.tok.text == "without" {
                    n.grouping = if self.tok.text == "by" { 1 } else { 2 };
                    self.next()?;
                    n.labels = self.names()?;
                }
                self.need("(")?;
                if !self.eat(")")? {
                    loop {
                        n.args.push(self.expr(0)?);
                        if !self.eat(",")? {
                            break;
                        }
                    }
                    self.need(")")?;
                }
                if agg && (self.tok.text == "by" || self.tok.text == "without") {
                    n.grouping = if self.tok.text == "by" { 1 } else { 2 };
                    self.next()?;
                    n.labels = self.names()?;
                }
            } else {
                n = Node::new(Kind::Selector);
                if !name.is_empty() {
                    n.matchers.push(Matcher {
                        name: "__name__".into(),
                        op: "=".into(),
                        value: name,
                    });
                }
                if self.eat("{")? {
                    if !self.eat("}")? {
                        loop {
                            if self.tok.kind == 3 {
                                n.matchers.push(Matcher {
                                    name: "__name__".into(),
                                    op: "=".into(),
                                    value: self.tok.text.clone(),
                                });
                                self.next()?;
                            } else {
                                if self.tok.kind != 1 {
                                    return Err("expected matcher label");
                                };
                                let label = self.tok.text.clone();
                                self.next()?;
                                let op = self.tok.text.clone();
                                if !matches!(op.as_str(), "=" | "!=" | "=~" | "!~") {
                                    return Err("expected matcher operator");
                                };
                                self.next()?;
                                if self.tok.kind != 3 {
                                    return Err("matcher needs string");
                                };
                                n.matchers.push(Matcher {
                                    name: label,
                                    op,
                                    value: self.tok.text.clone(),
                                });
                                self.next()?;
                            }
                            if !self.eat(",")? || self.tok.text == "}" {
                                break;
                            }
                        }
                        self.need("}")?;
                    }
                }
                if n.matchers.is_empty() {
                    return Err("empty selector");
                };
            }
        } else {
            return Err("expected expression");
        }
        if self.eat("[")? {
            let range = self.duration()?;
            if self.eat(":")? {
                let mut sub = Node::new(Kind::Subquery);
                sub.range = range;
                sub.step = if self.tok.text == "]" {
                    60000
                } else {
                    self.duration()?
                };
                sub.args.push(n);
                n = sub;
            } else {
                if n.kind != Kind::Selector {
                    return Err("range requires selector");
                };
                n.range = range;
            }
            self.need("]")?;
        }
        loop {
            if self.eat("offset")? {
                n.offset = self.duration()?;
            } else if self.eat("@")? {
                if self.tok.kind != 2
                    || !self.tok.number.is_finite()
                    || self.tok.number < 0.0
                    || self.tok.number > 9e12
                {
                    return Err("numeric @ required");
                };
                n.at = Some((self.tok.number * 1000.0) as i64);
                self.next()?;
            } else {
                break;
            }
        }
        Ok(n)
    }
    fn precedence(&self) -> i32 {
        if self.tok.kind == 3 {
            return -1;
        }
        match self.tok.text.as_str() {
            "or" => 1,
            "and" | "unless" => 2,
            "==" | "!=" | "<" | ">" | "<=" | ">=" => 3,
            "+" | "-" => 4,
            "*" | "/" | "%" => 5,
            "^" => 7,
            _ => -1,
        }
    }
    fn expr(&mut self, minimum: i32) -> R<Node> {
        self.depth += 1;
        if self.depth > 128 {
            return Err("parse depth limit");
        };
        let mut lhs = self.primary()?;
        while self.precedence() >= minimum {
            let op = self.tok.text.clone();
            let prec = self.precedence();
            self.next()?;
            let mut n = Node::new(Kind::Binary);
            n.text = op.clone();
            n.boolean = self.eat("bool")?;
            if self.tok.text == "on" || self.tok.text == "ignoring" {
                n.matching = if self.tok.text == "on" { 1 } else { 2 };
                self.next()?;
                n.labels = self.names()?;
            }
            if self.tok.text == "group_left" || self.tok.text == "group_right" {
                n.cardinality = if self.tok.text == "group_left" { 1 } else { 2 };
                self.next()?;
                if self.tok.text == "(" {
                    n.include = self.names()?;
                }
            }
            n.args.push(lhs);
            n.args
                .push(self.expr(prec + if op == "^" { 0 } else { 1 })?);
            lhs = n;
        }
        self.depth -= 1;
        Ok(lhs)
    }
    fn parse(mut self) -> R<Node> {
        let n = self.expr(0)?;
        if self.tok.kind != 0 {
            return Err("unexpected trailing token");
        };
        Ok(n)
    }
}
fn label<'a>(ls: &'a Labels, name: &str) -> &'a str {
    ls.get(name).map_or("", String::as_str)
}
fn project(ls: &Labels, names: &[String], keep: bool) -> Labels {
    ls.iter()
        .filter(|(k, _)| names.contains(k) == keep && (keep || k.as_str() != "__name__"))
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect()
}
fn unique(v: &Value) -> R<()> {
    let mut seen = BTreeSet::new();
    for r in &v.rows {
        if !seen.insert(&r.labels) {
            return Err("duplicate output labelset");
        }
    }
    Ok(())
}
fn check(v: &Value, k: i32) -> R<()> {
    if v.kind == k {
        Ok(())
    } else {
        Err("wrong argument type")
    }
}
fn sample(labels: Labels, t: i64, x: f64) -> Row {
    Row {
        labels,
        points: vec![PPPoint { t, v: x }],
    }
}
fn point(r: &Row) -> f64 {
    r.points.last().unwrap().v
}
fn comparison(op: &str) -> bool {
    matches!(op, "==" | "!=" | "<" | ">" | "<=" | ">=")
}
fn operation(op: &str, a: f64, b: f64) -> R<f64> {
    Ok(match op {
        "+" => a + b,
        "-" => a - b,
        "*" => a * b,
        "/" => a / b,
        "%" => a % b,
        "^" => a.powf(b),
        "==" => {
            if a == b {
                1.0
            } else {
                0.0
            }
        }
        "!=" => {
            if a != b {
                1.0
            } else {
                0.0
            }
        }
        "<" => {
            if a < b {
                1.0
            } else {
                0.0
            }
        }
        ">" => {
            if a > b {
                1.0
            } else {
                0.0
            }
        }
        "<=" => {
            if a <= b {
                1.0
            } else {
                0.0
            }
        }
        ">=" => {
            if a >= b {
                1.0
            } else {
                0.0
            }
        }
        _ => return Err("unsupported operator"),
    })
}
fn signature(r: &Row, n: &Node) -> Labels {
    if n.matching == 0 {
        project(&r.labels, &["__name__".into()], false)
    } else {
        project(&r.labels, &n.labels, n.matching == 1)
    }
}
fn order(a: f64, b: f64, desc: bool) -> std::cmp::Ordering {
    if a.is_nan() {
        return if b.is_nan() {
            std::cmp::Ordering::Equal
        } else {
            std::cmp::Ordering::Greater
        };
    }
    if b.is_nan() {
        return std::cmp::Ordering::Less;
    }
    let ord = a.partial_cmp(&b).unwrap();
    if desc {
        ord.reverse()
    } else {
        ord
    }
}
struct Evaluator<'a> {
    req: &'a PPRequest,
    work: u64,
    depth: usize,
}
impl<'a> Evaluator<'a> {
    fn tick(&mut self) -> R<()> {
        self.work += 1;
        if self.req.work_limit != 0 && self.work > self.req.work_limit {
            return Err("work budget exceeded");
        };
        if let Some(f) = self.req.cancelled {
            if unsafe { f(self.req.cancel_context) } != 0 {
                return Err("cancelled");
            }
        }
        Ok(())
    }
    fn selector(&mut self, n: &Node, time: i64) -> R<Value> {
        let mut out = Value::vector();
        if n.range != 0 {
            out.kind = 3;
        }
        let t = n.at.unwrap_or(time) - n.offset;
        let data = unsafe { &*self.req.data };
        let rows = if data.series_len == 0 {
            &[]
        } else {
            unsafe { slice::from_raw_parts(data.series, data.series_len) }
        };
        for s in rows {
            self.tick()?;
            let mut ls = Labels::new();
            let labels = if s.labels_len == 0 {
                &[]
            } else {
                unsafe { slice::from_raw_parts(s.labels, s.labels_len) }
            };
            let mut ok = true;
            for m in &n.matchers {
                let mut subject = b"\0".as_ptr().cast::<c_char>();
                for l in labels {
                    let name = unsafe { CStr::from_ptr(l.name) }
                        .to_str()
                        .map_err(|_| "invalid label UTF-8")?;
                    if name == m.name {
                        subject = l.value;
                        break;
                    }
                }
                let x = unsafe { CStr::from_ptr(subject) }
                    .to_str()
                    .map_err(|_| "invalid label UTF-8")?;
                let mut matched = if m.op == "=" || m.op == "!=" {
                    x == m.value
                } else {
                    let p = CString::new(m.value.as_str()).map_err(|_| "regex contains NUL")?;
                    let r = unsafe { pp_match(p.as_ptr(), subject) };
                    if r < 0 {
                        return Err("invalid regex");
                    };
                    r != 0
                };
                if m.op == "!=" || m.op == "!~" {
                    matched = !matched
                };
                if !matched {
                    ok = false;
                    break;
                }
            }
            if !ok {
                continue;
            }
            for l in labels {
                ls.insert(
                    unsafe { CStr::from_ptr(l.name) }
                        .to_str()
                        .map_err(|_| "invalid label UTF-8")?
                        .to_owned(),
                    unsafe { CStr::from_ptr(l.value) }
                        .to_str()
                        .map_err(|_| "invalid label UTF-8")?
                        .to_owned(),
                );
            }
            let ps = if s.points_len == 0 {
                &[]
            } else {
                unsafe { slice::from_raw_parts(s.points, s.points_len) }
            };
            let mut points = Vec::new();
            for p in ps {
                self.tick()?;
                if p.t <= t
                    && p.t
                        > t - if n.range != 0 {
                            n.range
                        } else {
                            self.req.lookback_ms
                        }
                {
                    points.push(*p);
                }
            }
            if points.is_empty() {
                continue;
            }
            if n.range == 0 {
                let v = points.last().unwrap().v;
                points = vec![PPPoint { t: time, v }];
            }
            out.rows.push(Row { labels: ls, points });
        }
        Ok(out)
    }
    fn binary(&mut self, n: &Node, a: Value, b: Value, t: i64) -> R<Value> {
        let cmp = comparison(&n.text);
        let set = matches!(n.text.as_str(), "or" | "and" | "unless");
        let mut out = Value::vector();
        if a.kind == 1 && b.kind == 1 {
            if set || (cmp && !n.boolean) {
                return Err("scalar comparison requires bool");
            };
            return Ok(Value::scalar(operation(&n.text, a.scalar, b.scalar)?));
        }
        if a.kind == 1 || b.kind == 1 {
            if set || n.matching != 0 || n.cardinality != 0 {
                return Err("vector matching requires vectors");
            };
            let left = a.kind == 2;
            let scalar = if left { b.scalar } else { a.scalar };
            let v = if left { a } else { b };
            check(&v, 2)?;
            for mut r in v.rows {
                self.tick()?;
                let old = point(&r);
                let x = operation(
                    &n.text,
                    if left { old } else { scalar },
                    if left { scalar } else { old },
                )?;
                if cmp && !n.boolean && x == 0.0 {
                    continue;
                }
                if !cmp || n.boolean {
                    r.labels.remove("__name__");
                }
                out.rows
                    .push(sample(r.labels, t, if cmp && !n.boolean { old } else { x }));
            }
            unique(&out)?;
            return Ok(out);
        }
        check(&a, 2)?;
        check(&b, 2)?;
        if set {
            if n.boolean || n.cardinality != 0 {
                return Err("invalid set modifier");
            };
            let right: BTreeSet<Labels> = b.rows.iter().map(|r| signature(r, n)).collect();
            let left: BTreeSet<Labels> = a.rows.iter().map(|r| signature(r, n)).collect();
            for r in a.rows {
                self.tick()?;
                let k = signature(&r, n);
                if n.text == "or" || ((n.text == "and") == right.contains(&k)) {
                    out.rows.push(r);
                }
            }
            if n.text == "or" {
                for r in b.rows {
                    if !left.contains(&signature(&r, n)) {
                        out.rows.push(r);
                    }
                }
            }
            return Ok(out);
        }
        if a.rows.is_empty() || b.rows.is_empty() {
            return Ok(out);
        }
        let reverse = n.cardinality == 2;
        let (many, one) = if reverse {
            (b.rows, a.rows)
        } else {
            (a.rows, b.rows)
        };
        let mut index = BTreeMap::new();
        for r in &one {
            if index.insert(signature(r, n), r).is_some() {
                return Err("duplicate series on one side");
            }
        }
        let mut used = BTreeSet::new();
        for mut r in many {
            self.tick()?;
            let k = signature(&r, n);
            let Some(other) = index.get(&k) else { continue };
            if n.cardinality == 0 && !used.insert(k) {
                return Err("many-to-one needs group modifier");
            };
            let lhs = if reverse { point(other) } else { point(&r) };
            let x = operation(&n.text, lhs, if reverse { point(&r) } else { point(other) })?;
            if cmp && !n.boolean && x == 0.0 {
                continue;
            }
            if !cmp || n.boolean {
                r.labels.remove("__name__");
            }
            if n.cardinality == 0 && n.matching != 0 {
                r.labels = project(&r.labels, &n.labels, n.matching == 1);
            }
            for name in &n.include {
                let v = label(&other.labels, name);
                if v.is_empty() {
                    r.labels.remove(name);
                } else {
                    r.labels.insert(name.clone(), v.to_owned());
                }
            }
            out.rows
                .push(sample(r.labels, t, if cmp && !n.boolean { lhs } else { x }));
        }
        unique(&out)?;
        Ok(out)
    }
    fn aggregate(&mut self, n: &Node, t: i64) -> R<Value> {
        let top = n.text == "topk" || n.text == "bottomk";
        if n.args.len() != if top { 2 } else { 1 } {
            return Err("aggregation arity");
        };
        let mut k = 0.0;
        if top {
            let v = self.eval(&n.args[0], t)?;
            check(&v, 1)?;
            k = v.scalar;
        }
        let a = self.eval(n.args.last().unwrap(), t)?;
        check(&a, 2)?;
        let mut groups: BTreeMap<Labels, Vec<Row>> = BTreeMap::new();
        for r in a.rows {
            self.tick()?;
            let ls = if n.grouping == 0 {
                Labels::new()
            } else {
                project(&r.labels, &n.labels, n.grouping == 1)
            };
            groups.entry(ls).or_default().push(r);
        }
        let mut out = Value::vector();
        for (ls, mut rows) in groups {
            if top {
                rows.sort_by(|a, b| order(point(a), point(b), n.text == "topk"));
                let count = if k.is_finite() && k > 0.0 {
                    (k.floor() as usize).min(rows.len())
                } else {
                    0
                };
                out.rows.extend(rows.into_iter().take(count));
                continue;
            }
            let mut x = if n.text == "sum" || n.text == "avg" {
                0.0
            } else {
                point(&rows[0])
            };
            for r in &rows {
                self.tick()?;
                let v = point(r);
                match n.text.as_str() {
                    "sum" | "avg" => x += v,
                    "min" => x = if x.is_nan() { v } else { x.min(v) },
                    "max" => x = if x.is_nan() { v } else { x.max(v) },
                    _ => {}
                }
            }
            if n.text == "avg" {
                x /= rows.len() as f64;
            }
            if n.text == "count" {
                x = rows.len() as f64;
            }
            out.rows.push(sample(ls, t, x));
        }
        Ok(out)
    }
    fn absent_labels(n: &Node) -> Labels {
        let mut ls = Labels::new();
        if n.kind == Kind::Selector {
            for m in &n.matchers {
                if m.name != "__name__" && m.op == "=" {
                    ls.insert(m.name.clone(), m.value.clone());
                }
            }
        }
        ls
    }
    fn call(&mut self, n: &Node, t: i64) -> R<Value> {
        if n.args.is_empty() {
            return Err("function needs arguments");
        };
        let mut args = Vec::new();
        for a in &n.args {
            args.push(self.eval(a, t)?);
        }
        let fnc = n.text.as_str();
        let mut out = Value::vector();
        if fnc == "vector" {
            if args.len() != 1 {
                return Err("vector arity");
            };
            check(&args[0], 1)?;
            out.rows.push(sample(Labels::new(), t, args[0].scalar));
            return Ok(out);
        }
        if fnc == "scalar" {
            if args.len() != 1 {
                return Err("scalar arity");
            };
            check(&args[0], 2)?;
            return Ok(Value::scalar(if args[0].rows.len() == 1 {
                point(&args[0].rows[0])
            } else {
                f64::NAN
            }));
        }
        if fnc == "absent" || fnc == "absent_over_time" {
            if args.len() != 1 {
                return Err("absent arity");
            };
            check(&args[0], if fnc == "absent" { 2 } else { 3 })?;
            if args[0].rows.is_empty() {
                out.rows
                    .push(sample(Self::absent_labels(&n.args[0]), t, 1.0));
            }
            return Ok(out);
        }
        if fnc == "histogram_quantile" {
            if args.len() != 2 {
                return Err("histogram_quantile arity");
            };
            check(&args[0], 1)?;
            check(&args[1], 2)?;
            let phi = args[0].scalar;
            let mut groups: BTreeMap<Labels, Vec<(f64, f64)>> = BTreeMap::new();
            for r in &args[1].rows {
                self.tick()?;
                let le = label(&r.labels, "le");
                let upper = match le {
                    "+Inf" => f64::INFINITY,
                    _ => match le.parse::<f64>() {
                        Ok(x) => x,
                        Err(_) => continue,
                    },
                };
                let mut ls = r.labels.clone();
                ls.remove("__name__");
                ls.remove("le");
                groups.entry(ls).or_default().push((upper, point(r)));
            }
            for (ls, mut bs) in groups {
                bs.sort_by(|a, b| a.0.total_cmp(&b.0));
                let mut buckets: Vec<(f64, f64)> = Vec::new();
                for b in bs {
                    if buckets.last().is_some_and(|p| p.0 == b.0) {
                        buckets.last_mut().unwrap().1 += b.1;
                    } else {
                        buckets.push(b);
                    }
                }
                let mut x = f64::NAN;
                if phi < 0.0 {
                    x = f64::NEG_INFINITY;
                } else if phi > 1.0 {
                    x = f64::INFINITY;
                } else if buckets.len() >= 2
                    && buckets.last().unwrap().0 == f64::INFINITY
                    && buckets.last().unwrap().1 > 0.0
                {
                    let mut prev = 0.0;
                    for b in &mut buckets {
                        b.1 = b.1.max(prev);
                        prev = b.1;
                    }
                    let rank = phi * buckets.last().unwrap().1;
                    let mut i = 0;
                    while i + 1 < buckets.len() && buckets[i].1 < rank {
                        i += 1;
                    }
                    if i == buckets.len() - 1 {
                        x = buckets[i - 1].0;
                    } else if i == 0 && buckets[0].0 <= 0.0 {
                        x = buckets[0].0;
                    } else {
                        let (lo, base) = if i == 0 { (0.0, 0.0) } else { buckets[i - 1] };
                        x = lo + (buckets[i].0 - lo) * (rank - base) / (buckets[i].1 - base);
                    }
                }
                out.rows.push(sample(ls, t, x));
            }
            return Ok(out);
        }
        if fnc == "label_replace" || fnc == "label_join" {
            if (fnc == "label_replace" && args.len() != 5)
                || (fnc == "label_join" && args.len() < 4)
            {
                return Err("label function arity");
            };
            check(&args[0], 2)?;
            for a in &args[1..] {
                check(a, 0)?;
            }
            let mut rows = std::mem::take(&mut args[0].rows);
            if fnc == "label_replace" {
                let pat = CString::new(args[4].text.as_str()).map_err(|_| "regex contains NUL")?;
                let empty = CString::new("").unwrap();
                if unsafe { pp_match(pat.as_ptr(), empty.as_ptr()) } < 0 {
                    return Err("invalid regex");
                };
            }
            for r in &mut rows {
                self.tick()?;
                let value = if fnc == "label_replace" {
                    let pattern =
                        CString::new(args[4].text.as_str()).map_err(|_| "regex contains NUL")?;
                    let subject = CString::new(label(&r.labels, &args[3].text))
                        .map_err(|_| "label contains NUL")?;
                    let replacement = CString::new(args[2].text.as_str())
                        .map_err(|_| "replacement contains NUL")?;
                    let mut raw = ptr::null_mut();
                    let rc = unsafe {
                        pp_replace(
                            pattern.as_ptr(),
                            subject.as_ptr(),
                            replacement.as_ptr(),
                            &mut raw,
                        )
                    };
                    if rc < 0 {
                        return Err("regex replacement failure");
                    };
                    if rc == 0 {
                        continue;
                    }
                    struct Guard(*mut c_char);
                    impl Drop for Guard {
                        fn drop(&mut self) {
                            unsafe { pp_string_free(self.0) }
                        }
                    }
                    let guard = Guard(raw);
                    unsafe { CStr::from_ptr(guard.0) }
                        .to_str()
                        .map_err(|_| "invalid replacement UTF-8")?
                        .to_owned()
                } else {
                    args[3..]
                        .iter()
                        .map(|a| label(&r.labels, &a.text))
                        .collect::<Vec<_>>()
                        .join(&args[2].text)
                };
                if value.is_empty() {
                    r.labels.remove(&args[1].text);
                } else {
                    r.labels.insert(args[1].text.clone(), value);
                }
            }
            out.rows = rows;
            unique(&out)?;
            return Ok(out);
        }
        if args[0].kind == 3 {
            if args.len() != 1 {
                return Err("range function arity");
            };
            let rate = matches!(fnc, "rate" | "increase" | "delta");
            let instant = matches!(fnc, "irate" | "idelta");
            let plain = matches!(
                fnc,
                "avg_over_time"
                    | "sum_over_time"
                    | "min_over_time"
                    | "max_over_time"
                    | "count_over_time"
                    | "last_over_time"
                    | "changes"
                    | "resets"
            );
            if !rate && !instant && !plain {
                return Err("unsupported range function");
            };
            let selector = &n.args[0];
            let end = selector.at.unwrap_or(t) - selector.offset;
            for mut r in std::mem::take(&mut args[0].rows) {
                self.tick()?;
                let ps = &r.points;
                if (rate || instant) && ps.len() < 2 {
                    continue;
                }
                let mut x = point(&r);
                if rate || instant {
                    let counter = matches!(fnc, "rate" | "increase" | "irate");
                    let first = if instant { ps.len() - 2 } else { 0 };
                    x = ps.last().unwrap().v - ps[first].v;
                    if counter {
                        if instant {
                            if ps.last().unwrap().v < ps[first].v {
                                x = ps.last().unwrap().v;
                            }
                        } else {
                            for i in 1..ps.len() {
                                self.tick()?;
                                if ps[i].v < ps[i - 1].v {
                                    x += ps[i - 1].v;
                                }
                            }
                        }
                    }
                    let elapsed = (ps.last().unwrap().t - ps[first].t) as f64 / 1000.0;
                    if elapsed <= 0.0 {
                        continue;
                    }
                    if instant {
                        if fnc == "irate" {
                            x /= elapsed;
                        }
                    } else {
                        let avg = elapsed / (ps.len() - 1) as f64;
                        let mut start = (ps[0].t - (end - selector.range)) as f64 / 1000.0;
                        let mut finish = (end - ps.last().unwrap().t) as f64 / 1000.0;
                        let threshold = avg * 1.1;
                        if start >= threshold {
                            start = avg / 2.0;
                        }
                        if counter && x > 0.0 && ps[0].v >= 0.0 {
                            start = start.min(elapsed * ps[0].v / x);
                        }
                        if finish >= threshold {
                            finish = avg / 2.0;
                        }
                        x *= (elapsed + start + finish) / elapsed;
                        if fnc == "rate" {
                            x /= selector.range as f64 / 1000.0;
                        }
                    }
                } else if fnc == "count_over_time" {
                    x = ps.len() as f64;
                } else if fnc == "changes" || fnc == "resets" {
                    x = 0.0;
                    for i in 1..ps.len() {
                        self.tick()?;
                        if if fnc == "resets" {
                            ps[i].v < ps[i - 1].v
                        } else {
                            ps[i].v != ps[i - 1].v && !(ps[i].v.is_nan() && ps[i - 1].v.is_nan())
                        } {
                            x += 1.0;
                        }
                    }
                } else if fnc != "last_over_time" {
                    x = if fnc == "min_over_time" || fnc == "max_over_time" {
                        ps[0].v
                    } else {
                        0.0
                    };
                    for p in ps {
                        self.tick()?;
                        x = match fnc {
                            "min_over_time" => {
                                if x.is_nan() {
                                    p.v
                                } else {
                                    x.min(p.v)
                                }
                            }
                            "max_over_time" => {
                                if x.is_nan() {
                                    p.v
                                } else {
                                    x.max(p.v)
                                }
                            }
                            _ => x + p.v,
                        };
                    }
                    if fnc == "avg_over_time" {
                        x /= ps.len() as f64;
                    }
                }
                if fnc != "last_over_time" {
                    r.labels.remove("__name__");
                }
                out.rows.push(sample(r.labels, t, x));
            }
            unique(&out)?;
            return Ok(out);
        }
        check(&args[0], 2)?;
        out.rows = std::mem::take(&mut args[0].rows);
        if fnc == "sort" || fnc == "sort_desc" {
            if args.len() != 1 {
                return Err("sort arity");
            };
            out.rows
                .sort_by(|a, b| order(point(a), point(b), fnc == "sort_desc"));
            return Ok(out);
        }
        if !matches!(fnc, "abs" | "clamp_min" | "clamp_max" | "round") {
            return Err("unsupported function");
        };
        if (fnc == "abs" && args.len() != 1)
            || (matches!(fnc, "clamp_min" | "clamp_max") && args.len() != 2)
            || (fnc == "round" && (args.is_empty() || args.len() > 2))
        {
            return Err("function arity");
        };
        let mut parameter = 1.0;
        if args.len() > 1 {
            check(&args[1], 1)?;
            parameter = args[1].scalar;
        }
        for r in &mut out.rows {
            self.tick()?;
            r.labels.remove("__name__");
            let x = &mut r.points.last_mut().unwrap().v;
            *x = match fnc {
                "abs" => x.abs(),
                "clamp_min" => {
                    if x.is_nan() || parameter.is_nan() {
                        f64::NAN
                    } else {
                        x.max(parameter)
                    }
                }
                "clamp_max" => {
                    if x.is_nan() || parameter.is_nan() {
                        f64::NAN
                    } else {
                        x.min(parameter)
                    }
                }
                _ => ((*x / parameter) + 0.5).floor() * parameter,
            };
        }
        unique(&out)?;
        Ok(out)
    }
    fn eval(&mut self, n: &Node, t: i64) -> R<Value> {
        if self.depth >= 128 {
            return Err("evaluation depth limit");
        };
        self.depth += 1;
        let result = self.eval_inner(n, t);
        self.depth -= 1;
        result
    }
    fn eval_inner(&mut self, n: &Node, t: i64) -> R<Value> {
        self.tick()?;
        match n.kind {
            Kind::Number => Ok(Value::scalar(n.number)),
            Kind::String => Ok(Value {
                kind: 0,
                text: n.text.clone(),
                ..Value::vector()
            }),
            Kind::Selector => self.selector(n, t),
            Kind::Unary => {
                let mut v = self.eval(&n.args[0], t)?;
                if v.kind == 1 {
                    if n.text == "-" {
                        v.scalar = -v.scalar;
                    }
                } else {
                    check(&v, 2)?;
                    if n.text == "-" {
                        for r in &mut v.rows {
                            r.labels.remove("__name__");
                            r.points.last_mut().unwrap().v = -point(r);
                        }
                    }
                    unique(&v)?;
                }
                Ok(v)
            }
            Kind::Binary => {
                let a = self.eval(&n.args[0], t)?;
                let b = self.eval(&n.args[1], t)?;
                self.binary(n, a, b, t)
            }
            Kind::Aggregate => self.aggregate(n, t),
            Kind::Call => self.call(n, t),
            Kind::Subquery => {
                let mut out = Value::vector();
                out.kind = 3;
                let end = n.at.unwrap_or(t) - n.offset;
                let begin = (end - n.range).div_euclid(n.step) * n.step + n.step;
                let mut index: BTreeMap<Labels, Vec<PPPoint>> = BTreeMap::new();
                let mut s = begin;
                while s <= end {
                    let v = self.eval(&n.args[0], s)?;
                    check(&v, 2)?;
                    for r in v.rows {
                        let v = point(&r);
                        index.entry(r.labels).or_default().push(PPPoint { t: s, v });
                    }
                    s += n.step;
                }
                out.rows = index
                    .into_iter()
                    .map(|(labels, points)| Row { labels, points })
                    .collect();
                Ok(out)
            }
        }
    }
}
struct Owned {
    result: PPResult,
    _strings: Vec<Vec<(CString, CString)>>,
    _labels: Vec<Vec<PPLabel>>,
    _points: Vec<Vec<PPPoint>>,
    _rows: Vec<PPSeries>,
    _error: Option<CString>,
}
fn own(v: Value, time: i64, work: u64) -> R<*mut PPResult> {
    let mut rows = v.rows;
    if v.kind == 1 {
        rows.push(sample(Labels::new(), time, v.scalar));
    }
    let mut strings = Vec::new();
    let mut points = Vec::new();
    for r in rows {
        let mut ls = Vec::new();
        for (k, v) in r.labels {
            ls.push((
                CString::new(k).map_err(|_| "label contains NUL")?,
                CString::new(v).map_err(|_| "label contains NUL")?,
            ));
        }
        strings.push(ls);
        points.push(r.points);
    }
    let labels: Vec<Vec<PPLabel>> = strings
        .iter()
        .map(|ls| {
            ls.iter()
                .map(|(k, v)| PPLabel {
                    name: k.as_ptr(),
                    value: v.as_ptr(),
                })
                .collect()
        })
        .collect();
    let views: Vec<PPSeries> = labels
        .iter()
        .zip(&points)
        .map(|(ls, ps)| PPSeries {
            labels: ls.as_ptr(),
            labels_len: ls.len(),
            points: ps.as_ptr(),
            points_len: ps.len(),
        })
        .collect();
    let result = PPResult {
        kind: v.kind,
        rows: views.as_ptr(),
        rows_len: views.len(),
        error: ptr::null(),
        work,
        owner: ptr::null_mut(),
        parse_ns: 0,
        evaluation_ns: 0,
    };
    let owner = Box::into_raw(Box::new(Owned {
        result,
        _strings: strings,
        _labels: labels,
        _points: points,
        _rows: views,
        _error: None,
    }));
    unsafe {
        (*owner).result.owner = owner.cast();
        Ok(&mut (*owner).result)
    }
}
fn own_error(message: &str, work: u64) -> *mut PPResult {
    let error = CString::new(message).unwrap();
    let result = PPResult {
        kind: 4,
        rows: ptr::null(),
        rows_len: 0,
        error: error.as_ptr(),
        work,
        owner: ptr::null_mut(),
        parse_ns: 0,
        evaluation_ns: 0,
    };
    let owner = Box::into_raw(Box::new(Owned {
        result,
        _strings: Vec::new(),
        _labels: Vec::new(),
        _points: Vec::new(),
        _rows: Vec::new(),
        _error: Some(error),
    }));
    unsafe {
        (*owner).result.owner = owner.cast();
        &mut (*owner).result
    }
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pp_eval(request: *const PPRequest) -> *mut PPResult {
    let mut work = 0;
    let result = catch_unwind(AssertUnwindSafe(|| -> R<*mut PPResult> {
        if request.is_null() {
            return Err("invalid request");
        };
        let req = unsafe { &*request };
        if req.data.is_null()
            || req.query.is_null()
            || req.lookback_ms <= 0
            || req.lookback_ms > 9000000000000000
            || req.time_ms < -9000000000000000
            || req.time_ms > 9000000000000000
        {
            return Err("invalid request");
        };
        let parse_began = unsafe { pp_monotonic_ns() };
        let query = unsafe { CStr::from_ptr(req.query) }
            .to_str()
            .map_err(|_| "invalid query UTF-8")?;
        let ast = Parser::new(query)?.parse()?;
        let evaluation_began = unsafe { pp_monotonic_ns() };
        let mut evaluator = Evaluator {
            req,
            work: 0,
            depth: 0,
        };
        let value = evaluator.eval(&ast, req.time_ms);
        work = evaluator.work;
        let value = value?;
        if value.kind == 0 {
            return Err("string result unsupported");
        };
        if req.inject_failure == 2 {
            panic!("injected evaluation panic")
        };
        if req.inject_failure != 0 {
            return Err("allocation failure (injected)");
        };
        let result = own(value, req.time_ms, work)?;
        let finished = unsafe { pp_monotonic_ns() };
        unsafe {
            (*result).parse_ns = if parse_began != 0 {
                evaluation_began.saturating_sub(parse_began)
            } else {
                0
            };
            (*result).evaluation_ns = if evaluation_began != 0 {
                finished.saturating_sub(evaluation_began)
            } else {
                0
            };
        }
        Ok(result)
    }));
    match result {
        Ok(Ok(result)) => result,
        Ok(Err(message)) => own_error(message, work),
        Err(_) => own_error("panic contained", work),
    }
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pp_free(result: *mut PPResult) {
    if !result.is_null() {
        let owner = unsafe { (*result).owner };
        if !owner.is_null() {
            drop(unsafe { Box::from_raw(owner.cast::<Owned>()) });
        }
    }
}
