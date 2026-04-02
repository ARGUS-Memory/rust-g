use crate::error::Result;
use std::borrow::Cow;

byond_fn!(fn url_encode(data) {
    Some(encode(data))
});

byond_fn!(fn url_decode(data) {
    decode(data).ok()
});

/// URL-encode a string using application/x-www-form-urlencoded rules.
/// Unreserved chars (RFC 3986) pass through; everything else is percent-encoded.
fn encode(string: &str) -> String {
    let mut out = String::with_capacity(string.len());
    for &b in string.as_bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char);
            }
            b' ' => out.push('+'),
            _ => {
                const HEX: &[u8; 16] = b"0123456789ABCDEF";
                out.push('%');
                out.push(HEX[(b >> 4) as usize] as char);
                out.push(HEX[(b & 0xf) as usize] as char);
            }
        }
    }
    out
}

/// URL-decode a percent-encoded string. Replaces '+' with ' '.
fn decode(string: &str) -> Result<String> {
    let replaced = replace_plus(string.as_bytes());
    let decoded = percent_decode(&replaced);
    Ok(String::from_utf8_lossy(&decoded).into_owned())
}

/// Percent-decode a byte slice: %XX sequences become the byte value.
fn percent_decode(input: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(input.len());
    let mut i = 0;
    while i < input.len() {
        if input[i] == b'%' && i + 2 < input.len() {
            if let (Some(hi), Some(lo)) = (hex_val(input[i + 1]), hex_val(input[i + 2])) {
                out.push((hi << 4) | lo);
                i += 3;
                continue;
            }
        }
        out.push(input[i]);
        i += 1;
    }
    out
}

#[inline]
fn hex_val(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

/// Replace b'+' with b' '
fn replace_plus<'a>(input: &'a [u8]) -> Cow<'a, [u8]> {
    match input.iter().position(|&b| b == b'+') {
        None => Cow::Borrowed(input),
        Some(first_position) => {
            let mut replaced = input.to_owned();
            replaced[first_position] = b' ';
            for byte in &mut replaced[first_position + 1..] {
                if *byte == b'+' {
                    *byte = b' ';
                }
            }
            Cow::Owned(replaced)
        }
    }
}
