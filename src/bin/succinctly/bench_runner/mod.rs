//! Unified benchmark runner for succinctly.
//!
//! Provides a single entry point for discovering, listing, and running all benchmarks
//! with automatic metadata tracking.

mod metadata;
mod registry;
mod runner;
mod utils;

pub use runner::{run_benchmarks, run_list, ListArgs, RunArgs};
