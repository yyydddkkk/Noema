use std::{
    collections::{BTreeMap, BTreeSet},
    io::{Read, Write},
    net::{IpAddr, Ipv4Addr, SocketAddr, TcpStream},
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    thread,
    time::{Duration, Instant},
};

use nix::{
    sys::signal::{Signal, kill},
    unistd::Pid,
};
use noema_ir::{ArtifactRef, HealthSpec, ObservedWorkloadState, WorkloadId};

use crate::{ExecutionBackend, ExecutionFailure};

const ARTIFACT_NORMAL: &str = "builtin:noema-test-workload";
const ARTIFACT_CRASH: &str = "builtin:noema-test-workload:crash";
const ARTIFACT_TIMEOUT: &str = "builtin:noema-test-workload:startup-timeout";
const ARTIFACT_UNHEALTHY: &str = "builtin:noema-test-workload:unhealthy";

#[derive(Clone, Debug)]
struct ProcessSpec {
    artifact: ArtifactRef,
    health: HealthSpec,
}

#[derive(Clone, Debug)]
struct ProcessSnapshot {
    resolved: BTreeSet<ArtifactRef>,
    prepared: BTreeMap<WorkloadId, ProcessSpec>,
    running: BTreeSet<WorkloadId>,
    failed: BTreeSet<WorkloadId>,
}

/// Real child-process backend used inside Noema's M3 Docker laboratory.
///
/// It accepts only the built-in test Workload artifact family and never
/// evaluates a Shell string. Rollback terminates newly started processes and
/// restores the runtime membership captured at transaction start.
pub struct ProcessBackend {
    executable: PathBuf,
    resolved: BTreeSet<ArtifactRef>,
    prepared: BTreeMap<WorkloadId, ProcessSpec>,
    running: BTreeMap<WorkloadId, Child>,
    failed: BTreeSet<WorkloadId>,
    transaction: Option<ProcessSnapshot>,
}

impl ProcessBackend {
    #[must_use]
    pub fn new(executable: impl Into<PathBuf>) -> Self {
        Self {
            executable: executable.into(),
            resolved: BTreeSet::new(),
            prepared: BTreeMap::new(),
            running: BTreeMap::new(),
            failed: BTreeSet::new(),
            transaction: None,
        }
    }

    #[must_use]
    pub fn executable(&self) -> &Path {
        &self.executable
    }

    #[must_use]
    pub fn process_id(&self, workload: &WorkloadId) -> Option<u32> {
        self.running.get(workload).map(Child::id)
    }

    /// Forces an already-running child to exit for recovery testing.
    ///
    /// # Errors
    ///
    /// Returns an error if the workload is not running or cannot be killed.
    pub fn force_crash(&mut self, workload: &WorkloadId) -> Result<(), ExecutionFailure> {
        let mut child = self
            .running
            .remove(workload)
            .ok_or_else(|| ExecutionFailure::WorkloadNotPrepared(workload.clone()))?;
        child
            .kill()
            .map_err(|error| ExecutionFailure::runtime("force process crash", error))?;
        child
            .wait()
            .map_err(|error| ExecutionFailure::runtime("reap crashed process", error))?;
        self.failed.insert(workload.clone());
        Ok(())
    }

