//! Tiny expression evaluator for YAML provider configs.
//!
//! Supports (by design, minimal):
//!   - Dot paths:            `a.b.c`          → walk into a serde_json::Value
//!   - Path alternative:     `a.b // c.d`     → left if non-null else right (jq-style)
//!   - Array index:          `a.0.b`          → numeric segment picks that array element
//!   - Literal values:       `"str"`, `true`, `false`, `null`, `42`
//!   - Comparisons:          `a == b`, `a != b`
//!   - Null-checks:          `a != null`, `a == null`
//!   - Logical operators:    `a and b`, `a or b`, `not a`
//!   - Parenthesis grouping: `(a or b) and c`
//!
//! This is **not** jq. If we ever need full jq semantics we can swap in `jaq`
//! without touching any YAML — simple paths are syntactically identical.

use std::cmp::Ordering;
use std::collections::HashMap;

use serde_json::Value;

// ── Public API ──────────────────────────────────────────────────────────────

/// A compiled expression. Cache these — parsing is the slow part.
#[derive(Debug, Clone)]
pub struct Expr {
    node: Node,
}

impl Expr {
    /// Parse an expression. Returns a helpful error on failure.
    pub fn parse(source: &str) -> Result<Self, String> {
        let tokens = tokenize(source)?;
        let mut parser = Parser { tokens, pos: 0 };
        let node = parser.parse_or()?;
        if parser.pos < parser.tokens.len() {
            return Err(format!(
                "unexpected trailing tokens at position {}: {:?}",
                parser.pos,
                &parser.tokens[parser.pos..]
            ));
        }
        Ok(Expr { node })
    }

    /// Evaluate against a JSON value. Returns the resulting Value.
    ///
    /// Missing paths return `Value::Null`. No panics.
    pub fn eval<'a>(&self, input: &'a Value) -> Value {
        eval(&self.node, input)
    }

    /// Convenience: evaluate and coerce to bool. `Null`/`false`/`0`/`""` → false.
    pub fn eval_bool(&self, input: &Value) -> bool {
        truthy(&self.eval(input))
    }

    /// Convenience: evaluate and render as a string (`None` if result is Null).
    pub fn eval_str(&self, input: &Value) -> Option<String> {
        match self.eval(input) {
            Value::Null => None,
            Value::String(s) => Some(s),
            Value::Bool(b) => Some(b.to_string()),
            Value::Number(n) => Some(n.to_string()),
            v => Some(v.to_string()),
        }
    }
}

/// Compile + cache. Use when the same expression is evaluated many times.
#[derive(Debug, Default)]
pub struct ExprCache {
    map: HashMap<String, Expr>,
}

impl ExprCache {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn get(&mut self, src: &str) -> Result<&Expr, String> {
        if !self.map.contains_key(src) {
            let e = Expr::parse(src)?;
            self.map.insert(src.to_string(), e);
        }
        Ok(self.map.get(src).unwrap())
    }
}

// ── AST ─────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
enum Node {
    Path(Vec<String>),
    /// lhs // rhs — if lhs is null, take rhs.
    Alt(Box<Node>, Box<Node>),
    Literal(Value),
    Eq(Box<Node>, Box<Node>),
    Ne(Box<Node>, Box<Node>),
    And(Box<Node>, Box<Node>),
    Or(Box<Node>, Box<Node>),
    Not(Box<Node>),
}

// ── Tokenizer ───────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq)]
enum Token {
    Ident(String), // path segment or keyword
    Number(f64),
    Str(String),
    Eq,      // ==
    Ne,      // !=
    And,     // 'and'
    Or,      // 'or'
    Not,     // 'not'
    DblSlash, // //
    Dot,     // .
    LParen,
    RParen,
}

