// ARGUS JSON engine — hand-rolled streaming parser, drop-in replacement for serde path.
// Respects VALID_JSON_MAX_RECURSION_DEPTH. SSE2 SIMD with SWAR fallback. Zero extra deps.

const VALID_JSON_MAX_RECURSION_DEPTH: usize = 8;

byond_fn!(fn json_is_valid(text) {
    match validate(text.as_bytes(), VALID_JSON_MAX_RECURSION_DEPTH) {
        Ok(()) => Some("true".to_owned()),
        Err(_) => Some("false".to_owned()),
    }
});

byond_fn!(fn json_prettify(text) {
    match reformat(text.as_bytes(), false) {
        Ok(s) => Some(s),
        Err(_) => None,
    }
});

byond_fn!(fn json_minify(text) {
    match reformat(text.as_bytes(), true) {
        Ok(s) => Some(s),
        Err(_) => None,
    }
});

byond_fn!(fn json_get(text, path) {
    match get_path(text.as_bytes(), path) {
        Ok(s) => Some(s),
        Err(_) => None,
    }
});

// Lookup tables

pub(crate) const WS_MASK: u64 = (1u64 << 0x20) | (1u64 << 0x09) | (1u64 << 0x0A) | (1u64 << 0x0D);
pub(crate) static PARSE_ESC: [u8; 256] = { let mut t = [0u8; 256];
    t[b'"' as usize]=b'"'; t[b'\\' as usize]=b'\\'; t[b'/' as usize]=b'/';
    t[b'n' as usize]=b'\n'; t[b'r' as usize]=b'\r'; t[b't' as usize]=b'\t';
    t[b'b' as usize]=0x08; t[b'f' as usize]=0x0C; t };
pub(crate) static WRITE_ESC: [u8; 256] = { let mut t = [0u8; 256];
    t[b'"' as usize]=b'"'; t[b'\\' as usize]=b'\\';
    t[b'\n' as usize]=b'n'; t[b'\r' as usize]=b'r'; t[b'\t' as usize]=b't';
    t[0x08]=b'b'; t[0x0C]=b'f'; t };
pub(crate) static VALUE_DISPATCH: [u8; 256] = { let mut t = [0u8; 256];
    t[b'"' as usize]=1; t[b'{' as usize]=2; t[b'[' as usize]=3;
    t[b't' as usize]=4; t[b'f' as usize]=5; t[b'n' as usize]=6; t[b'-' as usize]=7;
    let mut d=b'0'; while d<=b'9' { t[d as usize]=7; d+=1; } t };
pub(crate) static HEX: &[u8; 16] = b"0123456789abcdef";

#[inline(always)] pub(crate) fn is_ws(ch: u8) -> bool { ch <= 0x20 && (WS_MASK >> ch) & 1 != 0 }
#[inline(always)] pub(crate) fn is_digit(ch: u8) -> bool { ch.wrapping_sub(b'0') < 10 }
#[inline(always)] pub(crate) fn hex_val(ch: u8) -> u8 { (ch & 0xF) + (ch >> 6) * 9 }
#[inline(always)] pub(crate) fn needs_escape(ch: u8) -> bool { ch < 0x20 || ch == b'"' || ch == b'\\' }
#[inline(always)] fn has_byte(word: u64, target: u8) -> bool {
    let b = 0x0101_0101_0101_0101u64 * target as u64; let x = word ^ b;
    (x.wrapping_sub(0x0101_0101_0101_0101) & !x & 0x8080_8080_8080_8080) != 0
}

// SIMD string scan

#[inline]
pub(crate) fn find_special(src: &[u8], pos: usize) -> usize {
    let rem = &src[pos..];
    #[cfg(target_arch = "x86")] { if is_x86_feature_detected!("sse2") && rem.len() >= 16 { return unsafe { find_special_sse2(rem) }; } }
    #[cfg(target_arch = "x86_64")] { if is_x86_feature_detected!("sse2") && rem.len() >= 16 { return unsafe { find_special_sse2(rem) }; } }
    find_special_swar(rem)
}

