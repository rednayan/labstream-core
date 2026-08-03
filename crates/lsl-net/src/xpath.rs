//! The query language that a resolver uses.
//!
//! liblsl runs the query through pugixml and asks for a boolean
//! (`src/stream_info_impl.cpp:214`). That is XPath 1.0, evaluated with the
//! `<info>` element of the stream as the context node.
//!
//! Most queries come from `resolver_impl::build_query`, which only ever writes
//! `session_id='X' and prop='Y'` (`src/resolver_impl.cpp:66-73`). An
//! application that calls `lsl_resolve_bypred` supplies its own text, and
//! liblsl joins it to the session test with `and`. That text can be any XPath
//! expression, so a resolver that reads only equality hides streams that ought
//! to match.
//!
//! # What this reads
//!
//! `oracle/xpath.py` sends a battery of queries to a real outlet and records
//! the answers. That battery is the specification for this module, and every
//! shape in it is read here:
//!
//! * `and`, `or`, `not()`, parentheses
//! * `=`, `!=`, `<`, `>`, `<=`, `>=`
//! * `+`, `-`, `*`, `div`, `mod`, and a leading minus
//! * `contains`, `starts-with`, `substring`, `string-length`, `translate`,
//!   `concat`, `normalize-space`, `string`, `number`, `floor`, `ceiling`,
//!   `round`, `count`, `true`, `false`, `not`
//! * a child element by name, a path of names, and `//name` for a descendant
//!
//! # What this does not read
//!
//! Attributes, predicates in brackets, axes written in full, variables, and
//! the union operator. None of them appears in a description, which holds
//! elements and text and nothing else. An expression that uses one fails to
//! parse, and a query that fails to parse matches nothing. liblsl does the
//! same for an expression that pugixml refuses: it catches the error, writes a
//! warning, and reports no match (`src/stream_info_impl.cpp:239-241`).

use crate::desc::{Kind, Node};

/// A piece of an expression.
#[derive(Debug, Clone, PartialEq)]
enum Token {
    Name(String),
    Number(f64),
    Str(String),
    /// `(`, `)`, `,`, `/`, `//`, and every operator.
    Sym(&'static str),
}

fn lex(text: &str) -> Option<Vec<Token>> {
    let b: Vec<char> = text.chars().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < b.len() {
        let c = b[i];
        if c.is_whitespace() {
            i += 1;
            continue;
        }
        if c == '\'' || c == '"' {
            let quote = c;
            i += 1;
            let start = i;
            while i < b.len() && b[i] != quote {
                i += 1;
            }
            if i >= b.len() {
                return None;
            }
            out.push(Token::Str(b[start..i].iter().collect()));
            i += 1;
            continue;
        }
        if c.is_ascii_digit() || (c == '.' && i + 1 < b.len() && b[i + 1].is_ascii_digit()) {
            let start = i;
            while i < b.len() && (b[i].is_ascii_digit() || b[i] == '.') {
                i += 1;
            }
            let text: String = b[start..i].iter().collect();
            out.push(Token::Number(text.parse().ok()?));
            continue;
        }
        if c.is_alphabetic() || c == '_' {
            let start = i;
            while i < b.len() && (b[i].is_alphanumeric() || b[i] == '_' || b[i] == '.') {
                i += 1;
                // A name can hold a hyphen, as `starts-with` does. A hyphen
                // that does not begin another name part is the minus
                // operator.
                if i < b.len() && b[i] == '-' {
                    if i + 1 < b.len() && (b[i + 1].is_alphabetic() || b[i + 1] == '_') {
                        i += 1;
                    } else {
                        break;
                    }
                }
            }
            out.push(Token::Name(b[start..i].iter().collect()));
            continue;
        }
        let two: String = b[i..(i + 2).min(b.len())].iter().collect();
        let sym = match two.as_str() {
            "//" => Some("//"),
            "!=" => Some("!="),
            "<=" => Some("<="),
            ">=" => Some(">="),
            _ => None,
        };
        if let Some(s) = sym {
            out.push(Token::Sym(s));
            i += 2;
            continue;
        }
        let one = match c {
            '(' => "(",
            ')' => ")",
            ',' => ",",
            '/' => "/",
            '=' => "=",
            '<' => "<",
            '>' => ">",
            '+' => "+",
            '-' => "-",
            '*' => "*",
            _ => return None,
        };
        out.push(Token::Sym(one));
        i += 1;
    }
    Some(out)
}

#[derive(Debug, Clone)]
enum Expr {
    Number(f64),
    Str(String),
    /// A path of element names. `descendant` marks a leading `//`.
    Path {
        descendant: bool,
        steps: Vec<String>,
    },
    Binary(&'static str, Box<Expr>, Box<Expr>),
    Negate(Box<Expr>),
    Call(String, Vec<Expr>),
}

struct Parser {
    tokens: Vec<Token>,
    at: usize,
}

impl Parser {
    fn peek(&self) -> Option<&Token> {
        self.tokens.get(self.at)
    }