fn tokenize(src: &str) -> Result<Vec<Token>, String> {
    let mut out = Vec::new();
    let bytes = src.as_bytes();
    let mut i = 0usize;
    while i < bytes.len() {
        let c = bytes[i];
        match c {
            b' ' | b'\t' | b'\n' | b'\r' => { i += 1; }
            b'(' => { out.push(Token::LParen); i += 1; }
            b')' => { out.push(Token::RParen); i += 1; }
            b'.' => { out.push(Token::Dot); i += 1; }
            b'=' if i + 1 < bytes.len() && bytes[i + 1] == b'=' => { out.push(Token::Eq); i += 2; }
            b'!' if i + 1 < bytes.len() && bytes[i + 1] == b'=' => { out.push(Token::Ne); i += 2; }
            b'/' if i + 1 < bytes.len() && bytes[i + 1] == b'/' => { out.push(Token::DblSlash); i += 2; }
            b'"' => {
                // string literal, no escape support (keep it simple)
                let start = i + 1;
                i += 1;
                while i < bytes.len() && bytes[i] != b'"' {
                    i += 1;
                }
                if i >= bytes.len() {
                    return Err("unterminated string literal".into());
                }
                let s = std::str::from_utf8(&bytes[start..i])
                    .map_err(|e| format!("invalid utf8 in string: {e}"))?;
                out.push(Token::Str(s.to_string()));
                i += 1;
            }
            b'\'' => {
                // single-quoted string (convenience)
                let start = i + 1;
                i += 1;
                while i < bytes.len() && bytes[i] != b'\'' {
                    i += 1;
                }
                if i >= bytes.len() {
                    return Err("unterminated string literal".into());
                }
                let s = std::str::from_utf8(&bytes[start..i])
                    .map_err(|e| format!("invalid utf8 in string: {e}"))?;
                out.push(Token::Str(s.to_string()));
                i += 1;
            }
            b'-' | b'0'..=b'9' => {
                let start = i;
                if bytes[i] == b'-' {
                    i += 1;
                }
                // Integer only — `.` is always a path separator, never a decimal point.
                // This keeps path segments like `content.0.text` parseable.
                while i < bytes.len() && bytes[i].is_ascii_digit() {
                    i += 1;
                }
                let slice = std::str::from_utf8(&bytes[start..i])
                    .map_err(|e| format!("invalid utf8 in number: {e}"))?;
                let n: f64 = slice.parse().map_err(|e| format!("bad number '{slice}': {e}"))?;
                out.push(Token::Number(n));
            }
            c if c.is_ascii_alphabetic() || c == b'_' => {
                let start = i;
                while i < bytes.len() && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_' || bytes[i] == b'-') {
                    i += 1;
                }
                let word = std::str::from_utf8(&bytes[start..i])
                    .map_err(|e| format!("invalid utf8 in identifier: {e}"))?;
                match word {
                    "and" => out.push(Token::And),
                    "or" => out.push(Token::Or),
                    "not" => out.push(Token::Not),
                    "true" => out.push(Token::Ident("true".into())),
                    "false" => out.push(Token::Ident("false".into())),
                    "null" => out.push(Token::Ident("null".into())),
                    _ => out.push(Token::Ident(word.to_string())),
                }
            }
            _ => return Err(format!("unexpected character '{}' at position {i}", c as char)),
        }
    }
    Ok(out)
}

// ── Parser (recursive descent) ──────────────────────────────────────────────

struct Parser {
    tokens: Vec<Token>,
    pos: usize,
}

impl Parser {
    fn peek(&self) -> Option<&Token> { self.tokens.get(self.pos) }
    fn bump(&mut self) -> Option<Token> {
        let t = self.tokens.get(self.pos).cloned();
        self.pos += 1;
        t
    }
    fn eat(&mut self, t: &Token) -> bool {
        if self.peek() == Some(t) { self.pos += 1; true } else { false }
    }

