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
            make_s3_file_name(metadata_obj.instance, &self.profiling_group_name),
            zip,
        )
        .await?;

        Ok(())
    }
}

fn make_s3_file_name(metadata_obj: &AgentMetadata, profiling_group_name: &str) -> String {
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
            ecs_cluster_arn,
        } => {
            let task_arn = ecs_task_arn.replace("/", "-").replace("_", "-");
            let cluster_arn = ecs_cluster_arn.replace("/", "-").replace("_", "-");
            format!("ecs_{cluster_arn}_{task_arn}")
        }
        AgentMetadata::Other => "onprem__".to_string(),
    };
    let time: chrono::DateTime<chrono::Utc> = SystemTime::now().into();
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
