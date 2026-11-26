//! jq's own input reader, linked in-process from the vendored jq 1.7.1 and jq 1.8.2 sources, as
//! a test oracle for the input layer of `succinctly jq` (#7970, step 1 of the input-reader
//! migration plan).
//!
//! - [`reader`]: jq's real `jq_util_input_*` over files ([`reader::UtilReader`]), a Rust port of
//!   its read loop over memory ([`reader::PortReader`]), and drivers that print what jq prints
//!   for `jq -c .`, a `-n` trace program and the reader model's `inputs` program.
//! - [`diff`]: the differential harness. It generates edge inputs, runs a `succinctly` binary
//!   and both C readers on each, and classifies every difference.
//!
//! Never shipped: this package is not part of the succinctly crate (see `Cargo.toml`).

pub mod diff;
mod ffi;
pub mod reader;