    // precedence: or < and < not < eq/ne < alt < atom
    fn parse_or(&mut self) -> Result<Node, String> {
        let mut lhs = self.parse_and()?;
        while self.eat(&Token::Or) {
            let rhs = self.parse_and()?;
            lhs = Node::Or(Box::new(lhs), Box::new(rhs));
        }
        Ok(lhs)
    }
    fn parse_and(&mut self) -> Result<Node, String> {
        let mut lhs = self.parse_not()?;
        while self.eat(&Token::And) {
            let rhs = self.parse_not()?;
            lhs = Node::And(Box::new(lhs), Box::new(rhs));
        }
        Ok(lhs)
    }
    fn parse_not(&mut self) -> Result<Node, String> {
        if self.eat(&Token::Not) {
            let inner = self.parse_not()?;
            return Ok(Node::Not(Box::new(inner)));
        }
        self.parse_eq()
    }
    fn parse_eq(&mut self) -> Result<Node, String> {
        let lhs = self.parse_alt()?;
        if self.eat(&Token::Eq) {
            let rhs = self.parse_alt()?;
            return Ok(Node::Eq(Box::new(lhs), Box::new(rhs)));
        }
        if self.eat(&Token::Ne) {
            let rhs = self.parse_alt()?;
            return Ok(Node::Ne(Box::new(lhs), Box::new(rhs)));
        }
        Ok(lhs)
    }
    fn parse_alt(&mut self) -> Result<Node, String> {
        let mut lhs = self.parse_atom()?;
        while self.eat(&Token::DblSlash) {
            let rhs = self.parse_atom()?;
            lhs = Node::Alt(Box::new(lhs), Box::new(rhs));
        }
        Ok(lhs)
    }
    fn parse_atom(&mut self) -> Result<Node, String> {
        match self.peek().cloned() {
            Some(Token::LParen) => {
                self.bump();
                let n = self.parse_or()?;
                if !self.eat(&Token::RParen) {
                    return Err("expected ')'".into());
                }
                Ok(n)
            }
            Some(Token::Str(s)) => { self.bump(); Ok(Node::Literal(Value::String(s))) }
            Some(Token::Number(n)) => {
                self.bump();
                let v = serde_json::Number::from_f64(n)
                    .map(Value::Number)
                    .unwrap_or(Value::Null);
                Ok(Node::Literal(v))
            }
            Some(Token::Ident(id)) => {
                self.bump();
                // "true" / "false" / "null" — reserved pseudo-idents
                match id.as_str() {
                    "true" => return Ok(Node::Literal(Value::Bool(true))),
                    "false" => return Ok(Node::Literal(Value::Bool(false))),
                    "null" => return Ok(Node::Literal(Value::Null)),
                    _ => {}
                }
                // otherwise, a path: id (. segment)*
                let mut parts = vec![id];
                while self.eat(&Token::Dot) {
                    match self.bump() {
                        Some(Token::Ident(s)) => parts.push(s),
                        Some(Token::Number(n)) => parts.push((n as i64).to_string()),
                        other => return Err(format!("expected path segment, got {other:?}")),
                    }
                }
                Ok(Node::Path(parts))
            }
            other => Err(format!("unexpected token: {other:?}")),
        }
    }
}

// ── Evaluator ───────────────────────────────────────────────────────────────

fn eval(node: &Node, input: &Value) -> Value {
    match node {
        Node::Path(parts) => walk_path(input, parts),
        Node::Alt(a, b) => {
            let va = eval(a, input);
            if va.is_null() { eval(b, input) } else { va }
        }
        Node::Literal(v) => v.clone(),
        Node::Eq(a, b) => Value::Bool(value_eq(&eval(a, input), &eval(b, input))),
        Node::Ne(a, b) => Value::Bool(!value_eq(&eval(a, input), &eval(b, input))),
        Node::And(a, b) => Value::Bool(truthy(&eval(a, input)) && truthy(&eval(b, input))),
        Node::Or(a, b) => Value::Bool(truthy(&eval(a, input)) || truthy(&eval(b, input))),
        Node::Not(a) => Value::Bool(!truthy(&eval(a, input))),
    }
}

fn walk_path(input: &Value, parts: &[String]) -> Value {
    let mut cur: &Value = input;
    for p in parts {
        match cur {
            Value::Object(m) => {
                if let Some(v) = m.get(p) {
                    cur = v;
                } else {
                    return Value::Null;
                }
            }
            Value::Array(arr) => {
                if let Ok(idx) = p.parse::<usize>() {
                    if let Some(v) = arr.get(idx) {
                        cur = v;
                    } else {
                        return Value::Null;
                    }
                } else {
                    return Value::Null;
                }
            }
            _ => return Value::Null,
        }
    }
    cur.clone()
}

