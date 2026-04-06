//! Apple Notes table CRDT (MergeableData) codec.
//!
//! Tables in Apple Notes are stored as separate Attachment records with UTI
//! `com.apple.notes.table`. The table content lives in the `MergeableDataEncrypted`
//! field as a zlib-compressed protobuf using Apple's CRDT object model.
//!
//! Wire structure (simplified):
//! ```text
//! MergeableData {
//!   version(1): 0,
//!   content(2) {
//!     version(1): 0, format(2): 0,
//!     operations(3, repeated) {
//!       // Object registry entries (field 13)
//!       // Cell containers per column (field 16) — each has rows with cell docs
//!       // Row/column CRDT lists (field 6)
//!       // Cell content (field 10) — text + attribute runs (NoteDocument-like)
//!     }
//!     key_names(4, repeated): ["identity", "crTableColumnDirection", ...]
//!     type_names(5, repeated): ["com.apple.CRDT.NSNumber", ...]
//!     uuids(6, repeated): [16-byte UUIDs]
//!     version_vector(7) { ... }
//!   }
//! }
//! ```

use base64::Engine;
use flate2::write::ZlibEncoder;
use flate2::Compression;
use std::io::Write;

use crate::error::{Error, Result};
use crate::title_doc::{
    decompress, field_bytes, field_varint, position, proto_get_all_bytes, proto_get_bytes,
    proto_get_string,
};

const B64: base64::engine::GeneralPurpose = base64::engine::general_purpose::STANDARD;

/// Table data: a grid of cell text strings.
#[derive(Debug, Clone)]
pub struct TableData {
    pub cells: Vec<Vec<String>>, // cells[row][col]
}

impl TableData {
    pub fn rows(&self) -> usize {
        self.cells.len()
    }

    pub fn cols(&self) -> usize {
        self.cells.first().map_or(0, |r| r.len())
    }
}

// ── Decode ────────────────────────────────────────────────

/// Decode a base64-encoded, zlib-compressed MergeableData into a TableData.
pub fn decode_table(b64: &str) -> Result<TableData> {
    if b64.is_empty() {
        return Ok(TableData { cells: vec![] });
    }
    let raw = B64
        .decode(b64)
        .map_err(|e| Error::Notes(format!("table base64: {e}")))?;
    let decompressed = decompress(&raw);
    decode_mergeable_data(&decompressed)
}

/// Recursively collect all field 10 (cell content) and field 16 (column container)
/// entries from nested field 3 (operation) structures.
fn collect_table_fields(
    buf: &[u8],
    cell_texts: &mut Vec<String>,
    num_cols: &mut usize,
    rows_per_col: &mut Vec<usize>,
    depth: usize,
) {
    if depth > 10 {
        return;
    }

    // field 16 = cell column container
    for col_container in proto_get_all_bytes(buf, 16) {
        *num_cols += 1;
        // Count row entries within the column container
        if let Some(row_structure) = proto_get_bytes(col_container, 2) {
            let row_entries = proto_get_all_bytes(row_structure, 1);
            rows_per_col.push(row_entries.len());
        }
    }

    // field 10 = cell content (NoteDocument-like)
    for cell_doc in proto_get_all_bytes(buf, 10) {
        // field 2 = cell text
        let text = proto_get_string(cell_doc, 2).unwrap_or_default();
        cell_texts.push(text);
    }

    // Recurse into field 3 (nested operations)
    for nested_op in proto_get_all_bytes(buf, 3) {
        collect_table_fields(nested_op, cell_texts, num_cols, rows_per_col, depth + 1);
    }
}

