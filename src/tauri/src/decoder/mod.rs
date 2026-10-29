//! NCM 解码器（纯 Rust 实现）。

pub mod ncm;
pub mod wrapper;

pub use wrapper::{decode_ncm_to_dir, DecodeResult};
