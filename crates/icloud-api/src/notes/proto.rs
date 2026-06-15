//! Protobuf codec for Apple Notes NoteStoreProto format.
//!
//! Wire structure: NoteStoreProto { document(2): Document { version(2), note(3): Note {
//!   note_text(2), attribute_run(5)[]: AttributeRun { length(1), paragraph_style(2),
//!   font(3), font_weight(5), underlined(6), strikethrough(7), link(9),
//!   attachment_info(12) } } } }

use base64::Engine;
use flate2::write::ZlibEncoder;
use flate2::Compression;
use std::io::Write;

use crate::error::{Error, Result};
use crate::title_doc::{
    decompress, field_bytes, field_varint, position, proto_get_all_bytes, proto_get_bytes,
    proto_get_string, proto_get_varint,
};

use super::models::*;

const B64: base64::engine::GeneralPurpose = base64::engine::general_purpose::STANDARD;

// ── Decode ────────────────────────────────────────────────

/// Decode a base64-encoded, gzip-compressed NoteStoreProto into a NoteDocument.
pub fn decode_note_body(b64: &str) -> Result<NoteDocument> {
    if b64.is_empty() {
        return Ok(NoteDocument {
            text: String::new(),
            runs: vec![],
        });
    }
    let raw = B64
        .decode(b64)
        .map_err(|e| Error::Notes(format!("base64: {e}")))?;
    let decompressed = decompress(&raw);
    decode_proto(&decompressed)
}

/// Decode raw protobuf bytes into a NoteDocument.
pub fn decode_proto(buf: &[u8]) -> Result<NoteDocument> {
    // NoteStoreProto.document (field 2)
    let document = proto_get_bytes(buf, 2)
        .ok_or_else(|| Error::Notes("missing NoteStoreProto.document".into()))?;
    // Document.note (field 3)
    let note =
        proto_get_bytes(document, 3).ok_or_else(|| Error::Notes("missing Document.note".into()))?;
    // Note.note_text (field 2)
    let text = proto_get_string(note, 2).unwrap_or_default();
    // Note.attribute_run (field 5, repeated)
    let run_bufs = proto_get_all_bytes(note, 5);
    let mut runs = Vec::with_capacity(run_bufs.len());
    for rb in run_bufs {
        runs.push(decode_attribute_run(rb));
    }
    Ok(NoteDocument { text, runs })
}

/// Dump all protobuf fields present in a raw attribute run buffer (for debugging).
fn dump_raw_fields(buf: &[u8]) -> Vec<(u32, u8, String)> {
    let mut fields = Vec::new();
    let mut pos = 0;
    while pos < buf.len() {
        let Some((tag, n)) = crate::title_doc::decode_varint(&buf[pos..]) else {
            break;
        };
        pos += n;
        let wire_type = (tag & 0x07) as u8;
        let field_num = (tag >> 3) as u32;
        match wire_type {
            0 => {
                let Some((val, n)) = crate::title_doc::decode_varint(&buf[pos..]) else {
                    break;
                };
                pos += n;
                fields.push((field_num, wire_type, format!("varint={val}")));
            }
            2 => {
                let Some((len, n)) = crate::title_doc::decode_varint(&buf[pos..]) else {
                    break;
                };
                pos += n;
                let len = len as usize;
                if pos + len > buf.len() {
                    break;
                }
                let data = &buf[pos..pos + len];
                let repr = if let Ok(s) = std::str::from_utf8(data) {
                    format!("str={s:?}")
                } else {
                    format!("bytes[{len}]={data:02x?}")
                };
                fields.push((field_num, wire_type, repr));
                pos += len;
            }
            1 => {
                pos += 8;
                fields.push((field_num, wire_type, "fixed64".into()));
            }
            5 => {
                pos += 4;
                fields.push((field_num, wire_type, "fixed32".into()));
            }
            _ => break,
        }
    }
    fields
}

