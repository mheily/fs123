//! Core fs123 protocol types and client implementation.
//!
//! This crate provides the shared components for fs123 clients:
//! - Netstring encoding/decoding
//! - Protocol URL building
//! - HTTP client and response parsing
//! - Common types (stat results, errors)

pub mod error;
pub mod netstring;
pub mod protocol;
pub mod types;

pub use error::{Fs123Error, Result};
pub use netstring::{
    decode as netstring_decode,
    encode as netstring_encode,
    encode_str as netstring_encode_str,
    parse_response,
};
pub use protocol::{build_url, parse_url, Fs123Function, Fs123HttpClient, Fs123Request, Fs123Response};
pub use types::{DirEntryData, Fs123StatResult, Fs123StatvfsResult};
