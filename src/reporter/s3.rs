// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

//! A reporter that uploads reports to an S3 bucket

use async_trait::async_trait;
use aws_config::SdkConfig;
use chrono::SecondsFormat;
use serde::Serialize;
use std::io::Write;
use std::time::SystemTime;
use std::{fmt, io::Cursor};
use thiserror::Error;
use zip::result::ZipError;
use zip::{write::SimpleFileOptions, ZipWriter};

use crate::metadata::{AgentMetadata, ReportMetadata};

use super::Reporter;

/// Error reporting to S3
#[derive(Error, Debug)]
pub enum S3ReporterError {
    /// I/O error creating zip file
    #[error("io error creating zip file: {0}")]
    ZipIoError(std::io::Error),
    /// Error creating zip file
    #[error("creating zip file: {0}")]
    ZipError(#[from] ZipError),
    /// Error sending data to S3
    #[error("failed to send profile data directly to S3: {0}")]
    SendProfileS3Data(Box<aws_sdk_s3::Error>),
    /// Error joining Tokio task
    #[error("tokio task: {0}")]
    JoinError(#[from] tokio::task::JoinError),
}

/// This is the format of the metadata JSON uploaded to S3.
#[derive(Debug, Serialize)]
#[non_exhaustive]
pub struct MetadataJson {
    start: u64,
    end: u64,
    reporting_interval: u64,
}

/// Mandatory parameters in order to configure an S3 reporter
pub struct S3ReporterConfig<'a> {
    /// The SDK config to get credentials from
    pub sdk_config: &'a SdkConfig,
    /// The expected bucket owner account
    pub bucket_owner: String,
    /// The bucket name
    pub bucket_name: String,
    /// The profiling group name, used in the file names within the bucket
    pub profiling_group_name: String,
}

/// A reporter that uploads reports to an S3 bucket
pub struct S3Reporter {
    s3_client: aws_sdk_s3::Client,
    bucket_owner: String,
    bucket_name: String,
    profiling_group_name: String,
}

impl S3Reporter {
    /// Create a new S3Reporter
    pub fn new(config: S3ReporterConfig<'_>) -> Self {
        let S3ReporterConfig {
            sdk_config,
            bucket_owner,
            bucket_name,
            profiling_group_name,
        } = config;
        let s3_client_config = aws_sdk_s3::config::Builder::from(sdk_config).build();
        let s3_client = aws_sdk_s3::Client::from_conf(s3_client_config);

        S3Reporter {
            s3_client,
            bucket_owner,
            bucket_name,
            profiling_group_name,
        }
    }

    /// Makes a zip file, then uploads it.
    pub async fn report_profiling_data(
        &self,
        jfr: Vec<u8>,
        metadata_obj: &ReportMetadata<'_>,
    ) -> Result<(), S3ReporterError> {
        tracing::debug!("sending file to backend");

        let metadata_json = MetadataJson {
            start: metadata_obj.start.as_millis() as u64,
            end: metadata_obj.end.as_millis() as u64,
            reporting_interval: metadata_obj.reporting_interval.as_millis() as u64,
        };

        // Create a zip file.
        let zip = tokio::task::spawn_blocking(move || {
            add_files_to_zip("async_profiler_dump_0.jfr", &jfr, metadata_json)
        })
        .await??;

        // Send zip file to the S3 pre-signed URL.
        send_profile_data(
            &self.s3_client,
            self.bucket_owner.clone(),
            self.bucket_name.clone(),
            make_s3_file_name(
                metadata_obj.instance,
                &self.profiling_group_name,
                SystemTime::now(),
            ),
            zip,
        )
        .await?;

        Ok(())
    }
}

fn make_s3_file_name(
    metadata_obj: &AgentMetadata,
    profiling_group_name: &str,
    time: SystemTime,
) -> String {
    let machine = match metadata_obj {
        AgentMetadata::Ec2AgentMetadata {
            aws_account_id: _,
            aws_region_id: _,
            ec2_instance_id,
        } => {
            let ec2_instance_id = ec2_instance_id.replace("/", "-").replace("_", "-");
            format!("ec2_{ec2_instance_id}_")
        }
        AgentMetadata::FargateAgentMetadata {
            aws_account_id: _,
            aws_region_id: _,
            ecs_task_arn,
            ecs_cluster_arn: _,
            ..
        } => {
            let task_arn = ecs_task_arn.replace("/", "-").replace("_", "-");
            format!("ecs_{task_arn}_")
        }
        #[allow(deprecated)]
        AgentMetadata::Other => "onprem__".to_string(),
        AgentMetadata::NoMetadata => "unknown__".to_string(),
    };
    let time: chrono::DateTime<chrono::Utc> = time.into();
    let time = time
        .to_rfc3339_opts(SecondsFormat::Secs, true)
        .replace(":", "-");
    let pid = std::process::id();
    format!("profile_{profiling_group_name}_{machine}_{pid}_{time}.zip")
}

#[async_trait]
impl Reporter for S3Reporter {
    async fn report(
        &self,
        jfr: Vec<u8>,
        metadata: &ReportMetadata,
    ) -> Result<(), Box<dyn std::error::Error + Send>> {
        self.report_profiling_data(jfr, metadata)
            .await
            .map_err(|e| Box::new(e) as _)
    }
}

impl fmt::Debug for S3Reporter {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("S3Reporter").finish()
    }
}

fn add_files_to_zip(
    jfr_filename: &str,
    jfr_file: &[u8],
    metadata_json: MetadataJson,
) -> Result<Vec<u8>, S3ReporterError> {
    tracing::debug!("creating zip file");

    let file = Cursor::new(vec![]);
    let mut zip = ZipWriter::new(file);
    let metadata = serde_json::ser::to_string(&metadata_json).unwrap();
    add_bytes_to_zip(&mut zip, jfr_filename, jfr_file).map_err(S3ReporterError::ZipIoError)?;
    add_bytes_to_zip(&mut zip, "metadata.json", metadata.as_bytes())
        .map_err(S3ReporterError::ZipIoError)?;
    Ok(zip.finish()?.into_inner())
}

