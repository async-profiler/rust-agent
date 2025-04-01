// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

use crate::{
    asprof::{self, AsProfError},
    metadata::{aws::AwsProfilerMetadataError, AgentMetadata, ReportMetadata},
    reporter::Reporter,
};
use std::{
    fs::File,
    io,
    path::{Path, PathBuf},
    time::{Duration, SystemTime, SystemTimeError, UNIX_EPOCH},
};
use thiserror::Error;

struct JfrFile {
    active: std::fs::File,
    inactive: std::fs::File,
}

impl JfrFile {
    #[cfg(target_os = "linux")]
    fn new() -> Result<Self, io::Error> {
        Ok(Self {
            active: tempfile::tempfile().unwrap(),
            inactive: tempfile::tempfile().unwrap(),
        })
    }

    #[cfg(not(target_os = "linux"))]
    fn new() -> Result<Self, io::Error> {
        io::Error::new(
            io::ErrorKind::Other,
            "async-profiler is only supported on Linux",
        )
    }

    fn swap(&mut self) {
        std::mem::swap(&mut self.active, &mut self.inactive);
    }

    #[cfg(target_os = "linux")]
    fn file_path(file: &std::fs::File) -> PathBuf {
        use std::os::fd::AsRawFd;

        format!("/proc/self/fd/{}", file.as_raw_fd()).into()
    }

    #[cfg(not(target_os = "linux"))]
    fn file_path(_file: &std::fs::File) -> PathBuf {
        unimplemented!()
    }

    fn active_path(&self) -> PathBuf {
        Self::file_path(&self.active)
    }

    fn inactive_path(&self) -> PathBuf {
        Self::file_path(&self.inactive)
    }

    fn empty_inactive_file(&mut self) -> Result<(), io::Error> {
        // Empty the file, or create it for the first time if the profiler hasn't
        // started yet.
        File::create(Self::file_path(&self.inactive))?;
        tracing::debug!(message = "emptied the file");
        Ok(())
    }
}

/// Builds a [`Profiler`], panicking if any required fields were not set by the
/// time `build` is called. Most users should be using [`Profiler::new`] which
/// requires all known parameters up front; this type exists for those needing
/// fine-grained customization.
#[derive(Debug, Default)]
pub struct ProfilerBuilder {
    reporting_interval: Option<Duration>,
    reporter: Option<Box<dyn Reporter + Send + Sync>>,
    agent_metadata: Option<AgentMetadata>,
}

impl ProfilerBuilder {
    /// Sets the reporting interval.
    pub fn with_reporting_interval(mut self, i: Duration) -> ProfilerBuilder {
        self.reporting_interval = Some(i);
        self
    }

    /// Sets the reporter.
    pub fn with_reporter(mut self, r: impl Reporter + Send + Sync + 'static) -> ProfilerBuilder {
        self.reporter = Some(Box::new(r));
        self
    }

    /// Provide custom agent metadata.
    pub fn with_custom_agent_metadata(mut self, j: AgentMetadata) -> ProfilerBuilder {
        self.agent_metadata = Some(j);
        self
    }

    /// Turn this builder into a profiler!
    pub fn build(self) -> Profiler {
        Profiler {
            reporting_interval: self.reporting_interval.unwrap_or(Duration::from_secs(30)),
            reporter: self.reporter.expect("reporter is required"),
            agent_metadata: self.agent_metadata,
        }
    }
}

enum Status {
    Idle,
    Starting,
    Running(SystemTime),
}

/// This type provides wrapper APIs over [`asprof::AsProf`], to allow tracking
/// of the state of the profiler. The primary benefit of this is RAII - when
/// this type drops, it will stop the profiler if it's running.
struct ProfilerState<E: ProfilerEngine> {
    // this is only None in the destructor when stopping the async-profiler fails
    jfr_file: Option<JfrFile>,
    asprof: E,
    status: Status,
}

impl<E: ProfilerEngine> ProfilerState<E> {
    pub fn new(asprof: E) -> Result<Self, io::Error> {
        Ok(Self {
            jfr_file: Some(JfrFile::new()?),
            asprof,
            status: Status::Idle,
        })
    }

