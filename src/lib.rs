//! Zero-allocation lookup from an Autonomous System Number (ASN) to its
//! handle, description, and country code.
//!
//! The [ipverse/as-metadata](https://github.com/ipverse/as-metadata) dataset
//! is converted into compact binary tables by `build.rs` and embedded into
//! the binary at compile time. Records are varint-encoded in blocks; a
//! lookup binary-searches a small block index and decodes at most one block
//! (a few cache lines) — no parsing at startup, no initialization, no heap.
//!
//! The crate is `no_std` and depends only on `core`.
//!
//! ```
//! // ASN 0 is reserved by IANA and stable across dataset updates.
//! let info = asinfo::lookup(0).unwrap();
//! assert_eq!(info.country.as_str(), "US");
//! assert!(asinfo::lookup(u32::MAX).is_none());
//! ```

#![no_std]
#![deny(missing_docs)]

use core::fmt;

// See build.rs for the format of these tables.
static STREAM: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/entries.bin"));
static INDEX: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/index.bin"));
static STRINGS: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/strings.bin"));
static COUNTRIES: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/countries.bin"));
include!(concat!(env!("OUT_DIR"), "/meta.rs"));

/// Bytes per `INDEX` entry: first ASN (u32), stream position (u32),
/// blob cursor (u32), little-endian. Must stay in sync with build.rs.
const INDEX_ENTRY_SIZE: usize = 12;

/// An ISO 3166-1 alpha-2 country code (two ASCII uppercase letters).
#[derive(Copy, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Country([u8; 2]);

impl Country {
    /// The country code as a string slice, e.g. `"US"`.
    pub fn as_str(&self) -> &str {
        // Validated as ASCII uppercase by build.rs; cannot fail.
        core::str::from_utf8(&self.0).unwrap_or("??")
    }

    /// The country code as raw bytes, e.g. `b"US"`.
    pub fn as_bytes(&self) -> [u8; 2] {
        self.0
    }
}

impl fmt::Display for Country {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl fmt::Debug for Country {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Country({})", self.as_str())
    }
}

/// Metadata of an autonomous system.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct AsInfo {
    /// The autonomous system number.
    pub asn: u32,
    /// Short handle, e.g. `"LVLT-1"`.
    pub handle: &'static str,
    /// Human-readable description, e.g. `"Level 3 Parent LLC"`.
    pub description: &'static str,
    /// Registered country code.
    pub country: Country,
}

/// Looks up an ASN. Returns `None` if the ASN is not in the dataset.
///
/// Binary search over the block index, then a sequential decode of at most
/// one block. O(log N), no allocation.
pub fn lookup(asn: u32) -> Option<AsInfo> {
    // Find the last block whose first ASN is <= the target.
    let (mut lo, mut hi) = (0usize, block_count());
    while lo < hi {
        let mid = lo + (hi - lo) / 2;
        if index_entry(mid).0 <= asn {
            lo = mid + 1;
        } else {
            hi = mid;
        }
    }
    let block = lo.checked_sub(1)?;

    let (first_asn, stream_pos, blob_pos) = index_entry(block);
    let end = if block + 1 < block_count() {
        index_entry(block + 1).1
    } else {
        STREAM.len()
    };
    let mut cursor = Cursor {
        pos: stream_pos,
        blob: blob_pos,
        asn: first_asn,
        first: true,
    };
    while cursor.pos < end {
        let info = cursor.decode();
        if info.asn >= asn {
            return (info.asn == asn).then_some(info);
        }
    }
    None
}

/// The number of autonomous systems in the embedded dataset.
pub fn count() -> usize {
    RECORD_COUNT
}

/// Iterates over all entries in ascending ASN order.
pub fn iter() -> impl Iterator<Item = AsInfo> {
    Iter {
        cursor: Cursor {
            pos: 0,
            blob: 0,
            asn: 0,
            first: false,
        },
        next_block: 0,
    }
}

/// Decoder state while walking the varint record stream (see build.rs for
/// the format). `blob` tracks the position implicitly assigned to the next
/// new string; `first` marks a block's first record, whose ASN comes from
/// the index instead of a delta.
struct Cursor {
    pos: usize,
    blob: usize,
    asn: u32,
    first: bool,
}