#[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
#[target_feature(enable = "sse2")]
unsafe fn find_special_sse2(data: &[u8]) -> usize {
    #[cfg(target_arch = "x86")] use std::arch::x86::*;
    #[cfg(target_arch = "x86_64")] use std::arch::x86_64::*;
    unsafe {
        let q = _mm_set1_epi8(b'"' as i8); let b = _mm_set1_epi8(b'\\' as i8);
        let lim = _mm_set1_epi8(0x20); let sp = _mm_set1_epi8(0x20);
        let mut i = 0;
        while i + 16 <= data.len() {
            let chunk = _mm_loadu_si128(data.as_ptr().add(i) as *const __m128i);
            let ctrl = _mm_andnot_si128(_mm_cmpeq_epi8(chunk, sp), _mm_cmpeq_epi8(_mm_max_epu8(chunk, lim), lim));
            let mask = _mm_movemask_epi8(_mm_or_si128(_mm_or_si128(_mm_cmpeq_epi8(chunk, q), _mm_cmpeq_epi8(chunk, b)), ctrl)) as u32;
            if mask != 0 { return i + mask.trailing_zeros() as usize; }
            i += 16;
        }
        while i < data.len() { if data[i] == b'"' || data[i] == b'\\' || data[i] < 0x20 { return i; } i += 1; }
        data.len()
    }
}

fn find_special_swar(data: &[u8]) -> usize {
    let mut i = 0;
    while i + 8 <= data.len() {
        let w = u64::from_le_bytes([data[i],data[i+1],data[i+2],data[i+3],data[i+4],data[i+5],data[i+6],data[i+7]]);
        if has_byte(w, b'"') || has_byte(w, b'\\') || (w.wrapping_sub(0x2020202020202020) & !w & 0x8080808080808080) != 0 { break; }
        i += 8;
    }
    while i < data.len() { if data[i] == b'"' || data[i] == b'\\' || data[i] < 0x20 { return i; } i += 1; }
    data.len()
}

// Validator

struct Validator<'a> { pos: usize, src: &'a [u8], max_depth: usize }

impl<'a> Validator<'a> {
    fn new(input: &'a [u8], max_depth: usize) -> Self {
        let pos = if input.starts_with(&[0xEF, 0xBB, 0xBF]) { 3 } else { 0 };
        Self { pos, src: input, max_depth }
    }
    #[inline(always)] fn peek(&self) -> u8 { if self.pos < self.src.len() { self.src[self.pos] } else { 0 } }
    #[inline(always)] fn bump(&mut self) -> u8 { let ch = self.peek(); if self.pos < self.src.len() { self.pos += 1; } ch }
    #[inline(always)] fn eof(&self) -> bool { self.pos >= self.src.len() }
    fn skip_ws(&mut self) { while self.pos < self.src.len() && is_ws(self.src[self.pos]) { self.pos += 1; } }

