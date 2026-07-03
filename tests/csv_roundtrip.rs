//! Validates the embedded tables against an independent parse of the source
//! CSV: every row must round-trip through `asinfo::lookup`, and ASNs absent
//! from the CSV must return `None`.

use std::collections::BTreeSet;

#[test]
fn every_csv_row_round_trips() {
    let mut reader = csv::Reader::from_path("as-metadata/as.csv").unwrap();
    let mut asns = BTreeSet::new();
    let mut rows = 0usize;

    for record in reader.records() {
        let record = record.unwrap();
        let asn: u32 = record[0].parse().unwrap();
        let info = asinfo::lookup(asn)
            .unwrap_or_else(|| panic!("ASN {asn} from CSV missing in embedded table"));
        assert_eq!(info.asn, asn);
        assert_eq!(info.handle, &record[1], "handle mismatch for ASN {asn}");
        assert_eq!(
            info.description, &record[2],
            "description mismatch for ASN {asn}"
        );
        assert_eq!(
            info.country.as_str(),
            &record[3],
            "country mismatch for ASN {asn}"
        );
        asns.insert(asn);
        rows += 1;
    }

    assert_eq!(rows, asinfo::count(), "entry count mismatch");

    // Probe the gaps: ASNs not in the CSV must not resolve.
    let max = *asns.last().unwrap();
    for asn in (0..=max).step_by(1013) {
        if !asns.contains(&asn) {
            assert!(
                asinfo::lookup(asn).is_none(),
                "ASN {asn} not in CSV but found in embedded table"
            );
        }
    }
}
