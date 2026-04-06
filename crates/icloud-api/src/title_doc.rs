//! CRDT TitleDocument encoding for Reminders (protobuf wire, gzip, base64).
//! Ported from [icloud-reminders-cli/internal/utils](https://github.com/tarekbecker/icloud-reminders-cli).

use flate2::write::GzEncoder;
use flate2::Compression;
use std::io::Write;

pub fn encode_varint(mut v: u64) -> Vec<u8> {
    let mut buf = Vec::new();
    while v > 127 {
        buf.push((v as u8 & 0x7f) | 0x80);
        v >>= 7;
    }
    buf.push(v as u8);
    buf
}

pub fn field_varint(num: u32, v: u64) -> Vec<u8> {
    let tag = (num as u64) << 3; // wire type 0
    let mut o = encode_varint(tag);
    o.extend_from_slice(&encode_varint(v));
    o
}

pub fn field_bytes(num: u32, data: &[u8]) -> Vec<u8> {
    let tag = ((num as u64) << 3) | 2; // wire type 2
    let mut o = encode_varint(tag);
    o.extend_from_slice(&encode_varint(data.len() as u64));
    o.extend_from_slice(data);
    o
}

pub(crate) fn position(replica: u64, offset: i64) -> Vec<u8> {
    let mut out = field_varint(1, replica);
    if offset == -1 {
        let tag = (2 << 3) as u8;
        let mut v = vec![tag];
        v.extend_from_slice(&encode_varint(0xffffffff));
        out.extend_from_slice(&v);
    } else {
        out.extend_from_slice(&field_varint(2, offset as u64));
    }
    out
}

/// Encode plain title text as Apple's gzipped+base64 TitleDocument payload.
pub fn encode_title(title: &str) -> crate::Result<String> {
    let title_bytes = title.as_bytes();
    let char_len = title.chars().count() as u64;
    let doc_uuid = *uuid::Uuid::new_v4().as_bytes();

    let op1 = {
        let mut x = field_bytes(1, &position(0, 0));
        x.extend_from_slice(&field_varint(2, 0));
        x.extend_from_slice(&field_bytes(3, &position(0, 0)));
        x.extend_from_slice(&field_varint(5, 1));
        x
    };
    let op2 = {
        let mut x = field_bytes(1, &position(1, 0));
        x.extend_from_slice(&field_varint(2, char_len));
        x.extend_from_slice(&field_bytes(3, &position(1, 0)));
        x.extend_from_slice(&field_varint(5, 2));
        x
    };
    let op3 = {
        let mut x = field_bytes(1, &position(0, -1));
        x.extend_from_slice(&field_varint(2, 0));
        x.extend_from_slice(&field_bytes(3, &position(0, -1)));
        x
    };

    let clock_payload = field_varint(1, char_len);
    let replica_payload = field_varint(1, 1);
    let uuid_entry = {
        let mut u = field_bytes(1, &doc_uuid);
        u.extend_from_slice(&field_bytes(2, &clock_payload));
        u.extend_from_slice(&field_bytes(2, &replica_payload));
        u
    };
    let metadata = field_bytes(4, &uuid_entry);
    let attr_run = field_varint(1, char_len);

    let mut note = Vec::new();
    note.extend_from_slice(&field_bytes(2, title_bytes));
    note.extend_from_slice(&field_bytes(3, &op1));
    note.extend_from_slice(&field_bytes(3, &op2));
    note.extend_from_slice(&field_bytes(3, &op3));
    note.extend_from_slice(&field_bytes(4, &metadata));
    note.extend_from_slice(&field_bytes(5, &attr_run));

    let mut document = Vec::new();
    document.extend_from_slice(&field_varint(1, 0));
    document.extend_from_slice(&field_varint(2, 0));
    document.extend_from_slice(&field_bytes(3, &note));

    let mut outer = Vec::new();
    outer.extend_from_slice(&field_varint(1, 0));
    outer.extend_from_slice(&field_bytes(2, &document));

    let mut gz = GzEncoder::new(Vec::new(), Compression::default());
    gz.write_all(&outer)?;
    let compressed = gz.finish()?;
    Ok(base64::Engine::encode(
        &base64::engine::general_purpose::STANDARD,
        &compressed,
    ))
}

/// Extract title from a TitleDocument (base64 → gzip → protobuf wire format).
/// Structure: outer.field2(document).field3(note).field2(title_bytes).
pub fn extract_title(td_b64: &str) -> String {
    if td_b64.is_empty() {
        return String::new();
    }
    let Ok(raw) = base64::Engine::decode(&base64::engine::general_purpose::STANDARD, td_b64) else {
        return String::new();
    };
    let decompressed = decompress(&raw);

    // Navigate protobuf: outer.field2 → document.field3 → note.field2 → title bytes
    if let Some(document) = proto_get_bytes(&decompressed, 2) {
        if let Some(note) = proto_get_bytes(document, 3) {
            if let Some(title_bytes) = proto_get_bytes(note, 2) {
                if let Ok(s) = std::str::from_utf8(title_bytes) {
                    let s = s.trim();
                    if !s.is_empty() {
                        return s.to_string();
                    }
                }
            }
        }
    }
    String::new()
}

/// Read a varint from protobuf wire format, returns (value, bytes_consumed).
pub fn decode_varint(buf: &[u8]) -> Option<(u64, usize)> {
    let mut val = 0u64;
    for (i, &b) in buf.iter().enumerate() {
        if i >= 10 {
            return None;
        }
        val |= ((b & 0x7f) as u64) << (7 * i);
        if b & 0x80 == 0 {
            return Some((val, i + 1));
        }
    }
    None
}

