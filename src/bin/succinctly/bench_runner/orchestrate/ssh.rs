//! Remote command execution abstraction for SSH-based orchestration.
//!
//! [`RemoteExec`] lets scheduler/executor/sync logic run against either the
//! real `ssh`/`scp` system binaries ([`SystemSsh`]) or, in tests, a canned
//! fake (`test_support::FakeExec`) — without either side knowing which.
//! [`SystemSsh`] special-cases nodes whose `host` is `localhost`/`127.0.0.1`:
//! commands run directly (no `ssh` wrapper) and transfers become plain file
//! copies, so the local-node path is real end-to-end and CI-testable without
//! any network access.

use super::config::NodeConfig;
use crate::exit_status::exit_code_or_signal_death;
use anyhow::{Context, Result};
use std::fs;
use std::io::Read;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// Output of a remote (or local) command execution.
#[derive(Debug, Clone, Default)]
pub struct ExecOutput {
    pub stdout: String,
    pub stderr: String,
    pub exit_code: i32,
}

impl ExecOutput {
    pub fn success(&self) -> bool {
        self.exit_code == 0
    }
}

/// Abstraction over "run a command on a node" / "copy files to/from a node".
pub trait RemoteExec: Send + Sync {
    /// Run `command` on `node`, killing it if it hasn't exited after `timeout`.
    fn exec(&self, node: &NodeConfig, command: &str, timeout: Duration) -> Result<ExecOutput>;

    /// Upload a single local file to a remote path.
    fn upload(
        &self,
        node: &NodeConfig,
        local: &Path,
        remote: &str,
        connect_timeout: Duration,
    ) -> Result<()>;

    /// Download the *contents* of a remote directory into a local directory
    /// (the remote directory itself is not nested inside `local`).
    fn download(
        &self,
        node: &NodeConfig,
        remote_dir: &str,
        local_dir: &Path,
        connect_timeout: Duration,
    ) -> Result<()>;
}

/// Real implementation: shells out to the system `ssh`/`scp` binaries.
pub struct SystemSsh;

impl RemoteExec for SystemSsh {
    fn exec(&self, node: &NodeConfig, command: &str, timeout: Duration) -> Result<ExecOutput> {
        if node.is_local() {
            let mut cmd = Command::new("sh");
            cmd.arg("-c").arg(command);
            return run_with_timeout(cmd, timeout);
        }

        let args = build_ssh_args(node, command, timeout);
        let mut cmd = Command::new("ssh");
        cmd.args(&args);
        run_with_timeout(cmd, timeout)
            .with_context(|| format!("ssh exec on node '{}' failed", node.name))
    }

    fn upload(
        &self,
        node: &NodeConfig,
        local: &Path,
        remote: &str,
        connect_timeout: Duration,
    ) -> Result<()> {
        if node.is_local() {
            if let Some(parent) = Path::new(remote).parent() {
                fs::create_dir_all(parent).ok();
            }
            fs::copy(local, remote)
                .with_context(|| format!("local copy {} -> {remote} failed", local.display()))?;
            return Ok(());
        }

        let args = build_scp_args(node, local, remote, true, connect_timeout);
        let output = Command::new("scp")
            .args(&args)
            .output()
            .with_context(|| format!("failed to spawn scp for node '{}'", node.name))?;
        if !output.status.success() {
            anyhow::bail!(
                "scp upload to node '{}' failed: {}",
                node.name,
                String::from_utf8_lossy(&output.stderr)
            );
        }
        Ok(())
    }

    fn download(
        &self,
        node: &NodeConfig,
        remote_dir: &str,
        local_dir: &Path,
        connect_timeout: Duration,
    ) -> Result<()> {
        if node.is_local() {
            return copy_dir_contents(Path::new(remote_dir), local_dir);
        }

        fs::create_dir_all(local_dir)
            .with_context(|| format!("Failed to create {}", local_dir.display()))?;

        // Trailing "/." makes scp copy the directory's *contents* into
        // local_dir, matching the localhost branch's behavior above.
        let remote_spec = format!("{remote_dir}/.");
        let args = build_scp_args(node, local_dir, &remote_spec, false, connect_timeout);
        let output = Command::new("scp")
            .args(&args)
            .output()
            .with_context(|| format!("failed to spawn scp for node '{}'", node.name))?;
        if !output.status.success() {
            anyhow::bail!(
                "scp download from node '{}' failed: {}",
                node.name,
                String::from_utf8_lossy(&output.stderr)
            );
        }
        Ok(())
    }
}

/// Build the argument list for an `ssh` invocation. Pure and separately
/// testable — the actual `.output()`/spawn call is left to `SystemSsh`.
pub(crate) fn build_ssh_args(node: &NodeConfig, command: &str, timeout: Duration) -> Vec<String> {
    let mut args = Vec::new();
    if let Some(key) = node.expanded_ssh_key() {
        args.push("-i".to_string());
        args.push(key.display().to_string());
    }
    args.push("-o".to_string());
    args.push("StrictHostKeyChecking=accept-new".to_string());
    args.push("-o".to_string());
    args.push(format!("ConnectTimeout={}", connect_timeout_secs(timeout)));
    args.push(node.host.clone());
    args.push(command.to_string());
    args
}

