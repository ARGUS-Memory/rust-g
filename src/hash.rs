use crate::argus_json::escape_json_string;
use crate::error::{Error, Result};
// Fixed seed for xxh64 — deterministic per-build. Change if you need a different distribution.
const XXHASH_SEED: u64 = 0xA1B2_C3D4_E5F6_0718;
use hmac::{Hmac, Mac};
use md5::Md5;
use rand::{Rng, RngExt, distr::Alphanumeric};
use rand_chacha::{ChaCha20Rng, rand_core::SeedableRng};
use sha1::Sha1;
use sha2::{Digest, Sha256, Sha512};
use std::{
    cell::RefCell,
    convert::TryInto,
    fs::File,
    hash::Hasher,
    io::Read,
    time::{SystemTime, UNIX_EPOCH},
};
use twox_hash::XxHash64;

/// Encode bytes as lowercase hex string. Replaces the `hex` crate.
fn hex_encode(bytes: impl AsRef<[u8]>) -> String {
    let bytes = bytes.as_ref();
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut s = String::with_capacity(bytes.len() * 2);
    for &b in bytes {
        s.push(HEX[(b >> 4) as usize] as char);
        s.push(HEX[(b & 0xf) as usize] as char);
    }
    s
}

/// Encode bytes as standard base64 (A-Z, a-z, 0-9, +, /) with = padding.
fn base64_encode(data: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity((data.len() + 2) / 3 * 4);
    let chunks = data.chunks_exact(3);
    let remainder = chunks.remainder();
    for chunk in chunks {
        let n = (chunk[0] as u32) << 16 | (chunk[1] as u32) << 8 | (chunk[2] as u32);
        out.push(ALPHABET[((n >> 18) & 0x3F) as usize] as char);
        out.push(ALPHABET[((n >> 12) & 0x3F) as usize] as char);
        out.push(ALPHABET[((n >> 6) & 0x3F) as usize] as char);
        out.push(ALPHABET[(n & 0x3F) as usize] as char);
    }
    match remainder.len() {
        1 => {
            let n = (remainder[0] as u32) << 16;
            out.push(ALPHABET[((n >> 18) & 0x3F) as usize] as char);
            out.push(ALPHABET[((n >> 12) & 0x3F) as usize] as char);
            out.push('=');
            out.push('=');
        }
        2 => {
            let n = (remainder[0] as u32) << 16 | (remainder[1] as u32) << 8;
            out.push(ALPHABET[((n >> 18) & 0x3F) as usize] as char);
            out.push(ALPHABET[((n >> 12) & 0x3F) as usize] as char);
            out.push(ALPHABET[((n >> 6) & 0x3F) as usize] as char);
            out.push('=');
        }
        _ => {}
    }
    out
}

/// Decode a standard base64 string. Returns None on invalid input.
fn base64_decode(s: &str) -> Option<Vec<u8>> {
    #[inline]
    fn decode_char(c: u8) -> Option<u8> {
        match c {
            b'A'..=b'Z' => Some(c - b'A'),
            b'a'..=b'z' => Some(c - b'a' + 26),
            b'0'..=b'9' => Some(c - b'0' + 52),
            b'+' => Some(62),
            b'/' => Some(63),
            _ => None,
        }
    }

    let input = s.as_bytes();
    if input.is_empty() {
        return Some(Vec::new());
    }
    // Standard base64 requires total length to be a multiple of 4
    if input.len() % 4 != 0 {
        return None;
    }
    // Count and validate padding (0, 1, or 2 trailing '=' chars only)
    let pad_count = input.iter().rev().take_while(|&&b| b == b'=').count();
    if pad_count > 2 {
        return None;
    }
    let data_end = input.len() - pad_count;
    // Validate all non-padding chars are valid base64
    if input[..data_end].iter().any(|&b| decode_char(b).is_none()) {
        return None;
    }

    let mut out = Vec::with_capacity(data_end * 3 / 4);
    let mut buf: u32 = 0;
    let mut bits: u32 = 0;

    for &b in &input[..data_end] {
        let val = decode_char(b)?;
        buf = (buf << 6) | val as u32;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((buf >> bits) as u8);
            buf &= (1 << bits) - 1;
        }
    }
    Some(out)
}