/// Decode a NoteDocument and print a diagnostic dump of all attribute runs.
pub fn debug_dump(b64: &str) -> Result<String> {
    let doc = decode_note_body(b64)?;
    let mut out = String::new();
    let mut char_offset = 0;
    for (i, run) in doc.runs.iter().enumerate() {
        let end = char_offset + run.length;
        let text_slice: String = doc
            .text
            .chars()
            .skip(char_offset)
            .take(run.length)
            .collect();
        let preview: String = text_slice.chars().take(50).collect();
        let preview = preview.replace('\n', "\\n");
        out.push_str(&format!(
            "Run {i}: chars[{char_offset}..{end}] len={} style={:?} bold={} italic={} ul={} strike={} link={:?}\n  text: {:?}\n",
            run.length,
            run.style.style_type,
            run.font.bold,
            run.font.italic,
            run.underlined,
            run.strikethrough,
            run.link,
            preview,
        ));
        char_offset = end;
    }
    out.push_str(&format!(
        "\nTotal chars in text: {}\n",
        doc.text.chars().count()
    ));
    out.push_str(&format!("Total chars in runs: {char_offset}\n"));

    // Dump raw fields for ALL runs, plus paragraph_style sub-fields
    let raw = B64
        .decode(b64)
        .map_err(|e| crate::error::Error::Notes(format!("base64: {e}")))?;
    let decompressed = decompress(&raw);
    if let Some(document) = proto_get_bytes(&decompressed, 2) {
        if let Some(note) = proto_get_bytes(document, 3) {
            let run_bufs = proto_get_all_bytes(note, 5);
            out.push_str("\n=== Raw protobuf fields (last 10 runs) ===\n");
            let start = run_bufs.len().saturating_sub(10);
            for (i, rb) in run_bufs.iter().enumerate().skip(start) {
                let fields = dump_raw_fields(rb);
                out.push_str(&format!("Run {i} raw: {fields:?}\n"));
                // Also dump paragraph_style sub-fields if present
                if let Some(ps) = proto_get_bytes(rb, 2) {
                    let ps_fields = dump_raw_fields(ps);
                    out.push_str(&format!("  paragraph_style: {ps_fields:?}\n"));
                }
            }
        }
    }

    Ok(out)
}

fn decode_attribute_run(buf: &[u8]) -> AttributeRun {
    let length = proto_get_varint(buf, 1).unwrap_or(0) as usize;
    let style = proto_get_bytes(buf, 2)
        .map(decode_paragraph_style)
        .unwrap_or_default();
    let font_weight = FontWeight::from(proto_get_varint(buf, 5).unwrap_or(0) as i64);
    let underlined = proto_get_varint(buf, 6).unwrap_or(0) != 0;
    let strikethrough = proto_get_varint(buf, 7).unwrap_or(0) != 0;
    let link = proto_get_string(buf, 9);
    let attachment = proto_get_bytes(buf, 12).map(decode_attachment_info);

    AttributeRun {
        length,
        style,
        font: font_weight,
        underlined,
        strikethrough,
        link,
        attachment,
    }
}

fn decode_paragraph_style(buf: &[u8]) -> ParagraphStyle {
    // style_type is field 1, but it defaults to -1 when absent.
    // Protobuf varint can't represent -1 natively, but Apple encodes it
    // as the unsigned varint 0xFFFFFFFFFFFFFFFF (zigzag) or simply omits it.
    let style_type = match proto_get_varint(buf, 1) {
        Some(v) if v > i64::MAX as u64 => StyleType::Body, // -1 as unsigned
        Some(v) => StyleType::from(v as i64),
        None => StyleType::Body,
    };
    let indent = proto_get_varint(buf, 4).unwrap_or(0) as i32;
    let checklist = proto_get_bytes(buf, 5).map(|cb| {
        let done = proto_get_varint(cb, 2).unwrap_or(0) != 0;
        ChecklistInfo { done }
    });
    let block_quote = proto_get_varint(buf, 8).unwrap_or(0) != 0;

    ParagraphStyle {
        style_type,
        indent,
        checklist,
        block_quote,
    }
}

fn decode_attachment_info(buf: &[u8]) -> AttachmentInfo {
    AttachmentInfo {
        identifier: proto_get_string(buf, 1).unwrap_or_default(),
        type_uti: proto_get_string(buf, 2),
    }
}

