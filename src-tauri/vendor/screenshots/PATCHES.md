# Local compatibility patch

Source is the published `screenshots 0.8.10` crate from
[crates.io](https://crates.io/crates/screenshots/0.8.10).
Archive SHA-256 is `038df8746dbf7d8b70715d638470db956794e0f3d08608e4197f4053c2da1620`.
The original source, manifest, README and Apache-2.0 license are included.

The two discarded D-Bus `method_call` return values in
`src/linux/wayland_screenshot.rs` explicitly use `()`, preserving the type
inferred by older Rust versions. This fixes E0277 with Rust 1.100's
[never-type fallback change](https://github.com/rust-lang/rust/issues/148922).
