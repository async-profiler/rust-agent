// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

use async_profiler_agent::{
    profiler::ProfilerBuilder,
    reporter::s3::{S3Reporter, S3ReporterConfig},
};
use std::time::Duration;

use aws_config::BehaviorVersion;
use clap::Parser;

pub fn set_up_tracing() {
    use tracing_subscriber::{prelude::*, EnvFilter};

    let format = tracing_subscriber::fmt::layer().pretty();
    let filter = EnvFilter::try_from_default_env()
        .or_else(|_| EnvFilter::try_new("info"))
        .unwrap();
    tracing_subscriber::registry()
        .with(format)
        .with(filter)
        .init();
}

/// Simple program to test the profiler agent
#[derive(Parser, Debug)]
struct Args {
    #[arg(long)]
    profiling_group: String,
    #[arg(long)]
    bucket_owner: String,
    #[arg(long)]
    bucket: String,
}

#[tokio::main]
async fn main() -> Result<(), anyhow::Error> {
    set_up_tracing();
    tracing::info!("main started");

    let args = Args::parse();

    let sdk_config = aws_config::defaults(BehaviorVersion::latest()).load().await;
    let reporting_interval = Duration::from_secs(30);

    let profiler = ProfilerBuilder::default()
        .with_reporter(S3Reporter::new(S3ReporterConfig {
            sdk_config: &sdk_config,
            bucket_owner: args.bucket_owner,
            bucket_name: args.bucket,
            profiling_group_name: args.profiling_group,
        }))
        .with_reporting_interval(reporting_interval)
        .build();

    tracing::info!("starting profiler");
    profiler.spawn()?;
    tracing::info!("profiler started");

    let sleep_secs = 6;
    let sleep_duration = Duration::from_secs(sleep_secs);
    let mut random_string: String = String::with_capacity(1);
    loop {
        random_string.push('a');

        tracing::info!("inside loop: going to sleep {sleep_secs} seconds");
        tokio::time::sleep(sleep_duration).await;
        tracing::info!("application woke up");

        random_string.pop();
    }
}
