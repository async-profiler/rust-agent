## async-profiler Rust agent

An in-process Rust agent for profiling an application using [async-profiler] and uploading the resulting profiles.

[async-profiler]: https://github.com/async-profiler/async-profiler

### OS/CPU Support

The async-profiler Rust agent currently only supports Linux, on either x86-64 or aarch64.

The async-profiler library supports Mac as well. This agent currently does not support Mac due to a lack of interest.

### Usage

The agent runs the profiler and uploads the output periodically via a reporter.

When starting, the profiler [dlopen(3)]'s `libasyncProfiler.so` and returns an `Err` if it is not found, so make sure there is a `libasyncProfiler.so` in the search path[^1].

[^1]: the dlopen search path includes RPATH and LD_LIBRARY_PATH, but *not* the current directory to avoid current directory attacks.
[dlopen(3)]: https://linux.die.net/man/3/dlopen

You can write your own reporter (via the `Reporter` trait) to upload the profile results to your favorite profiler backend.

You can use the S3 reporter, which uploads the reports to an S3 bucket, as follows:

```no_run
let bucket_owner = "<your account id>";
let bucket_name = "<your bucket name>";
let profiling_group = "a-name-to-give-the-uploaded-data";

let sdk_config = aws_config::defaults(BehaviorVersion::latest()).load().await;

let profiler = ProfilerBuilder::default()
    .with_reporter(S3Reporter::new(S3ReporterConfig {
        sdk_config: &sdk_config,
        bucket_owner: bucket_owner.into(),
        bucket_name: bucket_name.into(),
        profiling_group: profiling_group.into(),
    }))
    .build();

profiler.spawn()?;
```

The S3 reporter uploads each report in a `zip` file, that currently contains 2 files:
1. a [JFR] as `async_profiler_dump_0.jfr`
2. metadata as `metadata.json`, in format `reporter::s3::MetadataJson`.

The `zip` file is uploaded to the bucket under the path `profile_{profiling_group_name}_{machine}_{pid}_{time}.zip`,
where `{machine}` is either `ec2_{ec2_instance_id}_`, `ecs_{cluster_arn}_{task_arn}`, or `onprem__`.

[JFR]: https://docs.oracle.com/javacomponents/jmc-5-4/jfr-runtime-guide/about.htm

#### Sample program

You can test the agent by using the sample program, for example:

```
LD_LIBRARY_PATH=/path/to/libasyncProfiler.so cargo run --release --example simple -- --profiling-group PG --bucket-owner YOUR-AWS-ACCOUNT-ID --bucket YOUR_BUCKET_ID
```

### Host Metadata Auto-Detection

The Rust agent currently auto-detects the machine's [EC2] or [Fargate] id using [IMDS].

If you want to run the agent on a machine that is not EC2 or Fargate, you can use `profiler::ProfilerBuilder::with_custom_agent_metadata` to provide your own metadata.

The metadata is not used by the agent directly, and only provided to the reporters, to allow them to associate the profiling data with the correct host. In the S3 reporter, it's used to generate the `metadata.json` and zip file name.

[EC2]: https://aws.amazon.com/ec2
[Fargate]: https://aws.amazon.com/fargate
[IMDS]: https://docs.aws.amazon.com/AWSEC2/latest/UserGuide/ec2-instance-metadata.html

### PollCatch

If you want to find long poll times, and you have `RUSTFLAGS="--cfg tokio_unstable"`, you can
emit `tokio.PollCatchV1` events this way:

```
    #[cfg(tokio_unstable)]
    {
        rt.on_before_task_poll(|_| async_profiler_agent::pollcatch::before_poll_hook())
            .on_after_task_poll(|_| async_profiler_agent::pollcatch::after_poll_hook());
    }
```

## Security

See [CONTRIBUTING](CONTRIBUTING.md#security-issue-notifications) for more information.

## License

This project is licensed under the Apache-2.0 License.
