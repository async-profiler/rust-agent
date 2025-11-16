// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

//! Contains functions for getting host metadata from [IMDS]
//!
//! [IMDS]: https://docs.aws.amazon.com/AWSEC2/latest/UserGuide/ec2-instance-metadata.html

use reqwest::Method;
use serde::Deserialize;
use thiserror::Error;

use crate::metadata::OrderedF64;

use super::AgentMetadata;

/// An error converting Fargate IMDS metadata to Agent metadata. This error
/// should probably not happen except in case of a bug in either this crate or IMDS.
#[derive(Error, Debug)]
pub enum FargateMetadataToAgentMetadataError {
    /// unable to parse task ARN as a valid ARN
    #[error("unable to parse task ARN as a valid ARN")]
    TaskArnInvalid(#[from] aws_arn::Error),
    /// AWS account id not found in Fargate metadata
    #[error("AWS account id not found in Fargate metadata")]
    AccountIdNotFound,
    /// AWS region not found in Fargate metadata
    #[error("AWS region not found in Fargate metadata")]
    AwsRegionNotFound,
}

/// An error getting IMDS metadata
#[derive(Error, Debug)]
#[error("profiler metadata error: {0}")]
pub enum AwsProfilerMetadataError {
    /// Internal IO error
    #[error("failed to create profiler metadata file: {0}")]
    FailedToCreateFile(#[from] std::io::Error),

    /// Error parsing IMDS metadata. Should normally not happen except in case of a bug
    #[error("failed fetching valid Fargate metadata: {0}")]
    FargateMetadataToAgentMetadataError(#[from] FargateMetadataToAgentMetadataError),

    /// Invalid endpoint URI in `ECS_CONTAINER_METADATA_URI_V4`
    #[error("retrieved invalid endpoint URI from ECS_CONTAINER_METADATA_URI_V4: {0}")]
    InvalidUri(String),

    /// Failed to fetch metadata from FarGate endpoint
    #[error("failed to fetch metadata from endpoint over HTTP: {0}")]
    FailedToFetchMetadataFromEndpoint(reqwest::Error),

    /// Failed to fetch metadata from IMDS
    #[error("failed to fetch metadata from IMDS endpoint over HTTP: {0}")]
    FailedToFetchMetadataFromImds(#[from] aws_config::imds::client::error::ImdsError),

    /// Failed to parse metadata from IMDS - this indicates a bug in this crate
    /// or in IMDS
    #[error("failed to parse metadata as valid UTF-8 from endpoint over HTTP: {0}")]
    FailedToParseMetadataFromEndpoint(reqwest::Error),

    /// Failed to serialize metadata file - this indicates a bug in this crate
    /// or in IMDS
    #[error("failed to serialize metadata file: {0}")]
    FailedToSerializeMetadataFile(#[from] serde_json::Error),
}

#[derive(Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
struct ImdsEc2InstanceMetadata {
    account_id: String,
    region: String,
    #[allow(dead_code)]
    instance_type: String,
    instance_id: String,
}

async fn read_ec2_metadata() -> Result<ImdsEc2InstanceMetadata, AwsProfilerMetadataError> {
    let imds = aws_config::imds::Client::builder().build();
    let imds_document = imds
        .get("/latest/dynamic/instance-identity/document")
        .await?;

    Ok(serde_json::from_str(imds_document.as_ref())?)
}

#[derive(Deserialize, Debug, PartialEq, Eq)]
struct FargateLimits {
    #[serde(rename = "CPU")]
    cpu: Option<OrderedF64>,
    #[serde(rename = "Memory")]
    memory: Option<u64>,
}

#[derive(Deserialize, Debug, PartialEq, Eq)]
struct FargateMetadata {
    #[serde(rename = "Cluster")]
    cluster: String,
    #[serde(rename = "TaskARN")]
    task_arn: String,
    // According to <https://docs.aws.amazon.com/AmazonECS/latest/developerguide/task-metadata-endpoint-v4-fargate-response.html>
    // Limits: The resource limits specified at the task levels such as CPU (expressed in vCPUs) and memory.
    // This parameter is omitted if no resource limits are defined.
    #[serde(rename = "Limits")]
    limits: Option<FargateLimits>,
}

async fn read_fargate_metadata(
    http_client: &reqwest::Client,
) -> Result<FargateMetadata, AwsProfilerMetadataError> {
    let Ok(md_uri) = std::env::var("ECS_CONTAINER_METADATA_URI_V4") else {
        return Err(AwsProfilerMetadataError::InvalidUri(
            "not running on fargate".into(),
        ));
    };
    // Only available if running on Fargate. Something like
    // `http://169.254.232.106/v4/5261e761e0e2a3d92da3f02c8e5bab1f-3356750833`.
    let uri = format!("{md_uri}/task",);

    let req = http_client
        .request(Method::GET, uri.clone())
        .build()
        // The only thing that can be invalid about this request is necessarily the URI.
        .map_err(|_e| AwsProfilerMetadataError::InvalidUri(uri))?;
    let res = http_client
        .execute(req)
        .await
        .map_err(AwsProfilerMetadataError::FailedToFetchMetadataFromEndpoint)?;
    let body_str = res
        .text()
        .await
        .map_err(AwsProfilerMetadataError::FailedToParseMetadataFromEndpoint)?;

    Ok(serde_json::from_str(&body_str)?)
}

impl super::AgentMetadata {
    fn from_imds_ec2_instance_metadata(
        imds_ec2_instance_metadata: ImdsEc2InstanceMetadata,
    ) -> Self {
        Self::Ec2AgentMetadata {
            aws_account_id: imds_ec2_instance_metadata.account_id,
            aws_region_id: imds_ec2_instance_metadata.region,
            ec2_instance_id: imds_ec2_instance_metadata.instance_id,
            #[cfg(feature = "__unstable-fargate-cpu-count")]
            ec2_instance_type: imds_ec2_instance_metadata.instance_type,
        }
    }

    fn try_from_fargate_metadata(
        fargate_metadata: FargateMetadata,
    ) -> Result<Self, FargateMetadataToAgentMetadataError> {
        let ecs_task_arn: aws_arn::ResourceName = fargate_metadata.task_arn.parse()?;

        Ok(Self::FargateAgentMetadata {
            aws_account_id: ecs_task_arn
                .account_id
                .ok_or(FargateMetadataToAgentMetadataError::AccountIdNotFound)?
                .to_string(),
            aws_region_id: ecs_task_arn
                .region
                .ok_or(FargateMetadataToAgentMetadataError::AwsRegionNotFound)?
                .to_string(),
            ecs_task_arn: fargate_metadata.task_arn,
            ecs_cluster_arn: fargate_metadata.cluster,
            #[cfg(feature = "__unstable-fargate-cpu-count")]
            cpu_limit: fargate_metadata
                .limits
                .as_ref()
                .and_then(|limits| limits.cpu),
            #[cfg(feature = "__unstable-fargate-cpu-count")]
            memory_limit: fargate_metadata
                .limits
                .as_ref()
                .and_then(|limits| limits.memory),
        })
    }
}

/// Load agent metadata from [Fargate] or [IMDS].
///
/// This will return an error if this machine does not appear to be a [Fargate] or [EC2].
///
/// [Fargate]: https://aws.amazon.com/fargate
/// [IMDS]: https://docs.aws.amazon.com/AWSEC2/latest/UserGuide/ec2-instance-metadata.html
/// [EC2]: https://aws.amazon.com/ec2
pub async fn load_agent_metadata() -> Result<AgentMetadata, AwsProfilerMetadataError> {
    let agent_metadata: AgentMetadata = match read_ec2_metadata().await {
        Ok(imds_ec2_instance_metadata) => {
            AgentMetadata::from_imds_ec2_instance_metadata(imds_ec2_instance_metadata)
        }
        Err(_) => {
            let http_client = reqwest::Client::new();
            let fargate_metadata = read_fargate_metadata(&http_client).await?;

            AgentMetadata::try_from_fargate_metadata(fargate_metadata)?
        }
    };
    Ok(agent_metadata)
}

#[cfg(test)]
mod tests {
    use super::*;
    use test_case::test_case;

    // these constants are "anonymized" (aka randomly generated in that format)

    // AMI_ID = 52d4b310b0a459ff
    // INSTANCE_ID = 92eba08c089f6325
    // ACCOUNT_ID = 123456789012
    // IP = 10.65.149.216
    // DATE = 2025-03-20T16:41:24Z
    // DATE1 = 2025-03-20T16:41:24.713942268Z
    // DATE2 = 2025-03-20T16:41:25.623883595Z
    // TASK_ARN = 5261e761e0e2a3d92da3f02c8e5bab1f
    // DOCKER_ID = 3356750833
    // PRIV_IP = 169.254.232.106
    // IMAGE_ID = ad9d89a36c31afef34c79e05263b06087ad354796cfd90c66ced30f40ea2dbf4
    // TASK_UUID = f4094744-1b40-4701-9f26-ad84ebb709d7
    // CLOCK_ERROR = 0.3148955

    #[test]
    fn test_imds_ec2_metadata() {
        let json_str = r#"
{
    "accountId" : "123456789012",
    "architecture" : "x86_64",
    "availabilityZone" : "eu-west-1b",
    "billingProducts" : null,
    "devpayProductCodes" : null,
    "marketplaceProductCodes" : null,
    "imageId" : "ami-052d4b310b0a459ff",
    "instanceId" : "i-092eba08c089f6325",
    "instanceType" : "c5.4xlarge",
    "kernelId" : null,
    "pendingTime" : "2025-03-20T16:41:24Z",
    "privateIp" : "10.65.149.216",
    "ramdiskId" : null,
    "region" : "eu-west-1",
    "version" : "2017-09-30"
}"#;

        let imds_ec2_instance_metadata: ImdsEc2InstanceMetadata =
            serde_json::from_str(&json_str).unwrap();

        assert_eq!(
            imds_ec2_instance_metadata,
            ImdsEc2InstanceMetadata {
                account_id: "123456789012".to_owned(),
                region: "eu-west-1".to_owned(),
                instance_type: "c5.4xlarge".to_owned(),
                instance_id: "i-092eba08c089f6325".to_owned(),
            }
        );

        let agent_metadata =
            AgentMetadata::from_imds_ec2_instance_metadata(imds_ec2_instance_metadata);

        let expected = AgentMetadata::ec2_agent_metadata(
            "123456789012".to_owned(),
            "eu-west-1".to_owned(),
            "i-092eba08c089f6325".to_owned(),
        )
        .with_ec2_instance_type("c5.4xlarge".to_owned())
        .build();

        assert_eq!(agent_metadata, expected);
    }

    #[test_case(
        r#"{
    "Cluster": "arn:aws:ecs:us-east-1:123456789012:cluster/profiler-metadata-cluster",
    "TaskARN": "arn:aws:ecs:us-east-1:123456789012:task/profiler-metadata-cluster/5261e761e0e2a3d92da3f02c8e5bab1f"
}"#,
        None,
        None,
        None
        ; "no_limits"
    )]
    #[test_case(
        r#"{
    "Cluster": "arn:aws:ecs:us-east-1:123456789012:cluster/profiler-metadata-cluster",
    "TaskARN": "arn:aws:ecs:us-east-1:123456789012:task/profiler-metadata-cluster/5261e761e0e2a3d92da3f02c8e5bab1f",
    "Limits": {}
}"#,
        Some(FargateLimits { cpu: None, memory: None }),
        None,
        None
        ; "empty_limits"
    )]
    #[test_case(
        r#"{
    "Cluster": "arn:aws:ecs:us-east-1:123456789012:cluster/profiler-metadata-cluster",
    "TaskARN": "arn:aws:ecs:us-east-1:123456789012:task/profiler-metadata-cluster/5261e761e0e2a3d92da3f02c8e5bab1f",
    "Family": "profiler-metadata",
    "Revision": "1",
    "DesiredStatus": "RUNNING",
    "KnownStatus": "NONE",
    "Limits": {
        "CPU": 0.25,
        "Memory": 2048
    },
    "PullStartedAt": "2025-03-20T16:41:24.713942268Z",
    "PullStoppedAt": "2025-03-20T16:41:25.623883595Z",
    "AvailabilityZone": "us-east-1f",
    "LaunchType": "FARGATE",
    "Containers": [
        {
            "DockerId": "5261e761e0e2a3d92da3f02c8e5bab1f-3356750833",
            "Name": "profiler-metadata",
            "DockerName": "profiler-metadata",
            "Image": "123456789012.dkr.ecr.us-east-1.amazonaws.com/profiler-metadata",
            "ImageID": "sha256:ad9d89a36c31afef34c79e05263b06087ad354796cfd90c66ced30f40ea2dbf4",
            "Labels": {
                "com.amazonaws.ecs.cluster": "arn:aws:ecs:us-east-1:123456789012:cluster/profiler-metadata-cluster",
                "com.amazonaws.ecs.container-name": "profiler-metadata",
                "com.amazonaws.ecs.task-arn": "arn:aws:ecs:us-east-1:123456789012:task/profiler-metadata-cluster/5261e761e0e2a3d92da3f02c8e5bab1f",
                "com.amazonaws.ecs.task-definition-family": "profiler-metadata",
                "com.amazonaws.ecs.task-definition-version": "1"
            },
            "DesiredStatus": "RUNNING",
            "KnownStatus": "PULLED",
            "Limits": {
                "CPU": 0
            },
            "Type": "NORMAL",
            "LogDriver": "awslogs",
            "LogOptions": {
                "awslogs-create-group": "true",
                "awslogs-group": "/ecs/profiler-metadata",
                "awslogs-region": "us-east-1",
                "awslogs-stream": "ecs/profiler-metadata/5261e761e0e2a3d92da3f02c8e5bab1f",
                "max-buffer-size": "25m",
                "mode": "non-blocking"
            },
            "ContainerARN": "arn:aws:ecs:us-east-1:123456789012:container/profiler-metadata-cluster/5261e761e0e2a3d92da3f02c8e5bab1f/f4094744-1b40-4701-9f26-ad84ebb709d7",
            "Networks": [
                {
                    "NetworkMode": "awsvpc",
                    "IPv4Addresses": [
                        "172.31.233.169"
                    ],
                    "AttachmentIndex": 0,
                    "MACAddress": "16:ff:d6:e1:dc:99",
                    "IPv4SubnetCIDRBlock": "172.31.192.0/20",
                    "DomainNameServers": [
                        "172.31.0.2"
                    ],
                    "DomainNameSearchList": [
                        "ec2.internal"
                    ],
                    "PrivateDNSName": "ip-172-31-233-169.ec2.internal",
                    "SubnetGatewayIpv4Address": "172.31.192.1/20"
                }
            ],
            "Snapshotter": "overlayfs"
        }
    ],
    "ClockDrift": {
        "ClockErrorBound": 0.3148955,
        "ReferenceTimestamp": "2025-03-20T16:41:24Z",
        "ClockSynchronizationStatus": "SYNCHRONIZED"
    },
    "EphemeralStorageMetrics": {
        "Utilized": 208,
        "Reserved": 20496
    }
}"#,
        Some(FargateLimits { cpu: Some(0.25.into()), memory: Some(2048) }),
        Some(0.25.into()),
        Some(2048)
        ; "with_limits"
    )]
    fn test_fargate_metadata(
        json_str: &str,
        expected_limits: Option<FargateLimits>,
        _expected_cpu_limit: Option<OrderedF64>,
        _expected_memory_limit: Option<u64>,
    ) {
        let fargate_metadata: FargateMetadata = serde_json::from_str(json_str).unwrap();

        assert_eq!(
            fargate_metadata,
            FargateMetadata {
                cluster: "arn:aws:ecs:us-east-1:123456789012:cluster/profiler-metadata-cluster"
                    .to_owned(),
                task_arn: "arn:aws:ecs:us-east-1:123456789012:task/profiler-metadata-cluster/5261e761e0e2a3d92da3f02c8e5bab1f".to_owned(),
                limits: expected_limits,
            }
        );

        let agent_metadata = AgentMetadata::try_from_fargate_metadata(fargate_metadata).unwrap();

        assert_eq!(
            agent_metadata,
            AgentMetadata::FargateAgentMetadata {
                aws_account_id: "123456789012".to_owned(),
                aws_region_id: "us-east-1".to_owned(),
                ecs_task_arn: "arn:aws:ecs:us-east-1:123456789012:task/profiler-metadata-cluster/5261e761e0e2a3d92da3f02c8e5bab1f".to_owned(),
                ecs_cluster_arn: "arn:aws:ecs:us-east-1:123456789012:cluster/profiler-metadata-cluster".to_owned(),
                #[cfg(feature = "__unstable-fargate-cpu-count")]
                cpu_limit: _expected_cpu_limit,
                #[cfg(feature = "__unstable-fargate-cpu-count")]
                memory_limit: _expected_memory_limit,
            }
        )
    }
}