    fn value(&mut self, depth: usize) -> Result<(), ()> {
        if depth >= self.max_depth { return Err(()); }
        self.skip_ws();
        match VALUE_DISPATCH[self.peek() as usize] {
            1 => self.string(), 2 => self.object(depth), 3 => self.array(depth),
            4 => { self.pos+=4; Ok(()) } 5 => { self.pos+=5; Ok(()) }
            6 => { self.pos+=4; Ok(()) } 7 => self.number(),
            _ => Err(()),
        }
    }
    fn string(&mut self) -> Result<(), ()> {
        if self.bump() != b'"' { return Err(()); }
        loop {
            let skip = find_special(self.src, self.pos);
            self.pos += skip;
            if self.eof() { return Err(()); }
            match self.bump() { b'"' => return Ok(()), b'\\' => { let e = self.bump(); if e == b'u' { self.pos += 4; } } _ => {} }
        }
    }
    fn number(&mut self) -> Result<(), ()> {
        if self.peek()==b'-' { self.bump(); }
        if !is_digit(self.peek()) { return Err(()); }
        while is_digit(self.peek()) { self.bump(); }
        if self.peek()==b'.' { self.bump(); if !is_digit(self.peek()) { return Err(()); } while is_digit(self.peek()) { self.bump(); } }
        if self.peek()|0x20==b'e' { self.bump(); if self.peek()==b'+'||self.peek()==b'-' { self.bump(); } if !is_digit(self.peek()) { return Err(()); } while is_digit(self.peek()) { self.bump(); } }
        Ok(())
    }
    fn array(&mut self, depth: usize) -> Result<(), ()> {
        self.pos+=1; self.skip_ws();
        if self.peek()==b']' { self.pos+=1; return Ok(()); }
        loop { self.value(depth+1)?; self.skip_ws(); match self.peek() { b','=> { self.pos+=1; } b']'=> { self.pos+=1; return Ok(()); } _=> return Err(()) } }
    }
    fn object(&mut self, depth: usize) -> Result<(), ()> {
        self.pos+=1; self.skip_ws();
        if self.peek()==b'}' { self.pos+=1; return Ok(()); }
        loop { self.skip_ws(); self.string()?; self.skip_ws(); if self.bump()!=b':' { return Err(()); }
            self.value(depth+1)?; self.skip_ws(); match self.peek() { b','=> { self.pos+=1; } b'}'=> { self.pos+=1; return Ok(()); } _=> return Err(()) } }
    }
}

pub fn validate(src: &[u8], max_depth: usize) -> Result<(), ()> {
    let mut v = Validator::new(src, max_depth);
    v.value(0)?; v.skip_ws();
    if !v.eof() { return Err(()); }
    Ok(())
}

// Streaming reformatter

use std::io::{self, Write};

struct Stream<'a> { pos: usize, src: &'a [u8] }

impl<'a> Stream<'a> {
    fn new(input: &'a [u8]) -> Self {
        let pos = if input.starts_with(&[0xEF, 0xBB, 0xBF]) { 3 } else { 0 };
        Self { pos, src: input }
    }
    #[inline(always)] fn peek(&self) -> u8 { if self.pos < self.src.len() { self.src[self.pos] } else { 0 } }
    #[inline(always)] fn bump(&mut self) -> u8 { let ch = self.peek(); if self.pos < self.src.len() { self.pos += 1; } ch }
    #[inline(always)] fn eof(&self) -> bool { self.pos >= self.src.len() }
    fn skip_ws(&mut self) { while self.pos < self.src.len() && is_ws(self.src[self.pos]) { self.pos += 1; } }

