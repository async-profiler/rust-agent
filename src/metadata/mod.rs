// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

//! This module includes ways to get metadata attached to profiling reports.

pub use std::time::Duration;

/// Host Metadata, which describes a host that runs a profiling agent. The current set of supported agent metadata is
/// AWS-specific. If you are not running on AWS, you can use [AgentMetadata::NoMetadata].
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum AgentMetadata {
    /// Metadata for an [EC2] instance running on AWS
    ///
    /// [EC2]: https://aws.amazon.com/ec2
    Ec2AgentMetadata {
        /// The AWS account id
        aws_account_id: String,
        /// The AWS region id
        aws_region_id: String,
        /// The EC2 instance id
        ec2_instance_id: String,
    },
    /// Metadata for a [Fargate] task running on AWS.
    ///
    /// [Fargate]: https://aws.amazon.com/fargate
    FargateAgentMetadata {
        /// The AWS account id
        aws_account_id: String,
        /// The AWS region id
        aws_region_id: String,
        /// The ECS task ARN
        ///
        /// For example, `arn:aws:ecs:us-east-1:123456789012:task/profiler-metadata-cluster/5261e761e0e2a3d92da3f02c8e5bab1f`
        ///
        /// See the ECS documentation for more details
        ecs_task_arn: String,
        /// The ECS cluster ARN
        ///
        /// For example, `arn:aws:ecs:us-east-1:123456789012:cluster/profiler-metadata-cluster`
        ///
        /// See the ECS documentation for more details
        ecs_cluster_arn: String,
    },
    /// Metadata for a host that is neither an EC2 nor a Fargate
    #[deprecated = "Use AgentMetadata::NoMetadata"]
    Other,
    /// A placeholder when a host has no metadata, or when a reporter does not
    /// use metadata.
    NoMetadata,
}

impl AgentMetadata {
    /// Create metadata corresponding to an EC2 instance running on AWS.
    ///
    /// This function is versioned to allow for extensibility.
    ///
    /// ```rust
    /// # use async_profiler_agent::metadata::AgentMetadata;
    /// let _metadata = AgentMetadata::ec2_agent_metadata_v1(
    ///     "123456789012".to_string(),
    ///     "us-east-1".to_string(),
    ///     "i-1234567890abcdef0".to_string(),
    /// );
    /// ```
    pub fn ec2_agent_metadata_v1(
        aws_account_id: String,
        aws_region_id: String,
        ec2_instance_id: String,
    ) -> Self {
        Self::Ec2AgentMetadata {
            aws_account_id,
            aws_region_id,
            ec2_instance_id,
        }
    }

    /// Create metadata corresponding to a Fargate task running on AWS.
    ///
    /// This function is versioned to allow for extensibility.
    ///
    /// ```rust
    /// # use async_profiler_agent::metadata::AgentMetadata;
    /// let _metadata = AgentMetadata::fargate_agent_metadata_v1(
    ///     "123456789012".to_string(),
    ///     "us-east-1".to_string(),
    ///     "arn:aws:ecs:us-east-1:123456789012:task/cluster/5261e761e0e2a3d92da3f02c8e5bab1f".to_string(),
    ///     "arn:aws:ecs:us-east-1:123456789012:cluster/profiler-metadata-cluster".to_string(),
    /// );
    /// ```
    pub fn fargate_agent_metadata_v1(
        aws_account_id: String,
        aws_region_id: String,
        ecs_task_arn: String,
        ecs_cluster_arn: String,
    ) -> Self {
        Self::FargateAgentMetadata {
            aws_account_id,
            aws_region_id,
            ecs_task_arn,
            ecs_cluster_arn,
        }
    }
}

/// Metadata associated with a specific individual profiling report
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReportMetadata<'a> {
    /// The host running the agent
    pub instance: &'a AgentMetadata,
    /// The start time of the profiling report, as a duration from the process start
    pub start: Duration,
    /// The end time of the profiling report, as a duration from the process start
    pub end: Duration,
    /// The desired reporting interval (on average, this should be
    /// approximately the same as `self.end - self.start`).
    pub reporting_interval: Duration,
}

#[cfg(feature = "aws-metadata-no-defaults")]
pub mod aws;

/// [private] dummy metadata to make testing easier
#[cfg(test)]
pub(crate) const DUMMY_METADATA: ReportMetadata<'static> = ReportMetadata {
    instance: &AgentMetadata::NoMetadata,
    start: Duration::from_secs(1),
    end: Duration::from_secs(2),
    reporting_interval: Duration::from_secs(1),
};
