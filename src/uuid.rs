use rand::Rng;

byond_fn!(
    fn uuid_v4() {
        Some(gen_uuid_v4())
    }
);

byond_fn!(
    fn uuid_v7() {
        Some(gen_uuid_v7())
    }
);

byond_fn!(
    fn cuid2() {
        Some(cuid2::create_id())
    }
);

byond_fn!(
    fn cuid2_len(length) {
        let length = length.parse::<u16>().ok()?;
        Some(
            cuid2::CuidConstructor::new()
                .with_length(length)
                .create_id()
        )
    }
);

/// Generate a UUID v4 (random). RFC 9562 section 5.4.
fn gen_uuid_v4() -> String {
    let mut bytes = [0u8; 16];
    rand::rng().fill_bytes(&mut bytes);
    // Set version 4 (bits 48-51)
    bytes[6] = (bytes[6] & 0x0F) | 0x40;
    // Set variant 1 (bits 64-65)
    bytes[8] = (bytes[8] & 0x3F) | 0x80;
    format_uuid(&bytes)
}

/// Generate a UUID v7 (Unix timestamp + random). RFC 9562 section 5.7.
fn gen_uuid_v7() -> String {
    let ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64;

    let mut bytes = [0u8; 16];
    rand::rng().fill_bytes(&mut bytes);

    // First 48 bits: Unix timestamp in milliseconds (big-endian)
    bytes[0] = (ms >> 40) as u8;
    bytes[1] = (ms >> 32) as u8;
    bytes[2] = (ms >> 24) as u8;
    bytes[3] = (ms >> 16) as u8;
    bytes[4] = (ms >> 8) as u8;
    bytes[5] = ms as u8;
    // Set version 7 (bits 48-51)
    bytes[6] = (bytes[6] & 0x0F) | 0x70;
    // Set variant 1 (bits 64-65)
    bytes[8] = (bytes[8] & 0x3F) | 0x80;
    format_uuid(&bytes)
}

/// Format 16 bytes as a UUID string: xxxxxxxx-xxxx-xxxx-xxxx-xxxxxxxxxxxx
fn format_uuid(bytes: &[u8; 16]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut s = String::with_capacity(36);
    for (i, &b) in bytes.iter().enumerate() {
        if i == 4 || i == 6 || i == 8 || i == 10 {
            s.push('-');
        }
        s.push(HEX[(b >> 4) as usize] as char);
        s.push(HEX[(b & 0xf) as usize] as char);
    }
    s
}