/// Decode raw MergeableData protobuf bytes.
fn decode_mergeable_data(buf: &[u8]) -> Result<TableData> {
    // MergeableData.content (field 2)
    let content = proto_get_bytes(buf, 2)
        .ok_or_else(|| Error::Notes("missing MergeableData.content".into()))?;

    // Recursively search for cell content and column containers
    let mut cell_texts: Vec<String> = Vec::new();
    let mut num_cols = 0usize;
    let mut rows_per_col: Vec<usize> = Vec::new();

    // Start from the top-level operations (field 3 in content)
    for op in proto_get_all_bytes(content, 3) {
        collect_table_fields(op, &mut cell_texts, &mut num_cols, &mut rows_per_col, 0);
    }

    // Determine grid shape
    let num_rows = rows_per_col.first().copied().unwrap_or(1).max(1);
    let num_cols = num_cols.max(1);

    // Cell texts appear in column-major order (cells interleaved with column containers).
    // If we have fewer cells than grid size, pad with empty strings.
    let expected = num_rows * num_cols;
    while cell_texts.len() < expected {
        cell_texts.push(String::new());
    }

    // Build the row-major grid from column-major cell order
    let mut grid = vec![vec![String::new(); num_cols]; num_rows];
    for col in 0..num_cols {
        for row in 0..num_rows {
            let idx = col * num_rows + row;
            if idx < cell_texts.len() {
                grid[row][col] = cell_texts[idx].clone();
            }
        }
    }

    Ok(TableData { cells: grid })
}

// ── Encode ────────────────────────────────────────────────

/// Encode a TableData into a base64-encoded, zlib-compressed MergeableData.
pub fn encode_table(table: &TableData) -> Result<String> {
    let proto_bytes = encode_mergeable_data(table);

    let mut zl = ZlibEncoder::new(Vec::new(), Compression::default());
    zl.write_all(&proto_bytes)?;
    let compressed = zl.finish()?;
    Ok(B64.encode(&compressed))
}

/// Build a cell content document (field 10 entry) for a cell with given text.
fn build_cell_doc(text: &str) -> Vec<u8> {
    let char_len = text.chars().count() as u64;

    let mut cell = Vec::new();
    // field 2: cell text
    cell.extend_from_slice(&field_bytes(2, text.as_bytes()));

    // CRDT operations (required for Apple Notes to parse)
    // op1: initial position
    let op1 = {
        let mut x = field_bytes(1, &position(0, 0));
        x.extend_from_slice(&field_varint(2, 0));
        x.extend_from_slice(&field_bytes(3, &position(0, 0)));
        x.extend_from_slice(&field_varint(5, 1));
        x
    };
    cell.extend_from_slice(&field_bytes(3, &op1));

    if char_len > 0 {
        // op2: text insertion
        let op2 = {
            let mut x = field_bytes(1, &position(1, 0));
            x.extend_from_slice(&field_varint(2, char_len));
            x.extend_from_slice(&field_bytes(3, &position(1, 0)));
            x.extend_from_slice(&field_varint(5, 2));
            x
        };
        cell.extend_from_slice(&field_bytes(3, &op2));
    }

    // op3: sentinel
    let op3 = {
        let mut x = field_bytes(1, &position(0, -1));
        x.extend_from_slice(&field_varint(2, 0));
        x.extend_from_slice(&field_bytes(3, &position(0, -1)));
        x
    };
    cell.extend_from_slice(&field_bytes(3, &op3));

    // field 5: attribute run (one run covering all text)
    if char_len > 0 {
        cell.extend_from_slice(&field_bytes(5, &field_varint(1, char_len)));
    }

    cell
}

