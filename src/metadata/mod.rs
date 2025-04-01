// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

pub use std::time::Duration;

#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum AgentMetadata {
    Ec2AgentMetadata {
        aws_account_id: String,
        aws_region_id: String,
        ec2_instance_id: String,
    },
    FargateAgentMetadata {
        aws_account_id: String,
        aws_region_id: String,
        ecs_task_arn: String,
        ecs_cluster_arn: String,
    },
    Other,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReportMetadata<'a> {
    pub instance: &'a AgentMetadata,
    pub start: Duration,
    pub end: Duration,
    pub reporting_interval: Duration,
}

#[cfg(feature="aws-metadata")]
pub mod aws;