    fn eat_sym(&mut self, s: &str) -> bool {
        if matches!(self.peek(), Some(Token::Sym(x)) if *x == s) {
            self.at += 1;
            return true;
        }
        false
    }

    fn eat_word(&mut self, w: &str) -> bool {
        if matches!(self.peek(), Some(Token::Name(x)) if x == w) {
            self.at += 1;
            return true;
        }
        false
    }

    fn or(&mut self) -> Option<Expr> {
        let mut left = self.and()?;
        while self.eat_word("or") {
            let right = self.and()?;
            left = Expr::Binary("or", Box::new(left), Box::new(right));
        }
        Some(left)
    }

    fn and(&mut self) -> Option<Expr> {
        let mut left = self.equality()?;
        while self.eat_word("and") {
            let right = self.equality()?;
            left = Expr::Binary("and", Box::new(left), Box::new(right));
        }
        Some(left)
    }

    fn equality(&mut self) -> Option<Expr> {
        let mut left = self.relational()?;
        loop {
            let op = if self.eat_sym("=") {
                "="
            } else if self.eat_sym("!=") {
                "!="
            } else {
                return Some(left);
            };
            let right = self.relational()?;
            left = Expr::Binary(op, Box::new(left), Box::new(right));
        }
    }

    fn relational(&mut self) -> Option<Expr> {
        let mut left = self.additive()?;
        loop {
            // The two-character forms are tested first.
            let op = if self.eat_sym("<=") {
                "<="
            } else if self.eat_sym(">=") {
                ">="
            } else if self.eat_sym("<") {
                "<"
            } else if self.eat_sym(">") {
                ">"
            } else {
                return Some(left);
            };
            let right = self.additive()?;
            left = Expr::Binary(op, Box::new(left), Box::new(right));
        }
    }

    fn additive(&mut self) -> Option<Expr> {
        let mut left = self.multiplicative()?;
        loop {
            let op = if self.eat_sym("+") {
                "+"
            } else if self.eat_sym("-") {
                "-"
            } else {
                return Some(left);
            };
            let right = self.multiplicative()?;
            left = Expr::Binary(op, Box::new(left), Box::new(right));
        }
    }

    fn multiplicative(&mut self) -> Option<Expr> {
        let mut left = self.unary()?;
        loop {
            let op = if self.eat_sym("*") {
                "*"
            } else if self.eat_word("div") {
                "div"
            } else if self.eat_word("mod") {
                "mod"
            } else {
                return Some(left);
            };
            let right = self.unary()?;
            left = Expr::Binary(op, Box::new(left), Box::new(right));
        }
    }

    fn unary(&mut self) -> Option<Expr> {
        if self.eat_sym("-") {
            return Some(Expr::Negate(Box::new(self.unary()?)));
        }
        self.primary()
    }

