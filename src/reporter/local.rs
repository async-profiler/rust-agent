// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

//! A reporter that reports into a directory.

use async_trait::async_trait;
use chrono::SecondsFormat;
use std::path::{Path, PathBuf};
use std::time::SystemTime;
use thiserror::Error;

use crate::metadata::ReportMetadata;

use super::Reporter;

#[derive(Error, Debug)]
enum LocalReporterError {
    #[error("{0}")]
    IoError(#[from] std::io::Error),
}

/// A reporter that reports into a directory.
///
/// The files are reported with the filename `yyyy-mm-ddTHH-MM-SSZ.jfr`
///
/// Unlike other reporters (e.g. S3), the local reporter will
/// flush any pending JFR data on drop when the profiler task is cancelled
/// (for example, during Tokio runtime shutdown). Other reporters require
/// an explicit call to [`RunningProfiler::stop`] to ensure the last sample
/// is uploaded.
///
/// It does not currently use the metadata, so if you are using
/// [LocalReporter] alone, rather than inside a [MultiReporter], you
/// can just use [AgentMetadata::NoMetadata] as metadata.
///
/// [AgentMetadata::NoMetadata]: crate::metadata::AgentMetadata::NoMetadata
/// [MultiReporter]: crate::reporter::multi::MultiReporter
/// [`RunningProfiler::stop`]: crate::profiler::RunningProfiler::stop
///
/// ### Example
///
/// ```
/// # use async_profiler_agent::metadata::AgentMetadata;
/// # use async_profiler_agent::profiler::{ProfilerBuilder, SpawnError};
/// # #[tokio::main]
/// # async fn main() -> Result<(), SpawnError> {
/// let profiler = ProfilerBuilder::default()
///    .with_local_reporter("/tmp/profiles")
///    .build();
/// # if false { // don't spawn the profiler in doctests
/// let profiler = profiler.spawn_controllable()?;
/// // ... your program goes here
/// profiler.stop().await; // make sure the last profile is flushed
/// # }
/// # Ok(())
/// # }
/// ```
#[derive(Debug)]
pub struct LocalReporter {
    directory: PathBuf,
}

impl LocalReporter {
    /// Instantiate a new LocalReporter writing into the provided directory.
    pub fn new(directory: impl Into<PathBuf>) -> Self {
        LocalReporter {
            directory: directory.into(),
        }
    }

    fn jfr_file_name() -> String {
        let time: chrono::DateTime<chrono::Utc> = SystemTime::now().into();
        let time = time
            .to_rfc3339_opts(SecondsFormat::Secs, true)
            .replace(":", "-");
        format!("{time}.jfr")
    }

    /// Writes the jfr file to disk.
    async fn report_profiling_data(
        &self,
        jfr: Vec<u8>,
        _metadata_obj: &ReportMetadata<'_>,
    ) -> Result<(), std::io::Error> {
        let file_name = Self::jfr_file_name();
        tracing::debug!("reporting {file_name}");
        tokio::fs::write(self.directory.join(file_name), jfr).await?;
        Ok(())
    }
}

#[async_trait]
impl Reporter for LocalReporter {
    async fn report(
        &self,
        jfr: Vec<u8>,
        metadata: &ReportMetadata,
    ) -> Result<(), Box<dyn std::error::Error + Send>> {
        self.report_profiling_data(jfr, metadata)
            .await
            .map_err(|e| Box::new(LocalReporterError::IoError(e)) as _)
    }

    fn report_blocking(
        &self,
        jfr_path: &Path,
        _metadata: &ReportMetadata,
    ) -> Result<(), Box<dyn std::error::Error + Send>> {
        let file_name = Self::jfr_file_name();
        tracing::debug!("reporting {file_name} (blocking)");
        std::fs::copy(jfr_path, self.directory.join(file_name))
            .map(|_| ())
            .map_err(|e| Box::new(LocalReporterError::IoError(e)) as _)
    }
}

#[cfg(test)]
mod test {
    use std::path::Path;

    use crate::{
        metadata::DUMMY_METADATA,
        reporter::{Reporter, local::LocalReporter},
    };

    #[tokio::test]
    async fn test_local_reporter() {
        let dir = tempfile::tempdir().unwrap();
        let reporter = LocalReporter::new(dir.path());
        reporter
            .report(b"JFR".into(), &DUMMY_METADATA)
            .await
            .unwrap();
        let jfr_file = std::fs::read_dir(dir.path())
            .unwrap()
            .flat_map(|f| f.ok())
            .filter(|f| {
                Path::new(&f.file_name())
                    .extension()
                    .is_some_and(|e| e == "jfr")
            })
            .next()
            .unwrap();
        assert_eq!(tokio::fs::read(jfr_file.path()).await.unwrap(), b"JFR");
    }

    #[test]
    fn test_local_reporter_report_blocking() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("input.jfr");
        std::fs::write(&src, b"JFR-DROP").unwrap();
        let out_dir = tempfile::tempdir().unwrap();
        let reporter = LocalReporter::new(out_dir.path());
        reporter.report_blocking(&src, &DUMMY_METADATA).unwrap();
        let jfr_file = std::fs::read_dir(out_dir.path())
            .unwrap()
            .flat_map(|f| f.ok())
            .filter(|f| {
                Path::new(&f.file_name())
                    .extension()
                    .is_some_and(|e| e == "jfr")
            })
            .next()
            .unwrap();
        assert_eq!(std::fs::read(jfr_file.path()).unwrap(), b"JFR-DROP");
    }
}