    pub fn jfr_file_mut(&mut self) -> &mut JfrFile {
        self.jfr_file.as_mut().unwrap()
    }

    async fn start(&mut self) -> Result<(), AsProfError> {
        let active = self.jfr_file.as_ref().unwrap().active_path();
        // drop guard - make sure the files are leaked if the profiler might have started
        self.status = Status::Starting;
        E::start_async_profiler(&self.asprof, &active)?;
        self.status = Status::Running(SystemTime::now());
        Ok(())
    }

    fn stop(&mut self) -> Result<Option<SystemTime>, AsProfError> {
        E::stop_async_profiler()?;
        let status = std::mem::replace(&mut self.status, Status::Idle);
        Ok(match status {
            Status::Idle | Status::Starting => None,
            Status::Running(since) => Some(since),
        })
    }

    fn is_started(&self) -> bool {
        matches!(self.status, Status::Running(_))
    }
}

impl<E: ProfilerEngine> Drop for ProfilerState<E> {
    fn drop(&mut self) {
        match self.status {
            Status::Running(_) => {
                if let Err(err) = self.stop() {
                    // SECURITY: avoid removing the JFR file if stopping the profiler fails,
                    // to avoid symlink races
                    std::mem::forget(self.jfr_file.take());
                    // XXX: Rust defines leaking resources during drop as safe.
                    tracing::warn!(?err, "unable to stop profiler during drop glue");
                }
            }
            Status::Idle => {}
            Status::Starting => {
                // SECURITY: avoid removing the JFR file if stopping the profiler fails,
                // to avoid symlink races
                std::mem::forget(self.jfr_file.take());
            }
        }
    }
}

pub(crate) trait ProfilerEngine: Send + Sync + 'static {
    fn init_async_profiler() -> Result<(), asprof::AsProfError>;
    fn start_async_profiler(&self, jfr_file_path: &Path) -> Result<(), asprof::AsProfError>;
    fn stop_async_profiler() -> Result<(), asprof::AsProfError>;
}

