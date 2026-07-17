use async_trait::async_trait;
use std::collections::BTreeMap;
use std::fmt;
use std::path::PathBuf;
use std::process::Stdio;
use std::time::{Duration, Instant};
use thiserror::Error;
use tokio::io::{AsyncRead, AsyncReadExt};
use tokio::process::{Child, Command};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

#[cfg(windows)]
use windows_sys::Win32::Foundation::{CloseHandle, HANDLE};
#[cfg(windows)]
use windows_sys::Win32::System::JobObjects::{
    AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
    JOBOBJECT_BASIC_PROCESS_ID_LIST, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
    JobObjectBasicProcessIdList, JobObjectExtendedLimitInformation, QueryInformationJobObject,
    SetInformationJobObject, TerminateJobObject,
};
#[cfg(windows)]
use windows_sys::Win32::System::Threading::CREATE_SUSPENDED;

/// The maximum amount retained from stdout and stderr independently.
pub const MAX_CAPTURE_BYTES_PER_STREAM: usize = 1024 * 1024;

const CLEANUP_TIMEOUT: Duration = Duration::from_secs(5);
const JOB_POLL_INTERVAL: Duration = Duration::from_millis(10);

#[derive(Clone, PartialEq, Eq)]
pub struct ProcessSpec {
    pub program: String,
    pub args: Vec<String>,
    pub current_dir: PathBuf,
    pub env: BTreeMap<String, String>,
    pub timeout: Duration,
}