    fn value(&mut self, out: &mut impl Write, c: bool, d: usize) -> io::Result<()> {
        self.skip_ws();
        match VALUE_DISPATCH[self.peek() as usize] {
            1 => self.string(out), 2 => self.object(out, c, d), 3 => self.array(out, c, d),
            4 => { self.pos+=4; out.write_all(b"true") } 5 => { self.pos+=5; out.write_all(b"false") }
            6 => { self.pos+=4; out.write_all(b"null") } 7 => self.number(out),
            _ => Err(io::Error::new(io::ErrorKind::InvalidData, "unexpected char")),
        }
    }
    fn string(&mut self, out: &mut impl Write) -> io::Result<()> {
        out.write_all(b"\"")?; self.pos += 1;
        loop {
            let skip = find_special(self.src, self.pos);
            if skip > 0 { out.write_all(&self.src[self.pos..self.pos+skip])?; self.pos += skip; }
            if self.eof() { return Err(io::Error::new(io::ErrorKind::InvalidData, "unterminated string")); }
            match self.bump() {
                b'"' => { return out.write_all(b"\""); }
                b'\\' => { let e = self.bump();
                    if e == b'u' { let mut buf = [b'\\', b'u', 0, 0, 0, 0]; for i in 0..4 { buf[2+i] = self.bump(); } out.write_all(&buf)?; }
                    else { out.write_all(&[b'\\', e])?; }
                }
                c => out.write_all(&[c])?,
            }
        }
    }
    fn number(&mut self, out: &mut impl Write) -> io::Result<()> {
        let start = self.pos;
        if self.peek()==b'-' { self.bump(); } while is_digit(self.peek()) { self.bump(); }
        if self.peek()==b'.' { self.bump(); while is_digit(self.peek()) { self.bump(); } }
        if self.peek()|0x20==b'e' { self.bump(); if self.peek()==b'+'||self.peek()==b'-' { self.bump(); } while is_digit(self.peek()) { self.bump(); } }
        out.write_all(&self.src[start..self.pos])
    }
    fn array(&mut self, out: &mut impl Write, c: bool, d: usize) -> io::Result<()> {
        self.pos+=1; self.skip_ws(); out.write_all(b"[")?;
        if self.peek()==b']' { self.pos+=1; return out.write_all(b"]"); }
        let dd=d+1; let mut first=true;
        loop {
            if !first { out.write_all(b",")?; } first=false;
            if !c { out.write_all(b"\n")?; wind(out, dd)?; }
            self.value(out, c, dd)?; self.skip_ws();
            match self.peek() { b','=> { self.pos+=1; } b']'=> { self.pos+=1; if !c { out.write_all(b"\n")?; wind(out,d)?; } return out.write_all(b"]"); } _=> return Err(io::Error::new(io::ErrorKind::InvalidData, "expected ',' or ']'")) }
        }
    }
    fn object(&mut self, out: &mut impl Write, c: bool, d: usize) -> io::Result<()> {
        self.pos+=1; self.skip_ws(); out.write_all(b"{")?;
        if self.peek()==b'}' { self.pos+=1; return out.write_all(b"}"); }
        let dd=d+1; let mut first=true;
        loop {
            if !first { out.write_all(b",")?; } first=false;
            if !c { out.write_all(b"\n")?; wind(out, dd)?; }
            self.skip_ws(); self.string(out)?; self.skip_ws(); self.pos+=1;
            out.write_all(if c { b":" } else { b": " })?;
            self.value(out, c, dd)?; self.skip_ws();
            match self.peek() { b','=> { self.pos+=1; } b'}'=> { self.pos+=1; if !c { out.write_all(b"\n")?; wind(out,d)?; } return out.write_all(b"}"); } _=> return Err(io::Error::new(io::ErrorKind::InvalidData, "expected ',' or '}'")) }
        }
    }
}

fn wind(out: &mut impl Write, level: usize) -> io::Result<()> {
    const SP: &[u8;128] = b"                                                                                                                                ";
    let n = level*2; if n<=SP.len() { out.write_all(&SP[..n]) } else { for _ in 0..level { out.write_all(b"  ")?; } Ok(()) }
}

pub fn reformat(src: &[u8], compact: bool) -> Result<String, &'static str> {
    let mut out = Vec::with_capacity(if compact { src.len() } else { src.len() * 2 });
    let mut p = Stream::new(src);
    p.value(&mut out, compact, 0).map_err(|_| "invalid json")?;
    if !compact { out.push(b'\n'); }
    p.skip_ws();
    if !p.eof() { return Err("trailing data"); }
    String::from_utf8(out).map_err(|_| "invalid utf-8 in output")
}

// Tree parser (json_get only)

use std::collections::BTreeMap;

enum Val { Null, Bool(bool), Num(f64), Str(String), Array(Vec<Val>), Object(BTreeMap<String, Val>) }

struct TreeParser<'a> { pos: usize, src: &'a [u8] }