/// Encode bytes as RFC 4648 base32 (A-Z, 2-7). Optionally pads with '='.
fn base32_encode(data: &[u8], pad: bool) -> String {
    const ALPHA: &[u8; 32] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ234567";
    let mut out = String::with_capacity((data.len() + 4) / 5 * 8);
    for chunk in data.chunks(5) {
        let mut buf = [0u8; 5];
        buf[..chunk.len()].copy_from_slice(chunk);
        let n = chunk.len();
        let emit = match n {
            1 => 2,
            2 => 4,
            3 => 5,
            4 => 7,
            _ => 8,
        };
        let bits: u64 = (buf[0] as u64) << 32
            | (buf[1] as u64) << 24
            | (buf[2] as u64) << 16
            | (buf[3] as u64) << 8
            | (buf[4] as u64);
        for i in 0..emit {
            let idx = ((bits >> (35 - i * 5)) & 0x1F) as usize;
            out.push(ALPHA[idx] as char);
        }
        if pad {
            let pad_count = 8 - emit;
            for _ in 0..pad_count {
                out.push('=');
            }
        }
    }
    out
}

/// Decode RFC 4648 base32. Accepts both padded and unpadded input (case-insensitive).
/// Returns None on invalid input.
fn base32_decode(s: &str) -> Option<Vec<u8>> {
    let s = s.trim_end_matches('=');
    if s.is_empty() {
        return Some(Vec::new());
    }
    let mut out = Vec::with_capacity(s.len() * 5 / 8);
    for chunk in s.as_bytes().chunks(8) {
        let mut buf = [0u8; 8];
        let n = chunk.len();
        for i in 0..n {
            let b = chunk[i];
            buf[i] = match b {
                b'A'..=b'Z' => b - b'A',
                b'a'..=b'z' => b - b'a',
                b'2'..=b'7' => b - b'2' + 26,
                _ => return None,
            };
        }
        let bits: u64 = (buf[0] as u64) << 35
            | (buf[1] as u64) << 30
            | (buf[2] as u64) << 25
            | (buf[3] as u64) << 20
            | (buf[4] as u64) << 15
            | (buf[5] as u64) << 10
            | (buf[6] as u64) << 5
            | (buf[7] as u64);
        let byte_count = match n {
            2 => 1,
            4 => 2,
            5 => 3,
            7 => 4,
            8 => 5,
            _ => return None,
        };
        for i in 0..byte_count {
            out.push((bits >> (32 - i * 8)) as u8);
        }
    }
    Some(out)
}

/// Decode hex string into a byte slice. Returns Err if input length is wrong or chars are invalid.
pub(crate) fn hex_decode_to_slice(hex: &str, out: &mut [u8]) -> std::result::Result<(), String> {
    if hex.len() != out.len() * 2 {
        return Err(format!("invalid hex length: expected {}, got {}", out.len() * 2, hex.len()));
    }
    let bytes = hex.as_bytes();
    for i in 0..out.len() {
        let hi = hex_nibble(bytes[i * 2]).ok_or_else(|| format!("invalid hex char at {}", i * 2))?;
        let lo = hex_nibble(bytes[i * 2 + 1]).ok_or_else(|| format!("invalid hex char at {}", i * 2 + 1))?;
        out[i] = (hi << 4) | lo;
    }
    Ok(())
}

