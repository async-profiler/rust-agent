## async-profiler Rust agent

[![crates.io](https://img.shields.io/crates/v/async-profiler-agent.svg)](https://crates.io/crates/async-profiler-agent)
[![Released API docs](https://docs.rs/async-profiler-agent/badge.svg)](https://docs.rs/async-profiler-agent)
[![Apache-2.0 licensed](https://img.shields.io/badge/license-Apache_2.0-blue.svg)](./LICENSE)
[![CI](https://github.com/async-profiler/rust-agent/actions/workflows/build.yml/badge.svg)](https://github.com/async-profiler/rust-agent/actions?query=workflow%3Abuild)

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
        profiling_group_name: profiling_group.into(),
    }))
    .build();

profiler.spawn()?;
```

The S3 reporter uploads each report in a `zip` file, that currently contains 2 files:
1. a [JFR] as `async_profiler_dump_0.jfr`
2. metadata as `metadata.json`, in format `reporter::s3::MetadataJson`.

The `zip` file is uploaded to the bucket under the path `profile_{profiling_group_name}_{machine}_{pid}_{time}.zip`,
where `{machine}` is either `ec2_{ec2_instance_id}_`, `ecs_{cluster_arn}_{task_arn}`, or `unknown__`.

In addition to the S3 reporter, `async-profiler-agent` also includes `LocalReporter` that writes to a directory, and a `MultiReporter` that allows combining reporters. You can also write your own reporter (via the `Reporter` trait) to upload the profile results to your favorite profiler backend.

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

### What information the profiler gathers

Memory samples (JFR `profiler.Malloc`) sample allocated memory every
so many bytes of allocated memory, and are matched by `profiler.Free`
to allow detecting if that memory is not free'd.

CPU-time samples (JFR `jdk.ExecutionSample`) sample only threads that
are currently running on a CPU, not threads that are sleeping.

Wall-clock samples (JFR `profiler.WallClockSample`) sample threads
whether they are sleeping or running, and can therefore be
very useful for finding threads that are blocked, for example
on a synchronous lock or a slow system call.

When using Tokio, since tasks are not threads, tasks that are not
currently running will not be sampled by a wall clock sample. However,
a wall clock sample is still very useful in Tokio, since it is what
you want to catch tasks that are blocking a thread by waiting on
synchronous operations.

The default is to do a wall-clock sample every second, and a CPU-time
sample every 100 CPU milliseconds. This can be configured via
[`ProfilerOptionsBuilder`].

Memory samples are not enabled by default, but can be enabled by [`with_native_mem_bytes`].

[`ProfilerOptionsBuilder`]: https://docs.rs/async-profiler-agent/0.1/async_profiler_agent/profiler/struct.ProfilerOptionsBuilder.html
[`with_native_mem_bytes`]: https://docs.rs/async-profiler-agent/0.1/async_profiler_agent/profiler/struct.ProfilerOptionsBuilder.html#method.with_native_mem_bytes

### Non-standard runtime configurations

The profiler always profiles an entire process. If a program has multiple Tokio (or non-Tokio, or non-Rust)
runtimes, it will profile all of them mostly without problems.

Even mixing native code and JVM in the same process works, no matter whether
async-profiler is started from Rust or Java, though there are occasionally bugs
there - if you are mixing native code and JVM and are encountering weird
problems, you should [report an issue to async-profiler].

Since async-profiler uses process-global resources such as signal handlers, a
process can only have one active instance of async-profiler at a time. This
applies across languages as well - if you have both native code and JVM code in
your process, only one of them should be starting async-profiler.

The most-often used [`Profiler::spawn`] and [`Profiler::spawn_controllable`] functions assume that they are run within
a Tokio runtime. The S3 reporter performs AWS SDK calls within that runtime, and therefore it
assumes that the runtime is appropriate for performing AWS SDK calls. Most Tokio applications should just
spawn async-profiler within their (primary and only) Tokio runtime.

Some services have especially strict (tens-of-microseconds or less) latency requirements, and therefore have
a "data plane" (either non-Tokio or weirdly-configured Tokio) runtime, and in addition to that a
normally-configured (or normally-configured but high-[niceness]) "control plane" runtime that
is suitable for performing AWS SDK calls. In these cases, it makes sense to spawn the async-profiler agent
within the "control plane" runtime.

Other applications just don't use Tokio for their main code. These applications can use
[`Profiler::spawn_thread`] and its variants to spawn async-profiler in a separate thread
that will come with a Tokio runtime managed by `async-profiler-agent`.

In all of these cases, the [pollcatch](#pollcatch) hooks should be enabled on the
runtime where you *intend to be catching long polls on* - presumably your data-plane runtime. They do not
introduce much overhead or unpredictable latency.

[report an issue to async-profiler]: https://github.com/async-profiler/async-profiler/issues
[`Profiler::spawn`]: https://docs.rs/async-profiler-agent/0.1/async_profiler_agent/profiler/struct.Profiler.html#method.spawn
[`Profiler::spawn_controllable`]: https://docs.rs/async-profiler-agent/0.1/async_profiler_agent/profiler/struct.Profiler.html#method.spawn_controllable
[`Profiler::spawn_thread`]: https://docs.rs/async-profiler-agent/0.1/async_profiler_agent/profiler/struct.Profiler.html#method.spawn_thread
[niceness]: https://linux.die.net/man/2/nice

### PollCatch

If you want to find long poll times, and you have `RUSTFLAGS="--cfg tokio_unstable"`, you can
emit `tokio.PollCatchV1` events this way:

```rust
    #[cfg(tokio_unstable)]
    {
        rt.on_before_task_poll(|_| async_profiler_agent::pollcatch::before_poll_hook())
            .on_after_task_poll(|_| async_profiler_agent::pollcatch::after_poll_hook());
    }
```

If you can't use `tokio_unstable`, it is possible to wrap your tasks by instrumentation that calls
`before_poll_hook` before user code runs and `after_poll_hook` after user code runs, but that
runs the risk of forgetting to instrument the task that is actually causing the high latency,
and therefore it is strongly recommended to use `on_before_task_poll`/`on_after_task_poll`.

#### Using pollcatch without the agent

The recommended way of using async-profiler-agent is via async-profiler-agent's agent. However, in case your
application is already integrated with some other mechanism that calls `async-profiler`, the
`on_before_task_poll`/`on_after_task_poll` hooks just call the async-profiler [JFR Event API]. They can be used
even if async-profiler is run via a mechanism different from the async-profiler Rust agent (for example, a
Java-native async-profiler integration), though currently, the results from the JFR Event API are only exposed in
async-profiler's JFR-format output mode.

You can see the `test_pollcatch_without_agent.sh` for an example that uses pollcatch with just async-profiler's
`LD_PRELOAD` mode.

However, in that case, it is only needed that the pollcatch hooks refer to the same `libasyncProfiler.so` that is
being used as a profiler, since the JFR Event API is based on global variables that must match. async-profiler-agent
uses [libloading] which uses [dlopen(3)] (currently passing [`RTLD_LOCAL | RTLD_LAZY`][libloadingflags]), which
performs [deduplication based on inode]. Therefore, if your system only has a single `libasyncProfiler.so`
on the search path, it will be shared and pollcatch will work.

The async-profiler-agent crate currently does not expose the JFR Event API to users, due to stability
reasons. As a user, using `libloading` to open `libasyncProfiler.so` and calling the API yourself
will work, but if you have a use case for the JFR Event API, consider opening an issue.

[deduplication based on inode]: https://stackoverflow.com/questions/45954861/how-to-circumvent-dlopen-caching/45955035#45955035
[JFR Event API]: https://github.com/async-profiler/async-profiler/blob/master/src/asprof.h#L99
[libloading]: https://crates.io/crates/libloading
[libloadingflags]: https://docs.rs/libloading/latest/libloading/os/unix/struct.Library.html#method.new

### Not enabling the AWS SDK / Reqwest default features

The `aws-metadata-no-defaults` and `s3-no-defaults` feature flags do not enable feature flags for the AWS SDK and `reqwest`. 

If you want things to work, you'll need to enable features for these crates to allow at least a TLS provider. This can be used to use a TLS provider other than the default Rustls

## Decoder

The `decoder` directory in the Git repository contains a decoder that can be used to view JFR files, especially with PollCatch information.

The decoder is NOT intended right now to be used in production. In particular, it uses the [`jfrs`] crate for parsing `.jfr` files, and while that crate seems to be purely safe Rust, to my knowledge it has not been audited for security and probably contains at least denial-of-service issues if not worse.

If you want to use the decoder for anything but debugging on trusted `.jfr` files, you bear full responsibility for the consequences.

To use the decoder, you can download the `.zip` file from s3, and then run it:
```
aws s3 cp s3://your-bucket/YOUR_PROFILE.zip .
# the last parameter is the long poll threshold
./decoder/target/release/pollcatch-decoder longpolls --zip profile_WHATEVER_*.zip 500us
```

The output should look like this
```
[930689.953296] thread 60898 - poll of 8885us
 -   1: libpthread-2.26.so.__nanosleep
 -   2: simple.std::thread::sleep_ms
 -   3: simple.simple::slow::accidentally_slow
 -   4: simple.simple::slow::accidentally_slow_2
 -   5: simple.simple::slow::run::{{closure}}::{{closure}}
 -  16 more frame(s) (pass --stack-depth=21 to show)

[930691.953294] thread 60898 - poll of 736us
 -   1: libpthread-2.26.so.__nanosleep
 -   2: simple.std::thread::sleep_ms
 -   3: simple.simple::slow::accidentally_slow
 -   4: simple.simple::slow::accidentally_slow_2
 -   5: simple.simple::slow::run::{{closure}}::{{closure}}
 -  16 more frame(s) (pass --stack-depth=21 to show)

[930709.953293] thread 60898 - poll of 2736us
 -   1: libpthread-2.26.so.__nanosleep
 -   2: simple.std::thread::sleep_ms
 -   3: simple.simple::slow::accidentally_slow
 -   4: simple.simple::slow::accidentally_slow_2
 -   5: simple.simple::slow::run::{{closure}}::{{closure}}
 -  16 more frame(s) (pass --stack-depth=21 to show)
```

If it does not work, make sure you are using the most recent version of `async-profiler` and that you enabled the pollcatch hooks.

### Missing samples

If you are missing pollcatch samples, you can also run the decoder with `--include-non-pollcatch` and a time
filter of 0 us to see all samples, as in:

```
./decoder/target/release/pollcatch-decoder longpolls --include-non-pollcatch --zip profile_WHATEVER_*.zip
```

#### Samples are still not showing

If the samples are still missing, this is probably a problem with async-profiler or its use. Make sure you are using
async-profiler correctly.

#### Sample found with "sample with no pollcatch information"

If this discovers the samples as "sample with no pollcatch information", for example:

```
[87.831540] thread 1878 - sample with no pollcatch information
 -   1: libc.so.6.clock_nanosleep
 -  52 more frame(s) (pass --stack-depth=53 to show)
```

It is likely that you do not have pollcatch enabled correctly. Make sure you are using the [`before_poll_hook`
and `after_poll_hook` hooks][pollcatch], which normally requires using `--cfg tokio_unstable`. Also make sure that
you are using async-profiler that supports user events (`v4.0` or higher, if you are using official versions
from upstream).

[pollcatch]: #pollcatch

### Dealing with `unknown` frames / truncated stack traces

If you are seeing `unknown` frames, this indicates that `async-profiler` has problems finding unwinding
information. There are 2 normal causes for that:

1. Your code, or the code of some library you are using, is missing debuginfo. To fix that, compile your
   code with `debuginfo` or at least `debuginfo-lines`.
2. The running stack frame contains code that is generated by a JIT. Current versions of `async-profiler`
   support unwinding JVM frames, but frames generated by other JITs will show up as `unknown`.

`async-profiler` is often not capable of unwinding past `unknown` frames, so they will often "truncate"
the stack trace which will not go past one or two `unknown` frames.

[`jfrs`]: https://docs.rs/jfrs

### Dealing with `Error: decoder only supports tsc profiles, not "monotonic"`

Pollcatch uses the CPU timestamp counter (tsc) to correlate profiles, and requires async-profiler to use the
CPU timestamp counter as well. async-profiler normally tries to use the tsc, and only reverts to using the
`monotonic` counter if it's unable to.

Potential causes for async-profiler not being able to use the timestamp counter:

1. An architecture that is not supported by async-profiler (currently, only x86-64 and aarch64 are supported)
2. If you are loading a Java Virtual Machine (JVM) into your process, async-profiler tries to coordinate with
   the JVM's timestamp mechanism. If that fails due to some JVM support or integration issue, it will revert to
   using the monotonic clock, which will not work with pollcatch.

   If you encounter this problem, you can work-around it by initializing the async-profiler Rust agent
   before you initialize the JVM. This will make the Java-specific analyses work less well, but they are
   generally less important in a pollcatch context.
   
In either case, it might be worth reporting an issue to the async-profiler team.

## Security

See [CONTRIBUTING](CONTRIBUTING.md#security-issue-notifications) for more information.

## License

This project is licensed under the Apache-2.0 License.