impl<'a> TreeParser<'a> {
    fn new(input: &'a [u8]) -> Self {
        let pos = if input.starts_with(&[0xEF, 0xBB, 0xBF]) { 3 } else { 0 };
        Self { pos, src: input }
    }
    #[inline(always)] fn peek(&self) -> u8 { if self.pos < self.src.len() { self.src[self.pos] } else { 0 } }
    #[inline(always)] fn bump(&mut self) -> u8 { let ch = self.peek(); if self.pos < self.src.len() { self.pos += 1; } ch }
    fn skip_ws(&mut self) { while self.pos < self.src.len() && is_ws(self.src[self.pos]) { self.pos += 1; } }
    fn expect(&mut self, ch: u8) -> Result<(), ()> { self.skip_ws(); if self.bump() == ch { Ok(()) } else { Err(()) } }

    fn value(&mut self) -> Result<Val, ()> {
        self.skip_ws();
        match VALUE_DISPATCH[self.peek() as usize] {
            1 => self.tstring().map(Val::Str), 2 => self.tobject(), 3 => self.tarray(),
            4 => { self.pos+=4; Ok(Val::Bool(true)) } 5 => { self.pos+=5; Ok(Val::Bool(false)) }
            6 => { self.pos+=4; Ok(Val::Null) } 7 => self.tnumber(),
            _ => Err(()),
        }
    }
    fn tnumber(&mut self) -> Result<Val, ()> {
        let start = self.pos;
        if self.peek()==b'-' { self.bump(); }
        if self.peek()==b'0' { self.bump(); } else { while is_digit(self.peek()) { self.bump(); } }
        if self.peek()==b'.' { self.bump(); while is_digit(self.peek()) { self.bump(); } }
        if self.peek()|0x20==b'e' { self.bump(); if self.peek()==b'+'||self.peek()==b'-' { self.bump(); } while is_digit(self.peek()) { self.bump(); } }
        let s = unsafe { std::str::from_utf8_unchecked(&self.src[start..self.pos]) };
        s.parse().map(Val::Num).map_err(|_| ())
    }
    fn tstring(&mut self) -> Result<String, ()> {
        self.expect(b'"')?; let mut s = String::new(); let start = self.pos;
        let skip = find_special(self.src, self.pos);
        if skip > 0 { self.pos += skip; s.push_str(unsafe { std::str::from_utf8_unchecked(&self.src[start..self.pos]) }); }
        loop {
            if self.pos >= self.src.len() { return Err(()); }
            match self.bump() {
                b'"' => return Ok(s),
                b'\\' => { let e = self.bump();
                    if e == b'u' { let mut v=0u16; for _ in 0..4 { v=(v<<4)|hex_val(self.bump()) as u16; }
                        if (0xD800..=0xDBFF).contains(&v) { if self.bump()!=b'\\'||self.bump()!=b'u' { return Err(()); }
                            let mut lo=0u16; for _ in 0..4 { lo=(lo<<4)|hex_val(self.bump()) as u16; }
                            s.push(char::from_u32(0x10000+((v as u32-0xD800)<<10)+(lo as u32-0xDC00)).unwrap_or('\u{FFFD}'));
                        } else { s.push(char::from_u32(v as u32).unwrap_or('\u{FFFD}')); }
                    } else { let r = PARSE_ESC[e as usize]; if r != 0 { s.push(r as char); } else { return Err(()); } }
                }
                c => s.push(c as char),
            }
        }
    }
    fn tarray(&mut self) -> Result<Val, ()> {
        self.expect(b'[')?; self.skip_ws(); let mut items = Vec::new();
        if self.peek()==b']' { self.bump(); return Ok(Val::Array(items)); }
        loop { items.push(self.value()?); self.skip_ws(); match self.peek() { b','=> { self.bump(); } b']'=> { self.bump(); return Ok(Val::Array(items)); } _=> return Err(()) } }
    }
    fn tobject(&mut self) -> Result<Val, ()> {
        self.expect(b'{')?; self.skip_ws(); let mut map = BTreeMap::new();
        if self.peek()==b'}' { self.bump(); return Ok(Val::Object(map)); }
        loop { self.skip_ws(); let k = self.tstring()?; self.expect(b':')?; map.insert(k, self.value()?); self.skip_ws();
            match self.peek() { b','=> { self.bump(); } b'}'=> { self.bump(); return Ok(Val::Object(map)); } _=> return Err(()) } }
    }
}