#[derive(Debug, Error)]
#[non_exhaustive]
enum TickError {
    #[error(transparent)]
    AsProf(#[from] AsProfError),
    #[error(transparent)]
    Metadata(#[from] AwsProfilerMetadataError),
    #[error("reporter: {0}")]
    Reporter(Box<dyn std::error::Error + Send>),
    #[error("broken clock: {0}")]
    BrokenClock(#[from] SystemTimeError),
    #[error("jfr read error: {0}")]
    JfrRead(io::Error),
    #[error("empty inactive file error: {0}")]
    EmptyInactiveFile(io::Error),
}

#[derive(Debug, Error)]
#[non_exhaustive]
pub enum SpawnError {
    #[error(transparent)]
    AsProf(#[from] asprof::AsProfError),
    #[error("tempfile error: {0}")]
    TempFile(io::Error),
}

/// Rust profiler based on [async-profiler].
///
/// [async-profiler]: https://github.com/async-profiler/async-profiler
pub struct Profiler {
    reporting_interval: Duration,
    reporter: Box<dyn Reporter + Send + Sync>,
    agent_metadata: Option<AgentMetadata>,
}

impl Profiler {
    /// Start profiling. The profiler will run in a tokio task at the configured interval.
    pub fn spawn(self) -> Result<tokio::task::JoinHandle<()>, SpawnError> {
        self.spawn_inner(asprof::AsProf::builder().build())
    }

    fn spawn_inner<E: ProfilerEngine>(
        self,
        asprof: E,
    ) -> Result<tokio::task::JoinHandle<()>, SpawnError> {
        // Initialize async profiler - needs to be done once.
        E::init_async_profiler()?;
        tracing::info!("successfully initialized async profiler.");

        let mut sampling_ticker = tokio::time::interval(self.reporting_interval);

        // Get profiles at the configured interval rate.
        Ok(tokio::spawn(async move {
            let mut state = match ProfilerState::new(asprof) {
                Ok(state) => state,
                Err(err) => {
                    tracing::error!(?err, "unable to create profiler state");
                    return;
                }
            };

            // Lazily-loaded if not specified up front.
            let mut agent_metadata = self.agent_metadata;

            loop {
                // Start timer.
                let _ = sampling_ticker.tick().await;
                tracing::debug!("profiler timer woke up");

                if let Err(err) = profiler_tick(
                    &mut state,
                    &mut agent_metadata,
                    &*self.reporter,
                    self.reporting_interval,
                )
                .await
                {
                    match &err {
                        TickError::Reporter(_) => {
                            // don't stop on IO errors
                            tracing::error!(?err, "error during profiling, continuing");
                        }
                        _stop => {
                            tracing::error!(?err, "error during profiling, stopping");
                            break;
                        }
                    }
                }
            }

            tracing::info!("profiling task finished");
        }))
    }
}

async fn profiler_tick<E: ProfilerEngine>(
    state: &mut ProfilerState<E>,
    agent_metadata: &mut Option<AgentMetadata>,
    reporter: &(dyn Reporter + Send + Sync),
    reporting_interval: Duration,
) -> Result<(), TickError> {
    if !state.is_started() {
        state.start().await?;
        return Ok(());
    }

    let Some(start) = state.stop()? else {
        tracing::warn!("stopped the profiler but it wasn't running?");
        return Ok(());
    };
    let start = start.duration_since(UNIX_EPOCH)?;
    let end = SystemTime::now().duration_since(UNIX_EPOCH)?;

    // Start it up immediately, writing to the "other" file, so that we keep
    // profiling the application while we're reporting data.
    state
        .jfr_file_mut()
        .empty_inactive_file()
        .map_err(TickError::EmptyInactiveFile)?;
    state.jfr_file_mut().swap();
    state.start().await?;

    // Lazily load the agent metadata if it was not provided in
    // the constructor. See the struct comments for why this is.
    // This code runs at most once.
    if agent_metadata.is_none() {
        #[cfg(feature = "aws-metadata")]
        let md = crate::metadata::aws::load_agent_metadata().await?;
        #[cfg(not(feature = "aws-metadata"))]
        let md = crate::metadata::AgentMetadata::Other;
        tracing::debug!("loaded metadata");
        agent_metadata.replace(md);
    }

    let report_metadata = ReportMetadata {
        instance: agent_metadata.as_ref().unwrap(),
        start,
        end,
        reporting_interval,
    };

    let jfr = tokio::fs::read(state.jfr_file_mut().inactive_path())
        .await
        .map_err(TickError::JfrRead)?;

    reporter
        .report(jfr, &report_metadata)
        .await
        .map_err(TickError::Reporter)?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{self, AtomicBool, AtomicU32};
    use std::sync::Arc;

    use test_case::test_case;

    use super::*;

    #[test]
    fn test_jfr_file_drop() {
        let mut jfr = JfrFile::new().unwrap();

        std::fs::write(jfr.active_path(), b"Hello, 2!").unwrap();
        jfr.swap();
        assert_eq!(std::fs::read(jfr.inactive_path()).unwrap(), b"Hello, 2!");
        jfr.empty_inactive_file().unwrap();
        assert_eq!(std::fs::read(jfr.inactive_path()).unwrap(), b"");
    }

    struct MockProfilerEngine {
        counter: AtomicU32,
    }
    impl ProfilerEngine for MockProfilerEngine {
        fn init_async_profiler() -> Result<(), asprof::AsProfError> {
            Ok(())
        }

        fn start_async_profiler(&self, jfr_file_path: &Path) -> Result<(), asprof::AsProfError> {
            let contents = format!(
                "JFR{}",
                self.counter.fetch_add(1, atomic::Ordering::Relaxed)
            );
            std::fs::write(jfr_file_path, contents.as_bytes()).unwrap();
            Ok(())
        }

        fn stop_async_profiler() -> Result<(), asprof::AsProfError> {
            Ok(())
        }
    }

    struct MockReporter(tokio::sync::mpsc::Sender<(String, AgentMetadata)>);
    impl std::fmt::Debug for MockReporter {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.debug_struct("MockReporter").finish()
        }
    }

    #[async_trait::async_trait]
    impl Reporter for MockReporter {
        async fn report(
            &self,
            jfr: Vec<u8>,
            metadata: &ReportMetadata,
        ) -> Result<(), Box<dyn std::error::Error + Send>> {
            self.0
                .send((String::from_utf8(jfr).unwrap(), metadata.instance.clone()))
                .await
                .unwrap();
            Ok(())
        }
    }

    fn make_mock_profiler() -> (
        Profiler,
        tokio::sync::mpsc::Receiver<(String, AgentMetadata)>,
    ) {
        let (tx, rx) = tokio::sync::mpsc::channel(1);
        let agent = ProfilerBuilder::default()
            .with_reporter(MockReporter(tx))
            .with_custom_agent_metadata(AgentMetadata::Ec2AgentMetadata {
                aws_account_id: "0".into(),
                aws_region_id: "us-east-1".into(),
                ec2_instance_id: "i-fake".into(),
            })
            .build();
        (agent, rx)
    }

    #[tokio::test(start_paused = true)]
    async fn test_profiler_agent() {
        let e_md = AgentMetadata::Ec2AgentMetadata {
            aws_account_id: "0".into(),
            aws_region_id: "us-east-1".into(),
            ec2_instance_id: "i-fake".into(),
        };
        let (agent, mut rx) = make_mock_profiler();
        agent
            .spawn_inner::<MockProfilerEngine>(MockProfilerEngine {
                counter: AtomicU32::new(0),
            })
            .unwrap();
        let (jfr, md) = rx.recv().await.unwrap();
        assert_eq!(jfr, "JFR0");
        assert_eq!(e_md, md);
        let (jfr, md) = rx.recv().await.unwrap();
        assert_eq!(jfr, "JFR1");
        assert_eq!(e_md, md);
    }

    // simulate a badly-behaved profiler that errors on start/stop and then
    // tries to access the JFR file
    struct StopErrorProfilerEngine {
        start_error: bool,
        counter: Arc<AtomicBool>,
    }
    impl ProfilerEngine for StopErrorProfilerEngine {
        fn init_async_profiler() -> Result<(), asprof::AsProfError> {
            Ok(())
        }

        fn start_async_profiler(&self, jfr_file_path: &Path) -> Result<(), asprof::AsProfError> {
            let jfr_file_path = jfr_file_path.to_owned();
            std::fs::write(&jfr_file_path, "JFR").unwrap();
            let counter = self.counter.clone();
            tokio::task::spawn(async move {
                tokio::time::sleep(Duration::from_secs(5)).await;
                assert_eq!(std::fs::read_to_string(jfr_file_path).unwrap(), "JFR");
                counter.store(true, atomic::Ordering::Release);
            });
            if self.start_error {
                Err(asprof::AsProfError::AsyncProfilerError("error".into()))
            } else {
                Ok(())
            }
        }

        fn stop_async_profiler() -> Result<(), asprof::AsProfError> {
            Err(asprof::AsProfError::AsyncProfilerError("error".into()))
        }
    }

    #[tokio::test(start_paused = true)]
    #[test_case(false; "error on stop")]
    #[test_case(true; "error on start")]
    async fn test_profiler_error(start_error: bool) {
        let (agent, mut rx) = make_mock_profiler();
        let counter = Arc::new(AtomicBool::new(false));
        let engine = StopErrorProfilerEngine {
            start_error,
            counter: counter.clone(),
        };
        agent.spawn_inner(engine).unwrap();
        assert!(rx.recv().await.is_none());
        // check that the "sleep 5" step in start_async_profiler succeeds
        for _ in 0..100 {
            tokio::time::sleep(Duration::from_secs(1)).await;
            if counter.load(atomic::Ordering::Acquire) {
                return;
            }
        }
        panic!("didn't read from file");
    }
}