#[inline]
fn hex_nibble(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

const TOTP_DIGITS: usize = 6;
const TOTP_STEP_SECONDS: u64 = 30;
const DIGITS_POWER: [u32; 9] = [
    1, 10, 100, 1000, 10000, 100000, 1000000, 10000000, 100000000,
];

byond_fn!(fn hash_string(algorithm, string) {
    string_hash(algorithm, string).ok()
});

byond_fn!(fn decode_base64(string) {
    base64_decode(string)
});

byond_fn!(fn decode_base32(string, _padding) {
    // padding param kept for BYOND API compat; decoder accepts both padded and unpadded
    base32_decode(string)
});

byond_fn!(fn hash_file(algorithm, string) {
    file_hash(algorithm, string).ok()
});

byond_fn!(fn generate_totp(algorithm, base32_seed) {
    match totp_generate(algorithm, base32_seed, 0, TOTP_DIGITS, None) {
        Ok(value) => Some(value),
        Err(error) => Some(format!("ERROR: {error:?}"))
    }
});

byond_fn!(fn csprng_chacha20(format, n_bytes) {
    let n_bytes: usize = match n_bytes.parse() {
        Ok(n) => n,
        Err(_) => return Some(String::from("ERROR: Unparseable n_bytes"))
    };
    if n_bytes < 1 {
        return Some(String::from("ERROR: Zero bytes not allowed"))
    }
    Some(gen_csprng_chacha20(format, n_bytes))
});

fn gen_csprng_chacha20(format: &str, n_bytes: usize) -> String {
    // Seed is generated by underlying OS random source (Linux: syscall to getrandom, /dev/urandom, Win: ProcessPrng)
    // The seed is presumably cryptographically secure as it is provided by hardware source.
    // This makes the RNG output non-deterministic and suitable for use in cryptography.
    let mut rng: ChaCha20Rng = rand::make_rng();
    format_rng(&mut rng, format, n_bytes)
}

byond_fn!(fn prng_chacha20_seeded(format, n_bytes, seed) {
    let n_bytes: usize = match n_bytes.parse() {
        Ok(n) => n,
        Err(_) => return Some(String::from("ERROR: Unparseable n_bytes"))
    };
    if n_bytes < 1 {
        return Some(String::from("ERROR: Zero bytes not allowed"))
    }
    Some(gen_prng_chacha20_seeded(format, n_bytes, seed))
});

fn gen_prng_chacha20_seeded(format: &str, n_bytes: usize, seed: &str) -> String {
    // SHA256 hash the seed and provide the raw SHA256 output bytes to the hasher.
    // This normalizes the seed's distribution of 0 and 1s, making it produce higher quality randomness.
    // It also normalizes the length of the seed to 32 bytes.
    let mut seed_hasher = Sha256::new();
    seed_hasher.update(seed.as_bytes());
    let hashed_seed: [u8; 32] = seed_hasher.finalize().into();
    let mut rng = ChaCha20Rng::from_seed(hashed_seed);
    format_rng(&mut rng, format, n_bytes)
}

fn format_rng<T: Rng>(rng: &mut T, format: &str, n_bytes: usize) -> String {
    match format {
        "alphanumeric" => (0..n_bytes)
            .map(|_| rng.sample(Alphanumeric) as char)
            .collect::<String>(),
        "hex" => {
            let mut bytes = vec![0u8; n_bytes];
            rng.fill_bytes(&mut bytes);
            hex_encode(bytes)
        }
        "base32_rfc4648" => {
            let mut bytes = vec![0u8; n_bytes];
            rng.fill_bytes(&mut bytes);
            base32_encode(&bytes, false)
        }
        "base32_rfc4648_pad" => {
            let mut bytes = vec![0u8; n_bytes];
            rng.fill_bytes(&mut bytes);
            base32_encode(&bytes, true)
        }
        "base64" => {
            let mut bytes = vec![0u8; n_bytes];
            rng.fill_bytes(&mut bytes);
            base64_encode(&bytes)
        }
        _ => String::from("ERROR: Invalid format"),
    }
}

byond_fn!(fn generate_totp_tolerance(algorithm, base32_seed, tolerance) {
    let tolerance_value: i32 = match tolerance.parse() {
        Ok(value) => value,
        Err(_) => return Some(String::from("ERROR: Tolerance not a valid integer"))
    };
    match totp_generate_tolerance(algorithm, base32_seed, tolerance_value, TOTP_DIGITS, None) {
        Ok(value) => Some(value),
        Err(error) => Some(format!("ERROR: {error:?}"))
    }
});

pub fn string_hash(algorithm: &str, string: &str) -> Result<String> {
    let mut hasher = HashDispatcher::new(algorithm)?;
    hasher.update(string);
    Ok(hasher.finish())
}

const BUFFER_SIZE: usize = 65536;
// don't allocate another buffer every time we hash a file, just reuse the same buffer.
thread_local!( static FILE_HASH_BUFFER: RefCell<[u8; BUFFER_SIZE]> = const { RefCell::new([0; BUFFER_SIZE]) } );

pub fn file_hash(algorithm: &str, path: &str) -> Result<String> {
    let mut hasher = HashDispatcher::new(algorithm)?;
    let mut file = File::open(path)?;

    FILE_HASH_BUFFER.with_borrow_mut(|buffer| {
        loop {
            let bytes_read = file.read(buffer)?;
            if bytes_read == 0 {
                break;
            }
            hasher.update(&buffer[..bytes_read]);
        }
        Ok(hasher.finish())
    })
}

/// Generates multiple TOTP codes from base32_seed, with time step +-tolerance
/// time_override is used as the current unix time instead of the current system time for testing
fn totp_generate_tolerance(
    algorithm: &str,
    base32_seed: &str,
    tolerance: i32,
    digits: usize,
    time_override: Option<u64>,
) -> Result<String> {
    let mut results: Vec<String> = Vec::new();
    for i in -tolerance..(tolerance + 1) {
        let result = totp_generate(algorithm, base32_seed, i.into(), digits, time_override)?;
        results.push(result)
    }
    // Serialize Vec<String> as JSON array: ["a","b","c"]
    let mut out = String::with_capacity(results.len() * 12);
    out.push('[');
    for (i, s) in results.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        out.push('"');
        escape_json_string(s, &mut out);
        out.push('"');
    }
    out.push(']');
    Ok(out)
}