impl Cursor {
    fn decode(&mut self) -> AsInfo {
        if self.first {
            self.first = false;
        } else {
            self.asn += self.varint();
        }
        let handle = self.decode_str();
        let description = self.decode_str();
        let country = 2 * self.varint() as usize;
        AsInfo {
            asn: self.asn,
            handle,
            description,
            country: Country([COUNTRIES[country], COUNTRIES[country + 1]]),
        }
    }

    fn decode_str(&mut self) -> &'static str {
        let tag = self.varint() as usize;
        let len = tag >> 1;
        let offset = if tag & 1 == 1 {
            self.blob - self.varint() as usize // back-reference to an earlier string
        } else {
            let offset = self.blob; // new string sits at the blob cursor
            self.blob += len;
            offset
        };
        let bytes = &STRINGS[offset..offset + len];
        // SAFETY: build.rs interns whole, valid UTF-8 strings and encodes
        // only offsets produced by that interning, so the slice starts and
        // ends on string boundaries within the blob.
        unsafe { core::str::from_utf8_unchecked(bytes) }
    }

    fn varint(&mut self) -> u32 {
        let mut value = 0u32;
        let mut shift = 0;
        loop {
            let byte = STREAM[self.pos];
            self.pos += 1;
            value |= ((byte & 0x7F) as u32) << shift;
            if byte & 0x80 == 0 {
                return value;
            }
            shift += 7;
        }
    }
}

struct Iter {
    cursor: Cursor,
    next_block: usize,
}

impl Iterator for Iter {
    type Item = AsInfo;

    fn next(&mut self) -> Option<AsInfo> {
        if self.cursor.pos >= STREAM.len() {
            return None;
        }
        if self.next_block < block_count() {
            let (first_asn, stream_pos, blob_pos) = index_entry(self.next_block);
            if self.cursor.pos == stream_pos {
                debug_assert_eq!(self.cursor.blob, blob_pos);
                self.cursor.asn = first_asn;
                self.cursor.first = true;
                self.next_block += 1;
            }
        }
        Some(self.cursor.decode())
    }
}

fn block_count() -> usize {
    INDEX.len() / INDEX_ENTRY_SIZE
}

fn index_entry(block: usize) -> (u32, usize, usize) {
    let base = block * INDEX_ENTRY_SIZE;
    (
        read_u32(base),
        read_u32(base + 4) as usize,
        read_u32(base + 8) as usize,
    )
}

fn read_u32(offset: usize) -> u32 {
    let bytes: [u8; 4] = INDEX[offset..offset + 4].try_into().unwrap();
    u32::from_le_bytes(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    // The dataset is refreshed on every release, so tests derive expectations
    // from the embedded data itself instead of hardcoding values that churn.
    // Exception: ASN 0 is reserved by IANA and stable.

    #[test]
    fn asn_zero_is_iana() {
        let info = lookup(0).unwrap();
        assert_eq!(info.asn, 0);
        assert_eq!(info.handle, "IANA-RSVD-0");
        assert_eq!(info.country.as_str(), "US");
    }

    #[test]
    fn table_is_consistent() {
        assert_eq!(INDEX.len() % INDEX_ENTRY_SIZE, 0);
        assert_eq!(iter().count(), count());
        assert!(count() > 100_000, "dataset suspiciously small: {}", count());
    }

    #[test]
    fn first_and_last_round_trip() {
        let first = iter().next().unwrap();
        assert_eq!(lookup(first.asn), Some(first));
        let last = iter().last().unwrap();
        assert_eq!(lookup(last.asn), Some(last));
    }

    #[test]
    fn missing_asns_return_none() {
        assert!(lookup(u32::MAX).is_none());
        let last_asn = iter().last().unwrap().asn;
        assert!(lookup(last_asn + 1).is_none());
    }

    #[test]
    fn asns_strictly_increasing() {
        let mut prev = None;
        for info in iter() {
            if let Some(prev) = prev {
                assert!(info.asn > prev, "ASN {} not after {prev}", info.asn);
            }
            prev = Some(info.asn);
        }
    }

    #[test]
    fn all_entries_well_formed() {
        for info in iter() {
            assert!(!info.handle.is_empty(), "ASN {} has empty handle", info.asn);
            assert!(
                !info.description.is_empty(),
                "ASN {} has empty description",
                info.asn
            );
            assert!(
                info.country
                    .as_bytes()
                    .iter()
                    .all(|b| b.is_ascii_uppercase()),
                "ASN {} has bad country {:?}",
                info.asn,
                info.country
            );
            assert_eq!(lookup(info.asn), Some(info));
        }
    }
}