    fn primary(&mut self) -> Option<Expr> {
        if self.eat_sym("(") {
            let inner = self.or()?;
            if !self.eat_sym(")") {
                return None;
            }
            return Some(inner);
        }
        match self.peek().cloned() {
            Some(Token::Number(n)) => {
                self.at += 1;
                Some(Expr::Number(n))
            }
            Some(Token::Str(s)) => {
                self.at += 1;
                Some(Expr::Str(s))
            }
            Some(Token::Sym("//")) => {
                self.at += 1;
                let steps = self.steps()?;
                Some(Expr::Path {
                    descendant: true,
                    steps,
                })
            }
            Some(Token::Sym("/")) => {
                self.at += 1;
                let steps = self.steps()?;
                Some(Expr::Path {
                    descendant: false,
                    steps,
                })
            }
            Some(Token::Name(word)) => {
                // A name followed by `(` is a call. Anything else is a path.
                if matches!(self.tokens.get(self.at + 1), Some(Token::Sym("("))) {
                    self.at += 2;
                    let mut args = Vec::new();
                    if !self.eat_sym(")") {
                        loop {
                            args.push(self.or()?);
                            if self.eat_sym(",") {
                                continue;
                            }
                            if self.eat_sym(")") {
                                break;
                            }
                            return None;
                        }
                    }
                    return Some(Expr::Call(word, args));
                }
                let steps = self.steps()?;
                Some(Expr::Path {
                    descendant: false,
                    steps,
                })
            }
            _ => None,
        }
    }

    /// One or more element names, divided by `/`.
    fn steps(&mut self) -> Option<Vec<String>> {
        let mut out = Vec::new();
        loop {
            match self.peek().cloned() {
                Some(Token::Name(w)) => {
                    self.at += 1;
                    out.push(w);
                }
                Some(Token::Sym("*")) => {
                    self.at += 1;
                    out.push("*".to_string());
                }
                _ => return None,
            }
            if !self.eat_sym("/") {
                return Some(out);
            }
        }
    }
}

/// The value of an expression.
///
/// A set of nodes is held as the text of each node. Nothing in this subset
/// asks which node a value came from, so the text is enough for a count and
/// for every comparison.
#[derive(Debug, Clone)]
enum Value {
    Bool(bool),
    Num(f64),
    Str(String),
    Nodes(Vec<String>),
}

/// The text of a node, which is the text of everything below it.
fn text_of(node: &Node) -> String {
    if node.kind != Kind::Element {
        return node.value.clone();
    }
    let mut out = String::new();
    for c in &node.children {
        out.push_str(&text_of(c));
    }
    out
}

impl Value {
    fn boolean(&self) -> bool {
        match self {
            Value::Bool(b) => *b,
            Value::Num(n) => *n != 0.0 && !n.is_nan(),
            Value::Str(s) => !s.is_empty(),
            Value::Nodes(n) => !n.is_empty(),
        }
    }

    fn string(&self) -> String {
        match self {
            Value::Bool(b) => if *b { "true" } else { "false" }.to_string(),
            Value::Num(n) => number_to_string(*n),
            Value::Str(s) => s.clone(),
            // The text of the first node, and nothing when there is none.
            Value::Nodes(n) => n.first().cloned().unwrap_or_default(),
        }
    }

