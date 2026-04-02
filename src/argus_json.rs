// ARGUS JSON shared utilities — parsing, serialization, and a lightweight value type.
// Reuses lookup tables and SIMD from json.rs. Always available (no feature gate).

#[cfg(feature = "json")]
use crate::json::{
    find_special, hex_val, is_digit, is_ws, needs_escape, HEX, PARSE_ESC, VALUE_DISPATCH,
    WRITE_ESC,
};

// When the json feature is disabled, we need local copies of the primitives.
// These are identical to json.rs but exist so argus_json compiles unconditionally.
#[cfg(not(feature = "json"))]
const WS_MASK: u64 = (1u64 << 0x20) | (1u64 << 0x09) | (1u64 << 0x0A) | (1u64 << 0x0D);
#[cfg(not(feature = "json"))]
static PARSE_ESC: [u8; 256] = {
    let mut t = [0u8; 256];
    t[b'"' as usize] = b'"';
    t[b'\\' as usize] = b'\\';
    t[b'/' as usize] = b'/';
    t[b'n' as usize] = b'\n';
    t[b'r' as usize] = b'\r';
    t[b't' as usize] = b'\t';
    t[b'b' as usize] = 0x08;
    t[b'f' as usize] = 0x0C;
    t
};
#[cfg(not(feature = "json"))]
static WRITE_ESC: [u8; 256] = {
    let mut t = [0u8; 256];
    t[b'"' as usize] = b'"';
    t[b'\\' as usize] = b'\\';
    t[b'\n' as usize] = b'n';
    t[b'\r' as usize] = b'r';
    t[b'\t' as usize] = b't';
    t[0x08] = b'b';
    t[0x0C] = b'f';
    t
};
#[cfg(not(feature = "json"))]
static VALUE_DISPATCH: [u8; 256] = {
    let mut t = [0u8; 256];
    t[b'"' as usize] = 1;
    t[b'{' as usize] = 2;
    t[b'[' as usize] = 3;
    t[b't' as usize] = 4;
    t[b'f' as usize] = 5;
    t[b'n' as usize] = 6;
    t[b'-' as usize] = 7;
    let mut d = b'0';
    while d <= b'9' {
        t[d as usize] = 7;
        d += 1;
    }
    t
};
#[cfg(not(feature = "json"))]
static HEX: &[u8; 16] = b"0123456789abcdef";

#[cfg(not(feature = "json"))]
#[inline(always)]
fn is_ws(ch: u8) -> bool {
    ch <= 0x20 && (WS_MASK >> ch) & 1 != 0
}
#[cfg(not(feature = "json"))]
#[inline(always)]
fn is_digit(ch: u8) -> bool {
    ch.wrapping_sub(b'0') < 10
}
#[cfg(not(feature = "json"))]
#[inline(always)]
fn hex_val(ch: u8) -> u8 {
    (ch & 0xF) + (ch >> 6) * 9
}
#[cfg(not(feature = "json"))]
#[inline(always)]
fn needs_escape(ch: u8) -> bool {
    ch < 0x20 || ch == b'"' || ch == b'\\'
}

// SIMD string scan — fallback when json feature is off
#[cfg(not(feature = "json"))]
#[inline(always)]
fn has_byte(word: u64, target: u8) -> bool {
    let b = 0x0101_0101_0101_0101u64 * target as u64;
    let x = word ^ b;
    (x.wrapping_sub(0x0101_0101_0101_0101) & !x & 0x8080_8080_8080_8080) != 0
}

#[cfg(not(feature = "json"))]
#[inline]
fn find_special(src: &[u8], pos: usize) -> usize {
    let rem = &src[pos..];
    #[cfg(target_arch = "x86")]
    {
        if is_x86_feature_detected!("sse2") && rem.len() >= 16 {
            return unsafe { find_special_sse2(rem) };
        }
    }
    #[cfg(target_arch = "x86_64")]
    {
        if is_x86_feature_detected!("sse2") && rem.len() >= 16 {
            return unsafe { find_special_sse2(rem) };
        }
    }
    find_special_swar(rem)
}

#[cfg(not(feature = "json"))]
#[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
#[target_feature(enable = "sse2")]
unsafe fn find_special_sse2(data: &[u8]) -> usize {
    #[cfg(target_arch = "x86")]
    use std::arch::x86::*;
    #[cfg(target_arch = "x86_64")]
    use std::arch::x86_64::*;
    unsafe {
        let q = _mm_set1_epi8(b'"' as i8);
        let b = _mm_set1_epi8(b'\\' as i8);
        let lim = _mm_set1_epi8(0x20);
        let sp = _mm_set1_epi8(0x20);
        let mut i = 0;
        while i + 16 <= data.len() {
            let chunk = _mm_loadu_si128(data.as_ptr().add(i) as *const __m128i);
            let ctrl = _mm_andnot_si128(
                _mm_cmpeq_epi8(chunk, sp),
                _mm_cmpeq_epi8(_mm_max_epu8(chunk, lim), lim),
            );
            let mask = _mm_movemask_epi8(_mm_or_si128(
                _mm_or_si128(_mm_cmpeq_epi8(chunk, q), _mm_cmpeq_epi8(chunk, b)),
                ctrl,
            )) as u32;
            if mask != 0 {
                return i + mask.trailing_zeros() as usize;
            }
            i += 16;
        }
        while i < data.len() {
            if data[i] == b'"' || data[i] == b'\\' || data[i] < 0x20 {
                return i;
            }
            i += 1;
        }
        data.len()
    }
}

