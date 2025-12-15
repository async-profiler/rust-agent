# Design Document: async-profiler Rust Agent

## Overview

The async-profiler Rust agent is an in-process profiling library that integrates with [async-profiler](https://github.com/async-profiler/async-profiler) to collect performance data and upload it to various backends. The agent is designed to run continuously in production environments with minimal overhead.

For a more how-to-focused guide on running the profiler in various contexts, read the README.

This guide is based on an AI-driven summary, but it includes many comments from the development team.

This is a *design* document. It does not make stability promises and can change at any time.

## Architecture

The async-profiler agent runs as an agent within a Rust process and profiles it using [async-profiler].

async-profiler is loaded, currently the agent only supports loading a `libasyncProfiler.so` dynamically
via [libloading], but in future versions it might also be possible to statically or plain-dynamically
link against it.

The async-profiler configuration is controlled by the user, though only a limited set of configurations
is made available to control the support burden.

The agent collects periodic profiling [JFR]s and sends them to a reporter, which uploads them to
some location. The library supports a file-based reporter, that stores the JFRs on the filesystem,
and S3-based reporters, which upload the JFRs from async-profiler after wrapping them into a
`zip`. The library also allows users to implement their own reporters.

The agent can also perform autodetection of AWS IMDS metadata, which is passed to the reporter
as an argument, and in the S3-based reporter, used to determine the name of the uploaded files.

In addition, the library includes a Tokio integration for pollcatch, which allows detecting
long polls in Tokio applications. That integration uses the same `libasyncProfiler.so`
as the rest of the agent but is otherwise independent.

[async-profiler]: https://github.com/async-profiler/async-profiler
[libloading]: https://crates.io/crates/libloading
[JFR]: https://docs.oracle.com/javacomponents/jmc-5-4/jfr-runtime-guide/about.htm

## Code Architecture

The crate follows a modular architecture with clear separation of concerns:

```
async-profiler-agent/
├── src/
│   ├── lib.rs              # Public API and documentation
│   ├── profiler.rs         # Core profiler orchestration
│   ├── asprof/             # async-profiler FFI bindings
│   ├── metadata/           # Host and report metadata
│   ├── pollcatch/          # Tokio poll time tracking
│   └── reporter/           # Data upload backends
├── examples/               # Sample applications
├── decoder/                # JFR analysis tool
└── tests/                  # Integration tests
```

## Core Modules

### 1. Profiler (`profiler`)

**Purpose**: Central orchestration of profiling lifecycle and data collection.

**Key Components**:
- `Profiler` & `ProfilerBuilder`: Main entry point for starting profiling
- `ProfilerOptions`: Profiling behavior configuration
- `RunningProfiler`: Handle for controlling active profiler
- `ProfilerEngine` trait: used to allow mocking async-profiler (the C library) during tests

#### Profiler lifecycle management

As of async-profiler version 4.1, async-profiler does not have a mode where it can run continuously
with bounded memory usage and periodically collect samples.

Therefore, every [`reporting_interval`] seconds, the async-profiler agent restarts async-profiler by sending a `stop` (which flushes the JFR file) and `start` commands.

This is managed by `Profiler` (see the [`profiler_tick`] function).

This is a supported async-profiler operation mode.

[`reporting_interval`]: https://docs.rs/async-profiler-agent/0.1/async_profiler_agent/profiler/struct.ProfilerBuilder.html#method.with_reporting_interval
[`profiler_tick`]: https://github.com/async-profiler/rust-agent/blob/506718fff274b49cf2eb03305a4f9547b61720e3/src/profiler.rs#L1083

#### Agent lifecycle management

The async-profiler agent can be stopped and started at run-time.

When stopped, the async-profiler agent stops async-profiler, flushes the last profile to the reporter, and then
releases the stop handle from waiting. After the stop is done, it is possible to start a different instance of
the async-profiler agent on the same process.

The start/stop functionality is useful for several purposes:

1. "Chicken bit" stopping of the profiler if it causes application issues.
2. Stopping and starting a profiler with new configuration.
3. Stopping the profiler and uploading the last sample before application exit.

The profiler intentionally does *not* automatically flush the last profile on `Drop`. This is because
reporters can take an arbitrary amount of time to finish, and slowing an application on exit is likely 
to be a worse default than missing some profiling samples.

#### Profiler configuration

async-profiler is configured via [`ProfilerOptions`] and [`ProfilerOptionsBuilder`]. You
should read these docs along with the [async-profiler options docs], for more details.

[`ProfilerOptions`]: https://docs.rs/async-profiler-agent/0.1/async_profiler_agent/profiler/struct.ProfilerOptionsBuilder.html
[`ProfilerOptionsBuilder`]: https://docs.rs/async-profiler-agent/0.1/async_profiler_agent/profiler/struct.ProfilerOptionsBuilder.html
[async-profiler options docs]: https://github.com/async-profiler/async-profiler/blob/v4.0/docs/ProfilerOptions.md

#### JFR file rotation

async-profiler expects to be writing the current JFR to a "fresh" file path. To that
effect, async-profiler creates 2 unnamed temporary files via `JfrFile`, and gives to
async-profiler alternating paths of the form `/proc/self/fd/<N>` to write the
JFRs into.

### 2. async-profiler FFI (`asprof`)

**Purpose**: Safe Rust bindings to the native async-profiler library.

**Key Components**:
- `AsProf`: Safe interface to async-profiler
- `raw`: Low-level FFI declarations

**Responsibilities**:
- Dynamic loading of `libasyncProfiler.so` using [`libloading`]
- Safe, Rust-native wrappers around C API calls

[libloading]: https://crates.io/crates/libloading

### 3. Metadata (`metadata/`)

**Purpose**: Host identification and report context information.

**Key Components**:
- `AgentMetadata`: Host identification (EC2, Fargate, or generic)
- `aws`: AWS-specific metadata autodetection via IMDS

The metadata is sent to the [`Reporter`] implementation, and can be used to
identify the host that generated a particular profiling report. In the local reporter,
it is ignored. In the S3 reporter, it is used to determine the uploaded file name.

### 4. Reporters (`reporter/`)

**Purpose**: Pluggable backends for uploading profiling data.

**Key Components**:
- [`Reporter`] trait: Common interface for all backends
- [`LocalReporter`]: Filesystem output for development/testing
- [`S3Reporter`]: AWS S3 upload with metadata
- [`MultiReporter`]: Composition of multiple reporters

[`Reporter`]: https://docs.rs/async-profiler-agent/0.1/async_profiler_agent/reporter/trait.Reporter.html
[`LocalReporter`]: https://docs.rs/async-profiler-agent/0.1/async_profiler_agent/reporter/local/struct.LocalReporter.html
[`S3Reporter`]: https://docs.rs/async-profiler-agent/0.1/async_profiler_agent/reporter/s3/struct.S3Reporter.html
[`MultiReporter`]: https://docs.rs/async-profiler-agent/0.1/async_profiler_agent/reporter/multi/struct.MultiReporter.html

The reporter trait is as follows:

```rust
#[async_trait]
pub trait Reporter: fmt::Debug {
    async fn report(
        &self,
        jfr: Vec<u8>,
        metadata: &ReportMetadata,
    ) -> Result<(), Box<dyn std::error::Error + Send>>;
}
```

Customers whose needs are not suited by the built-in reporters might write their
own reporters.

### 5. PollCatch (`pollcatch/`)

**Purpose**: Tokio-specific instrumentation for detecting long poll times.

**Key Components**:
- `before_poll_hook()`: Pre-poll timestamp capture
- `after_poll_hook()`: Post-poll analysis and reporting
- `tsc.rs`: CPU timestamp counter utilities, works on x86 and ARM

The idea of pollcatch is that if a wall-clock profiling event happens in the middle of a Tokio poll,
when that Tokio poll *ends*, a `tokio.PollCatchV1` event is emitted that contains the start and
end times of that poll, and therefore it is possible to correlate long polls with stack traces
that happen within them.

The way this is done is that `before_poll_hook` saves the before timestamp and the async-profiler
`sample_counter` from `asprof_thread_local_data` into a (private) thread-local variable, and
`after_poll_hook` checks if the `sample_counter` changes, emits a `tokio.PollCatchV1` event
containing the stored before-timestamp and the current timestamp as an after-timestamp.

By emitting only 1 `tokio.PollCatchV1` event per wall-clock profiling event, the pollcatch profiling overhead
is kept bounded and low.

By only emitting the event at the `after_poll_hook`, which is normally run as a Tokio after-poll hook,
the event is basically emitted "at the Tokio main loop", in a context where "no locks are held" and
is outside of a signal handler.

The `tokio.PollCatchV1` event contains the following payload:

```rust
before_timestamp: LittleEndianU64,
after_timestamp: LittleEndianU64,
```

Where both timestamps come from the TSC. The pollcatch decoder uses the fact that the async-profiler profiling samples
contain a clock which uses the same TSC to correlate profiling samples corresponding to a single Tokio poll (though
normally, since the wall-clock interval is normally 1/second, unless a Tokio poll is *horribly* slow it will bracket at
most a single sample) - and in addition, to determine how long that particular poll is by observing the
difference between the timestamps.

## The decoder (`decoder/`)

The decoder is a JFR decoder using `jfrs` that can decode the JFRs from async-profiler and display pollcatch
metadata in a nice format.

The decoder implementation is quite ugly currently.

## Data Flow

1. **Initialization**: Profiler loads `libasyncProfiler.so` and initializes
2. **Session Start**: Creates temporary JFR files and starts async-profiler
3. **Continuous Profiling**: async-profiler collects samples to active JFR file
4. **Periodic Reporting**: 
   - Stop profiler and rotate JFR files
   - Read completed JFR data
   - Package with metadata
   - Upload via configured reporters
   - Restart profiler with new JFR file
5. **Shutdown**: Stop profiler and perform final report

## Feature Flags

All AWS dependencies are optional and only enabled if an AWS feature flag is passed.

In addition, for every AWS feature flag, there is an "X-no-defaults" version of that flag
that does not enable default features for the AWS libraries.

The main reason for this design is that the AWS SDK needs to have a selected TLS backend
in order to connect to https services, but users might want to enable a backend other
than the default one and not have the default backend linked in to their executable.

- `s3`: Full S3 reporter with default AWS SDK features
- `s3-no-defaults`: S3 reporter without default features (for custom TLS)
- `aws-metadata`: AWS metadata detection with default features
- `aws-metadata-no-defaults`: AWS metadata without default features
- `__unstable-fargate-cpu-count`: Tombstone for experimental Fargate CPU metrics