pub fn get_path(src: &[u8], path: &str) -> Result<String, ()> {
    let mut p = TreeParser::new(src);
    let root = p.value()?;
    let mut cur = &root;
    let mut remaining = path;
    while !remaining.is_empty() {
        let (key, rest) = match remaining.find('.') { Some(i) => (&remaining[..i], &remaining[i+1..]), None => (remaining, "") };
        remaining = rest;
        cur = match cur {
            Val::Object(m) => m.get(key).ok_or(())?,
            Val::Array(a) => { let mut idx=0usize; for b in key.bytes() { idx=idx*10+(b.wrapping_sub(b'0')) as usize; } a.get(idx).ok_or(())? }
            _ => return Err(()),
        };
    }
    Ok(ser_val(cur))
}

fn ser_val(val: &Val) -> String {
    let mut out = String::new(); ser(val, &mut out, 0); out
}

fn ser(val: &Val, out: &mut String, d: usize) {
    match val {
        Val::Null => out.push_str("null"),
        Val::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
        Val::Num(n) => { if n.is_finite() && *n == (*n as i64) as f64 { out.push_str(&(*n as i64).to_string()); } else { out.push_str(&format!("{}", n)); } }
        Val::Str(s) => { out.push('"'); for ch in s.bytes() {
            if needs_escape(ch) { let e=WRITE_ESC[ch as usize]; if e!=0 { out.push('\\'); out.push(e as char); } else { out.push_str("\\u00"); out.push(HEX[(ch>>4) as usize] as char); out.push(HEX[(ch&0xF) as usize] as char); } }
            else { out.push(ch as char); }
        } out.push('"'); }
        Val::Array(items) => {
            if items.is_empty() { out.push_str("[]"); return; }
            out.push('['); let dd=d+1;
            for (i,item) in items.iter().enumerate() { if i>0 { out.push(','); } out.push('\n'); sindent(out,dd); ser(item,out,dd); }
            out.push('\n'); sindent(out,d); out.push(']');
        }
        Val::Object(map) => {
            if map.is_empty() { out.push_str("{}"); return; }
            out.push('{'); let dd=d+1;
            for (i,(k,v)) in map.iter().enumerate() { if i>0 { out.push(','); } out.push('\n'); sindent(out,dd);
                out.push('"'); out.push_str(k); out.push('"'); out.push_str(": "); ser(v,out,dd); }
            out.push('\n'); sindent(out,d); out.push('}');
        }
    }
}

