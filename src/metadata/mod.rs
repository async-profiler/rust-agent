// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

//! This module includes ways to get metadata attached to profiling reports.

use ordered_float::OrderedFloat;
pub use std::time::Duration;

/// An ordered float. Individual type to avoid public API dependencies.
#[derive(Debug, Copy, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(transparent)]
pub struct OrderedF64(OrderedFloat<f64>);

impl OrderedF64 {
    /// Create a new `OrderedF64`
    ///
    /// ```rust
    /// # use async_profiler_agent::metadata::OrderedF64;
    /// let _v = OrderedF64::new(0.0);
    /// ```
    pub const fn new(value: f64) -> Self {
        Self(OrderedFloat(value))
    }
}

impl From<f64> for OrderedF64 {
    fn from(value: f64) -> Self {
        Self(OrderedFloat(value))
    }
}

impl From<OrderedF64> for f64 {
    fn from(value: OrderedF64) -> Self {
        value.0.0
    }
}

/// Host Metadata, which describes a host that runs a profiling agent. The current set of supported agent metadata is
/// AWS-specific. If you are not running on AWS, you can use [AgentMetadata::NoMetadata].
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum AgentMetadata {
    /// Metadata for an [EC2] instance running on AWS
    ///
    /// [EC2]: https://aws.amazon.com/ec2
    #[cfg_attr(feature = "__unstable-fargate-cpu-count", non_exhaustive)]
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
    #[cfg_attr(feature = "__unstable-fargate-cpu-count", non_exhaustive)]
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
        /// The CPU limit for the Fargate cluster
        ///
        /// For example, `Some(0.25)`. This will be `None` if the CPU limit is not specified.
        ///
        /// See the ECS documentation for more details
        #[cfg(feature = "__unstable-fargate-cpu-count")]
        cpu_limit: Option<OrderedF64>,
        /// The memory limit for the Fargate cluster (in megabytes)
        ///
        /// For example, `Some(2048)`. This will be `None` if the memory limit is not specified.
        ///
        /// See the ECS documentation for more details
        #[cfg(feature = "__unstable-fargate-cpu-count")]
        memory_limit: Option<u64>,
    },
    /// Metadata for a host that is neither an EC2 nor a Fargate
    #[deprecated = "Use AgentMetadata::NoMetadata"]
    Other,
    /// A placeholder when a host has no metadata, or when a reporter does not
    /// use metadata.
    NoMetadata,
}

impl AgentMetadata {
    /// Create a builder for EC2 agent metadata
    ///
    /// Normally, for real-world use, you would get the metadata using
    /// autodetection via
    #[cfg_attr(
        feature = "aws-metadata-no-defaults",
        doc = "[aws::load_agent_metadata],"
    )]
    #[cfg_attr(
        feature = "aws-metadata-no-defaults",
        doc = "`aws::load_agent_metadata`,"
    )]
    /// this function is intended for use in tests.
    ///
    /// ## Example
    ///
    /// ```rust
    /// # use async_profiler_agent::metadata::AgentMetadata;
    /// let metadata = AgentMetadata::ec2_agent_metadata(
    ///     "123456789012".to_string(),
    ///     "us-east-1".to_string(),
    ///     "i-1234567890abcdef0".to_string(),
    /// ).build();
    /// ```
    pub fn ec2_agent_metadata(
        aws_account_id: String,
        aws_region_id: String,
        ec2_instance_id: String,
    ) -> Ec2AgentMetadataBuilder {
        Ec2AgentMetadataBuilder {
            aws_account_id,
            aws_region_id,
            ec2_instance_id,
        }
    }

    /// Create a builder for Fargate agent metadata
    ///
    /// Normally, for real-world use, you would get the metadata using
    /// autodetection via
    #[cfg_attr(
        feature = "aws-metadata-no-defaults",
        doc = "[aws::load_agent_metadata],"
    )]
    #[cfg_attr(
        feature = "aws-metadata-no-defaults",
        doc = "`aws::load_agent_metadata`,"
    )]
    /// this function is intended for use in tests.
    ///
    /// ## Example
    ///
    /// ```rust
    /// # use async_profiler_agent::metadata::AgentMetadata;
    /// let metadata = AgentMetadata::fargate_agent_metadata(
    ///     "123456789012".to_string(),
    ///     "us-east-1".to_string(),
    ///     "arn:aws:ecs:us-east-1:123456789012:task/cluster/5261e761e0e2a3d92da3f02c8e5bab1f".to_string(),
    ///     "arn:aws:ecs:us-east-1:123456789012:cluster/profiler-metadata-cluster".to_string(),
    /// )
    /// .with_cpu_limit(0.25)
    /// .with_memory_limit(2048)
    /// .build();
    /// ```
    pub fn fargate_agent_metadata(
        aws_account_id: String,
        aws_region_id: String,
        ecs_task_arn: String,
        ecs_cluster_arn: String,
    ) -> FargateAgentMetadataBuilder {
        FargateAgentMetadataBuilder {
            aws_account_id,
            aws_region_id,
            ecs_task_arn,
            ecs_cluster_arn,
            cpu_limit: None,
            memory_limit: None,
        }
    }
}

/// Builder for EC2 agent metadata
#[derive(Debug, Clone)]
pub struct Ec2AgentMetadataBuilder {
    aws_account_id: String,
    aws_region_id: String,
    ec2_instance_id: String,
}

impl Ec2AgentMetadataBuilder {
    /// Build the AgentMetadata
    pub fn build(self) -> AgentMetadata {
        AgentMetadata::Ec2AgentMetadata {
            aws_account_id: self.aws_account_id,
            aws_region_id: self.aws_region_id,
            ec2_instance_id: self.ec2_instance_id,
        }
    }
}

/// Builder for Fargate agent metadata
#[derive(Debug, Clone)]
pub struct FargateAgentMetadataBuilder {
    aws_account_id: String,
    aws_region_id: String,
    ecs_task_arn: String,
    ecs_cluster_arn: String,
    cpu_limit: Option<f64>,
    memory_limit: Option<u64>,
}

impl FargateAgentMetadataBuilder {
    /// Set the CPU limit (in vCPUs)
    pub fn with_cpu_limit(mut self, cpu_limit: f64) -> Self {
        self.cpu_limit = Some(cpu_limit);
        self
    }

    /// Set the memory limit (in megabytes)
    pub fn with_memory_limit(mut self, memory_limit: u64) -> Self {
        self.memory_limit = Some(memory_limit);
        self
    }

    /// Build the AgentMetadata
    pub fn build(self) -> AgentMetadata {
        AgentMetadata::FargateAgentMetadata {
            aws_account_id: self.aws_account_id,
            aws_region_id: self.aws_region_id,
            ecs_task_arn: self.ecs_task_arn,
            ecs_cluster_arn: self.ecs_cluster_arn,
            #[cfg(feature = "__unstable-fargate-cpu-count")]
            cpu_limit: self.cpu_limit.map(Into::into),
            #[cfg(feature = "__unstable-fargate-cpu-count")]
            memory_limit: self.memory_limit,
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