fn hmac<D>(seed: &[u8], data: &[u8]) -> Result<Vec<u8>>
where
    D: Mac + hmac::digest::KeyInit,
{
    let mut mac = <D as Mac>::new_from_slice(seed).map_err(|_| Error::BadSeed)?;
    mac.update(data);
    Ok(mac.finalize().into_bytes().to_vec())
}

/// Generates a single TOTP code from base32_seed offset by offset time steps
/// base32_seed should be at least 16 input bytes, 128 bits, but is recommended to be 20 input bytes, 160 bits
/// Please use a proper hardware-seeded CSPRNG to seed the TOTP and store it in a secure location.
/// Maximum usable length of seed is 104 characters (64 bytes, 512 bits)
/// time_override is used as the current unix time instead of the current system time for testing
/// TOTP algorithm described https://blogs.unimelb.edu.au/sciencecommunication/2021/09/30/totp/
fn totp_generate(
    algorithm: &str,
    base32_seed: &str,
    offset: i64,
    digits: usize,
    time_override: Option<u64>,
) -> Result<String> {
    let mut seed: [u8; 64] = [0; 64];

    if !(1..=8).contains(&digits) {
        return Err(Error::BadDigits);
    }

    // Always accept padding
    match base32_decode(base32_seed) {
        Some(base32_bytes) => {
            if base32_bytes.len() < 10 || base32_bytes.len() > 64 {
                return Err(Error::BadSeed);
            }
            seed[..base32_bytes.len()].copy_from_slice(&base32_bytes);
        }
        None => return Err(Error::BadSeed),
    }
    // Will panic if the date is not between Jan 1 1970 and the year ~200 billion
    let curr_time: u64 = time_override.unwrap_or_else(|| {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("SystemTime is before Unix Epoc")
            .as_secs()
    }) / TOTP_STEP_SECONDS;
    let time: u64 = curr_time.saturating_add_signed(offset);
    let time_bytes: [u8; 8] = time.to_be_bytes();

    let hmac_bytes: Vec<u8> = match algorithm {
        "sha1" => hmac::<Hmac<Sha1>>(&seed, &time_bytes)?,
        "sha256" => hmac::<Hmac<Sha256>>(&seed, &time_bytes)?,
        "sha512" => hmac::<Hmac<Sha512>>(&seed, &time_bytes)?,
        _ => return Err(Error::InvalidAlgorithm),
    };

    let totp_byte_offset: usize = (*hmac_bytes.last().ok_or(Error::InvalidAlgorithm)? & 0x0F) as usize;
    let totp_bytes: [u8; 4] = hmac_bytes
        .get(totp_byte_offset..totp_byte_offset + 4)
        .ok_or(Error::InvalidAlgorithm)?
        .try_into()
        .map_err(|_| Error::InvalidAlgorithm)?;
    let totp_untruncated: u32 = u32::from_be_bytes(totp_bytes);
    let totp_sized_code: u32 = (totp_untruncated & 0x7FFFFFFF) % DIGITS_POWER[digits];
    // Pad the digits in constant time to reduce effectiveness of timing attacks
    let totp_code_str = (10u64.pow(digits as u32)
        + (totp_sized_code as u64 % 10u64.pow(digits as u32)))
    .to_string();
    let totp_code_resized = &totp_code_str.as_bytes()[totp_code_str.len() - digits..];
    // we know that the UTF-8 is valid as it just came from a UTF-8 string.
    // it will only be digits which do not include any multi-byte UTF-8 characters
    unsafe { Ok(String::from_utf8_unchecked(totp_code_resized.to_vec())) }
}