/// Build a column container (field 16) with row references.
fn build_column_container(
    num_rows: usize,
    cell_uuid_indices: &[usize],
    row_uuid_indices: &[usize],
    col_doc_uuid: &[u8; 16],
) -> Vec<u8> {
    // Build the inner NoteDocument-like structure for this column's cells
    let mut text = String::new();
    for _ in 0..num_rows {
        text.push('\u{FFFC}'); // placeholder per row
    }

    // Build CRDT ops for the column document
    let char_len = text.chars().count() as u64;
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

    // Build Note-like message
    let mut note = Vec::new();
    note.extend_from_slice(&field_bytes(2, text.as_bytes()));
    note.extend_from_slice(&field_bytes(3, &op1));
    note.extend_from_slice(&field_bytes(3, &op2));
    note.extend_from_slice(&field_bytes(3, &op3));
    // Attribute runs: one per row (each length=1 for the FFFC char)
    for _ in 0..num_rows {
        note.extend_from_slice(&field_bytes(5, &field_varint(1, 1)));
        note.extend_from_slice(&field_bytes(5, &field_varint(1, 1)));
    }

    // UUID metadata for this column document
    let clock_payload = field_varint(1, char_len);
    let replica_payload = field_varint(1, 1);
    let uuid_entry = {
        let mut u = field_bytes(1, col_doc_uuid);
        u.extend_from_slice(&field_bytes(2, &clock_payload));
        u.extend_from_slice(&field_bytes(2, &replica_payload));
        u
    };
    note.extend_from_slice(&field_bytes(4, &field_bytes(1, &uuid_entry)));

    let mut doc = field_bytes(1, &note);

    // field 2: row-to-cell mapping
    let mut row_map = Vec::new();
    for i in 0..num_rows {
        let mut entry = Vec::new();
        // field 1: row UUID index reference
        let row_ref = field_varint(6, row_uuid_indices[i] as u64);
        entry.extend_from_slice(&field_bytes(1, &row_ref));
        // field 2: cell UUID index reference
        let cell_ref = field_varint(6, cell_uuid_indices[i] as u64);
        entry.extend_from_slice(&field_bytes(2, &cell_ref));
        // field 3: CRDT position
        entry.extend_from_slice(&field_bytes(
            3,
            &field_bytes(1, &[0x08, 0x00, 0x10, 0x01, 0x18, 0x00]),
        ));
        row_map.extend_from_slice(&field_bytes(1, &entry));
    }
    doc.extend_from_slice(&field_bytes(2, &row_map));

    doc
}