    fn artifact_mode(artifact: &ArtifactRef) -> Option<&'static str> {
        match artifact.as_str() {
            ARTIFACT_NORMAL => Some("normal"),
            ARTIFACT_CRASH => Some("crash"),
            ARTIFACT_TIMEOUT => Some("startup-timeout"),
            ARTIFACT_UNHEALTHY => Some("unhealthy"),
            _ => None,
        }
    }

    fn spawn_process(&self, spec: &ProcessSpec) -> Result<Child, ExecutionFailure> {
        let mode = Self::artifact_mode(&spec.artifact)
            .ok_or_else(|| ExecutionFailure::UnsupportedArtifact(spec.artifact.clone()))?;
        let mut command = Command::new(&self.executable);
        command
            .arg("--mode")
            .arg(mode)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        if let HealthSpec::Http { port, .. } = spec.health {
            command.arg("--port").arg(port.to_string());
        }
        command
            .spawn()
            .map_err(|error| ExecutionFailure::runtime("spawn workload", error))
    }

    fn start_prepared(
        &mut self,
        workload: &WorkloadId,
    ) -> Result<ObservedWorkloadState, ExecutionFailure> {
        let spec = self
            .prepared
            .get(workload)
            .cloned()
            .ok_or_else(|| ExecutionFailure::WorkloadNotPrepared(workload.clone()))?;
        let mut child = self.spawn_process(&spec)?;
        let deadline = Instant::now() + Duration::from_millis(75);
        while Instant::now() < deadline {
            if let Some(status) = child
                .try_wait()
                .map_err(|error| ExecutionFailure::runtime("observe startup", error))?
            {
                return Err(ExecutionFailure::ProcessExited {
                    workload: workload.clone(),
                    code: status.code(),
                });
            }
            thread::sleep(Duration::from_millis(5));
        }
        self.failed.remove(workload);
        self.running.insert(workload.clone(), child);
        Ok(ObservedWorkloadState::Running)
    }

    fn stop_process(&mut self, workload: &WorkloadId) -> Result<(), ExecutionFailure> {
        let Some(mut child) = self.running.remove(workload) else {
            self.failed.remove(workload);
            return Ok(());
        };
        terminate(&mut child)?;
        self.failed.remove(workload);
        Ok(())
    }

    fn refresh_process(&mut self, workload: &WorkloadId) -> Result<(), ExecutionFailure> {
        let exited = if let Some(child) = self.running.get_mut(workload) {
            child
                .try_wait()
                .map_err(|error| ExecutionFailure::runtime("observe process", error))?
                .is_some()
        } else {
            false
        };
        if exited {
            self.running.remove(workload);
            self.failed.insert(workload.clone());
        }
        Ok(())
    }

    fn restore_snapshot(&mut self, snapshot: ProcessSnapshot) -> Result<(), ExecutionFailure> {
        let snapshot_failed = snapshot.failed;
        let current: Vec<_> = self.running.keys().cloned().collect();
        for workload in current {
            if !snapshot.running.contains(&workload) {
                self.stop_process(&workload)?;
            }
        }
        self.resolved = snapshot.resolved;
        self.prepared = snapshot.prepared;
        self.failed = snapshot_failed.clone();
        for workload in snapshot.running {
            self.refresh_process(&workload)?;
            if !self.running.contains_key(&workload) {
                self.start_prepared(&workload)?;
            }
        }
        self.failed = snapshot_failed;
        Ok(())
    }
}

impl ExecutionBackend for ProcessBackend {
    fn begin_transaction(&mut self) -> Result<(), ExecutionFailure> {
        if self.transaction.is_some() {
            return Err(ExecutionFailure::TransactionAlreadyActive);
        }
        let workloads: Vec<_> = self.running.keys().cloned().collect();
        for workload in workloads {
            self.refresh_process(&workload)?;
        }
        self.transaction = Some(ProcessSnapshot {
            resolved: self.resolved.clone(),
            prepared: self.prepared.clone(),
            running: self.running.keys().cloned().collect(),
            failed: self.failed.clone(),
        });
        Ok(())
    }

    fn commit_transaction(&mut self) -> Result<(), ExecutionFailure> {
        self.transaction
            .take()
            .ok_or(ExecutionFailure::NoActiveTransaction)?;
        Ok(())
    }

    fn rollback_transaction(&mut self) -> Result<(), ExecutionFailure> {
        let snapshot = self
            .transaction
            .take()
            .ok_or(ExecutionFailure::NoActiveTransaction)?;
        self.restore_snapshot(snapshot)
    }

    fn resolve(&mut self, artifact: &ArtifactRef) -> Result<(), ExecutionFailure> {
        if Self::artifact_mode(artifact).is_none() || !self.executable.is_file() {
            return Err(ExecutionFailure::UnsupportedArtifact(artifact.clone()));
        }
        self.resolved.insert(artifact.clone());
        Ok(())
    }

    fn prepare(
        &mut self,
        workload: &WorkloadId,
        artifact: &ArtifactRef,
        health: &HealthSpec,
    ) -> Result<ObservedWorkloadState, ExecutionFailure> {
        if !self.resolved.contains(artifact) {
            return Err(ExecutionFailure::ArtifactNotResolved(artifact.clone()));
        }
        self.prepared.insert(
            workload.clone(),
            ProcessSpec {
                artifact: artifact.clone(),
                health: health.clone(),
            },
        );
        Ok(ObservedWorkloadState::Stopped)
    }

    fn start(&mut self, workload: &WorkloadId) -> Result<ObservedWorkloadState, ExecutionFailure> {
        self.refresh_process(workload)?;
        if self.running.contains_key(workload) && !self.failed.contains(workload) {
            return Ok(ObservedWorkloadState::Running);
        }
        if self.running.contains_key(workload) {
            self.stop_process(workload)?;
        }
        self.start_prepared(workload)
    }