/// Build the argument list for an `scp` invocation. `local` is always the
/// local-side path and `remote` the remote-side path string — for `upload`,
/// local is the source and remote the destination; for a download, it's the
/// reverse. Pure and separately testable, like [`build_ssh_args`].
pub(crate) fn build_scp_args(
    node: &NodeConfig,
    local: &Path,
    remote: &str,
    upload: bool,
    connect_timeout: Duration,
) -> Vec<String> {
    let mut args = Vec::new();
    if let Some(key) = node.expanded_ssh_key() {
        args.push("-i".to_string());
        args.push(key.display().to_string());
    }
    args.push("-o".to_string());
    args.push("StrictHostKeyChecking=accept-new".to_string());
    args.push("-o".to_string());
    args.push(format!(
        "ConnectTimeout={}",
        connect_timeout_secs(connect_timeout)
    ));
    args.push("-r".to_string());

    let local_spec = local.display().to_string();
    let remote_spec = format!("{}:{remote}", node.host);
    if upload {
        args.push(local_spec);
        args.push(remote_spec);
    } else {
        args.push(remote_spec);
        args.push(local_spec);
    }
    args
}

fn connect_timeout_secs(timeout: Duration) -> u64 {
    timeout.as_secs().clamp(1, 10)
}

/// Spawn `cmd`, draining stdout/stderr on background threads (so a chatty
/// child can never deadlock on a full pipe while we're polling), and kill it
/// if it hasn't exited after `timeout`.
fn run_with_timeout(mut cmd: Command, timeout: Duration) -> Result<ExecOutput> {
    let mut child = cmd
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .context("failed to spawn process")?;

    let mut stdout_pipe = child.stdout.take().context("child had no stdout pipe")?;
    let mut stderr_pipe = child.stderr.take().context("child had no stderr pipe")?;

    // Raw bytes, decoded only after the exit-code/signal check below: a
    // signal-killed child can leave a truncated multi-byte UTF-8 sequence
    // at the end of a buffer it was writing when killed, and a strict
    // `read_to_string` discards *all* already-read output (not just the
    // trailing partial sequence) the moment that happens (#1696 review).
    let stdout_handle = std::thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = stdout_pipe.read_to_end(&mut buf);
        buf
    });
    let stderr_handle = std::thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = stderr_pipe.read_to_end(&mut buf);
        buf
    });

    let start = Instant::now();
    let status = loop {
        if let Some(status) = child.try_wait().context("failed to poll child status")? {
            break status;
        }
        if start.elapsed() >= timeout {
            let _ = child.kill();
            let _ = child.wait();
            anyhow::bail!("command timed out after {timeout:?}");
        }
        std::thread::sleep(Duration::from_millis(50));
    };

    let stdout_bytes = stdout_handle.join().unwrap_or_default();
    let stderr_bytes = stderr_handle.join().unwrap_or_default();

    // Checked against the raw bytes, before either is decoded, so a
    // signal-killed child (OOM kill, network drop, another process's
    // `pkill`) is diagnosed by name instead of coerced into a fake `-1`
    // exit code (#1696) — and so that check can never itself fail to
    // decode (`String::from_utf8_lossy` below cannot fail).
    let exit_code = exit_code_or_signal_death(status, &stderr_bytes)?;

    Ok(ExecOutput {
        stdout: String::from_utf8_lossy(&stdout_bytes).into_owned(),
        stderr: String::from_utf8_lossy(&stderr_bytes).into_owned(),
        exit_code,
    })
}

