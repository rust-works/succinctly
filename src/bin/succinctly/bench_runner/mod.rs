//! Unified benchmark runner for succinctly.
//!
//! Provides a single entry point for discovering, listing, and running all benchmarks
//! with automatic metadata tracking.

mod metadata;
mod orchestrate;
mod registry;
mod runner;
mod utils;

pub use orchestrate::{
    run_nodes, run_orchestrate, run_report, run_sync, NodesArgs, OrchestrateArgs, ReportArgs,
    SyncArgs,
};
pub use runner::{run_benchmarks, run_list, ListArgs, RunArgs};