#[cfg(not(feature = "json"))]
fn find_special_swar(data: &[u8]) -> usize {
    let mut i = 0;
    while i + 8 <= data.len() {
        let w = u64::from_le_bytes([
            data[i],
            data[i + 1],
            data[i + 2],
            data[i + 3],
            data[i + 4],
            data[i + 5],
            data[i + 6],
            data[i + 7],
        ]);
        if has_byte(w, b'"')
            || has_byte(w, b'\\')
            || (w.wrapping_sub(0x2020202020202020) & !w & 0x8080808080808080) != 0
        {
            break;
        }
        i += 8;
    }
    while i < data.len() {
        if data[i] == b'"' || data[i] == b'\\' || data[i] < 0x20 {
            return i;
        }
        i += 1;
    }
    data.len()
}

// ---------------------------------------------------------------------------
// Streaming parser core
// ---------------------------------------------------------------------------

struct Parser<'a> {
    pos: usize,
    src: &'a [u8],
}

impl<'a> Parser<'a> {
    fn new(input: &'a [u8]) -> Self {
        let pos = if input.starts_with(&[0xEF, 0xBB, 0xBF]) { 3 } else { 0 };
        Self { pos, src: input }
    }

    #[inline(always)]
    fn peek(&self) -> u8 {
        if self.pos < self.src.len() {
            self.src[self.pos]
        } else {
            0
        }
    }

    #[inline(always)]
    fn bump(&mut self) -> u8 {
        let ch = self.peek();
        if self.pos < self.src.len() {
            self.pos += 1;
        }
        ch
    }

    #[inline(always)]
    fn eof(&self) -> bool {
        self.pos >= self.src.len()
    }

    fn skip_ws(&mut self) {
        while self.pos < self.src.len() && is_ws(self.src[self.pos]) {
            self.pos += 1;
        }
    }

    fn expect(&mut self, ch: u8) -> Result<(), ()> {
        self.skip_ws();
        if self.bump() == ch {
            Ok(())
        } else {
            Err(())
        }
    }

    // Parse a JSON string, returning owned String
    fn parse_string(&mut self) -> Result<String, ()> {
        self.expect(b'"')?;
        let mut s = String::new();
        let start = self.pos;
        let skip = find_special(self.src, self.pos);
        if skip > 0 {
            self.pos += skip;
            s.push_str(std::str::from_utf8(&self.src[start..self.pos]).map_err(|_| ())?);
        }
        loop {
            if self.pos >= self.src.len() {
                return Err(());
            }
            match self.bump() {
                b'"' => return Ok(s),
                b'\\' => {
                    let e = self.bump();
                    if e == b'u' {
                        let mut v = 0u16;
                        for _ in 0..4 {
                            v = (v << 4) | hex_val(self.bump()) as u16;
                        }
                        if (0xD800..=0xDBFF).contains(&v) {
                            if self.bump() != b'\\' || self.bump() != b'u' {
                                return Err(());
                            }
                            let mut lo = 0u16;
                            for _ in 0..4 {
                                lo = (lo << 4) | hex_val(self.bump()) as u16;
                            }
                            s.push(
                                char::from_u32(
                                    0x10000 + ((v as u32 - 0xD800) << 10) + (lo as u32 - 0xDC00),
                                )
                                .unwrap_or('\u{FFFD}'),
                            );
                        } else {
                            s.push(char::from_u32(v as u32).unwrap_or('\u{FFFD}'));
                        }
                    } else {
                        let r = PARSE_ESC[e as usize];
                        if r != 0 {
                            s.push(r as char);
                        } else {
                            return Err(());
                        }
                    }
                }
                c => s.push(c as char),
            }
        }
    }

    // Parse a JSON number, returning f64
    fn parse_number(&mut self) -> Result<f64, ()> {
        let start = self.pos;
        if self.peek() == b'-' {
            self.bump();
        }
        if self.peek() == b'0' {
            self.bump();
        } else {
            if !is_digit(self.peek()) {
                return Err(());
            }
            while is_digit(self.peek()) {
                self.bump();
            }
        }
        if self.peek() == b'.' {
            self.bump();
            if !is_digit(self.peek()) {
                return Err(());
            }
            while is_digit(self.peek()) {
                self.bump();
            }
        }
        if self.peek() | 0x20 == b'e' {
            self.bump();
            if self.peek() == b'+' || self.peek() == b'-' {
                self.bump();
            }
            if !is_digit(self.peek()) {
                return Err(());
            }
            while is_digit(self.peek()) {
                self.bump();
            }
        }
        let s = std::str::from_utf8(&self.src[start..self.pos]).map_err(|_| ())?;
        s.parse().map_err(|_| ())
    }

    // Skip a JSON value without allocating (used by specialized parsers)
    #[allow(dead_code)]
    fn skip_value(&mut self) -> Result<(), ()> {
        self.skip_ws();
        match VALUE_DISPATCH[self.peek() as usize] {
            1 => self.skip_string(),
            2 => self.skip_object(),
            3 => self.skip_array(),
            4 => {
                self.pos += 4;
                Ok(())
            }
            5 => {
                self.pos += 5;
                Ok(())
            }
            6 => {
                self.pos += 4;
                Ok(())
            }
            7 => {
                self.parse_number()?;
                Ok(())
            }
            _ => Err(()),
        }
    }