    fn number(&self) -> f64 {
        match self {
            Value::Bool(b) => {
                if *b {
                    1.0
                } else {
                    0.0
                }
            }
            Value::Num(n) => *n,
            _ => self.string().trim().parse().unwrap_or(f64::NAN),
        }
    }
}

/// Write a number the way XPath writes one.
fn number_to_string(n: f64) -> String {
    if n.is_nan() {
        return "NaN".to_string();
    }
    if n.is_infinite() {
        return if n < 0.0 { "-Infinity" } else { "Infinity" }.to_string();
    }
    if n == n.trunc() && n.abs() < 1e18 {
        return format!("{}", n as i64);
    }
    let mut s = format!("{n}");
    if s.contains('e') {
        s = format!("{n:.10}");
        while s.ends_with('0') {
            s.pop();
        }
        if s.ends_with('.') {
            s.pop();
        }
    }
    s
}

/// Gather the nodes that a path names.
fn walk(context: &Node, descendant: bool, steps: &[String]) -> Vec<String> {
    // `//x` looks through the whole document. The context node is the root of
    // the description here, so its own subtree is the whole document.
    let mut current: Vec<&Node> = vec![context];
    if descendant {
        let mut all = Vec::new();
        collect(context, &mut all);
        current = all;
    }
    for (depth, step) in steps.iter().enumerate() {
        let mut next: Vec<&Node> = Vec::new();
        for node in &current {
            for child in &node.children {
                if child.kind == Kind::Element && (step == "*" || &child.name == step) {
                    next.push(child);
                }
            }
        }
        current = next;
        if current.is_empty() && depth + 1 < steps.len() {
            break;
        }
    }
    current.into_iter().map(text_of).collect()
}

fn collect<'a>(node: &'a Node, out: &mut Vec<&'a Node>) {
    out.push(node);
    for c in &node.children {
        if c.kind == Kind::Element {
            collect(c, out);
        }
    }
}

fn compare(op: &str, left: &Value, right: &Value) -> bool {
    // A comparison that names a set of nodes is true when **any** node of the
    // set satisfies it. XPath 1.0, section 3.4.
    let relational = matches!(op, "<" | ">" | "<=" | ">=");
    let numbers = |v: &Value| -> Vec<f64> {
        match v {
            Value::Nodes(n) => n
                .iter()
                .map(|s| s.trim().parse().unwrap_or(f64::NAN))
                .collect(),
            other => vec![other.number()],
        }
    };
    if relational {
        for a in numbers(left) {
            for b in numbers(right) {
                let ok = match op {
                    "<" => a < b,
                    ">" => a > b,
                    "<=" => a <= b,
                    _ => a >= b,
                };
                if ok {
                    return true;
                }
            }
        }
        return false;
    }

    let equal = |a: bool| if op == "=" { a } else { !a };
    match (left, right) {
        (Value::Nodes(a), Value::Nodes(b)) => {
            for x in a {
                for y in b {
                    if equal(x == y) {
                        return true;
                    }
                }
            }
            false
        }
        (Value::Nodes(a), other) | (other, Value::Nodes(a)) => match other {
            Value::Bool(_) => equal(Value::Nodes(a.clone()).boolean() == other.boolean()),
            Value::Num(n) => a
                .iter()
                .any(|s| equal(s.trim().parse::<f64>().map(|v| v == *n).unwrap_or(false))),
            _ => {
                let want = other.string();
                a.iter().any(|s| equal(*s == want))
            }
        },
        (Value::Bool(_), _) | (_, Value::Bool(_)) => equal(left.boolean() == right.boolean()),
        (Value::Num(_), _) | (_, Value::Num(_)) => equal(left.number() == right.number()),
        _ => equal(left.string() == right.string()),
    }
}

fn eval(expr: &Expr, context: &Node) -> Option<Value> {
    Some(match expr {
        Expr::Number(n) => Value::Num(*n),
        Expr::Str(s) => Value::Str(s.clone()),
        Expr::Path { descendant, steps } => Value::Nodes(walk(context, *descendant, steps)),
        Expr::Negate(inner) => Value::Num(-eval(inner, context)?.number()),
        Expr::Binary(op, a, b) => {
            // `and` and `or` stop as soon as the answer is known.
            if *op == "and" {
                return Some(Value::Bool(
                    eval(a, context)?.boolean() && eval(b, context)?.boolean(),
                ));
            }
            if *op == "or" {
                return Some(Value::Bool(
                    eval(a, context)?.boolean() || eval(b, context)?.boolean(),
                ));
            }
            let left = eval(a, context)?;
            let right = eval(b, context)?;
            match *op {
                "=" | "!=" | "<" | ">" | "<=" | ">=" => Value::Bool(compare(op, &left, &right)),
                "+" => Value::Num(left.number() + right.number()),
                "-" => Value::Num(left.number() - right.number()),
                "*" => Value::Num(left.number() * right.number()),
                "div" => Value::Num(left.number() / right.number()),
                "mod" => Value::Num(left.number() % right.number()),
                _ => return None,
            }
        }
        Expr::Call(name, args) => {
            let arg = |k: usize| -> Option<Value> { args.get(k).and_then(|e| eval(e, context)) };
            match name.as_str() {
                "true" if args.is_empty() => Value::Bool(true),
                "false" if args.is_empty() => Value::Bool(false),
                "not" if args.len() == 1 => Value::Bool(!arg(0)?.boolean()),
                "count" if args.len() == 1 => match arg(0)? {
                    Value::Nodes(n) => Value::Num(n.len() as f64),
                    _ => return None,
                },
                "string" if args.len() == 1 => Value::Str(arg(0)?.string()),
                "number" if args.len() == 1 => Value::Num(arg(0)?.number()),
                "floor" if args.len() == 1 => Value::Num(arg(0)?.number().floor()),
                "ceiling" if args.len() == 1 => Value::Num(arg(0)?.number().ceil()),
                // XPath rounds a half up, which is not what `f64::round` does
                // for a negative number.
                "round" if args.len() == 1 => Value::Num((arg(0)?.number() + 0.5).floor()),
                "string-length" if args.len() == 1 => {
                    Value::Num(arg(0)?.string().chars().count() as f64)
                }
                "concat" if args.len() >= 2 => {
                    let mut out = String::new();
                    for k in 0..args.len() {
                        out.push_str(&arg(k)?.string());
                    }
                    Value::Str(out)
                }
                "contains" if args.len() == 2 => {
                    Value::Bool(arg(0)?.string().contains(&arg(1)?.string()))
                }
                "starts-with" if args.len() == 2 => {
                    Value::Bool(arg(0)?.string().starts_with(&arg(1)?.string()))
                }
                "normalize-space" if args.len() == 1 => Value::Str(
                    arg(0)?
                        .string()
                        .split_whitespace()
                        .collect::<Vec<_>>()
                        .join(" "),
                ),
                "translate" if args.len() == 3 => {
                    let from: Vec<char> = arg(1)?.string().chars().collect();
                    let to: Vec<char> = arg(2)?.string().chars().collect();
                    let mut out = String::new();
                    for c in arg(0)?.string().chars() {
                        match from.iter().position(|x| *x == c) {
                            // A character with no partner is removed.
                            Some(k) => {
                                if let Some(r) = to.get(k) {
                                    out.push(*r);
                                }
                            }
                            None => out.push(c),
                        }
                    }
                    Value::Str(out)
                }
                // The first character is at one, and each bound is rounded.
                "substring" if args.len() == 2 || args.len() == 3 => {
                    let s: Vec<char> = arg(0)?.string().chars().collect();
                    let start = (arg(1)?.number() + 0.5).floor();
                    let end = match args.len() {
                        3 => start + (arg(2)?.number() + 0.5).floor(),
                        _ => f64::INFINITY,
                    };
                    let mut out = String::new();
                    for (k, c) in s.iter().enumerate() {
                        let pos = (k + 1) as f64;
                        if pos >= start && pos < end {
                            out.push(*c);
                        }
                    }
                    Value::Str(out)
                }
                _ => return None,
            }
        }
    })
}

/// Test one description against one query.
///
/// An empty query matches every stream (`src/stream_info_impl.cpp:198`). A
/// query that this module cannot read matches nothing, which is what liblsl
/// reports for a query that pugixml refuses.
pub fn matches(query: &str, document: &Node) -> bool {
    let q = query.trim();
    if q.is_empty() {
        return true;
    }
    let tokens = match lex(q) {
        Some(t) => t,
        None => return false,
    };
    let mut parser = Parser { tokens, at: 0 };
    let expr = match parser.or() {
        Some(e) => e,
        None => return false,
    };
    // Text left over means the query was not read to the end.
    if parser.at != parser.tokens.len() {
        return false;
    }
    match eval(&expr, document) {
        Some(v) => v.boolean(),
        None => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The document that the battery in `oracle/xpath.py` runs against.
    fn document() -> Node {
        let mut info = Node::element("info");
        info.append_child_value("name", "XpTest");
        info.append_child_value("type", "EEG");
        info.append_child_value("channel_count", "8");
        info.append_child_value("channel_format", "float32");
        info.append_child_value("source_id", "xp_src");
        info.append_child_value("nominal_srate", "100.0000000000000");
        info.append_child_value("session_id", "default");
        info.append_child_value("v4data_port", "16572");
        info.append_child("desc");
        info
    }

    fn hit(q: &str) -> bool {
        matches(q, &document())
    }

    #[test]
    fn the_queries_that_a_resolver_builds() {
        assert!(hit(""));
        assert!(hit("session_id='default'"));
        assert!(hit("session_id='default' and name='XpTest'"));
        assert!(!hit("session_id='default' and name='Other'"));
        assert!(hit("name='XpTest'"));
        assert!(hit("type='EEG'"));
        assert!(hit("source_id='xp_src'"));
    }

    #[test]
    fn the_boolean_operators() {
        assert!(hit("name='XpTest' or name='Nothing'"));
        assert!(!hit("name='Nothing' or name='AlsoNothing'"));
        assert!(hit("not(name='Nothing')"));
        assert!(!hit("not(name='XpTest')"));
        assert!(hit("name='XpTest' and type='EEG' and source_id='xp_src'"));
        assert!(hit("(name='XpTest' or name='X') and type='EEG'"));
        assert!(hit("true()"));
        assert!(!hit("false()"));
    }

    #[test]
    fn the_string_functions() {
        assert!(hit("contains(name,'XpT')"));
        assert!(!hit("contains(name,'zzz')"));
        assert!(hit("starts-with(name,'Xp')"));
        assert!(!hit("starts-with(name,'zz')"));
        assert!(hit("string-length(name)>3"));
        assert!(hit("substring(name,1,2)='Xp'"));
        assert!(hit("translate(name,'X','x')='xpTest'"));
        assert!(hit("concat(name,type)='XpTestEEG'"));
        assert!(hit("normalize-space(name)='XpTest'"));
    }

    #[test]
    fn the_numbers() {
        assert!(hit("channel_count=8"));
        assert!(hit("channel_count>4"));
        assert!(!hit("channel_count<4"));
        assert!(hit("channel_count>=8"));
        assert!(hit("nominal_srate=100"));
        assert!(hit("number(channel_count)+1=9"));
        assert!(hit("floor(nominal_srate div 3)=33"));
    }

    #[test]
    fn the_paths() {
        // The context is `<info>`, so `info/name` looks for an `<info>` inside
        // it and finds none.
        assert!(!hit("info/name='XpTest'"));
        assert!(hit("//name='XpTest'"));
        assert!(hit("name"));
        assert!(!hit("missing_field"));
        assert!(hit("count(//name)=1"));
        assert!(hit("//v4data_port>0"));
    }

    #[test]
    fn a_query_that_cannot_be_read_matches_nothing() {
        assert!(!hit("in'va'lid"));
        assert!(!hit("name="));
        assert!(!hit("and and and"));
        assert!(!hit("name='XpTest'))"));
        assert!(!hit("!!!"));
    }
}