impl fmt::Debug for ProcessSpec {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ProcessSpec")
            .field("program", &self.program)
            .field("args", &self.args)
            .field("current_dir", &self.current_dir)
            .field("env_keys", &self.env.keys().collect::<Vec<_>>())
            .field("timeout", &self.timeout)
            .finish()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProcessOutput {
    pub exit_code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
    pub duration_ms: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutputStream {
    Stdout,
    Stderr,
}

#[derive(Debug, Error)]
pub enum ProcessError {
    #[error("process could not start: {0}")]
    Spawn(#[source] std::io::Error),
    #[error("process supervision could not start: {0}")]
    Supervision(#[source] std::io::Error),
    #[error("process could not be awaited: {0}")]
    Wait(#[source] std::io::Error),
    #[error("process output capture failed")]
    CaptureFailed,
    #[error("process was cancelled")]
    Cancelled,
    #[error("process exceeded the agent budget")]
    TimedOut,
    #[error("process {stream:?} exceeded the capture limit")]
    OutputLimit { stream: OutputStream },
    #[error("process tree cleanup could not be confirmed")]
    TerminationFailed,
    #[error("process duration exceeds the supported range")]
    DurationOverflow,
}

#[async_trait]
pub trait ProcessRunner: Send + Sync {
    async fn run(
        &self,
        spec: ProcessSpec,
        cancellation: CancellationToken,
    ) -> Result<ProcessOutput, ProcessError>;
}

pub struct TokioProcessRunner;

#[async_trait]
impl ProcessRunner for TokioProcessRunner {
    async fn run(
        &self,
        spec: ProcessSpec,
        cancellation: CancellationToken,
    ) -> Result<ProcessOutput, ProcessError> {
        if cancellation.is_cancelled() {
            return Err(ProcessError::Cancelled);
        }

        let started = Instant::now();
        let mut supervisor = ProcessSupervisor::new().map_err(ProcessError::Supervision)?;
        let mut command = Command::new(&spec.program);
        command
            .args(&spec.args)
            .current_dir(&spec.current_dir)
            .envs(&spec.env)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        supervisor.configure_command(&mut command);

        let mut child = command.spawn().map_err(ProcessError::Spawn)?;
        if let Err(error) = supervisor.assign_and_resume(&child) {
            return finish_with_error(
                &mut child,
                &mut supervisor,
                ProcessError::Supervision(error),
            )
            .await;
        }

        let stdout = match child.stdout.take() {
            Some(stdout) => stdout,
            None => {
                return finish_with_error(&mut child, &mut supervisor, ProcessError::CaptureFailed)
                    .await;
            }
        };
        let stderr = match child.stderr.take() {
            Some(stderr) => stderr,
            None => {
                return finish_with_error(&mut child, &mut supervisor, ProcessError::CaptureFailed)
                    .await;
            }
        };

        let (events, mut event_receiver) = mpsc::unbounded_channel();
        let _stdout_capture =
            tokio::spawn(capture_stream(stdout, OutputStream::Stdout, events.clone()));
        let _stderr_capture = tokio::spawn(capture_stream(stderr, OutputStream::Stderr, events));
        let timeout = tokio::time::sleep(spec.timeout);
        tokio::pin!(timeout);
        let mut job_poll = tokio::time::interval(JOB_POLL_INTERVAL);
        job_poll.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

        let mut stdout = None;
        let mut stderr = None;
        let mut exit_code = None;

        loop {
            tokio::select! {
                status = child.wait(), if exit_code.is_none() => {
                    exit_code = match status {
                        Ok(status) => Some(status.code()),
                        Err(error) => return finish_with_error(
                            &mut child,
                            &mut supervisor,
                            ProcessError::Wait(error),
                        ).await,
                    };
                }
                event = event_receiver.recv(), if stdout.is_none() || stderr.is_none() => {
                    match event {
                        Some(CaptureEvent::Completed(OutputStream::Stdout, output)) => stdout = Some(output),
                        Some(CaptureEvent::Completed(OutputStream::Stderr, output)) => stderr = Some(output),
                        Some(CaptureEvent::LimitExceeded(stream)) => return finish_with_error(
                            &mut child,
                            &mut supervisor,
                            ProcessError::OutputLimit { stream },
                        ).await,
                        Some(CaptureEvent::Failed) | None => return finish_with_error(
                            &mut child,
                            &mut supervisor,
                            ProcessError::CaptureFailed,
                        ).await,
                    }
                }
                _ = cancellation.cancelled() => return finish_with_error(
                    &mut child,
                    &mut supervisor,
                    ProcessError::Cancelled,
                ).await,
                _ = &mut timeout => return finish_with_error(
                    &mut child,
                    &mut supervisor,
                    ProcessError::TimedOut,
                ).await,
                _ = job_poll.tick(), if exit_code.is_some() && stdout.is_some() && stderr.is_some() => {}
            }

            if exit_code.is_some() && stdout.is_some() && stderr.is_some() {
                match supervisor.is_empty() {
                    Ok(true) => {
                        return Ok(ProcessOutput {
                            exit_code: exit_code.take().expect("checked above"),
                            stdout: stdout.take().expect("checked above"),
                            stderr: stderr.take().expect("checked above"),
                            duration_ms: elapsed_ms(started)?,
                        });
                    }
                    Ok(false) => {}
                    Err(error) => {
                        return finish_with_error(
                            &mut child,
                            &mut supervisor,
                            ProcessError::Supervision(error),
                        )
                        .await;
                    }
                }
            }
        }
    }
}

enum CaptureEvent {
    Completed(OutputStream, String),
    LimitExceeded(OutputStream),
    Failed,
}

async fn capture_stream<R>(
    mut reader: R,
    stream: OutputStream,
    events: mpsc::UnboundedSender<CaptureEvent>,
) where
    R: AsyncRead + Unpin,
{
    let mut output = Vec::with_capacity(MAX_CAPTURE_BYTES_PER_STREAM.min(8192));
    let mut buffer = [0_u8; 8192];

    loop {
        let read = match reader.read(&mut buffer).await {
            Ok(read) => read,
            Err(_) => {
                let _ = events.send(CaptureEvent::Failed);
                return;
            }
        };
        if read == 0 {
            let _ = events.send(CaptureEvent::Completed(
                stream,
                String::from_utf8_lossy(&output).into_owned(),
            ));
            return;
        }
        if read > MAX_CAPTURE_BYTES_PER_STREAM.saturating_sub(output.len()) {
            let _ = events.send(CaptureEvent::LimitExceeded(stream));
            return;
        }
        output.extend_from_slice(&buffer[..read]);
    }
}

async fn finish_with_error(
    child: &mut Child,
    supervisor: &mut ProcessSupervisor,
    error: ProcessError,
) -> Result<ProcessOutput, ProcessError> {
    match supervisor.terminate_and_confirm(child).await {
        Ok(()) => Err(error),
        Err(()) => Err(ProcessError::TerminationFailed),
    }
}

#[cfg(windows)]
struct ProcessSupervisor {
    job: WindowsJob,
    assigned: bool,
}

#[cfg(windows)]
impl ProcessSupervisor {
    fn new() -> std::io::Result<Self> {
        Ok(Self {
            job: WindowsJob::new()?,
            assigned: false,
        })
    }

    fn configure_command(&self, command: &mut Command) {
        command.creation_flags(CREATE_SUSPENDED);
    }

    fn assign_and_resume(&mut self, child: &Child) -> std::io::Result<()> {
        self.job.assign(child)?;
        self.assigned = true;
        self.job.resume(child)
    }

    fn is_empty(&self) -> std::io::Result<bool> {
        self.job.is_empty()
    }

    async fn terminate_and_confirm(&mut self, child: &mut Child) -> Result<(), ()> {
        if self.assigned {
            self.job.terminate().map_err(|_| ())?;
            let deadline = tokio::time::Instant::now() + CLEANUP_TIMEOUT;
            loop {
                if self.job.is_empty().map_err(|_| ())? {
                    break;
                }
                if tokio::time::Instant::now() >= deadline {
                    return Err(());
                }
                tokio::time::sleep(JOB_POLL_INTERVAL).await;
            }
        } else {
            child.start_kill().map_err(|_| ())?;
        }

        tokio::time::timeout(CLEANUP_TIMEOUT, child.wait())
            .await
            .map_err(|_| ())?
            .map_err(|_| ())?;
        Ok(())
    }
}

#[cfg(windows)]
struct WindowsJob {
    handle: HANDLE,
}

#[cfg(windows)]
unsafe impl Send for WindowsJob {}

#[cfg(windows)]
impl WindowsJob {
    fn new() -> std::io::Result<Self> {
        let handle = unsafe { CreateJobObjectW(std::ptr::null(), std::ptr::null()) };
        if handle.is_null() {
            return Err(std::io::Error::last_os_error());
        }
        let job = Self { handle };
        let mut limits = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
        limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        let configured = unsafe {
            SetInformationJobObject(
                job.handle,
                JobObjectExtendedLimitInformation,
                &limits as *const _ as *const _,
                std::mem::size_of_val(&limits) as u32,
            )
        };
        if configured == 0 {
            return Err(std::io::Error::last_os_error());
        }
        Ok(job)
    }

    fn assign(&self, child: &Child) -> std::io::Result<()> {
        let process = child
            .raw_handle()
            .ok_or_else(|| std::io::Error::other("missing process handle"))?;
        let assigned = unsafe { AssignProcessToJobObject(self.handle, process as HANDLE) };
        if assigned == 0 {
            return Err(std::io::Error::last_os_error());
        }
        Ok(())
    }

    fn resume(&self, child: &Child) -> std::io::Result<()> {
        let process = child
            .raw_handle()
            .ok_or_else(|| std::io::Error::other("missing process handle"))?;
        let status = unsafe { NtResumeProcess(process as HANDLE) };
        if status != 0 {
            return Err(std::io::Error::from_raw_os_error(status));
        }
        Ok(())
    }

    fn terminate(&self) -> std::io::Result<()> {
        let terminated = unsafe { TerminateJobObject(self.handle, 1) };
        if terminated == 0 {
            return Err(std::io::Error::last_os_error());
        }
        Ok(())
    }

    fn is_empty(&self) -> std::io::Result<bool> {
        let mut process_ids = vec![
            0_usize;
            (std::mem::size_of::<JOBOBJECT_BASIC_PROCESS_ID_LIST>()
                + 1024 * std::mem::size_of::<usize>())
            .div_ceil(std::mem::size_of::<usize>())
        ];
        let queried = unsafe {
            QueryInformationJobObject(
                self.handle,
                JobObjectBasicProcessIdList,
                process_ids.as_mut_ptr() as *mut _,
                std::mem::size_of_val(process_ids.as_slice()) as u32,
                std::ptr::null_mut(),
            )
        };
        if queried == 0 {
            return Err(std::io::Error::last_os_error());
        }
        let process_list = process_ids.as_ptr() as *const JOBOBJECT_BASIC_PROCESS_ID_LIST;
        Ok(unsafe { (*process_list).NumberOfAssignedProcesses == 0 })
    }
}

#[cfg(windows)]
impl Drop for WindowsJob {
    fn drop(&mut self) {
        unsafe {
            CloseHandle(self.handle);
        }
    }
}

#[cfg(windows)]
#[link(name = "ntdll")]
unsafe extern "system" {
    fn NtResumeProcess(process_handle: HANDLE) -> i32;
}

#[cfg(not(windows))]
struct ProcessSupervisor;

#[cfg(not(windows))]
impl ProcessSupervisor {
    fn new() -> std::io::Result<Self> {
        Ok(Self)
    }

    fn configure_command(&self, _command: &mut Command) {}

    fn assign_and_resume(&mut self, _child: &Child) -> std::io::Result<()> {
        Ok(())
    }

    fn is_empty(&self) -> std::io::Result<bool> {
        Ok(true)
    }

    async fn terminate_and_confirm(&mut self, child: &mut Child) -> Result<(), ()> {
        child.start_kill().map_err(|_| ())?;
        tokio::time::timeout(CLEANUP_TIMEOUT, child.wait())
            .await
            .map_err(|_| ())?
            .map_err(|_| ())?;
        Ok(())
    }
}

fn elapsed_ms(started: Instant) -> Result<u64, ProcessError> {
    u64::try_from(started.elapsed().as_millis()).map_err(|_| ProcessError::DurationOverflow)
}