    fn stop(&mut self, workload: &WorkloadId) -> Result<ObservedWorkloadState, ExecutionFailure> {
        self.stop_process(workload)?;
        Ok(if self.prepared.contains_key(workload) {
            ObservedWorkloadState::Stopped
        } else {
            ObservedWorkloadState::Absent
        })
    }

    fn remove(&mut self, workload: &WorkloadId) -> Result<ObservedWorkloadState, ExecutionFailure> {
        self.stop_process(workload)?;
        self.prepared.remove(workload);
        Ok(ObservedWorkloadState::Absent)
    }

    fn check_health(&mut self, workload: &WorkloadId) -> Result<bool, ExecutionFailure> {
        if self.observed(workload)? != ObservedWorkloadState::Running {
            return Ok(false);
        }
        let health = self
            .prepared
            .get(workload)
            .map(|spec| spec.health.clone())
            .ok_or_else(|| ExecutionFailure::WorkloadNotPrepared(workload.clone()))?;
        match health {
            HealthSpec::None => Ok(false),
            HealthSpec::Process => Ok(true),
            HealthSpec::Http {
                port,
                path,
                timeout_ms,
            } => Ok(probe_http(port, &path, Duration::from_millis(timeout_ms))),
        }
    }

    fn observed(
        &mut self,
        workload: &WorkloadId,
    ) -> Result<ObservedWorkloadState, ExecutionFailure> {
        self.refresh_process(workload)?;
        if self.failed.contains(workload) {
            Ok(ObservedWorkloadState::Failed)
        } else if self.running.contains_key(workload) {
            Ok(ObservedWorkloadState::Running)
        } else if self.prepared.contains_key(workload) {
            Ok(ObservedWorkloadState::Stopped)
        } else {
            Ok(ObservedWorkloadState::Absent)
        }
    }

    fn mark_failed(&mut self, workload: &WorkloadId) -> Result<(), ExecutionFailure> {
        if !self.prepared.contains_key(workload) {
            return Err(ExecutionFailure::WorkloadNotPrepared(workload.clone()));
        }
        self.failed.insert(workload.clone());
        Ok(())
    }

    fn name(&self) -> &'static str {
        "process"
    }
}

impl Drop for ProcessBackend {
    fn drop(&mut self) {
        let workloads: Vec<_> = self.running.keys().cloned().collect();
        for workload in workloads {
            let _ = self.stop_process(&workload);
        }
    }
}

fn terminate(child: &mut Child) -> Result<(), ExecutionFailure> {
    if child
        .try_wait()
        .map_err(|error| ExecutionFailure::runtime("observe before stop", error))?
        .is_some()
    {
        return Ok(());
    }
    let raw_pid = i32::try_from(child.id())
        .map_err(|error| ExecutionFailure::runtime("convert process identifier", error))?;
    kill(Pid::from_raw(raw_pid), Signal::SIGTERM)
        .map_err(|error| ExecutionFailure::runtime("send SIGTERM", error))?;
    let deadline = Instant::now() + Duration::from_secs(1);
    while Instant::now() < deadline {
        if child
            .try_wait()
            .map_err(|error| ExecutionFailure::runtime("wait for SIGTERM", error))?
            .is_some()
        {
            return Ok(());
        }
        thread::sleep(Duration::from_millis(10));
    }
    child
        .kill()
        .map_err(|error| ExecutionFailure::runtime("send SIGKILL", error))?;
    child
        .wait()
        .map_err(|error| ExecutionFailure::runtime("reap killed process", error))?;
    Ok(())
}

fn probe_http(port: u16, path: &str, timeout: Duration) -> bool {
    let deadline = Instant::now() + timeout;
    let address = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), port);
    while Instant::now() < deadline {
        let remaining = deadline.saturating_duration_since(Instant::now());
        let connect_timeout = remaining.min(Duration::from_millis(50));
        if let Ok(mut stream) = TcpStream::connect_timeout(&address, connect_timeout) {
            let _ = stream.set_read_timeout(Some(remaining));
            let _ = stream.set_write_timeout(Some(remaining));
            let request =
                format!("GET {path} HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n");
            if stream.write_all(request.as_bytes()).is_ok() {
                let mut response = String::new();
                if stream.read_to_string(&mut response).is_ok() {
                    return response.starts_with("HTTP/1.1 200 ");
                }
            }
        }
        thread::sleep(Duration::from_millis(10));
    }
    false
}