fn value_eq(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Null, Value::Null) => true,
        (Value::Bool(x), Value::Bool(y)) => x == y,
        (Value::String(x), Value::String(y)) => x == y,
        (Value::Number(x), Value::Number(y)) => {
            match (x.as_f64(), y.as_f64()) {
                (Some(xf), Some(yf)) => xf.partial_cmp(&yf) == Some(Ordering::Equal),
                _ => x.to_string() == y.to_string(),
            }
        }
        // cross-type: coerce numbers from strings for convenience
        (Value::Number(_), Value::String(s)) | (Value::String(s), Value::Number(_)) => {
            // numeric text may appear in logs — compare as string only
            s.is_empty() && false
        }
        _ => false,
    }
}

fn truthy(v: &Value) -> bool {
    match v {
        Value::Null => false,
        Value::Bool(b) => *b,
        Value::String(s) => !s.is_empty(),
        Value::Number(n) => n.as_f64().map(|f| f != 0.0).unwrap_or(true),
        Value::Array(a) => !a.is_empty(),
        Value::Object(o) => !o.is_empty(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn path_simple() {
        let e = Expr::parse("a.b").unwrap();
        assert_eq!(e.eval(&json!({"a":{"b":"hi"}})), json!("hi"));
    }

    #[test]
    fn path_alt() {
        let e = Expr::parse("message.content // data.content").unwrap();
        assert_eq!(e.eval(&json!({"data":{"content":"fallback"}})), json!("fallback"));
        assert_eq!(e.eval(&json!({"message":{"content":"primary"}})), json!("primary"));
    }

    #[test]
    fn eq_string() {
        let e = Expr::parse("type == \"user\"").unwrap();
        assert!(e.eval_bool(&json!({"type":"user"})));
        assert!(!e.eval_bool(&json!({"type":"assistant"})));
    }

    #[test]
    fn ne_null() {
        let e = Expr::parse("cwd != null").unwrap();
        assert!(e.eval_bool(&json!({"cwd":"/foo"})));
        assert!(!e.eval_bool(&json!({"cwd":null})));
        assert!(!e.eval_bool(&json!({})));
    }

    #[test]
    fn and_or() {
        let e = Expr::parse("type == \"user\" and message.role == \"u\"").unwrap();
        assert!(e.eval_bool(&json!({"type":"user","message":{"role":"u"}})));
        assert!(!e.eval_bool(&json!({"type":"user","message":{"role":"a"}})));
    }

    #[test]
    fn not_paren() {
        let e = Expr::parse("not (type == \"file-history-snapshot\")").unwrap();
        assert!(!e.eval_bool(&json!({"type":"file-history-snapshot"})));
        assert!(e.eval_bool(&json!({"type":"user"})));
    }

    #[test]
    fn array_index() {
        let e = Expr::parse("message.content.0.text").unwrap();
        let v = json!({"message":{"content":[{"text":"hi"}]}});
        assert_eq!(e.eval(&v), json!("hi"));
    }

    #[test]
    fn literals_bool_null() {
        assert_eq!(Expr::parse("true").unwrap().eval(&json!({})), json!(true));
        assert_eq!(Expr::parse("null").unwrap().eval(&json!({})), json!(null));
    }

    #[test]
    fn cache_reuse() {
        let mut c = ExprCache::new();
        let e1 = c.get("a.b").unwrap().clone();
        let e2 = c.get("a.b").unwrap().clone();
        assert_eq!(e1.eval(&json!({"a":{"b":1}})), e2.eval(&json!({"a":{"b":1}})));
    }

    #[test]
    fn isMeta_filter() {
        let e = Expr::parse("isMeta == true").unwrap();
        assert!(e.eval_bool(&json!({"isMeta":true})));
        assert!(!e.eval_bool(&json!({"isMeta":false})));
        assert!(!e.eval_bool(&json!({})));
    }
}
