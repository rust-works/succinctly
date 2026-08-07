//! SSH-based distributed benchmark orchestration (issue #98).
//!
//! Extends the unified benchmark runner (`bench_runner::{list,run}`) with
//! commands that fan the existing benchmark suite out across multiple
//! SSH-reachable nodes and aggregate the results. See `nodes.yaml.example`
//! at the repo root for the configuration format.

mod aggregate;
mod config;
mod executor;
mod nodes;
mod report;
mod scheduler;
mod ssh;
mod sync;

#[cfg(test)]
mod test_support;

pub use executor::{run_orchestrate, OrchestrateArgs};
pub use nodes::{run_nodes, NodesArgs};
pub use report::{run_report, ReportArgs};
pub use sync::{run_sync, SyncArgs};