fn add_bytes_to_zip(
    zip: &mut ZipWriter<Cursor<Vec<u8>>>,
    filename: &str,
    data: &[u8],
) -> Result<(), std::io::Error> {
    let options = SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
    zip.start_file(filename, options)?;
    zip.write_all(data)?;

    Ok(())
}

async fn send_profile_data(
    s3_client: &aws_sdk_s3::Client,
    bucket_owner: String,
    bucket_name: String,
    object_name: String,
    zip: Vec<u8>,
) -> Result<(), S3ReporterError> {
    tracing::debug!(message="uploading to s3", bucket_name=?bucket_name, object_name=?object_name);
    // Make http call to upload JFR to S3.
    s3_client
        .put_object()
        .expected_bucket_owner(bucket_owner)
        .bucket(bucket_name)
        .key(object_name)
        .body(zip.into())
        .content_type("application/zip")
        .send()
        .await
        .map_err(|x| S3ReporterError::SendProfileS3Data(Box::new(x.into())))?;
    Ok(())
}

#[cfg(test)]
mod test {
    use std::{
        io,
        sync::{Arc, Mutex},
        time::SystemTime,
    };

    use aws_sdk_s3::operation::put_object::PutObjectOutput;
    use aws_smithy_mocks::{mock, mock_client};

    use test_case::test_case;

    use crate::{
        metadata::{AgentMetadata, DUMMY_METADATA},
        reporter::s3::S3Reporter,
    };

    fn assert_zip(zip_file: Vec<u8>) {
        let zip = zip::ZipArchive::new(io::Cursor::new(&zip_file)).unwrap();
        let mut file_names: Vec<_> = zip.file_names().collect();
        file_names.sort();
        assert_eq!(
            file_names,
            vec!["async_profiler_dump_0.jfr", "metadata.json"]
        );
    }

    #[test_case(#[allow(deprecated)] { AgentMetadata::Other }, "profile_pg_onprem___<pid>_<time>.zip"; "other")]
    #[test_case(AgentMetadata::NoMetadata, "profile_pg_unknown___<pid>_<time>.zip"; "no-metadata")]
    #[test_case(AgentMetadata::Ec2AgentMetadata {
        aws_account_id: "1".into(),
        aws_region_id: "us-east-1".into(),
        ec2_instance_id: "i-0".into()
    }, "profile_pg_ec2_i-0__<pid>_<time>.zip"; "ec2")]
    #[test_case(AgentMetadata::FargateAgentMetadata {
        aws_account_id: "1".into(),
        aws_region_id: "us-east-1".into(),
        ecs_task_arn: "arn:aws:ecs:us-east-1:123456789012:task/profiler-metadata-cluster/5261e761e0e2a3d92da3f02c8e5bab1f".into(),
        ecs_cluster_arn: "arn:aws:ecs:us-east-1:123456789012:cluster/profiler-metadata-cluster".into(),
        task_cpu: 0.25,
        task_memory: 2048,
    }, "profile_pg_ecs_arn:aws:ecs:us-east-1:123456789012:task-profiler-metadata-cluster-5261e761e0e2a3d92da3f02c8e5bab1f__<pid>_<time>.zip"; "ecs")]
    fn test_make_s3_file_name(metadata: AgentMetadata, expected: &str) {
        let file_name = super::make_s3_file_name(&metadata, "pg", SystemTime::UNIX_EPOCH);
        assert_eq!(
            file_name,
            expected
                .replace("<pid>", &std::process::id().to_string())
                .replace("<time>", "1970-01-01T00-00-00Z")
        );
    }

    #[tokio::test]
    async fn test_reporter() {
        let uploaded_file = Arc::new(Mutex::new(None));
        let uploaded_file_ = uploaded_file.clone();
        let put_object_rule = mock!(aws_sdk_s3::Client::put_object)
            .match_requests(move |req| {
                *uploaded_file_.lock().unwrap() = Some(req.body().bytes().unwrap().to_vec());
                true
            })
            .then_output(|| PutObjectOutput::builder().build());

        // Create a mocked client with the rule
        // Use the standard Builder instead of with_test_defaults
        let reporter = S3Reporter {
            s3_client: mock_client!(aws_sdk_s3, [&put_object_rule]),
            bucket_owner: "123456789012".into(),
            bucket_name: "123456789012-bucket".into(),
            profiling_group_name: "test-profiling-group".into(),
        };
        reporter
            .report_profiling_data(b"JFR".into(), &DUMMY_METADATA)
            .await
            .unwrap();
        assert_zip(uploaded_file.lock().unwrap().take().unwrap());
    }

    #[tokio::test]
    async fn test_reporter_error() {
        let put_object_rule = mock!(aws_sdk_s3::Client::put_object).then_error(|| {
            aws_sdk_s3::operation::put_object::PutObjectError::unhandled(io::Error::new(
                io::ErrorKind::Other,
                "oh no",
            ))
        });

        // Create a mocked client with the rule
        // Use the standard Builder instead of with_test_defaults
        let reporter = S3Reporter {
            s3_client: mock_client!(aws_sdk_s3, [&put_object_rule]),
            bucket_owner: "123456789012".into(),
            bucket_name: "123456789012-bucket".into(),
            profiling_group_name: "test-profiling-group".into(),
        };
        reporter
            .report_profiling_data(b"JFR".into(), &DUMMY_METADATA)
            .await
            .unwrap_err();
    }
}
