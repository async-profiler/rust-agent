// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

use std::{
    fs::File,
    io,
    path::Path,
    time::{Duration, SystemTime, SystemTimeError, UNIX_EPOCH},
};

use thiserror::Error;

use crate::{
    asprof::{self, AsProf, AsProfError},
    metadata::{aws::AwsProfilerMetadataError, AgentMetadata, ReportMetadata},
    reporter::Reporter,
};

struct JfrFile {
    active: tempfile::NamedTempFile,
    inactive: tempfile::NamedTempFile,
}

impl JfrFile {
    fn new() -> Result<Self, io::Error> {
        Ok(Self {
            active: tempfile::Builder::new().suffix(".jfr").tempfile()?,
            inactive: tempfile::Builder::new().suffix(".jfr").tempfile()?,
        })
    }

    fn swap(&mut self) {
        std::mem::swap(&mut self.active, &mut self.inactive);
    }

    fn empty_inactive_file(&mut self) -> Result<(), io::Error> {
        // Empty the file, or create it for the first time if the profiler hasn't
        // started yet.
        File::create(&self.inactive)?;
        tracing::debug!(message="emptied the file", file=%self.inactive.path().display());
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
    Running(SystemTime),
}

/// This type provides wrapper APIs over [`asprof::AsProf`], to allow tracking
/// of the state of the profiler. The primary benefit of this is RAII - when
/// this type drops, it will stop the profiler if it's running.
struct ProfilerState {
    asprof: AsProf,
    status: Status,
}

impl ProfilerState {
    pub fn new(asprof: AsProf) -> Self {
        Self {
            asprof,
            status: Status::Idle,
        }
    }

    async fn start(&mut self, jfr: &Path) -> Result<(), AsProfError> {
        self.asprof.start_async_profiler(jfr)?;
        self.status = Status::Running(SystemTime::now());
        Ok(())
    }

    fn stop(&mut self) -> Result<Option<SystemTime>, AsProfError> {
        asprof::AsProf::stop_async_profiler()?;
        let status = std::mem::replace(&mut self.status, Status::Idle);
        Ok(match status {
            Status::Idle => None,
            Status::Running(since) => Some(since),
        })
    }

    fn is_started(&self) -> bool {
        matches!(self.status, Status::Running(_))
    }
}

impl Drop for ProfilerState {
    fn drop(&mut self) {
        if self.is_started() {
            if let Err(err) = self.stop() {
                // XXX: Rust defines leaking resources during drop as safe.
                tracing::warn!(?err, "unable to stop profiler during drop glue");
            }
        }
    }
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
        let asprof = asprof::AsProf::builder().build();

        // Initialize async profiler - needs to be done once.
        asprof::AsProf::init_async_profiler()?;
        tracing::info!("successfully initialized async profiler.");

        let mut sampling_ticker = tokio::time::interval(self.reporting_interval);
        let mut jfr_file = JfrFile::new().map_err(SpawnError::TempFile)?;

        // Get profiles at the configured interval rate.
        Ok(tokio::spawn(async move {
            let mut state = ProfilerState::new(asprof);

            // Lazily-loaded if not specified up front.
            let mut agent_metadata = self.agent_metadata;

            loop {
                // Start timer.
                let _ = sampling_ticker.tick().await;
                tracing::debug!("profiler timer woke up");

                if let Err(err) = profiler_tick(
                    &mut state,
                    &mut agent_metadata,
                    &mut jfr_file,
                    &self.reporter,
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

async fn profiler_tick(
    state: &mut ProfilerState,
    agent_metadata: &mut Option<AgentMetadata>,
    jfr_file: &mut JfrFile,
    reporter: &Box<dyn Reporter + Send + Sync>,
    reporting_interval: Duration,
) -> Result<(), TickError> {
    if !state.is_started() {
        state.start(jfr_file.active.path()).await?;
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
    jfr_file
        .empty_inactive_file()
        .map_err(TickError::EmptyInactiveFile)?;
    jfr_file.swap();
    state.start(jfr_file.active.path()).await?;

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

    let jfr = tokio::fs::read(jfr_file.inactive.path())
        .await
        .map_err(TickError::JfrRead)?;

    reporter
        .report(jfr, &report_metadata)
        .await
        .map_err(TickError::Reporter)?;

    Ok(())
}