enum HashDispatcher {
    Md5(Md5),
    Sha1(Sha1),
    Sha256(Sha256),
    Sha512(Sha512),
    Xxh64(XxHash64),
    Base32(Vec<u8>),
    Base32Pad(Vec<u8>),
    Base64(Vec<u8>),
}

impl HashDispatcher {
    fn new(name: &str) -> Result<Self> {
        match name {
            "md5" => Ok(Self::Md5(Md5::new())),
            "sha1" => Ok(Self::Sha1(Sha1::new())),
            "sha256" => Ok(Self::Sha256(Sha256::new())),
            "sha512" => Ok(Self::Sha512(Sha512::new())),
            "xxh64" => Ok(Self::Xxh64(XxHash64::with_seed(XXHASH_SEED))),
            "xxh64_fixed" => Ok(Self::Xxh64(XxHash64::with_seed(17479268743136991876))), // this seed is just a random number that should stay the same between builds and runs
            "base32_rfc4648" => Ok(Self::Base32(Vec::new())),
            "base32_rfc4648_pad" => Ok(Self::Base32Pad(Vec::new())),
            "base64" => Ok(Self::Base64(Vec::new())),
            _ => Err(Error::InvalidAlgorithm),
        }
    }

    fn update(&mut self, data: impl AsRef<[u8]>) {
        let data = data.as_ref();
        match self {
            HashDispatcher::Md5(hasher) => hasher.update(data),
            HashDispatcher::Sha1(hasher) => hasher.update(data),
            HashDispatcher::Sha256(hasher) => hasher.update(data),
            HashDispatcher::Sha512(hasher) => hasher.update(data),
            HashDispatcher::Xxh64(hasher) => hasher.write(data),
            HashDispatcher::Base32(buffer) => buffer.extend_from_slice(data),
            HashDispatcher::Base32Pad(buffer) => buffer.extend_from_slice(data),
            HashDispatcher::Base64(buffer) => buffer.extend_from_slice(data),
        }
    }