/// Recursively copy the *contents* of `src` into `dst` (creating `dst` if
/// needed). Used by [`SystemSsh`] for localhost "transfers", which are just
/// same-filesystem copies. Missing `src` is not an error — a node that
/// produced no results yet is a normal, reportable-elsewhere condition.
fn copy_dir_contents(src: &Path, dst: &Path) -> Result<()> {
    if !src.exists() {
        return Ok(());
    }
    fs::create_dir_all(dst).with_context(|| format!("Failed to create {}", dst.display()))?;
    for entry in fs::read_dir(src).with_context(|| format!("Failed to read {}", src.display()))? {
        let entry = entry?;
        let dst_path = dst.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_dir_contents(&entry.path(), &dst_path)?;
        } else {
            fs::copy(entry.path(), &dst_path).with_context(|| {
                format!(
                    "Failed to copy {} -> {}",
                    entry.path().display(),
                    dst_path.display()
                )
            })?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::super::config::Architecture;
    use super::*;

    fn node(host: &str) -> NodeConfig {
        NodeConfig {
            name: "n".to_string(),
            host: host.to_string(),
            arch: Architecture::X86_64,
            features: vec![],
            max_concurrent: 1,
            ssh_key: None,
            working_dir: None,
            target_triple: None,
            ec2_instance_id: None,
            ec2_region: None,
        }
    }

    #[test]
    fn ssh_args_without_key() {
        let args = build_ssh_args(&node("example.com"), "echo hi", Duration::from_secs(5));
        assert!(!args.iter().any(|a| a == "-i"));
        assert!(args.contains(&"ConnectTimeout=5".to_string()));
        assert_eq!(args.last().unwrap(), "echo hi");
        assert_eq!(args[args.len() - 2], "example.com");
    }

    #[test]
    fn ssh_args_with_key() {
        let mut n = node("example.com");
        n.ssh_key = Some(std::path::PathBuf::from("/tmp/key.pem"));
        let args = build_ssh_args(&n, "echo hi", Duration::from_secs(30));
        assert_eq!(args[0], "-i");
        assert_eq!(args[1], "/tmp/key.pem");
        // ConnectTimeout is capped at 10s even for a long overall timeout.
        assert!(args.contains(&"ConnectTimeout=10".to_string()));
    }

    #[test]
    fn connect_timeout_is_clamped() {
        assert_eq!(connect_timeout_secs(Duration::from_secs(0)), 1);
        assert_eq!(connect_timeout_secs(Duration::from_secs(3)), 3);
        assert_eq!(connect_timeout_secs(Duration::from_secs(3600)), 10);
    }

    #[test]
    fn localhost_exec_runs_directly() {
        let out = SystemSsh
            .exec(&node("localhost"), "echo hello", Duration::from_secs(5))
            .unwrap();
        assert!(out.success());
        assert_eq!(out.stdout.trim(), "hello");
    }

    #[test]
    fn localhost_exec_captures_nonzero_exit() {
        let out = SystemSsh
            .exec(&node("127.0.0.1"), "exit 3", Duration::from_secs(5))
            .unwrap();
        assert_eq!(out.exit_code, 3);
        assert!(!out.success());
    }

    #[test]
    #[cfg(unix)]
    fn localhost_exec_signal_death_names_the_signal() {
        // `kill -9 $$` terminates the shell itself via SIGKILL (9), so
        // `ExitStatus::code()` returns `None` -- the case this test exists
        // to cover, rather than a plain nonzero exit.
        let err = SystemSsh
            .exec(&node("localhost"), "kill -9 $$", Duration::from_secs(5))
            .unwrap_err();
        assert!(
            err.to_string().contains("signal 9"),
            "expected error to name signal 9, got: {err}"
        );
    }

    #[test]
    #[cfg(unix)]
    fn localhost_exec_signal_death_preserves_truncated_utf8_stderr() {
        // Writes the first two bytes of a three-byte UTF-8 sequence (U+20AC)
        // to stderr, then kills itself with SIGKILL before ever completing
        // it -- exactly the truncated-multi-byte-sequence scenario that
        // made a strict `read_to_string` silently discard *all* captured
        // output, not just the trailing partial bytes (#1696 review). Only
        // a raw-bytes capture, decoded lossily after the exit-code check,
        // survives this: the invalid tail becomes a replacement character
        // instead of erasing the whole buffer.
        let err = SystemSsh
            .exec(
                &node("localhost"),
                "printf '\\xe2\\x82' >&2; kill -9 $$",
                Duration::from_secs(5),
            )
            .unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("signal 9"), "expected signal 9, got: {msg}");
        assert!(
            msg.contains('\u{FFFD}'),
            "expected the truncated stderr bytes to survive as a replacement character, got: {msg}"
        );
    }

    #[test]
    fn localhost_exec_times_out() {
        let err = SystemSsh
            .exec(&node("localhost"), "sleep 5", Duration::from_millis(100))
            .unwrap_err();
        assert!(err.to_string().contains("timed out"));
    }

    #[test]
    fn localhost_download_copies_directory_contents() {
        let tmp = tempfile::tempdir().unwrap();
        let src = tmp.path().join("src");
        let dst = tmp.path().join("dst");
        fs::create_dir_all(src.join("nested")).unwrap();
        fs::write(src.join("a.jsonl"), "{}\n").unwrap();
        fs::write(src.join("nested/b.jsonl"), "{}\n").unwrap();

        SystemSsh
            .download(
                &node("localhost"),
                src.to_str().unwrap(),
                &dst,
                Duration::from_secs(5),
            )
            .unwrap();

        assert!(dst.join("a.jsonl").exists());
        assert!(dst.join("nested/b.jsonl").exists());
    }

    #[test]
    fn localhost_download_missing_source_is_not_an_error() {
        let tmp = tempfile::tempdir().unwrap();
        let dst = tmp.path().join("dst");
        SystemSsh
            .download(
                &node("localhost"),
                tmp.path().join("missing").to_str().unwrap(),
                &dst,
                Duration::from_secs(5),
            )
            .unwrap();
    }
}
