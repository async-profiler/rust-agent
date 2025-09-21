// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

//! A reporter that reports into a directory.

use async_trait::async_trait;
use chrono::SecondsFormat;
use std::path::PathBuf;
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
/// It does not currently use the metadata, so if you are using
/// [LocalReporter] alone, rather than inside a [MultiReporter], you
/// can just use [AgentMetadata::Other] as metadata.
///
/// [AgentMetadata::Other]: crate::metadata::AgentMetadata::Other
/// [MultiReporter]: crate::reporter::multi::MultiReporter
///
/// ### Example
///
/// ```
/// # use async_profiler_agent::metadata::AgentMetadata;
/// # use async_profiler_agent::profiler::{ProfilerBuilder, SpawnError};
/// # use async_profiler_agent::reporter::local::LocalReporter;
/// # #[tokio::main]
/// # async fn main() -> Result<(), SpawnError> {
/// let profiler = ProfilerBuilder::default()
///    .with_reporter(LocalReporter::new("/tmp/profiles"))
///    .with_custom_agent_metadata(AgentMetadata::Other)
///    .build();
/// # if false { // don't spawn the profiler in doctests
/// profiler.spawn()?;
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

    /// Writes the jfr file to disk.
    async fn report_profiling_data(
        &self,
        jfr: Vec<u8>,
        _metadata_obj: &ReportMetadata<'_>,
    ) -> Result<(), std::io::Error> {
        let time: chrono::DateTime<chrono::Utc> = SystemTime::now().into();
        let time = time
            .to_rfc3339_opts(SecondsFormat::Secs, true)
            .replace(":", "-");
        tracing::debug!("reporting {time}.jfr");
        let file_name = format!("{time}.jfr");
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
}

#[cfg(test)]
mod test {
    use std::path::Path;

    use crate::{
        metadata::DUMMY_METADATA,
        reporter::{local::LocalReporter, Reporter},
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
}