    #[allow(dead_code)]
    fn skip_string(&mut self) -> Result<(), ()> {
        if self.bump() != b'"' {
            return Err(());
        }
        loop {
            let skip = find_special(self.src, self.pos);
            self.pos += skip;
            if self.eof() {
                return Err(());
            }
            match self.bump() {
                b'"' => return Ok(()),
                b'\\' => {
                    let e = self.bump();
                    if e == b'u' {
                        self.pos += 4;
                    }
                }
                _ => {}
            }
        }
    }

    #[allow(dead_code)]
    fn skip_array(&mut self) -> Result<(), ()> {
        self.pos += 1;
        self.skip_ws();
        if self.peek() == b']' {
            self.pos += 1;
            return Ok(());
        }
        loop {
            self.skip_value()?;
            self.skip_ws();
            match self.peek() {
                b',' => {
                    self.pos += 1;
                }
                b']' => {
                    self.pos += 1;
                    return Ok(());
                }
                _ => return Err(()),
            }
        }
    }

    #[allow(dead_code)]
    fn skip_object(&mut self) -> Result<(), ()> {
        self.pos += 1;
        self.skip_ws();
        if self.peek() == b'}' {
            self.pos += 1;
            return Ok(());
        }
        loop {
            self.skip_ws();
            self.skip_string()?;
            self.skip_ws();
            if self.bump() != b':' {
                return Err(());
            }
            self.skip_value()?;
            self.skip_ws();
            match self.peek() {
                b',' => {
                    self.pos += 1;
                }
                b'}' => {
                    self.pos += 1;
                    return Ok(());
                }
                _ => return Err(()),
            }
        }
    }

    // Full value parse into JsonValue
    fn value(&mut self) -> Result<JsonValue, ()> {
        self.skip_ws();
        match VALUE_DISPATCH[self.peek() as usize] {
            1 => self.parse_string().map(JsonValue::Str),
            2 => self.object(),
            3 => self.array(),
            4 => {
                self.pos += 4;
                Ok(JsonValue::Bool(true))
            }
            5 => {
                self.pos += 5;
                Ok(JsonValue::Bool(false))
            }
            6 => {
                self.pos += 4;
                Ok(JsonValue::Null)
            }
            7 => self.parse_number().map(JsonValue::Number),
            _ => Err(()),
        }
    }

    fn array(&mut self) -> Result<JsonValue, ()> {
        self.expect(b'[')?;
        self.skip_ws();
        let mut items = Vec::new();
        if self.peek() == b']' {
            self.bump();
            return Ok(JsonValue::Array(items));
        }
        loop {
            items.push(self.value()?);
            self.skip_ws();
            match self.peek() {
                b',' => {
                    self.bump();
                }
                b']' => {
                    self.bump();
                    return Ok(JsonValue::Array(items));
                }
                _ => return Err(()),
            }
        }
    }