/// Extract the first occurrence of a length-delimited field (wire type 2) by field number.
pub fn proto_get_bytes(buf: &[u8], field_num: u32) -> Option<&[u8]> {
    let mut pos = 0;
    while pos < buf.len() {
        let (tag, n) = decode_varint(&buf[pos..])?;
        pos += n;
        let wire_type = (tag & 0x07) as u8;
        let num = (tag >> 3) as u32;
        match wire_type {
            0 => {
                // varint — skip
                let (_, n) = decode_varint(&buf[pos..])?;
                pos += n;
            }
            2 => {
                // length-delimited
                let (len, n) = decode_varint(&buf[pos..])?;
                pos += n;
                let len = len as usize;
                if pos + len > buf.len() {
                    return None;
                }
                if num == field_num {
                    return Some(&buf[pos..pos + len]);
                }
                pos += len;
            }
            1 => pos += 8, // 64-bit fixed
            5 => pos += 4, // 32-bit fixed
            _ => return None,
        }
    }
    None
}

/// Extract ALL occurrences of a length-delimited field (wire type 2) by field number.
pub fn proto_get_all_bytes(buf: &[u8], field_num: u32) -> Vec<&[u8]> {
    let mut results = Vec::new();
    let mut pos = 0;
    while pos < buf.len() {
        let Some((tag, n)) = decode_varint(&buf[pos..]) else {
            break;
        };
        pos += n;
        let wire_type = (tag & 0x07) as u8;
        let num = (tag >> 3) as u32;
        match wire_type {
            0 => {
                let Some((_, n)) = decode_varint(&buf[pos..]) else {
                    break;
                };
                pos += n;
            }
            2 => {
                let Some((len, n)) = decode_varint(&buf[pos..]) else {
                    break;
                };
                pos += n;
                let len = len as usize;
                if pos + len > buf.len() {
                    break;
                }
                if num == field_num {
                    results.push(&buf[pos..pos + len]);
                }
                pos += len;
            }
            1 => pos += 8,
            5 => pos += 4,
            _ => break,
        }
    }
    results
}

/// Extract the first varint field by field number.
pub fn proto_get_varint(buf: &[u8], field_num: u32) -> Option<u64> {
    let mut pos = 0;
    while pos < buf.len() {
        let (tag, n) = decode_varint(&buf[pos..])?;
        pos += n;
        let wire_type = (tag & 0x07) as u8;
        let num = (tag >> 3) as u32;
        match wire_type {
            0 => {
                let (val, n) = decode_varint(&buf[pos..])?;
                pos += n;
                if num == field_num {
                    return Some(val);
                }
            }
            2 => {
                let (len, n) = decode_varint(&buf[pos..])?;
                pos += n;
                pos += len as usize;
            }
            1 => pos += 8,
            5 => pos += 4,
            _ => return None,
        }
    }
    None
}

/// Extract the first string (length-delimited) field by field number.
pub fn proto_get_string(buf: &[u8], field_num: u32) -> Option<String> {
    let bytes = proto_get_bytes(buf, field_num)?;
    std::str::from_utf8(bytes).ok().map(|s| s.to_string())
}

/// Decompress gzip/zlib data, or return as-is if not compressed.
pub fn decompress(raw: &[u8]) -> Vec<u8> {
    if raw.len() >= 2 && raw[0] == 0x1f && raw[1] == 0x8b {
        let mut d = flate2::read::GzDecoder::new(std::io::Cursor::new(raw));
        let mut buf = Vec::new();
        if std::io::Read::read_to_end(&mut d, &mut buf).is_ok() {
            return buf;
        }
    } else if raw.len() >= 2 && raw[0] == 0x78 && matches!(raw[1], 0x01 | 0x5e | 0x9c | 0xda) {
        let mut d = flate2::read::ZlibDecoder::new(std::io::Cursor::new(raw));
        let mut buf = Vec::new();
        if std::io::Read::read_to_end(&mut d, &mut buf).is_ok() {
            return buf;
        }
    }
    raw.to_vec()
}

/// Format a CloudKit timestamp (millis) as date or datetime.
/// Shows time component only if it's not midnight UTC.
pub fn ts_to_str(ts_ms: i64) -> Option<String> {
    if ts_ms == 0 {
        return None;
    }
    let dt = chrono::DateTime::from_timestamp_millis(ts_ms)?;
    let local = dt.with_timezone(&chrono::Local);
    if local.time() == chrono::NaiveTime::from_hms_opt(0, 0, 0).unwrap() {
        Some(local.format("%Y-%m-%d").to_string())
    } else {
        Some(local.format("%Y-%m-%d %H:%M").to_string())
    }
}

pub fn str_to_ts(date: &str) -> Result<i64, chrono::ParseError> {
    let d = chrono::NaiveDate::parse_from_str(date, "%Y-%m-%d")?;
    let dt = d.and_hms_opt(0, 0, 0).unwrap().and_utc();
    Ok(dt.timestamp_millis())
}

/// Generate a new CloudKit record name in `Reminder/UUID` format.
pub fn new_record_name() -> String {
    format!(
        "Reminder/{}",
        uuid::Uuid::new_v4()
            .as_hyphenated()
            .to_string()
            .to_uppercase()
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_title_payload_contains_text() {
        let enc = encode_title("hello").expect("encode");
        let dec = extract_title(&enc);
        assert!(dec.contains("hello"));
    }
}