    fn finish(self) -> String {
        match self {
            HashDispatcher::Md5(hasher) => hex_encode(hasher.finalize()),
            HashDispatcher::Sha1(hasher) => hex_encode(hasher.finalize()),
            HashDispatcher::Sha256(hasher) => hex_encode(hasher.finalize()),
            HashDispatcher::Sha512(hasher) => hex_encode(hasher.finalize()),
            HashDispatcher::Xxh64(hasher) => format!("{:x}", hasher.finish()),
            HashDispatcher::Base32(buffer) => base32_encode(&buffer, false),
            HashDispatcher::Base32Pad(buffer) => base32_encode(&buffer, true),
            HashDispatcher::Base64(buffer) => base64_encode(&buffer),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn totp_generate_test() {
        // https://datatracker.ietf.org/doc/html/rfc6238#autoid-18 Test Vectors
        // See: https://www.rfc-editor.org/errata/eid2866 for seed discrepancy
        const TOTP_TEST_TIMES: [u64; 6] = [
            59,
            1111111109,
            1111111111,
            1234567890,
            2000000000,
            20000000000,
        ];
        const TOTP_TEST_ALGORITHMS: [&str; 3] = ["sha1", "sha256", "sha512"];
        const TOTP_TEST_SEEDS: [&str; 3] = [
            "12345678901234567890",
            "12345678901234567890123456789012",
            "1234567890123456789012345678901234567890123456789012345678901234",
        ];
        const TOTP_TEST_VALUES_TIME_ALGO: [[&str; 3]; 6] = [
            ["94287082", "46119246", "90693936"],
            ["07081804", "68084774", "25091201"],
            ["14050471", "67062674", "99943326"],
            ["89005924", "91819424", "93441116"],
            ["69279037", "90698825", "38618901"],
            ["65353130", "77737706", "47863826"],
        ];
        TOTP_TEST_TIMES
            .iter()
            .enumerate()
            .for_each(|(time_idx, time)| {
                TOTP_TEST_ALGORITHMS
                    .iter()
                    .zip(TOTP_TEST_SEEDS)
                    .enumerate()
                    .for_each(|(algo_idx, (algo, seed))| {
                        let totp = totp_generate(
                            algo,
                            &base32_encode(seed.as_bytes(), false), // test it unpadded
                            0,
                            8,
                            Some(*time),
                        );
                        assert_eq!(
                            totp.unwrap(),
                            TOTP_TEST_VALUES_TIME_ALGO[time_idx][algo_idx]
                        );
                    })
            });

        // The big offset is so that it always uses the same time, allowing for verification that the algorithm is correct
        // Seed, time, and result for zero offset taken from https://blogs.unimelb.edu.au/sciencecommunication/2021/09/30/totp/
        let result = totp_generate(
            "sha1",
            "XE7ZREYZTLXYK444",
            0,
            6,
            Some(54424722u64 * TOTP_STEP_SECONDS + (TOTP_STEP_SECONDS - 1)),
        );
        assert_eq!(result.unwrap(), "417714");
        let result2 = totp_generate(
            "sha1",
            "XE7ZREYZTLXYK444",
            -1,
            6,
            Some(54424722u64 * TOTP_STEP_SECONDS + (TOTP_STEP_SECONDS - 1)),
        );
        assert_eq!(result2.unwrap(), "358747");
        let result3 = totp_generate(
            "sha1",
            "XE7ZREYZTLXYK444",
            1,
            6,
            Some(54424722u64 * TOTP_STEP_SECONDS + (TOTP_STEP_SECONDS - 1)),
        );
        assert_eq!(result3.unwrap(), "539257");
        let result4 = totp_generate(
            "sha1",
            "XE7ZREYZTLXYK444",
            2,
            6,
            Some(54424722u64 * TOTP_STEP_SECONDS + (TOTP_STEP_SECONDS - 1)),
        );
        assert_eq!(result4.unwrap(), "679828");

        let json_result = totp_generate_tolerance(
            "sha1",
            "XE7ZREYZTLXYK444",
            1,
            6,
            Some(54424722u64 * TOTP_STEP_SECONDS + (TOTP_STEP_SECONDS - 1)),
        );
        assert_eq!(json_result.unwrap(), "[\"358747\",\"417714\",\"539257\"]");
        let err_result = totp_generate_tolerance("sha1", "66", 0, 6, None);
        assert!(err_result.is_err());
        let err_result = totp_generate_tolerance("sha1", "XE7ZREYZTLXYK444", 0, 10, None);
        assert!(err_result.is_err());
        let err_result = totp_generate_tolerance("invalid", "XE7ZREYZTLXYK444", 0, 6, None);
        assert!(err_result.is_err());
    }

    // --- Additional hash tests for optimization coverage ---

    #[test]
    fn test_string_hash_algorithms() {
        // Known MD5 of empty string
        assert_eq!(string_hash("md5", "").unwrap(), "d41d8cd98f00b204e9800998ecf8427e");
        // Known SHA1 of empty string
        assert_eq!(string_hash("sha1", "").unwrap(), "da39a3ee5e6b4b0d3255bfef95601890afd80709");
        // Known SHA256 of empty string
        assert_eq!(string_hash("sha256", "").unwrap(), "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855");
    }

    #[test]
    fn test_string_hash_sha512() {
        let result = string_hash("sha512", "test").unwrap();
        assert_eq!(result.len(), 128); // SHA512 produces 64 bytes = 128 hex chars
    }

    #[test]
    fn test_string_hash_invalid_algorithm() {
        assert!(string_hash("invalid", "test").is_err());
    }

    #[test]
    fn test_string_hash_xxh64_fixed_deterministic() {
        let h1 = string_hash("xxh64_fixed", "hello").unwrap();
        let h2 = string_hash("xxh64_fixed", "hello").unwrap();
        assert_eq!(h1, h2);
        // Different input should produce different hash
        let h3 = string_hash("xxh64_fixed", "world").unwrap();
        assert_ne!(h1, h3);
    }

    #[test]
    fn test_string_hash_base32() {
        let result = string_hash("base32_rfc4648", "hello").unwrap();
        assert!(!result.is_empty());
        // base32 should only contain A-Z and 2-7
        assert!(result.chars().all(|c| c.is_ascii_uppercase() || ('2'..='7').contains(&c)));
    }

    #[test]
    fn test_string_hash_base32_pad() {
        let result = string_hash("base32_rfc4648_pad", "hi").unwrap();
        assert!(!result.is_empty());
        // Padded base32 may contain '=' padding
        assert!(result.chars().all(|c| c.is_ascii_uppercase() || ('2'..='7').contains(&c) || c == '='));
    }

    #[test]
    fn test_string_hash_base64() {
        let result = string_hash("base64", "hello").unwrap();
        assert_eq!(result, "aGVsbG8=");
    }

    #[test]
    fn test_hash_dispatcher_md5_known() {
        let result = string_hash("md5", "The quick brown fox jumps over the lazy dog").unwrap();
        assert_eq!(result, "9e107d9d372bb6826bd81d3542a419d6");
    }

    #[test]
    fn test_gen_prng_chacha20_seeded_deterministic() {
        let r1 = gen_prng_chacha20_seeded("hex", 16, "seed123");
        let r2 = gen_prng_chacha20_seeded("hex", 16, "seed123");
        assert_eq!(r1, r2);
        // Different seed should produce different output
        let r3 = gen_prng_chacha20_seeded("hex", 16, "seed456");
        assert_ne!(r1, r3);
    }

    #[test]
    fn test_format_rng_alphanumeric() {
        let result = gen_prng_chacha20_seeded("alphanumeric", 32, "test_seed");
        assert_eq!(result.len(), 32);
        assert!(result.chars().all(|c| c.is_ascii_alphanumeric()));
    }

    #[test]
    fn test_format_rng_hex() {
        let result = gen_prng_chacha20_seeded("hex", 16, "test_seed");
        assert_eq!(result.len(), 32); // 16 bytes = 32 hex chars
        assert!(result.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn test_format_rng_invalid() {
        let result = gen_prng_chacha20_seeded("invalid_format", 8, "seed");
        assert_eq!(result, "ERROR: Invalid format");
    }

    #[test]
    fn test_totp_zero_digits() {
        let result = totp_generate("sha1", "XE7ZREYZTLXYK444", 0, 0, Some(1000));
        assert!(result.is_err());
    }

    #[test]
    fn test_totp_nine_digits() {
        let result = totp_generate("sha1", "XE7ZREYZTLXYK444", 0, 9, Some(1000));
        assert!(result.is_err());
    }

    #[test]
    fn test_totp_seed_too_short() {
        // Less than 10 bytes decoded
        let short_seed = base32_encode(b"12345678", false);
        let result = totp_generate("sha1", &short_seed, 0, 6, Some(1000));
        assert!(result.is_err());
    }

    #[test]
    fn test_totp_seed_too_long() {
        // More than 64 bytes decoded
        let long_data = vec![0x42u8; 65];
        let long_seed = base32_encode(&long_data, false);
        let result = totp_generate("sha1", &long_seed, 0, 6, Some(1000));
        assert!(result.is_err());
    }

    #[test]
    fn test_totp_invalid_base32() {
        let result = totp_generate("sha1", "!!!invalid!!!", 0, 6, Some(1000));
        assert!(result.is_err());
    }

    #[test]
    fn test_totp_sha256_and_sha512() {
        // Just verify they produce valid 6-digit codes
        let _seed = "JBSWY3DPEHPK3PXP"; // base32 of "Hello!" - too short (5 bytes)
        // Use a longer one
        let good_seed = base32_encode(b"12345678901234567890", false);
        let r256 = totp_generate("sha256", &good_seed, 0, 6, Some(1000)).unwrap();
        assert_eq!(r256.len(), 6);
        assert!(r256.chars().all(|c| c.is_ascii_digit()));

        let r512 = totp_generate("sha512", &good_seed, 0, 6, Some(1000)).unwrap();
        assert_eq!(r512.len(), 6);
        assert!(r512.chars().all(|c| c.is_ascii_digit()));
    }
}
