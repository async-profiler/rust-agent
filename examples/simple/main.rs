// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

use async_profiler_agent::{
    metadata::AgentMetadata,
    profiler::{ProfilerBuilder, ProfilerOptionsBuilder},
    reporter::{
        local::LocalReporter,
        s3::{S3Reporter, S3ReporterConfig},
    },
};
use std::time::Duration;

use aws_config::BehaviorVersion;
use clap::{ArgGroup, Parser};

mod slow;

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
#[command(group(
    ArgGroup::new("options")
        .required(true)
        .args(["local", "bucket"]),
))]
struct Args {
    #[arg(long)]
    profiling_group: Option<String>,
    #[arg(long)]
    bucket_owner: Option<String>,
    #[arg(long, requires = "bucket_owner", requires = "profiling_group")]
    bucket: Option<String>,
    #[arg(long)]
    local: Option<String>,
    #[arg(long)]
    #[clap(value_parser = humantime::parse_duration)]
    duration: Option<Duration>,
    #[arg(long, default_value = "30s")]
    #[clap(value_parser = humantime::parse_duration)]
    reporting_interval: Duration,
    #[arg(long)]
    worker_threads: Option<usize>,
    #[arg(long)]
    native_mem: Option<String>,
}

#[allow(unexpected_cfgs)]
pub fn main() -> anyhow::Result<()> {
    let args = Args::parse();

    let mut rt: tokio::runtime::Builder = tokio::runtime::Builder::new_multi_thread();
    rt.enable_all();
    if let Some(worker_threads) = args.worker_threads {
        rt.worker_threads(worker_threads);
    }

    #[cfg(tokio_unstable)]
    {
        rt.on_before_task_poll(|_| async_profiler_agent::pollcatch::before_poll_hook())
            .on_after_task_poll(|_| async_profiler_agent::pollcatch::after_poll_hook());
    }
    let rt = rt.build().unwrap();
    rt.block_on(main_internal(args))
}

async fn main_internal(args: Args) -> Result<(), anyhow::Error> {
    set_up_tracing();
    tracing::info!("main started");

    let profiler = ProfilerBuilder::default();

    let profiler = match (
        args.local,
        args.bucket,
        args.bucket_owner,
        args.profiling_group,
    ) {
        (Some(local), _, _, _) => profiler
            .with_reporter(LocalReporter::new(local))
            .with_custom_agent_metadata(AgentMetadata::Other),
        (_, Some(bucket), Some(bucket_owner), Some(profiling_group)) => {
            profiler.with_reporter(S3Reporter::new(S3ReporterConfig {
                sdk_config: &aws_config::defaults(BehaviorVersion::latest()).load().await,
                bucket_owner: bucket_owner,
                bucket_name: bucket,
                profiling_group_name: profiling_group,
            }))
        }
        _ => unreachable!(),
    };

    let mut builder = ProfilerOptionsBuilder::default();
    if let Some(native_mem) = &args.native_mem {
        builder = builder.with_native_mem(native_mem.clone());
    }
    let profiler_options = builder.build();

    let profiler = profiler
        .with_reporting_interval(args.reporting_interval)
        .with_profiler_options(profiler_options)
        .build();

    tracing::info!("starting profiler");
    profiler.spawn()?;
    tracing::info!("profiler started");

    if let Some(timeout) = args.duration {
        tokio::time::timeout(timeout, slow::run())
            .await
            .unwrap_err();
    } else {
        slow::run().await;
    }

    Ok(())
}