fn encode_mergeable_data(table: &TableData) -> Vec<u8> {
    let num_rows = table.rows().max(1);
    let num_cols = table.cols().max(1);

    // Generate UUIDs for all objects
    let table_uuid = *uuid::Uuid::new_v4().as_bytes();
    let mut all_uuids: Vec<[u8; 16]> = vec![table_uuid];

    // Generate UUIDs for: rows, columns, cells, column docs
    let mut row_uuid_indices = Vec::new();
    for _ in 0..num_rows {
        row_uuid_indices.push(all_uuids.len());
        all_uuids.push(*uuid::Uuid::new_v4().as_bytes());
    }
    let mut col_uuid_indices = Vec::new();
    for _ in 0..num_cols {
        col_uuid_indices.push(all_uuids.len());
        all_uuids.push(*uuid::Uuid::new_v4().as_bytes());
    }
    let mut cell_uuid_indices = Vec::new(); // [col][row]
    for _ in 0..num_cols {
        let mut col_cells = Vec::new();
        for _ in 0..num_rows {
            col_cells.push(all_uuids.len());
            all_uuids.push(*uuid::Uuid::new_v4().as_bytes());
        }
        cell_uuid_indices.push(col_cells);
    }
    let mut col_doc_uuids: Vec<[u8; 16]> = Vec::new();
    for _ in 0..num_cols {
        col_doc_uuids.push(*uuid::Uuid::new_v4().as_bytes());
    }

    // Build operations (field 3 entries)
    let mut ops: Vec<Vec<u8>> = Vec::new();

    // Op 0: Table root object (field 1 with identity)
    {
        let inner = {
            let mut x = Vec::new();
            x.extend_from_slice(&field_bytes(
                1,
                &[0x08, 0x00, 0x10, 0x01, 0x18, 0x00],
            ));
            x
        };
        ops.push(field_bytes(1, &inner));
    }

    // Op 1: Table configuration (field 13 — object registry)
    // Sets up initial type mappings and references
    {
        let mut obj = field_varint(1, 4); // type index = 4 (com.apple.notes.CRTable pair at index 4,5)
        // Identity: zero-filled UUID (32 hex chars)
        let identity = format!(" {}", "0".repeat(32));
        obj.extend_from_slice(&field_bytes(
            3,
            &{
                let mut e = field_varint(1, 0);
                e.extend_from_slice(&field_bytes(2, identity.as_bytes()));
                e
            },
        ));
        // crTableColumnDirection (key index 1): value 1 (left-to-right)
        obj.extend_from_slice(&field_bytes(
            3,
            &{
                let mut e = field_varint(1, 1);
                e.extend_from_slice(&field_bytes(2, &field_varint(6, 1)));
                e
            },
        ));
        // crRows (key index 3)
        obj.extend_from_slice(&field_bytes(
            3,
            &{
                let mut e = field_varint(1, 3);
                e.extend_from_slice(&field_bytes(2, &field_varint(6, 8)));
                e
            },
        ));
        // crColumns (key index 5)
        obj.extend_from_slice(&field_bytes(
            3,
            &{
                let mut e = field_varint(1, 5);
                e.extend_from_slice(&field_bytes(2, &field_varint(6, 13)));
                e
            },
        ));
        ops.push(field_bytes(13, &obj));
    }

    // Op 2: UUIDIndex reference
    {
        let mut obj = Vec::new();
        obj.extend_from_slice(&field_bytes(1, &field_bytes(1, &[0x08, 0x00, 0x10, 0x01])));
        obj.extend_from_slice(&field_bytes(2, &field_varint(6, 2)));
        ops.push(field_bytes(1, &obj));
    }

    // Op 3: CRTableColumnDirection (field 13)
    {
        let mut obj = field_varint(1, 1); // type index 1
        obj.extend_from_slice(&field_bytes(
            3,
            &{
                let mut e = field_varint(1, 2);
                e.extend_from_slice(&field_bytes(
                    2,
                    b"\"!CRTableColumnDirectionLeftToRight",
                ));
                e
            },
        ));
        ops.push(field_bytes(13, &obj));
    }

    // Column containers (field 16) — one per column
    for col in 0..num_cols {
        let container = build_column_container(
            num_rows,
            &cell_uuid_indices[col],
            &row_uuid_indices,
            &col_doc_uuids[col],
        );
        ops.push(field_bytes(16, &container));
    }

    // Cell object registrations (field 13) — register each cell as type 2
    for col in 0..num_cols {
        for row in 0..num_rows {
            let mut obj = field_varint(1, 2); // type for cell reference
            obj.extend_from_slice(&field_bytes(
                3,
                &{
                    let mut e = field_varint(1, 4); // key index 4 = UUIDIndex
                    e.extend_from_slice(&field_bytes(
                        2,
                        &field_varint(2, cell_uuid_indices[col][row] as u64),
                    ));
                    e
                },
            ));
            ops.push(field_bytes(13, &obj));
        }
    }

    // Row CRDT list entries (field 6) — register rows
    {
        let mut list = Vec::new();
        for i in 0..num_rows {
            let mut entry = Vec::new();
            entry.extend_from_slice(&field_bytes(
                1,
                &field_varint(6, col_uuid_indices.first().copied().unwrap_or(0) as u64),
            ));
            entry.extend_from_slice(&field_bytes(
                2,
                &field_varint(6, row_uuid_indices[i] as u64),
            ));
            entry.extend_from_slice(&field_bytes(
                3,
                &field_bytes(1, &[0x08, 0x00, 0x10, 0x01, 0x18, 0x00]),
            ));
            list.extend_from_slice(&field_bytes(1, &entry));
        }
        ops.push(field_bytes(6, &list));
    }

    // Column CRDT list entries (field 6) — register columns
    {
        let mut list = Vec::new();
        for i in 0..num_cols {
            let mut entry = Vec::new();
            entry.extend_from_slice(&field_bytes(
                1,
                &field_varint(6, row_uuid_indices.first().copied().unwrap_or(0) as u64),
            ));
            entry.extend_from_slice(&field_bytes(
                2,
                &field_varint(6, col_uuid_indices[i] as u64),
            ));
            entry.extend_from_slice(&field_bytes(
                3,
                &field_bytes(1, &[0x08, 0x00, 0x10, 0x01, 0x18, 0x00]),
            ));
            list.extend_from_slice(&field_bytes(1, &entry));
        }
        ops.push(field_bytes(6, &list));
    }

    // Cell content documents (field 10) — one per cell, column-major order
    for col in 0..num_cols {
        for row in 0..num_rows {
            let text = table
                .cells
                .get(row)
                .and_then(|r| r.get(col))
                .map(|s| s.as_str())
                .unwrap_or("");
            ops.push(field_bytes(10, &build_cell_doc(text)));
        }
    }

    // Build key names (field 4)
    let key_names = [
        "identity",
        "crTableColumnDirection",
        "self",
        "crRows",
        "UUIDIndex",
        "crColumns",
        "cellColumns",
    ];

    // Build type names (field 5) — pairs of CRTable/ICTable
    let base_types = [
        "com.apple.CRDT.NSNumber",
        "com.apple.CRDT.NSString",
        "com.apple.CRDT.NSUUID",
    ];
    let mut type_names: Vec<&str> = base_types.to_vec();
    // Add CRTable/ICTable pairs for each object
    let num_objects = 2 + num_cols * num_rows + num_rows + num_cols + 2;
    for _ in 0..num_objects {
        type_names.push("com.apple.notes.CRTable");
        type_names.push("com.apple.notes.ICTable");
    }

    // Build content (field 2)
    let mut content = Vec::new();
    content.extend_from_slice(&field_varint(1, 0)); // version
    content.extend_from_slice(&field_varint(2, 0)); // format
    for op in &ops {
        content.extend_from_slice(&field_bytes(3, op));
    }
    for name in &key_names {
        content.extend_from_slice(&field_bytes(4, name.as_bytes()));
    }
    for name in &type_names {
        content.extend_from_slice(&field_bytes(5, name.as_bytes()));
    }
    // UUIDs (field 6)
    for uuid in &all_uuids {
        content.extend_from_slice(&field_bytes(6, uuid));
    }
    // Version vector (field 7)
    let total_chars: u64 = table
        .cells
        .iter()
        .flat_map(|r| r.iter())
        .map(|s| s.chars().count() as u64)
        .sum();
    let version_entry = {
        let mut u = field_bytes(1, &table_uuid);
        u.extend_from_slice(&field_bytes(2, &field_varint(1, total_chars + 10)));
        u.extend_from_slice(&field_bytes(2, &field_varint(1, 1)));
        u
    };
    content.extend_from_slice(&field_bytes(7, &field_bytes(1, &version_entry)));

    // Build outer MergeableData
    let mut outer = Vec::new();
    outer.extend_from_slice(&field_varint(1, 0)); // version
    outer.extend_from_slice(&field_bytes(2, &content));
    outer
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn simple_table_encode_decode_roundtrip() {
        let table = TableData {
            cells: vec![
                vec!["Header 1".to_string(), "Header 2".to_string()],
                vec!["Cell A".to_string(), "Cell B".to_string()],
            ],
        };
        let encoded = encode_table(&table).unwrap();
        let decoded = decode_table(&encoded).unwrap();
        assert_eq!(decoded.rows(), 2);
        assert_eq!(decoded.cols(), 2);
        assert_eq!(decoded.cells[0][0], "Header 1");
        assert_eq!(decoded.cells[0][1], "Header 2");
        assert_eq!(decoded.cells[1][0], "Cell A");
        assert_eq!(decoded.cells[1][1], "Cell B");
    }

    #[test]
    fn empty_cells_roundtrip() {
        let table = TableData {
            cells: vec![
                vec!["A".to_string(), "".to_string()],
                vec!["".to_string(), "D".to_string()],
            ],
        };
        let encoded = encode_table(&table).unwrap();
        let decoded = decode_table(&encoded).unwrap();
        assert_eq!(decoded.cells[0][0], "A");
        assert_eq!(decoded.cells[0][1], "");
        assert_eq!(decoded.cells[1][0], "");
        assert_eq!(decoded.cells[1][1], "D");
    }

    #[test]
    fn three_by_three_table() {
        let table = TableData {
            cells: vec![
                vec!["1".into(), "2".into(), "3".into()],
                vec!["4".into(), "5".into(), "6".into()],
                vec!["7".into(), "8".into(), "9".into()],
            ],
        };
        let encoded = encode_table(&table).unwrap();
        let decoded = decode_table(&encoded).unwrap();
        assert_eq!(decoded.rows(), 3);
        assert_eq!(decoded.cols(), 3);
        for row in 0..3 {
            for col in 0..3 {
                let expected = format!("{}", row * 3 + col + 1);
                assert_eq!(decoded.cells[row][col], expected);
            }
        }
    }
}