// ── Encode ────────────────────────────────────────────────

/// Encode a NoteDocument into a base64-encoded, zlib-compressed NoteStoreProto.
pub fn encode_note_body(doc: &NoteDocument) -> Result<String> {
    let proto_bytes = encode_proto(doc);
    encode_compressed(&proto_bytes)
}

/// Encode an updated Note body while reusing the existing TopoText replica UUID
/// when it can be recovered from the current server body.
pub fn encode_note_body_for_update(existing_b64: &str, doc: &NoteDocument) -> Result<String> {
    let doc_uuid = extract_primary_replica_uuid(existing_b64).unwrap_or_else(uuid::Uuid::new_v4);
    let proto_bytes = encode_proto_with_uuid(doc, *doc_uuid.as_bytes());
    encode_compressed(&proto_bytes)
}

fn encode_compressed(proto_bytes: &[u8]) -> Result<String> {
    let mut zl = ZlibEncoder::new(Vec::new(), Compression::default());
    zl.write_all(proto_bytes)?;
    let compressed = zl.finish()?;
    Ok(B64.encode(&compressed))
}

fn extract_primary_replica_uuid(b64: &str) -> Option<uuid::Uuid> {
    if b64.is_empty() {
        return None;
    }
    let raw = B64.decode(b64).ok()?;
    let decompressed = decompress(&raw);
    let document = proto_get_bytes(&decompressed, 2)?;
    let note = proto_get_bytes(document, 3)?;
    let metadata = proto_get_bytes(note, 4)?;
    let uuid_entry = proto_get_bytes(metadata, 1)?;
    let uuid_bytes = proto_get_bytes(uuid_entry, 1)?;
    uuid::Uuid::from_slice(uuid_bytes).ok()
}

fn encode_proto(doc: &NoteDocument) -> Vec<u8> {
    encode_proto_with_uuid(doc, *uuid::Uuid::new_v4().as_bytes())
}

fn encode_proto_with_uuid(doc: &NoteDocument, doc_uuid: [u8; 16]) -> Vec<u8> {
    let char_len = doc.text.chars().count() as u64;

    // CRDT operations (required for Apple Notes to render the body)
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

    // CRDT metadata (UUID + clock vector)
    let clock_payload = field_varint(1, char_len);
    let replica_payload = field_varint(1, 1);
    let uuid_entry = {
        let mut u = field_bytes(1, &doc_uuid);
        u.extend_from_slice(&field_bytes(2, &clock_payload));
        u.extend_from_slice(&field_bytes(2, &replica_payload));
        u
    };
    let metadata = field_bytes(4, &field_bytes(1, &uuid_entry));

    // Build Note message
    let mut note = Vec::new();
    note.extend_from_slice(&field_bytes(2, doc.text.as_bytes()));
    note.extend_from_slice(&field_bytes(3, &op1));
    note.extend_from_slice(&field_bytes(3, &op2));
    note.extend_from_slice(&field_bytes(3, &op3));
    note.extend_from_slice(&metadata);
    for run in &doc.runs {
        let run_bytes = encode_attribute_run(run);
        note.extend_from_slice(&field_bytes(5, &run_bytes));
    }

    // Build Document message
    let mut document = Vec::new();
    document.extend_from_slice(&field_varint(1, 0));
    document.extend_from_slice(&field_varint(2, 0));
    document.extend_from_slice(&field_bytes(3, &note));

    // Build NoteStoreProto
    let mut outer = Vec::new();
    outer.extend_from_slice(&field_varint(1, 0));
    outer.extend_from_slice(&field_bytes(2, &document));
    outer
}

