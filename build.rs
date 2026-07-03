//! Converts `as-metadata/as.csv` into compact binary tables in `OUT_DIR`.
//!
//! * `strings.bin` — deduplicated UTF-8 blob in first-use order, referenced
//!   verbatim by the library (strings must exist contiguously so lookups can
//!   return `&'static str`).
//! * `entries.bin` — varint-encoded records sorted by ASN. Per record:
//!   - varint ASN delta from the previous record (omitted for the first
//!     record of a block; the index stores that ASN directly)
//!   - handle, then description, each as varint(len << 1 | reused):
//!     a new string implicitly sits at the current blob cursor (no offset
//!     stored); a reused string is followed by varint(back-distance from
//!     the current blob cursor to its earlier occurrence)
//!   - varint country index into `countries.bin`
//! * `index.bin` — one entry per `BLOCK_SIZE` records for random access:
//!   first ASN (u32), stream position (u32), blob cursor (u32), all
//!   little-endian. Lookups binary-search this and decode one block.
//! * `countries.bin` — two bytes per country code, most frequent first, so
//!   varint country indices are one byte for the common cases.
//! * `meta.rs` — record count constant, included by src/lib.rs.

use std::cmp::Reverse;
use std::collections::HashMap;
use std::env;
use std::fs;
use std::path::Path;

/// Records per index block. Lookups decode at most this many records after
/// the binary search; the index costs 12 bytes per block.
const BLOCK_SIZE: usize = 32;

struct Row {
    asn: u32,
    handle: String,
    description: String,
    country: [u8; 2],
}

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=as-metadata/as.csv");

    let manifest_dir = env::var("CARGO_MANIFEST_DIR").unwrap();
    let csv_path = Path::new(&manifest_dir).join("as-metadata/as.csv");
    let out_dir = env::var("OUT_DIR").unwrap();

    let mut rows = read_rows(&csv_path);
    rows.sort_by_key(|r| r.asn);
    for pair in rows.windows(2) {
        if pair[0].asn == pair[1].asn {
            panic!("duplicate ASN {} in {}", pair[0].asn, csv_path.display());
        }
    }

    // Country table ordered by frequency (ties broken by code for
    // determinism), so varint indices are one byte for common countries.
    let mut freq: HashMap<[u8; 2], u64> = HashMap::new();
    for row in &rows {
        *freq.entry(row.country).or_insert(0) += 1;
    }
    let mut countries: Vec<[u8; 2]> = freq.keys().copied().collect();
    countries.sort_by_key(|c| (Reverse(freq[c]), *c));
    let country_index: HashMap<[u8; 2], u32> = countries
        .iter()
        .enumerate()
        .map(|(i, c)| (*c, i as u32))
        .collect();

    let mut blob: Vec<u8> = Vec::new();
    let mut offsets: HashMap<String, usize> = HashMap::new();
    let mut stream: Vec<u8> = Vec::new();
    let mut index: Vec<u8> = Vec::new();
    let mut prev_asn = 0u32;

    for (i, row) in rows.iter().enumerate() {
        if i % BLOCK_SIZE == 0 {
            index.extend_from_slice(&row.asn.to_le_bytes());
            index.extend_from_slice(&u32_of(stream.len(), "stream").to_le_bytes());
            index.extend_from_slice(&u32_of(blob.len(), "blob").to_le_bytes());
        } else {
            put_varint(&mut stream, row.asn - prev_asn);
        }
        prev_asn = row.asn;
        encode_string(&mut stream, &mut blob, &mut offsets, &row.handle);
        encode_string(&mut stream, &mut blob, &mut offsets, &row.description);
        put_varint(&mut stream, country_index[&row.country]);
    }

    let country_table: Vec<u8> = countries.iter().flatten().copied().collect();
    let out = Path::new(&out_dir);
    fs::write(out.join("entries.bin"), &stream).unwrap();
    fs::write(out.join("index.bin"), &index).unwrap();
    fs::write(out.join("strings.bin"), &blob).unwrap();
    fs::write(out.join("countries.bin"), &country_table).unwrap();
    fs::write(
        out.join("meta.rs"),
        format!("const RECORD_COUNT: usize = {};\n", rows.len()),
    )
    .unwrap();
}

fn encode_string(
    stream: &mut Vec<u8>,
    blob: &mut Vec<u8>,
    offsets: &mut HashMap<String, usize>,
    s: &str,
) {
    let tag = u32_of(s.len() << 1, "string length");
    if let Some(&off) = offsets.get(s) {
        put_varint(stream, tag | 1);
        put_varint(stream, u32_of(blob.len() - off, "back-distance"));
    } else {
        put_varint(stream, tag);
        offsets.insert(s.to_owned(), blob.len());
        blob.extend_from_slice(s.as_bytes());
    }
}

fn put_varint(out: &mut Vec<u8>, mut value: u32) {
    loop {
        let byte = (value & 0x7F) as u8;
        value >>= 7;
        if value == 0 {
            out.push(byte);
            return;
        }
        out.push(byte | 0x80);
    }
}

fn u32_of(value: usize, what: &str) -> u32 {
    value
        .try_into()
        .unwrap_or_else(|_| panic!("{what} {value} exceeds u32"))
}

fn read_rows(csv_path: &Path) -> Vec<Row> {
    let mut reader = csv::ReaderBuilder::new()
        .has_headers(true)
        .from_path(csv_path)
        .unwrap_or_else(|e| panic!("cannot read {}: {e}", csv_path.display()));

    let headers = reader.headers().expect("cannot read CSV header").clone();
    let expected = ["asn", "handle", "description", "country-code"];
    if headers.iter().ne(expected) {
        panic!("unexpected CSV header {headers:?}, expected {expected:?}");
    }

    let mut rows = Vec::new();
    for (i, record) in reader.records().enumerate() {
        let line = i + 2; // 1-based, after the header
        let record = record.unwrap_or_else(|e| panic!("CSV error on line {line}: {e}"));
        if record.len() != 4 {
            panic!("line {line}: expected 4 fields, got {}", record.len());
        }

        let asn: u32 = record[0]
            .parse()
            .unwrap_or_else(|e| panic!("line {line}: bad ASN {:?}: {e}", &record[0]));
        let country = record[3].as_bytes();
        if country.len() != 2 || !country.iter().all(|b| b.is_ascii_uppercase()) {
            panic!(
                "line {line}: country code {:?} is not two ASCII uppercase letters",
                &record[3]
            );
        }

        rows.push(Row {
            asn,
            handle: record[1].to_owned(),
            description: record[2].to_owned(),
            country: [country[0], country[1]],
        });
    }
    if rows.is_empty() {
        panic!("{} contains no data rows", csv_path.display());
    }
    rows
}