    fn object(&mut self) -> Result<JsonValue, ()> {
        self.expect(b'{')?;
        self.skip_ws();
        let mut pairs = Vec::new();
        if self.peek() == b'}' {
            self.bump();
            return Ok(JsonValue::Object(pairs));
        }
        loop {
            self.skip_ws();
            let k = self.parse_string()?;
            self.expect(b':')?;
            let v = self.value()?;
            pairs.push((k, v));
            self.skip_ws();
            match self.peek() {
                b',' => {
                    self.bump();
                }
                b'}' => {
                    self.bump();
                    return Ok(JsonValue::Object(pairs));
                }
                _ => return Err(()),
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Lightweight JSON value type
// ---------------------------------------------------------------------------

/// A lightweight JSON value — no external deps, Vec-backed objects for speed.
#[derive(Debug, Clone, PartialEq)]
pub enum JsonValue {
    Null,
    Bool(bool),
    Number(f64),
    Str(String),
    Array(Vec<JsonValue>),
    /// Preserves insertion order. Vec is faster than BTreeMap for small objects.
    Object(Vec<(String, JsonValue)>),
}

impl JsonValue {
    pub fn as_str(&self) -> Option<&str> {
        match self {
            JsonValue::Str(s) => Some(s.as_str()),
            _ => None,
        }
    }

    pub fn as_f64(&self) -> Option<f64> {
        match self {
            JsonValue::Number(n) => Some(*n),
            _ => None,
        }
    }

    pub fn as_i64(&self) -> Option<i64> {
        match self {
            JsonValue::Number(n) => {
                let i = *n as i64;
                if (i as f64) == *n {
                    Some(i)
                } else {
                    None
                }
            }
            _ => None,
        }
    }

    pub fn as_bool(&self) -> Option<bool> {
        match self {
            JsonValue::Bool(b) => Some(*b),
            _ => None,
        }
    }

    pub fn as_array(&self) -> Option<&[JsonValue]> {
        match self {
            JsonValue::Array(a) => Some(a.as_slice()),
            _ => None,
        }
    }

    pub fn as_object(&self) -> Option<&[(String, JsonValue)]> {
        match self {
            JsonValue::Object(o) => Some(o.as_slice()),
            _ => None,
        }
    }

    /// Lookup a key in an Object. Linear scan — fast for small objects.
    pub fn get(&self, key: &str) -> Option<&JsonValue> {
        match self {
            JsonValue::Object(pairs) => {
                for (k, v) in pairs {
                    if k == key {
                        return Some(v);
                    }
                }
                None
            }
            _ => None,
        }
    }

    pub fn is_null(&self) -> bool {
        matches!(self, JsonValue::Null)
    }
}

// ---------------------------------------------------------------------------
// From conversions
// ---------------------------------------------------------------------------

impl From<bool> for JsonValue {
    fn from(b: bool) -> Self {
        JsonValue::Bool(b)
    }
}

impl From<f64> for JsonValue {
    fn from(n: f64) -> Self {
        JsonValue::Number(n)
    }
}

impl From<i64> for JsonValue {
    fn from(n: i64) -> Self {
        JsonValue::Number(n as f64)
    }
}

impl From<i32> for JsonValue {
    fn from(n: i32) -> Self {
        JsonValue::Number(n as f64)
    }
}

impl From<u32> for JsonValue {
    fn from(n: u32) -> Self {
        JsonValue::Number(n as f64)
    }
}

impl From<u64> for JsonValue {
    fn from(n: u64) -> Self {
        JsonValue::Number(n as f64)
    }
}

impl From<usize> for JsonValue {
    fn from(n: usize) -> Self {
        JsonValue::Number(n as f64)
    }
}

impl From<String> for JsonValue {
    fn from(s: String) -> Self {
        JsonValue::Str(s)
    }
}

impl From<&str> for JsonValue {
    fn from(s: &str) -> Self {
        JsonValue::Str(s.to_owned())
    }
}

impl<T: Into<JsonValue>> From<Vec<T>> for JsonValue {
    fn from(v: Vec<T>) -> Self {
        JsonValue::Array(v.into_iter().map(Into::into).collect())
    }
}

impl<T: Into<JsonValue>> From<Option<T>> for JsonValue {
    fn from(opt: Option<T>) -> Self {
        match opt {
            Some(v) => v.into(),
            None => JsonValue::Null,
        }
    }
}

// ---------------------------------------------------------------------------
// Public parsing functions
// ---------------------------------------------------------------------------

/// Parse a JSON array of strings: `["a","b","c"]` -> `Vec<String>`.
pub fn parse_string_array(src: &[u8]) -> Result<Vec<String>, ()> {
    let mut p = Parser::new(src);
    p.expect(b'[')?;
    p.skip_ws();
    let mut result = Vec::new();
    if p.peek() == b']' {
        p.bump();
        return Ok(result);
    }
    loop {
        p.skip_ws();
        result.push(p.parse_string()?);
        p.skip_ws();
        match p.peek() {
            b',' => {
                p.bump();
            }
            b']' => {
                p.bump();
                return Ok(result);
            }
            _ => return Err(()),
        }
    }
}

/// Parse a JSON array of string pairs: `[["k","v"],["k2","v2"]]` -> `Vec<(String, String)>`.
pub fn parse_string_pair_array(src: &[u8]) -> Result<Vec<(String, String)>, ()> {
    let mut p = Parser::new(src);
    p.expect(b'[')?;
    p.skip_ws();
    let mut result = Vec::new();
    if p.peek() == b']' {
        p.bump();
        return Ok(result);
    }
    loop {
        p.skip_ws();
        p.expect(b'[')?;
        p.skip_ws();
        let k = p.parse_string()?;
        p.skip_ws();
        if p.bump() != b',' {
            return Err(());
        }
        p.skip_ws();
        let v = p.parse_string()?;
        p.skip_ws();
        if p.bump() != b']' {
            return Err(());
        }
        result.push((k, v));
        p.skip_ws();
        match p.peek() {
            b',' => {
                p.bump();
            }
            b']' => {
                p.bump();
                return Ok(result);
            }
            _ => return Err(()),
        }
    }
}

/// Parse a JSON object with string values: `{"k":"v","k2":"v2"}` -> `Vec<(String, String)>`.
/// Preserves insertion order.
pub fn parse_string_map(src: &[u8]) -> Result<Vec<(String, String)>, ()> {
    let mut p = Parser::new(src);
    p.expect(b'{')?;
    p.skip_ws();
    let mut result = Vec::new();
    if p.peek() == b'}' {
        p.bump();
        return Ok(result);
    }
    loop {
        p.skip_ws();
        let k = p.parse_string()?;
        p.expect(b':')?;
        p.skip_ws();
        let v = p.parse_string()?;
        result.push((k, v));
        p.skip_ws();
        match p.peek() {
            b',' => {
                p.bump();
            }
            b'}' => {
                p.bump();
                return Ok(result);
            }
            _ => return Err(()),
        }
    }
}

/// Parse a JSON object with numeric values: `{"k":1.5,"k2":42}` -> `Vec<(String, f64)>`.
/// Preserves insertion order.
pub fn parse_number_map(src: &[u8]) -> Result<Vec<(String, f64)>, ()> {
    let mut p = Parser::new(src);
    p.expect(b'{')?;
    p.skip_ws();
    let mut result = Vec::new();
    if p.peek() == b'}' {
        p.bump();
        return Ok(result);
    }
    loop {
        p.skip_ws();
        let k = p.parse_string()?;
        p.expect(b':')?;
        p.skip_ws();
        let v = p.parse_number()?;
        result.push((k, v));
        p.skip_ws();
        match p.peek() {
            b',' => {
                p.bump();
            }
            b'}' => {
                p.bump();
                return Ok(result);
            }
            _ => return Err(()),
        }
    }
}

/// Parse any JSON value into a `JsonValue` tree.
pub fn parse_value(src: &[u8]) -> Result<JsonValue, ()> {
    let mut p = Parser::new(src);
    let val = p.value()?;
    p.skip_ws();
    if !p.eof() {
        return Err(());
    }
    Ok(val)
}

// ---------------------------------------------------------------------------
// Serialization
// ---------------------------------------------------------------------------

/// Escape a JSON string value, writing `\"`, escaped chars, and `\"` into `out`.
/// Does NOT include the surrounding quotes — caller adds those if needed.
pub fn escape_json_string(s: &str, out: &mut String) {
    for ch in s.bytes() {
        if needs_escape(ch) {
            let e = WRITE_ESC[ch as usize];
            if e != 0 {
                out.push('\\');
                out.push(e as char);
            } else {
                out.push_str("\\u00");
                out.push(HEX[(ch >> 4) as usize] as char);
                out.push(HEX[(ch & 0xF) as usize] as char);
            }
        } else {
            out.push(ch as char);
        }
    }
}

/// Serialize a `JsonValue` to a compact JSON string.
pub fn serialize_value(val: &JsonValue) -> String {
    let mut out = String::new();
    write_value(val, &mut out);
    out
}

fn write_value(val: &JsonValue, out: &mut String) {
    match val {
        JsonValue::Null => out.push_str("null"),
        JsonValue::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
        JsonValue::Number(n) => {
            if n.is_finite() && *n == (*n as i64) as f64 {
                // Integer-valued float: emit without decimal
                use std::fmt::Write;
                let _ = write!(out, "{}", *n as i64);
            } else if n.is_finite() {
                use std::fmt::Write;
                let _ = write!(out, "{}", n);
            } else {
                out.push_str("null"); // NaN/Inf -> null per JSON spec
            }
        }
        JsonValue::Str(s) => {
            out.push('"');
            escape_json_string(s, out);
            out.push('"');
        }
        JsonValue::Array(items) => {
            out.push('[');
            for (i, item) in items.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                write_value(item, out);
            }
            out.push(']');
        }
        JsonValue::Object(pairs) => {
            out.push('{');
            for (i, (k, v)) in pairs.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                out.push('"');
                escape_json_string(k, out);
                out.push_str("\":");
                write_value(v, out);
            }
            out.push('}');
        }
    }
}

// ---------------------------------------------------------------------------
// Convenience builders
// ---------------------------------------------------------------------------

/// Build a JSON object from string key-value pairs.
/// `json_obj(&[("name","Alice"),("role","admin")])` -> `{"name":"Alice","role":"admin"}`
pub fn json_obj(pairs: &[(&str, &str)]) -> String {
    let mut out = String::with_capacity(pairs.len() * 24);
    out.push('{');
    for (i, (k, v)) in pairs.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        out.push('"');
        escape_json_string(k, &mut out);
        out.push_str("\":\"");
        escape_json_string(v, &mut out);
        out.push('"');
    }
    out.push('}');
    out
}

/// A typed JSON fragment for mixed-type object building.
#[derive(Debug, Clone, Copy)]
pub enum JsonPart<'a> {
    Str(&'a str),
    Int(i64),
    Float(f64),
    Bool(bool),
    Null,
    /// Raw JSON — inserted verbatim (caller must ensure validity).
    Raw(&'a str),
}

/// Build a JSON object from mixed-type key-value pairs.
/// ```ignore
/// json_obj_mixed(&[
///     ("name", JsonPart::Str("Alice")),
///     ("age", JsonPart::Int(30)),
///     ("score", JsonPart::Float(9.5)),
///     ("active", JsonPart::Bool(true)),
///     ("meta", JsonPart::Null),
///     ("tags", JsonPart::Raw("[\"a\",\"b\"]")),
/// ])
/// ```
pub fn json_obj_mixed(pairs: &[(&str, JsonPart<'_>)]) -> String {
    let mut out = String::with_capacity(pairs.len() * 32);
    out.push('{');
    for (i, (k, v)) in pairs.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        out.push('"');
        escape_json_string(k, &mut out);
        out.push_str("\":");
        match v {
            JsonPart::Str(s) => {
                out.push('"');
                escape_json_string(s, &mut out);
                out.push('"');
            }
            JsonPart::Int(n) => {
                use std::fmt::Write;
                let _ = write!(out, "{}", n);
            }
            JsonPart::Float(n) => {
                if n.is_finite() {
                    use std::fmt::Write;
                    let _ = write!(out, "{}", n);
                } else {
                    out.push_str("null");
                }
            }
            JsonPart::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
            JsonPart::Null => out.push_str("null"),
            JsonPart::Raw(raw) => out.push_str(raw),
        }
    }
    out.push('}');
    out
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_string_array() {
        let r = parse_string_array(b"[\"hello\",\"world\"]").unwrap();
        assert_eq!(r, vec!["hello", "world"]);
        assert_eq!(parse_string_array(b"[]").unwrap(), Vec::<String>::new());
    }

    #[test]
    fn test_parse_string_pair_array() {
        let r = parse_string_pair_array(b"[[\"a\",\"1\"],[\"b\",\"2\"]]").unwrap();
        assert_eq!(
            r,
            vec![
                ("a".to_owned(), "1".to_owned()),
                ("b".to_owned(), "2".to_owned())
            ]
        );
    }

    #[test]
    fn test_parse_string_map() {
        let r = parse_string_map(b"{\"name\":\"Alice\",\"role\":\"admin\"}").unwrap();
        assert_eq!(
            r,
            vec![
                ("name".to_owned(), "Alice".to_owned()),
                ("role".to_owned(), "admin".to_owned())
            ]
        );
        assert_eq!(parse_string_map(b"{}").unwrap(), Vec::<(String, String)>::new());
    }

    #[test]
    fn test_parse_number_map() {
        let r = parse_number_map(b"{\"x\":1.5,\"y\":42}").unwrap();
        assert_eq!(r, vec![("x".to_owned(), 1.5), ("y".to_owned(), 42.0)]);
    }

    #[test]
    fn test_parse_value_null() {
        assert_eq!(parse_value(b"null").unwrap(), JsonValue::Null);
    }

    #[test]
    fn test_parse_value_bool() {
        assert_eq!(parse_value(b"true").unwrap(), JsonValue::Bool(true));
        assert_eq!(parse_value(b"false").unwrap(), JsonValue::Bool(false));
    }

    #[test]
    fn test_parse_value_number() {
        assert_eq!(parse_value(b"42").unwrap(), JsonValue::Number(42.0));
        assert_eq!(parse_value(b"-3.14").unwrap(), JsonValue::Number(-3.14));
        assert_eq!(parse_value(b"1e10").unwrap(), JsonValue::Number(1e10));
    }

    #[test]
    fn test_parse_value_string() {
        assert_eq!(
            parse_value(b"\"hello\\nworld\"").unwrap(),
            JsonValue::Str("hello\nworld".to_owned())
        );
    }

    #[test]
    fn test_parse_value_array() {
        let v = parse_value(b"[1,\"two\",true,null]").unwrap();
        let arr = v.as_array().unwrap();
        assert_eq!(arr.len(), 4);
        assert_eq!(arr[0].as_f64(), Some(1.0));
        assert_eq!(arr[1].as_str(), Some("two"));
        assert_eq!(arr[2].as_bool(), Some(true));
        assert!(arr[3].is_null());
    }

    #[test]
    fn test_parse_value_object() {
        let v = parse_value(b"{\"a\":1,\"b\":\"hi\"}").unwrap();
        assert_eq!(v.get("a").unwrap().as_i64(), Some(1));
        assert_eq!(v.get("b").unwrap().as_str(), Some("hi"));
        assert!(v.get("c").is_none());
    }

    #[test]
    fn test_parse_value_nested() {
        let v = parse_value(b"{\"arr\":[1,2],\"obj\":{\"x\":true}}").unwrap();
        let arr = v.get("arr").unwrap().as_array().unwrap();
        assert_eq!(arr.len(), 2);
        assert_eq!(v.get("obj").unwrap().get("x").unwrap().as_bool(), Some(true));
    }

    #[test]
    fn test_parse_value_unicode_escape() {
        let v = parse_value(b"\"\\u0041\\u0042\"").unwrap();
        assert_eq!(v.as_str(), Some("AB"));
    }

    #[test]
    fn test_parse_value_surrogate_pair() {
        // U+1F600 = \uD83D\uDE00
        let v = parse_value(b"\"\\uD83D\\uDE00\"").unwrap();
        assert_eq!(v.as_str().unwrap(), "\u{1F600}");
    }

    #[test]
    fn test_serialize_value() {
        let v = JsonValue::Object(vec![
            ("name".to_owned(), JsonValue::Str("Alice".to_owned())),
            ("age".to_owned(), JsonValue::Number(30.0)),
            ("active".to_owned(), JsonValue::Bool(true)),
            ("data".to_owned(), JsonValue::Null),
            (
                "tags".to_owned(),
                JsonValue::Array(vec![
                    JsonValue::Str("a".to_owned()),
                    JsonValue::Str("b".to_owned()),
                ]),
            ),
        ]);
        let s = serialize_value(&v);
        assert_eq!(s, "{\"name\":\"Alice\",\"age\":30,\"active\":true,\"data\":null,\"tags\":[\"a\",\"b\"]}");
    }

    #[test]
    fn test_serialize_roundtrip() {
        let input = b"{\"a\":[1,2.5,true,null,\"hi\"],\"b\":{\"c\":false}}";
        let v = parse_value(input).unwrap();
        let s = serialize_value(&v);
        let v2 = parse_value(s.as_bytes()).unwrap();
        assert_eq!(v, v2);
    }

    #[test]
    fn test_escape_json_string() {
        let mut out = String::new();
        escape_json_string("hello\n\"world\"\t\\", &mut out);
        assert_eq!(out, "hello\\n\\\"world\\\"\\t\\\\");
    }

    #[test]
    fn test_escape_control_chars() {
        let mut out = String::new();
        escape_json_string("\x00\x01\x1f", &mut out);
        assert_eq!(out, "\\u0000\\u0001\\u001f");
    }

    #[test]
    fn test_json_obj() {
        let s = json_obj(&[("name", "Alice"), ("role", "admin")]);
        assert_eq!(s, "{\"name\":\"Alice\",\"role\":\"admin\"}");
    }

    #[test]
    fn test_json_obj_empty() {
        assert_eq!(json_obj(&[]), "{}");
    }

    #[test]
    fn test_json_obj_mixed() {
        let s = json_obj_mixed(&[
            ("name", JsonPart::Str("Alice")),
            ("age", JsonPart::Int(30)),
            ("score", JsonPart::Float(9.5)),
            ("active", JsonPart::Bool(true)),
            ("data", JsonPart::Null),
            ("tags", JsonPart::Raw("[\"a\"]")),
        ]);
        let v = parse_value(s.as_bytes()).unwrap();
        assert_eq!(v.get("name").unwrap().as_str(), Some("Alice"));
        assert_eq!(v.get("age").unwrap().as_i64(), Some(30));
        assert_eq!(v.get("active").unwrap().as_bool(), Some(true));
        assert!(v.get("data").unwrap().is_null());
    }

    #[test]
    fn test_from_conversions() {
        assert_eq!(JsonValue::from(true), JsonValue::Bool(true));
        assert_eq!(JsonValue::from(42i64), JsonValue::Number(42.0));
        assert_eq!(JsonValue::from(3.14f64), JsonValue::Number(3.14));
        assert_eq!(JsonValue::from("hi"), JsonValue::Str("hi".to_owned()));
        assert_eq!(
            JsonValue::from("hello".to_owned()),
            JsonValue::Str("hello".to_owned())
        );
        assert_eq!(JsonValue::from(None::<i64>), JsonValue::Null);
        assert_eq!(JsonValue::from(Some(5i64)), JsonValue::Number(5.0));
    }

    #[test]
    fn test_as_i64_non_integer() {
        assert_eq!(JsonValue::Number(3.7).as_i64(), None);
        assert_eq!(JsonValue::Number(3.0).as_i64(), Some(3));
        assert_eq!(JsonValue::Number(-5.0).as_i64(), Some(-5));
    }

    #[test]
    fn test_as_object_order() {
        let v = parse_value(b"{\"z\":1,\"a\":2,\"m\":3}").unwrap();
        let obj = v.as_object().unwrap();
        assert_eq!(obj[0].0, "z");
        assert_eq!(obj[1].0, "a");
        assert_eq!(obj[2].0, "m");
    }

    #[test]
    fn test_trailing_data_rejected() {
        assert!(parse_value(b"42 trailing").is_err());
        assert!(parse_value(b"{} []").is_err());
    }

    #[test]
    fn test_bom_handled() {
        let input = b"\xEF\xBB\xBF{\"a\":1}";
        let v = parse_value(input).unwrap();
        assert_eq!(v.get("a").unwrap().as_i64(), Some(1));
    }

    #[test]
    fn test_whitespace_tolerance() {
        let v = parse_value(b"  { \"a\" : [ 1 , 2 ] }  ").unwrap();
        assert_eq!(v.get("a").unwrap().as_array().unwrap().len(), 2);
    }

    #[test]
    fn test_empty_containers() {
        assert_eq!(parse_value(b"[]").unwrap(), JsonValue::Array(vec![]));
        assert_eq!(parse_value(b"{}").unwrap(), JsonValue::Object(vec![]));
    }

    #[test]
    fn test_number_formats() {
        assert_eq!(parse_value(b"0").unwrap().as_f64(), Some(0.0));
        assert_eq!(parse_value(b"-0").unwrap().as_f64(), Some(0.0));
        assert_eq!(parse_value(b"1.5e2").unwrap().as_f64(), Some(150.0));
        assert_eq!(parse_value(b"1.5E-1").unwrap().as_f64(), Some(0.15));
        assert_eq!(parse_value(b"1e+3").unwrap().as_f64(), Some(1000.0));
    }

    #[test]
    fn test_serialize_nan_inf() {
        assert_eq!(serialize_value(&JsonValue::Number(f64::NAN)), "null");
        assert_eq!(serialize_value(&JsonValue::Number(f64::INFINITY)), "null");
    }

    // --- Additional edge case tests for optimization coverage ---

    #[test]
    fn test_parse_deeply_nested() {
        let json = b"{\"a\":{\"b\":{\"c\":{\"d\":[1,2,{\"e\":true}]}}}}";
        let v = parse_value(json).unwrap();
        let e = v.get("a").unwrap()
            .get("b").unwrap()
            .get("c").unwrap()
            .get("d").unwrap()
            .as_array().unwrap()[2]
            .get("e").unwrap();
        assert_eq!(e.as_bool(), Some(true));
    }

    #[test]
    fn test_parse_unicode_string_content() {
        let v = parse_value(b"\"caf\\u00E9\"").unwrap();
        assert_eq!(v.as_str(), Some("caf\u{00E9}"));
    }

    #[test]
    fn test_parse_large_number() {
        let v = parse_value(b"9007199254740992").unwrap(); // 2^53
        assert_eq!(v.as_f64(), Some(9007199254740992.0));
    }

    #[test]
    fn test_parse_negative_zero() {
        let v = parse_value(b"-0").unwrap();
        assert_eq!(v.as_f64(), Some(0.0));
    }

    #[test]
    fn test_parse_scientific_notation() {
        assert_eq!(parse_value(b"1e100").unwrap().as_f64(), Some(1e100));
        assert_eq!(parse_value(b"1E-5").unwrap().as_f64(), Some(1e-5));
        assert_eq!(parse_value(b"-1.5e+3").unwrap().as_f64(), Some(-1500.0));
    }

    #[test]
    fn test_parse_empty_string() {
        let v = parse_value(b"\"\"").unwrap();
        assert_eq!(v.as_str(), Some(""));
    }

    #[test]
    fn test_parse_string_with_all_escapes() {
        let v = parse_value(b"\"\\\"\\\\\\n\\r\\t\\b\\f\\/\"").unwrap();
        assert_eq!(v.as_str(), Some("\"\\\n\r\t\x08\x0C/"));
    }

    #[test]
    fn test_serialize_empty_array() {
        assert_eq!(serialize_value(&JsonValue::Array(vec![])), "[]");
    }

    #[test]
    fn test_serialize_empty_object() {
        assert_eq!(serialize_value(&JsonValue::Object(vec![])), "{}");
    }

    #[test]
    fn test_serialize_nested_empty() {
        let v = JsonValue::Object(vec![
            ("empty_arr".to_owned(), JsonValue::Array(vec![])),
            ("empty_obj".to_owned(), JsonValue::Object(vec![])),
        ]);
        let s = serialize_value(&v);
        assert_eq!(s, "{\"empty_arr\":[],\"empty_obj\":{}}");
    }

    #[test]
    fn test_parse_string_array_with_escapes() {
        let r = parse_string_array(b"[\"hello\\nworld\",\"tab\\there\"]").unwrap();
        assert_eq!(r[0], "hello\nworld");
        assert_eq!(r[1], "tab\there");
    }

    #[test]
    fn test_parse_string_array_invalid() {
        assert!(parse_string_array(b"not array").is_err());
        assert!(parse_string_array(b"[1,2]").is_err()); // not strings
        assert!(parse_string_array(b"[\"a\",]").is_err()); // trailing comma
    }

    #[test]
    fn test_parse_string_map_empty() {
        assert_eq!(parse_string_map(b"{}").unwrap(), Vec::<(String, String)>::new());
    }

    #[test]
    fn test_parse_string_map_invalid() {
        assert!(parse_string_map(b"not json").is_err());
        assert!(parse_string_map(b"{\"a\":1}").is_err()); // value not string
    }

    #[test]
    fn test_parse_number_map_empty() {
        assert_eq!(parse_number_map(b"{}").unwrap(), Vec::<(String, f64)>::new());
    }

    #[test]
    fn test_parse_number_map_invalid() {
        assert!(parse_number_map(b"not json").is_err());
        assert!(parse_number_map(b"{\"a\":\"not_num\"}").is_err());
    }

    #[test]
    fn test_parse_string_pair_array_empty() {
        assert_eq!(parse_string_pair_array(b"[]").unwrap(), Vec::<(String, String)>::new());
    }

    #[test]
    fn test_parse_string_pair_array_invalid() {
        assert!(parse_string_pair_array(b"not json").is_err());
        assert!(parse_string_pair_array(b"[[\"a\"]]").is_err()); // only one element
    }

    #[test]
    fn test_json_value_get_nonexistent_key() {
        let v = JsonValue::Object(vec![("a".to_owned(), JsonValue::Null)]);
        assert!(v.get("b").is_none());
    }

    #[test]
    fn test_json_value_get_on_non_object() {
        let v = JsonValue::Array(vec![]);
        assert!(v.get("a").is_none());
    }

    #[test]
    fn test_json_value_as_str_on_non_string() {
        assert!(JsonValue::Number(42.0).as_str().is_none());
        assert!(JsonValue::Bool(true).as_str().is_none());
        assert!(JsonValue::Null.as_str().is_none());
    }

    #[test]
    fn test_json_value_as_array_on_non_array() {
        assert!(JsonValue::Str("hi".to_owned()).as_array().is_none());
    }

    #[test]
    fn test_json_value_as_object_on_non_object() {
        assert!(JsonValue::Array(vec![]).as_object().is_none());
    }

    #[test]
    fn test_serialize_integer_valued_float() {
        assert_eq!(serialize_value(&JsonValue::Number(42.0)), "42");
        assert_eq!(serialize_value(&JsonValue::Number(-100.0)), "-100");
        assert_eq!(serialize_value(&JsonValue::Number(0.0)), "0");
    }

    #[test]
    fn test_serialize_fractional_float() {
        let s = serialize_value(&JsonValue::Number(3.14));
        assert!(s.contains("3.14"));
    }

    #[test]
    fn test_serialize_neg_infinity() {
        assert_eq!(serialize_value(&JsonValue::Number(f64::NEG_INFINITY)), "null");
    }

    #[test]
    fn test_json_obj_mixed_float_nan() {
        let s = json_obj_mixed(&[("val", JsonPart::Float(f64::NAN))]);
        assert!(s.contains("null"));
    }

    #[test]
    fn test_json_obj_escaping() {
        let s = json_obj(&[("k\"ey", "v\"al")]);
        assert!(s.contains("k\\\"ey"));
        assert!(s.contains("v\\\"al"));
    }

    #[test]
    fn test_parse_value_invalid_inputs() {
        assert!(parse_value(b"").is_err());
        assert!(parse_value(b"   ").is_err());
        assert!(parse_value(b",").is_err());
        assert!(parse_value(b"{").is_err());
        assert!(parse_value(b"[").is_err());
        assert!(parse_value(b"\"unterminated").is_err());
    }

    #[test]
    fn test_from_vec_conversion() {
        let v: JsonValue = vec![1i64, 2, 3].into();
        match v {
            JsonValue::Array(arr) => {
                assert_eq!(arr.len(), 3);
                assert_eq!(arr[0].as_f64(), Some(1.0));
            }
            _ => panic!("Expected array"),
        }
    }

    #[test]
    fn test_from_u32() {
        assert_eq!(JsonValue::from(42u32), JsonValue::Number(42.0));
    }

    #[test]
    fn test_from_u64() {
        assert_eq!(JsonValue::from(100u64), JsonValue::Number(100.0));
    }

    #[test]
    fn test_from_usize() {
        assert_eq!(JsonValue::from(7usize), JsonValue::Number(7.0));
    }
}
