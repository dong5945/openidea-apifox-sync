//! A small, dependency-free Java source scanner.
//!
//! OpenIDEA plugins do not get IntelliJ's PSI: a plugin sees project files as
//! text. This scanner therefore reads the text and recovers just enough
//! structure for API documentation — classes, annotations, fields, methods,
//! Javadoc/`///` comments and 1-based line numbers. It is deliberately not a
//! Java parser: it never resolves imports, type parameters or expressions.

use std::collections::BTreeMap;

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Ann {
    pub name: String,
    /// Raw text inside the parentheses, without them (`"/users", method = GET`).
    pub args: String,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Doc {
    pub summary: String,
    pub description: String,
    pub params: BTreeMap<String, String>,
    pub ret: String,
    pub deprecated: bool,
    /// Any other Javadoc/KDoc tags (`@ignore`, `@module`, `@example` ...).
    pub tags: BTreeMap<String, String>,
}

impl Doc {
    pub fn is_empty(&self) -> bool {
        self.summary.is_empty() && self.description.is_empty() && self.params.is_empty()
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct JavaField {
    pub name: String,
    pub ty: String,
    pub doc: String,
    pub anns: Vec<Ann>,
    pub line: u32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct JavaParam {
    pub name: String,
    pub ty: String,
    pub anns: Vec<Ann>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct JavaMethod {
    pub name: String,
    pub ret: String,
    pub params: Vec<JavaParam>,
    pub doc: Doc,
    pub anns: Vec<Ann>,
    pub line: u32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct JavaClass {
    pub name: String,
    pub anns: Vec<Ann>,
    pub doc: Doc,
    pub fields: Vec<JavaField>,
    pub methods: Vec<JavaMethod>,
    /// Constants of an `enum`; empty for classes/interfaces.
    pub enum_values: Vec<String>,
    pub line: u32,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct JavaFile {
    pub classes: Vec<JavaClass>,
}

struct Scanner<'a> {
    s: &'a [char],
    i: usize,
    line: u32,
}

impl<'a> Scanner<'a> {
    fn new(s: &'a [char]) -> Self {
        Self { s, i: 0, line: 1 }
    }

    fn eof(&self, end: usize) -> bool {
        self.i >= end
    }

    fn peek(&self) -> Option<char> {
        self.s.get(self.i).copied()
    }

    fn bump(&mut self) -> Option<char> {
        let c = self.peek()?;
        self.i += 1;
        if c == '\n' {
            self.line += 1;
        }
        Some(c)
    }

    fn at_str(&self, pat: &str) -> bool {
        let mut j = self.i;
        for p in pat.chars() {
            if self.s.get(j).copied() != Some(p) {
                return false;
            }
            j += 1;
        }
        true
    }

    fn ident(&mut self) -> String {
        let mut out = String::new();
        while let Some(c) = self.peek() {
            if is_ident(c) {
                out.push(c);
                self.bump();
            } else {
                break;
            }
        }
        out
    }

    /// Skips whitespace and comments. Returns the text of the last
    /// Javadoc / `///` block seen, if any.
    fn skip_trivia(&mut self) -> Option<String> {
        let mut doc: Option<String> = None;
        loop {
            match self.peek() {
                Some(c) if c.is_whitespace() => {
                    self.bump();
                }
                Some('/') if self.at_str("///") => {
                    let mut text = String::new();
                    while self.at_str("///") {
                        self.bump();
                        self.bump();
                        self.bump();
                        while let Some(c) = self.peek() {
                            if c == '\n' {
                                break;
                            }
                            text.push(c);
                            self.bump();
                        }
                        text.push('\n');
                        self.bump(); // newline
                        // allow indentation before the next `///`
                        while matches!(self.peek(), Some(c) if c == ' ' || c == '\t') {
                            self.bump();
                        }
                    }
                    doc = Some(text);
                }
                Some('/') if self.at_str("/**") => {
                    self.bump();
                    self.bump();
                    self.bump();
                    let mut text = String::new();
                    while !self.eof(self.s.len()) {
                        if self.at_str("*/") {
                            self.bump();
                            self.bump();
                            break;
                        }
                        text.push(self.bump().unwrap());
                    }
                    doc = Some(text);
                }
                Some('/') if self.at_str("/*") => {
                    while !self.eof(self.s.len()) {
                        if self.at_str("*/") {
                            self.bump();
                            self.bump();
                            break;
                        }
                        self.bump();
                    }
                }
                Some('/') if self.at_str("//") => {
                    while let Some(c) = self.peek() {
                        if c == '\n' {
                            break;
                        }
                        self.bump();
                    }
                }
                _ => break,
            }
        }
        doc
    }

    fn skip_annotation_args(&mut self) {
        self.bump(); // '('
        let mut depth = 1usize;
        while let Some(c) = self.peek() {
            match c {
                '"' => {
                    self.skip_string();
                    continue;
                }
                '(' => depth += 1,
                ')' => {
                    depth -= 1;
                    if depth == 0 {
                        self.bump();
                        return;
                    }
                }
                _ => {}
            }
            self.bump();
        }
    }

    fn skip_string(&mut self) {
        let quote = self.bump().unwrap();
        while let Some(c) = self.bump() {
            if c == '\\' {
                self.bump();
            } else if c == quote {
                return;
            }
        }
    }

    /// Text inside the current `{ ... }`, consumed; the braces are excluded.
    fn block_text(&mut self) -> String {
        let start = self.i;
        self.skip_block();
        let end = self.i.saturating_sub(1).max(start);
        self.s[start..end].iter().collect()
    }
    fn skip_block(&mut self) {
        // current position is just after the opening '{'
        let mut depth = 1usize;
        while let Some(c) = self.peek() {
            match c {
                '"' | '\'' => {
                    self.skip_string();
                    continue;
                }
                '{' => depth += 1,
                '}' => {
                    depth -= 1;
                    if depth == 0 {
                        self.bump();
                        return;
                    }
                }
                _ => {}
            }
            self.bump();
        }
    }

    fn collect_anns(&mut self, end: usize) -> Vec<Ann> {
        let mut out = Vec::new();
        loop {
            self.skip_trivia();
            if self.peek() != Some('@') || self.i >= end {
                break;
            }
            // `@interface` is a declaration, not an annotation.
            if self.s[self.i..].starts_with(&['@', 'i', 'n', 't', 'e', 'r', 'f', 'a', 'c', 'e']) {
                break;
            }
            self.bump(); // '@'
            let name = self.ident();
            if name.is_empty() {
                break;
            }
            self.skip_trivia();
            let mut args = String::new();
            if self.peek() == Some('(') {
                let start = self.i + 1;
                self.skip_annotation_args();
                args = self.s[start..self.i.saturating_sub(1)].iter().collect();
                args = args.trim().to_string();
            }
            out.push(Ann { name, args });
        }
        out
    }

    /// Reads a declaration header up to a top-level `;` or `{`.
    fn read_header(&mut self, end: usize) -> Header {
        let start = self.i;
        let line = self.line;
        let mut paren = 0i32;
        let mut bracket = 0i32;
        let mut after_eq = false;
        while self.i < end {
            match self.peek().unwrap() {
                '"' | '\'' => {
                    self.skip_string();
                    continue;
                }
                '(' => paren += 1,
                ')' => paren -= 1,
                '[' => bracket += 1,
                ']' => bracket -= 1,
                '=' if paren == 0 && bracket == 0 => after_eq = true,
                ';' if paren == 0 && bracket == 0 => {
                    let text: String = self.s[start..self.i].iter().collect();
                    self.bump();
                    return Header { text, term: ';', line };
                }
                '{' if paren == 0 && bracket == 0 => {
                    if after_eq {
                        // `int[] a = {1, 2};` — an initializer, not a body.
                        self.bump();
                        self.skip_block();
                        continue;
                    }
                    let text: String = self.s[start..self.i].iter().collect();
                    self.bump();
                    return Header { text, term: '{', line };
                }
                '}' if paren == 0 && bracket == 0 => {
                    let text: String = self.s[start..self.i].iter().collect();
                    self.bump();
                    return Header { text, term: '}', line };
                }
                _ => {}
            }
            self.bump();
        }
        let text: String = self.s[start..self.i.min(self.s.len())].iter().collect();
        Header { text, term: '\0', line }
    }
}

struct Header {
    text: String,
    term: char,
    line: u32,
}

struct Members {
    classes: Vec<JavaClass>,
    fields: Vec<JavaField>,
    methods: Vec<JavaMethod>,
}

/// Parses one Java source text. Nested types are ignored (their members are
/// skipped), which is what documentation of controllers and DTOs needs.
pub fn parse_file(src: &str) -> JavaFile {
    let chars: Vec<char> = src.chars().collect();
    let mut sc = Scanner::new(&chars);
    let members = parse_scope(&mut sc, chars.len());
    JavaFile { classes: members.classes }
}

fn parse_scope(sc: &mut Scanner, end: usize) -> Members {
    let mut out = Members { classes: Vec::new(), fields: Vec::new(), methods: Vec::new() };
    loop {
        let doc_text = sc.skip_trivia();
        if sc.eof(end) {
            break;
        }
        let anns = sc.collect_anns(end);
        let doc = doc_text.map(|t| clean_doc(&t)).unwrap_or_default();
        if sc.eof(end) {
            break;
        }
        let header = sc.read_header(end);
        let text = header.text.trim().to_string();
        if text.is_empty() {
            if header.term == '}' {
                break;
            }
            continue;
        }
        match header.term {
            '{' => {
                if let Some((kind, name)) = type_decl(&text) {
                    if kind == "class" || kind == "record" || kind == "interface" {
                        let inner = parse_scope(sc, end);
                        out.classes.push(JavaClass {
                            name,
                            anns,
                            doc,
                            fields: inner.fields,
                            methods: inner.methods,
                            enum_values: Vec::new(),
                            line: header.line,
                        });
                    } else if kind == "enum" {
                        let body = sc.block_text();
                        out.classes.push(JavaClass {
                            name,
                            anns,
                            doc,
                            fields: Vec::new(),
                            methods: Vec::new(),
                            enum_values: enum_constants(&body),
                            line: header.line,
                        });
                    } else {
                        sc.skip_block();
                    }
                    continue;
                }
                if let Some((name, ret, params)) = signature(&text) {
                    sc.skip_block();
                    if !name.is_empty() && !ret.is_empty() {
                        out.methods.push(JavaMethod {
                            name,
                            ret,
                            params,
                            doc,
                            anns,
                            line: header.line,
                        });
                    }
                    continue;
                }
                sc.skip_block();
            }
            ';' => {
                if text.contains('(') {
                    if let Some((name, ret, params)) = signature(&text) {
                        if !name.is_empty() && !ret.is_empty() {
                            out.methods.push(JavaMethod {
                                name,
                                ret,
                                params,
                                doc,
                                anns,
                                line: header.line,
                            });
                        }
                    }
                } else if let Some((ty, name)) = field_decl(&text) {
                    out.fields.push(JavaField {
                        name,
                        ty,
                        doc: doc.summary.clone(),
                        anns,
                        line: header.line,
                    });
                }
            }
            '}' => break,
            _ => break,
        }
    }
    out
}

fn type_decl(text: &str) -> Option<(&'static str, String)> {
    let words: Vec<&str> = text.split(|c: char| !is_ident(c)).filter(|w| !w.is_empty()).collect();
    for (i, w) in words.iter().enumerate() {
        let kind = match *w {
            "class" => "class",
            "interface" => "interface",
            "enum" => "enum",
            "record" => "record",
            _ => continue,
        };
        let name = words.get(i + 1)?.to_string();
        return Some((kind, name));
    }
    None
}

const MODIFIERS: &[&str] = &[
    "public",
    "protected",
    "private",
    "static",
    "final",
    "abstract",
    "native",
    "synchronized",
    "transient",
    "volatile",
    "strictfp",
    "default",
    "sealed",
    "non-sealed",
];

fn is_modifier(w: &str) -> bool {
    MODIFIERS.contains(&w)
}

/// `A, B("x"), @JsonValue C;` -> `["A", "B", "C"]` (methods are ignored).
fn enum_constants(body: &str) -> Vec<String> {
    let head = body.split(';').next().unwrap_or("");
    split_top_level(head, ',')
        .iter()
        .filter_map(|part| constant_name(part))
        .collect()
}

fn constant_name(raw: &str) -> Option<String> {
    let mut s = raw.trim();
    while s.starts_with('@') {
        s = skip_annotation(s)?;
    }
    let name: String = s.chars().take_while(|c| is_ident(*c)).collect();
    if name.is_empty() {
        None
    } else {
        Some(name)
    }
}

fn skip_annotation(s: &str) -> Option<&str> {
    let after = s.strip_prefix('@')?;
    let end = after.find(|c: char| !is_ident(c)).unwrap_or(after.len());
    let rest = after[end..].trim_start();
    if rest.starts_with('(') {
        let close = matching_paren(rest, 0)?;
        Some(rest[close + 1..].trim_start())
    } else {
        Some(rest)
    }
}
pub fn split_top_level(s: &str, sep: char) -> Vec<String> {
    let mut out = Vec::new();
    let mut buf = String::new();
    let mut depth = 0i32;
    let mut in_str: Option<char> = None;
    let mut esc = false;
    for c in s.chars() {
        if let Some(q) = in_str {
            buf.push(c);
            if esc {
                esc = false;
            } else if c == '\\' {
                esc = true;
            } else if c == q {
                in_str = None;
            }
            continue;
        }
        match c {
            '"' | '\'' => {
                in_str = Some(c);
                buf.push(c);
            }
            '<' | '(' | '[' | '{' => {
                depth += 1;
                buf.push(c);
            }
            '>' | ')' | ']' | '}' => {
                depth -= 1;
                buf.push(c);
            }
            c if c == sep && depth == 0 => {
                out.push(buf.trim().to_string());
                buf.clear();
            }
            _ => buf.push(c),
        }
    }
    if !buf.trim().is_empty() {
        out.push(buf.trim().to_string());
    }
    out
}

fn field_decl(text: &str) -> Option<(String, String)> {
    if text.starts_with("package ") || text.starts_with("import ") {
        return None;
    }
    let mut tokens: Vec<String> = Vec::new();
    let mut buf = String::new();
    let mut depth = 0i32;
    for c in text.chars() {
        match c {
            '<' | '(' | '[' => {
                depth += 1;
                buf.push(c);
            }
            '>' | ')' | ']' => {
                depth -= 1;
                buf.push(c);
            }
            c if c.is_whitespace() && depth <= 0 => {
                if !buf.is_empty() {
                    tokens.push(std::mem::take(&mut buf));
                }
            }
            _ => buf.push(c),
        }
    }
    if !buf.is_empty() {
        tokens.push(buf);
    }
    let tokens: Vec<String> = tokens.into_iter().filter(|t| !is_modifier(t)).collect();
    if tokens.len() < 2 {
        return None;
    }
    let name = tokens.last()?.trim_end_matches(';').to_string();
    if !name.chars().next().map(is_ident).unwrap_or(false) {
        return None;
    }
    let ty = tokens[..tokens.len() - 1].join(" ");
    Some((ty, name))
}

/// `public Result<User> page(@RequestParam String k) throws X` -> name/ret/params
fn signature(text: &str) -> Option<(String, String, Vec<JavaParam>)> {
    let open = top_level_paren(text)?;
    let before = &text[..open];
    let close = matching_paren(text, open)?;
    let inside = &text[open + 1..close];

    // Name = the identifier right before '('.
    let name: String = before
        .chars()
        .rev()
        .take_while(|c| is_ident(*c))
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    if name.is_empty() {
        return None;
    }
    let ret_part = before[..before.len() - name.len()].trim();
    let ret_tokens: Vec<&str> = ret_part
        .split_whitespace()
        .filter(|w| !is_modifier(w))
        .collect();
    let ret = ret_tokens.join(" ");
    if ret.is_empty() {
        return None; // constructor
    }
    let mut params = Vec::new();
    for raw in split_top_level(inside, ',') {
        if raw.trim().is_empty() {
            continue;
        }
        let (anns, rest) = split_param_anns(&raw);
        let rest = rest.trim();
        let mut words: Vec<&str> = rest.split_whitespace().collect();
        if words.is_empty() {
            continue;
        }
        let last = words.pop().unwrap();
        let pname = last.trim_start_matches("...").to_string();
        let pty = words.join(" ").replace("...", "").trim().to_string();
        if pname.is_empty() || pty.is_empty() {
            continue;
        }
        params.push(JavaParam { name: pname, ty: pty, anns });
    }
    Some((name, ret, params))
}

fn split_param_anns(raw: &str) -> (Vec<Ann>, String) {
    let chars: Vec<char> = raw.chars().collect();
    let mut sc = Scanner::new(&chars);
    let anns = sc.collect_anns(chars.len());
    let rest: String = chars[sc.i.min(chars.len())..].iter().collect();
    (anns, rest)
}

fn top_level_paren(text: &str) -> Option<usize> {
    let mut depth = 0i32;
    for (i, c) in text.char_indices() {
        match c {
            '<' => depth += 1,
            '>' => depth -= 1,
            '(' if depth <= 0 => return Some(i),
            _ => {}
        }
    }
    None
}

fn matching_paren(text: &str, open: usize) -> Option<usize> {
    let mut depth = 0i32;
    for (i, c) in text.char_indices().skip_while(|(i, _)| *i < open) {
        match c {
            '(' => depth += 1,
            ')' => {
                depth -= 1;
                if depth == 0 {
                    return Some(i);
                }
            }
            _ => {}
        }
    }
    None
}

pub fn is_ident(c: char) -> bool {
    c.is_alphanumeric() || c == '_' || c == '$'
}

/// The first string literal inside an annotation's arguments:
/// `@GetMapping("/users")` -> `/users`.
pub fn ann_first_string(a: &Ann) -> Option<String> {
    let chars: Vec<char> = a.args.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '"' {
            let mut out = String::new();
            i += 1;
            while i < chars.len() {
                match chars[i] {
                    '\\' => {
                        i += 1;
                        if let Some(c) = chars.get(i) {
                            out.push(*c);
                        }
                    }
                    '"' => return Some(out),
                    c => out.push(c),
                }
                i += 1;
            }
            return Some(out);
        }
        i += 1;
    }
    None
}

/// `ann_string(args, "value")` for `value = "/users"` / `name = "x"`.
pub fn ann_string(args: &str, key: &str) -> Option<String> {
    let raw = ann_attr(args, key)?;
    let t = raw.trim();
    if t.starts_with('"') && t.ends_with('"') && t.len() >= 2 {
        Some(t[1..t.len() - 1].to_string())
    } else {
        Some(t.to_string())
    }
}

/// Reads one named annotation argument at parenthesis depth 0.
pub fn ann_attr(args: &str, key: &str) -> Option<String> {
    let chars: Vec<char> = args.chars().collect();
    let mut i = 0usize;
    let mut depth = 0i32;
    let mut in_str: Option<char> = None;
    while i < chars.len() {
        let c = chars[i];
        if let Some(q) = in_str {
            if c == '\\' {
                i += 1;
            } else if c == q {
                in_str = None;
            }
            i += 1;
            continue;
        }
        match c {
            '"' | '\'' => {
                in_str = Some(c);
                i += 1;
            }
            '(' | '{' | '[' => {
                depth += 1;
                i += 1;
            }
            ')' | '}' | ']' => {
                depth -= 1;
                i += 1;
            }
            _ if depth == 0
                && args_char_matches(&chars, i, key)
                && (i == 0 || !is_ident(chars[i - 1])) =>
            {
                let mut j = i + key.chars().count();
                while j < chars.len() && chars[j].is_whitespace() {
                    j += 1;
                }
                if j < chars.len() && chars[j] == '=' {
                    j += 1;
                    while j < chars.len() && chars[j].is_whitespace() {
                        j += 1;
                    }
                    let start = j;
                    let mut d = 0i32;
                    let mut q: Option<char> = None;
                    while j < chars.len() {
                        let c = chars[j];
                        if let Some(qq) = q {
                            if c == '\\' {
                                j += 1;
                            } else if c == qq {
                                q = None;
                            }
                        } else if c == '"' || c == '\'' {
                            q = Some(c);
                        } else if c == '(' || c == '{' || c == '[' {
                            d += 1;
                        } else if c == ')' || c == '}' || c == ']' {
                            if d == 0 {
                                break;
                            }
                            d -= 1;
                        } else if c == ',' && d == 0 {
                            break;
                        }
                        j += 1;
                    }
                    let value: String = chars[start..j.min(chars.len())].iter().collect();
                    return Some(value.trim().to_string());
                }
                i += key.chars().count();
            }
            _ => i += 1,
        }
    }
    None
}

fn args_char_matches(chars: &[char], at: usize, key: &str) -> bool {
    let mut j = at;
    for k in key.chars() {
        if chars.get(j).copied() != Some(k) {
            return false;
        }
        j += 1;
    }
    chars.get(j).map(|c| !is_ident(*c)).unwrap_or(true)
}

fn clean_doc(raw: &str) -> Doc {
    let mut lines: Vec<String> = Vec::new();
    for line in raw.lines() {
        let t = line.trim();
        let t = t.trim_start_matches('*').trim();
        lines.push(t.to_string());
    }
    let mut doc = Doc::default();
    let mut in_tags = false;
    let mut tag_lines: Vec<String> = Vec::new();
    let mut body: Vec<String> = Vec::new();
    for line in lines {
        if line.starts_with('@') {
            in_tags = true;
        }
        if in_tags {
            tag_lines.push(line);
        } else {
            body.push(line);
        }
    }
    // summary = first paragraph, description = the rest
    let mut para: Vec<String> = Vec::new();
    let mut desc: Vec<String> = Vec::new();
    let mut seen_blank = false;
    for line in body {
        if line.is_empty() {
            if !para.is_empty() {
                seen_blank = true;
            }
            continue;
        }
        if seen_blank {
            desc.push(line);
        } else {
            para.push(line);
        }
    }
    doc.summary = para.join(" ");
    doc.description = desc.join("\n");

    let mut current: Option<(String, String)> = None; // (tag, name)
    let flush = |current: &mut Option<(String, String)>, text: &str, doc: &mut Doc| {
        if let Some((tag, name)) = current.take() {
            let text = text.trim();
            match tag.as_str() {
                "param" => {
                    doc.params.insert(name, text.to_string());
                }
                "return" => doc.ret = text.to_string(),
                "deprecated" => doc.deprecated = true,
                other => {
                    doc.tags.insert(other.to_string(), text.to_string());
                }
            }
        }
    };
    let mut buffer = String::new();
    for line in tag_lines {
        if !line.starts_with('@') {
            if !buffer.is_empty() {
                buffer.push(' ');
            }
            buffer.push_str(&line);
            continue;
        }
        flush(&mut current, &buffer, &mut doc);
        buffer.clear();
        let (tag, rest) = match line.split_once(char::is_whitespace) {
            Some((t, r)) => (t.trim_start_matches('@').to_string(), r.trim().to_string()),
            None => (line.trim_start_matches('@').to_string(), String::new()),
        };
        if tag == "deprecated" {
            doc.deprecated = true;
        }
        let (name, text) = if tag == "param" {
            match rest.split_once(char::is_whitespace) {
                Some((n, t)) => (n.to_string(), t.to_string()),
                None => (rest, String::new()),
            }
        } else {
            (String::new(), rest)
        };
        buffer = text;
        current = Some((tag, name));
    }
    flush(&mut current, &buffer, &mut doc);
    doc
}

#[cfg(test)]
mod tests {
    use super::*;

    const SRC: &str = r#"
package com.example.web;

/**
 * 用户管理接口
 */
@RestController
@RequestMapping("/api/users")
public class UserController {
    /** 用户服务 */
    @Autowired
    private UserService userService;

    /**
     * 分页查询用户
     *
     * @param keyword 搜索关键字
     */
    @GetMapping("/page")
    public Result<PageResult<UserDto>> page(@RequestParam String keyword,
                                            @RequestParam(defaultValue = "1") int page) {
        if (page > 0) {
            return null;
        }
        return null;
    }
}
"#;

    #[test]
    fn parses_class_annotations_fields_and_methods() {
        let file = parse_file(SRC);
        assert_eq!(file.classes.len(), 1);
        let cls = &file.classes[0];
        assert_eq!(cls.name, "UserController");
        assert_eq!(cls.doc.summary, "用户管理接口");
        assert!(cls.anns.iter().any(|a| a.name == "RestController"));
        assert!(cls.anns.iter().any(|a| a.name == "RequestMapping" && a.args.contains("/api/users")));
        assert_eq!(cls.fields.len(), 1);
        assert_eq!(cls.fields[0].name, "userService");
        assert_eq!(cls.fields[0].ty, "UserService");
        assert_eq!(cls.fields[0].doc, "用户服务");

        assert_eq!(cls.methods.len(), 1);
        let m = &cls.methods[0];
        assert_eq!(m.name, "page");
        assert_eq!(m.ret, "Result<PageResult<UserDto>>");
        assert_eq!(m.doc.summary, "分页查询用户");
        assert_eq!(m.doc.params.get("keyword").map(String::as_str), Some("搜索关键字"));
        assert_eq!(m.params.len(), 2);
        assert_eq!(m.params[0].name, "keyword");
        assert_eq!(m.params[0].ty, "String");
        assert_eq!(m.params[1].anns[0].name, "RequestParam");
    }

    #[test]
    fn parses_markdown_doc_comments() {
        let src = "class A {\n/// 查询用户\n///\n/// 第二段\npublic String x() { return null; }\n}";
        let file = parse_file(src);
        let m = &file.classes[0].methods[0];
        assert_eq!(m.doc.summary, "查询用户");
        assert_eq!(m.doc.description, "第二段");
    }

    #[test]
    fn annotation_arguments_with_parentheses_and_commas() {
        let src = "class A { @PostMapping(path = \"/x\", consumes = {\"a\", \"b\"}) public void x() {} }";
        let file = parse_file(src);
        let m = &file.classes[0].methods[0];
        assert_eq!(m.anns[0].name, "PostMapping");
        assert!(m.anns[0].args.contains("consumes"));
    }
}