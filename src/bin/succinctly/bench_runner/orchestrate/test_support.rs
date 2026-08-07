//! Test-only fakes for `RemoteExec`/`BuildRunner`/`Ec2Control`, used by
//! orchestration unit tests so they never need a real SSH connection,
//! `cargo build`, or `aws` CLI invocation.

use super::config::NodeConfig;
use super::nodes::{Ec2Control, Ec2State};
use super::ssh::{ExecOutput, RemoteExec};
use super::sync::BuildRunner;
use anyhow::Result;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;
use std::time::Duration;

/// Records every `exec` call (as `(node_name, command)`) and lets tests
/// script specific failures, responses, or observe concurrency, all
/// in-memory.
#[derive(Default)]
pub(crate) struct FakeExec {
    calls: Mutex<Vec<(String, String)>>,
    fail_connectivity: Mutex<HashSet<String>>,
    /// `(node_name, command_substring, canned_output)`, checked in order —
    /// first match wins. Falls back to a generic success response.
    responses: Mutex<Vec<(String, String, ExecOutput)>>,
    running: AtomicUsize,
    max_concurrent_seen: AtomicUsize,
}

impl FakeExec {
    pub fn new() -> Self {
        Self::default()
    }

    /// Make the connectivity check (`echo ok`) fail for a specific node.
    pub fn fail_connectivity_check(self, node_name: &str) -> Self {
        self.fail_connectivity
            .lock()
            .unwrap()
            .insert(node_name.to_string());
        self
    }

    /// Script a canned response for any command on `node_name` containing
    /// `command_substring` (e.g. `"--version"`).
    pub fn respond(self, node_name: &str, command_substring: &str, output: ExecOutput) -> Self {
        self.responses.lock().unwrap().push((
            node_name.to_string(),
            command_substring.to_string(),
            output,
        ));
        self
    }

    pub fn calls(&self) -> Vec<(String, String)> {
        self.calls.lock().unwrap().clone()
    }

    /// Highest number of concurrently in-flight `exec` calls observed so
    /// far — used by Phase 2's max-concurrency / exclusivity assertions.
    pub fn max_concurrent_seen(&self) -> usize {
        self.max_concurrent_seen.load(Ordering::SeqCst)
    }
}

impl RemoteExec for FakeExec {
    fn exec(&self, node: &NodeConfig, command: &str, _timeout: Duration) -> Result<ExecOutput> {
        self.calls
            .lock()
            .unwrap()
            .push((node.name.clone(), command.to_string()));

        let now_running = self.running.fetch_add(1, Ordering::SeqCst) + 1;
        self.max_concurrent_seen
            .fetch_max(now_running, Ordering::SeqCst);
        std::thread::sleep(Duration::from_millis(5));
        self.running.fetch_sub(1, Ordering::SeqCst);

        if command == "echo ok" && self.fail_connectivity.lock().unwrap().contains(&node.name) {
            return Ok(ExecOutput {
                stdout: String::new(),
                stderr: "connection refused".to_string(),
                exit_code: 1,
            });
        }

        for (n, sub, output) in self.responses.lock().unwrap().iter() {
            if n == &node.name && command.contains(sub.as_str()) {
                return Ok(output.clone());
            }
        }

        Ok(ExecOutput {
            stdout: "ok".to_string(),
            stderr: String::new(),
            exit_code: 0,
        })
    }

    fn upload(
        &self,
        _node: &NodeConfig,
        _local: &Path,
        _remote: &str,
        _connect_timeout: Duration,
    ) -> Result<()> {
        Ok(())
    }

    fn download(
        &self,
        _node: &NodeConfig,
        _remote_dir: &str,
        local_dir: &Path,
        _connect_timeout: Duration,
    ) -> Result<()> {
        std::fs::create_dir_all(local_dir)?;
        Ok(())
    }
}

/// Records `cross_compile` calls and returns a small stand-in file instead
/// of running a real `cargo build`.
#[derive(Default)]
pub(crate) struct FakeBuild {
    calls: Mutex<Vec<String>>,
}

impl FakeBuild {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn call_count(&self) -> usize {
        self.calls.lock().unwrap().len()
    }

    pub fn last_triple(&self) -> Option<String> {
        self.calls.lock().unwrap().last().cloned()
    }
}

impl BuildRunner for FakeBuild {
    fn cross_compile(&self, target_triple: &str) -> Result<PathBuf> {
        self.calls.lock().unwrap().push(target_triple.to_string());

        let mut path = std::env::temp_dir();
        path.push(format!("fake-succinctly-{target_triple}"));
        std::fs::write(&path, b"fake binary")?;
        Ok(path)
    }
}

/// Records `start`/`stop` calls and lets tests script a per-instance
/// [`Ec2State`] for `describe`, all in-memory.
#[derive(Default)]
pub(crate) struct FakeAwsCli {
    states: Mutex<HashMap<String, Ec2State>>,
    start_calls: Mutex<Vec<String>>,
    stop_calls: Mutex<Vec<String>>,
}

impl FakeAwsCli {
    pub fn new() -> Self {
        Self::default()
    }

    /// Script `describe(instance_id, ..)` to return `state`. Instances with
    /// no scripted state default to `Ec2State::Running`.
    pub fn set_state(self, instance_id: &str, state: Ec2State) -> Self {
        self.states
            .lock()
            .unwrap()
            .insert(instance_id.to_string(), state);
        self
    }

    pub fn start_calls(&self) -> Vec<String> {
        self.start_calls.lock().unwrap().clone()
    }

    pub fn stop_calls(&self) -> Vec<String> {
        self.stop_calls.lock().unwrap().clone()
    }
}

impl Ec2Control for FakeAwsCli {
    fn describe(&self, instance_id: &str, _region: &str) -> Result<Ec2State> {
        Ok(self
            .states
            .lock()
            .unwrap()
            .get(instance_id)
            .cloned()
            .unwrap_or(Ec2State::Running))
    }

    fn start(&self, instance_id: &str, _region: &str) -> Result<()> {
        self.start_calls
            .lock()
            .unwrap()
            .push(instance_id.to_string());
        Ok(())
    }

    fn stop(&self, instance_id: &str, _region: &str) -> Result<()> {
        self.stop_calls
            .lock()
            .unwrap()
            .push(instance_id.to_string());
        Ok(())
    }
}