fn encode_attribute_run(run: &AttributeRun) -> Vec<u8> {
    let mut buf = Vec::new();
    buf.extend_from_slice(&field_varint(1, run.length as u64));

    // Always emit paragraph_style — Apple Notes requires it on every run.
    let style_bytes = encode_paragraph_style(&run.style);
    buf.extend_from_slice(&field_bytes(2, &style_bytes));

    let fw: i64 = run.font.into();
    if fw != 0 {
        buf.extend_from_slice(&field_varint(5, fw as u64));
    }
    if run.underlined {
        buf.extend_from_slice(&field_varint(6, 1));
    }
    if run.strikethrough {
        buf.extend_from_slice(&field_varint(7, 1));
    }
    if let Some(ref url) = run.link {
        buf.extend_from_slice(&field_bytes(9, url.as_bytes()));
    }
    if let Some(ref att) = run.attachment {
        let att_bytes = encode_attachment_info(att);
        buf.extend_from_slice(&field_bytes(12, &att_bytes));
    }
    buf
}

fn encode_attachment_info(att: &AttachmentInfo) -> Vec<u8> {
    let mut buf = field_bytes(1, att.identifier.as_bytes());
    if let Some(ref uti) = att.type_uti {
        buf.extend_from_slice(&field_bytes(2, uti.as_bytes()));
    }
    buf
}

fn encode_paragraph_style(style: &ParagraphStyle) -> Vec<u8> {
    let mut buf = Vec::new();
    let st: i64 = style.style_type.into();
    // Apple Notes web uses style_type=3 for body text, not -1/absent.
    // Always emit style_type so Apple Notes can parse the document.
    let wire_st = if st == -1 { 3 } else { st };
    buf.extend_from_slice(&field_varint(1, wire_st as u64));
    // Apple Notes web always sets alignment=4 (natural/default).
    buf.extend_from_slice(&field_varint(2, 4));
    if style.indent > 0 {
        buf.extend_from_slice(&field_varint(4, style.indent as u64));
    }
    if let Some(ref cl) = style.checklist {
        let mut cb = Vec::new();
        cb.extend_from_slice(&field_bytes(1, uuid::Uuid::new_v4().as_bytes()));
        if cl.done {
            cb.extend_from_slice(&field_varint(2, 1));
        }
        buf.extend_from_slice(&field_bytes(5, &cb));
    }
    if style.block_quote {
        buf.extend_from_slice(&field_varint(8, 1));
    }
    buf
}

// ── Helpers for simple field extraction from CloudKit JSON ─

