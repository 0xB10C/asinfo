# asinfo

[![crates.io](https://img.shields.io/crates/v/asinfo.svg)](https://crates.io/crates/asinfo)
[![docs.rs](https://docs.rs/asinfo/badge.svg)](https://docs.rs/asinfo)
[![ci](https://github.com/0xB10C/asinfo/actions/workflows/ci.yml/badge.svg)](https://github.com/0xB10C/asinfo/actions/workflows/ci.yml)

A lightweight Rust library for looking up the handle, description, and
country of an Autonomous System by its ASN.

The [ipverse/as-metadata](https://github.com/ipverse/as-metadata) dataset is
preprocessed by `build.rs` into compact binary tables (a sorted fixed-size
record table plus a deduplicated string blob) and embedded into your binary
at compile time.

* **no runtime parsing or initialization** — lookups are a binary search
  over the embedded table
* **no allocations** — returned strings are `&'static str` slices into the
  embedded data
* **`no_std`** — depends only on `core`
* **no runtime dependencies**

## Usage

```rust
let info = asinfo::lookup(3856).unwrap();
println!(
    "AS{}: {} ({}) [{}]",
    info.asn, info.description, info.handle, info.country
);
// AS3856: Packet Clearing House Inc. (PCH-AS) [US]
```

## Data updates

A GitHub Actions workflow refreshes the dataset monthly and publishes a new
version as `0.1.YYYYMMDD` — the patch level is the date of the data
snapshot. Bump your dependency to get newer data.

## License

The code is licensed under [MIT](LICENSE). The embedded dataset comes from
[ipverse/as-metadata](https://github.com/ipverse/as-metadata) and is released
under CC0 1.0 Universal.
