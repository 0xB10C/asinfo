//! Criterion benchmarks for `asinfo::lookup` and `asinfo::iter`.
//! Run with `cargo bench`.

use std::collections::HashSet;
use std::hint::black_box;

use criterion::{criterion_group, criterion_main, Criterion};

const POOL: usize = 1 << 16;

fn benches(c: &mut Criterion) {
    let asns: Vec<u32> = asinfo::iter().map(|e| e.asn).collect();

    // Deterministic xorshift so runs are comparable.
    let mut state = 0x9E37_79B9u32;
    let mut rng = move || {
        state ^= state << 13;
        state ^= state >> 17;
        state ^= state << 5;
        state
    };

    let hits: Vec<u32> = (0..POOL)
        .map(|_| asns[rng() as usize % asns.len()])
        .collect();
    let existing: HashSet<u32> = asns.iter().copied().collect();
    let mut misses = Vec::with_capacity(POOL);
    while misses.len() < POOL {
        let asn = rng();
        if !existing.contains(&asn) {
            misses.push(asn);
        }
    }

    let mask = POOL - 1;
    let mut i = 0;
    c.bench_function("lookup_existing_asn", |b| {
        b.iter(|| {
            i += 1;
            black_box(asinfo::lookup(black_box(hits[i & mask])))
        })
    });

    let mut i = 0;
    c.bench_function("lookup_missing_asn", |b| {
        b.iter(|| {
            i += 1;
            black_box(asinfo::lookup(black_box(misses[i & mask])))
        })
    });

    c.bench_function("iter_full_scan", |b| {
        b.iter(|| {
            asinfo::iter()
                .map(|e| e.asn as u64 + e.handle.len() as u64)
                .sum::<u64>()
        })
    });
}

criterion_group!(benches_group, benches);
criterion_main!(benches_group);