/// Decode a base64 BYTES field to a UTF-8 string (for TitleEncrypted, SnippetEncrypted).
pub fn decode_b64_text(b64: &str) -> String {
    if b64.is_empty() {
        return String::new();
    }
    let Ok(raw) = B64.decode(b64) else {
        return String::new();
    };
    let decompressed = decompress(&raw);
    // Try as protobuf note body first (extract text field)
    if let Some(document) = proto_get_bytes(&decompressed, 2) {
        if let Some(note) = proto_get_bytes(document, 3) {
            if let Some(text) = proto_get_string(note, 2) {
                let trimmed = text.trim();
                if !trimmed.is_empty() {
                    return trimmed.to_string();
                }
            }
        }
    }
    // Fall back to plain UTF-8
    String::from_utf8(decompressed)
        .map(|s| s.trim().to_string())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_body_roundtrip() {
        let doc = NoteDocument {
            text: String::new(),
            runs: vec![],
        };
        let encoded = encode_note_body(&doc).unwrap();
        let decoded = decode_note_body(&encoded).unwrap();
        assert_eq!(decoded.text, "");
        assert!(decoded.runs.is_empty());
    }

    #[test]
    fn simple_title_body_roundtrip() {
        let doc = NoteDocument {
            text: "Hello\nWorld\n".to_string(),
            runs: vec![
                AttributeRun {
                    length: 6,
                    style: ParagraphStyle {
                        style_type: StyleType::Title,
                        ..Default::default()
                    },
                    ..Default::default()
                },
                AttributeRun {
                    length: 6,
                    style: ParagraphStyle::default(),
                    ..Default::default()
                },
            ],
        };
        let encoded = encode_note_body(&doc).unwrap();
        let decoded = decode_note_body(&encoded).unwrap();
        assert_eq!(decoded.text, "Hello\nWorld\n");
        assert_eq!(decoded.runs.len(), 2);
        assert_eq!(decoded.runs[0].length, 6);
        assert_eq!(decoded.runs[0].style.style_type, StyleType::Title);
        assert_eq!(decoded.runs[1].length, 6);
        assert_eq!(decoded.runs[1].style.style_type, StyleType::Body);
    }

    #[test]
    fn compare_with_web_app_format() {
        // The web app's working protobuf for "TEst\t\nHello world\n"
        let web_b64 = "eJzjYBDqZORgEGCQamIUEgpxLS7h5PJIzcnJVyjPL8pJ4ZIS4GIBSQMVgGkNRrAII1BESApMazBJiXFxAOX+AwE/UB2crSTDJcUl4FPoNIf9lBpDp01OfLuzio0QE4cQEDNqcXCwCYHMZAGyeIAsZgEWAA0AGKQ=";
        let web_raw = base64::Engine::decode(&B64, web_b64).unwrap();
        let web_proto = decompress(&web_raw);

        // Our encoding of the same text
        let doc = NoteDocument {
            text: "TEst\t\nHello world\n".to_string(),
            runs: vec![
                AttributeRun {
                    length: 6,
                    style: ParagraphStyle {
                        style_type: StyleType::Title,
                        ..Default::default()
                    },
                    ..Default::default()
                },
                AttributeRun {
                    length: 12,
                    style: ParagraphStyle::default(),
                    ..Default::default()
                },
            ],
        };
        let our_proto = encode_proto(&doc);

        eprintln!(
            "WEB ({} bytes): {}",
            web_proto.len(),
            hex::encode(&web_proto)
        );
        eprintln!(
            "OUR ({} bytes): {}",
            our_proto.len(),
            hex::encode(&our_proto)
        );

        // Both should decode back to the same text
        let web_doc = decode_proto(&web_proto).unwrap();
        let our_doc = decode_proto(&our_proto).unwrap();
        assert_eq!(web_doc.text, our_doc.text);
        assert_eq!(web_doc.text, "TEst\t\nHello world\n");
    }

    #[test]
    fn rich_text_roundtrip() {
        let doc = NoteDocument {
            text: "Bold\n".to_string(),
            runs: vec![AttributeRun {
                length: 5,
                font: FontWeight {
                    bold: true,
                    italic: false,
                },
                strikethrough: true,
                ..Default::default()
            }],
        };
        let encoded = encode_note_body(&doc).unwrap();
        let decoded = decode_note_body(&encoded).unwrap();
        assert!(decoded.runs[0].font.bold);
        assert!(!decoded.runs[0].font.italic);
        assert!(decoded.runs[0].strikethrough);
    }

    #[test]
    fn update_body_reuses_existing_replica_uuid() {
        let original = NoteDocument {
            text: "Title\nOriginal body\n".to_string(),
            runs: vec![
                AttributeRun {
                    length: 6,
                    style: ParagraphStyle {
                        style_type: StyleType::Title,
                        ..Default::default()
                    },
                    ..Default::default()
                },
                AttributeRun {
                    length: 14,
                    style: ParagraphStyle::default(),
                    ..Default::default()
                },
            ],
        };
        let updated = NoteDocument {
            text: "Title\nUpdated body\n".to_string(),
            runs: vec![
                AttributeRun {
                    length: 6,
                    style: ParagraphStyle {
                        style_type: StyleType::Title,
                        ..Default::default()
                    },
                    ..Default::default()
                },
                AttributeRun {
                    length: 13,
                    style: ParagraphStyle::default(),
                    ..Default::default()
                },
            ],
        };

        let original_b64 = encode_note_body(&original).unwrap();
        let original_uuid = extract_primary_replica_uuid(&original_b64).unwrap();

        let updated_b64 = encode_note_body_for_update(&original_b64, &updated).unwrap();
        let updated_uuid = extract_primary_replica_uuid(&updated_b64).unwrap();
        let decoded = decode_note_body(&updated_b64).unwrap();

        assert_eq!(updated_uuid, original_uuid);
        assert_eq!(decoded.text, updated.text);
        assert_eq!(decoded.runs.len(), updated.runs.len());
    }
}