fn sindent(out: &mut String, level: usize) {
    const SP: &str = "                                                                                                                                ";
    let n=level*2; if n<=SP.len() { out.push_str(&SP[..n]); } else { for _ in 0..level { out.push_str("  "); } }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_valid() {
        assert!(validate(b"[]", 8).is_ok());
        assert!(validate(b"[[]]", 8).is_ok());
        assert!(validate(b"{\"a\":1}", 8).is_ok());
        assert!(validate(b"true", 8).is_ok());
        assert!(validate(b"null", 8).is_ok());
    }

    #[test]
    fn test_invalid() {
        assert!(validate(b"{", 8).is_err());
        assert!(validate(b"", 8).is_err());
    }

    #[test]
    fn test_max_depth() {
        let deep = format!("{}{}", "[".repeat(9), "]".repeat(9));
        assert!(validate(deep.as_bytes(), VALID_JSON_MAX_RECURSION_DEPTH).is_err());
        let ok = format!("{}{}", "[".repeat(7), "]".repeat(7));
        assert!(validate(ok.as_bytes(), VALID_JSON_MAX_RECURSION_DEPTH).is_ok());
    }

    #[test]
    fn test_minify() {
        assert_eq!(reformat(b"{ \"a\" : 1 }", true).unwrap(), "{\"a\":1}");
    }

    #[test]
    fn test_get_path() {
        assert_eq!(get_path(b"{\"a\":{\"b\":42}}", "a.b").unwrap(), "42");
    }

    // --- Edge cases added for optimization coverage ---

    #[test]
    fn test_deeply_nested_object() {
        let json = b"{\"a\":{\"b\":{\"c\":{\"d\":{\"e\":\"deep\"}}}}}";
        assert!(validate(json, 8).is_ok());
        assert_eq!(get_path(json, "a.b.c.d.e").unwrap(), "\"deep\"");
    }

    #[test]
    fn test_unicode_strings() {
        let json = b"{\"emoji\":\"\\u0048\\u0065\\u006C\\u006C\\u006F\"}";
        assert!(validate(json, 8).is_ok());
        let result = get_path(json, "emoji").unwrap();
        assert_eq!(result, "\"Hello\"");
    }

    #[test]
    fn test_empty_containers() {
        assert!(validate(b"[]", 8).is_ok());
        assert!(validate(b"{}", 8).is_ok());
        assert!(validate(b"[[],[]]", 8).is_ok());
        assert!(validate(b"{\"a\":{}}", 8).is_ok());
    }

    #[test]
    fn test_large_numbers() {
        assert!(validate(b"1e308", 8).is_ok());
        assert!(validate(b"-1e308", 8).is_ok());
        assert!(validate(b"9999999999999999", 8).is_ok());
        assert!(validate(b"0.000000001", 8).is_ok());
    }

    #[test]
    fn test_prettify_roundtrip() {
        let compact = b"{\"a\":1,\"b\":[2,3]}";
        let pretty = reformat(compact, false).unwrap();
        let re_compact = reformat(pretty.as_bytes(), true).unwrap();
        assert_eq!(re_compact, "{\"a\":1,\"b\":[2,3]}");
    }

    #[test]
    fn test_get_path_array_index() {
        assert_eq!(get_path(b"[10,20,30]", "1").unwrap(), "20");
        assert_eq!(get_path(b"{\"a\":[\"x\",\"y\"]}", "a.0").unwrap(), "\"x\"");
    }

    #[test]
    fn test_get_path_missing_key() {
        assert!(get_path(b"{\"a\":1}", "b").is_err());
        assert!(get_path(b"{\"a\":1}", "a.b").is_err());
    }

    #[test]
    fn test_get_path_array_out_of_bounds() {
        assert!(get_path(b"[1,2]", "5").is_err());
    }

    #[test]
    fn test_invalid_json_variants() {
        assert!(validate(b"{\"a\":}", 8).is_err());
        assert!(validate(b"[,]", 8).is_err());
        assert!(validate(b"{\"a\"}", 8).is_err());
        assert!(validate(b"\"unterminated", 8).is_err());
        assert!(validate(b"truee", 8).is_err());  // trailing data
    }

    #[test]
    fn test_minify_whitespace_heavy() {
        let input = b"  {  \"a\"  :  [  1  ,  2  ]  }  ";
        assert_eq!(reformat(input, true).unwrap(), "{\"a\":[1,2]}");
    }

    #[test]
    fn test_bom_handling() {
        let input = b"\xEF\xBB\xBF{\"key\":\"value\"}";
        assert!(validate(input, 8).is_ok());
        assert_eq!(reformat(input, true).unwrap(), "{\"key\":\"value\"}");
    }

    #[test]
    fn test_escaped_strings() {
        let json = b"{\"s\":\"line1\\nline2\\ttab\\\\backslash\"}";
        assert!(validate(json, 8).is_ok());
        let minified = reformat(json, true).unwrap();
        assert_eq!(minified, "{\"s\":\"line1\\nline2\\ttab\\\\backslash\"}");
    }

    #[test]
    fn test_number_edge_cases() {
        assert!(validate(b"-0", 8).is_ok());
        assert!(validate(b"1.5e+10", 8).is_ok());
        assert!(validate(b"1.5E-10", 8).is_ok());
        assert!(validate(b"0.0", 8).is_ok());
    }
}
