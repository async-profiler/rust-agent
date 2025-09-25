// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

//! A profiler that periodically uploads profiling samples of your program to a [Reporter]

use crate::{
    asprof::{self, AsProfError},
    metadata::{AgentMetadata, ReportMetadata},
    reporter::{local::LocalReporter, Reporter},
};
use std::{
    fs::File,
    io,
    path::{Path, PathBuf},
    time::{Duration, SystemTime, SystemTimeError, UNIX_EPOCH},
};
use thiserror::Error;

struct JfrFile {
    active: std::fs::File,
    inactive: std::fs::File,
}

impl JfrFile {
    #[cfg(target_os = "linux")]
    fn new() -> Result<Self, io::Error> {
        Ok(Self {
            active: tempfile::tempfile().unwrap(),
            inactive: tempfile::tempfile().unwrap(),
        })
    }

    #[cfg(not(target_os = "linux"))]
    fn new() -> Result<Self, io::Error> {
        io::Error::new(
            io::ErrorKind::Other,
            "async-profiler is only supported on Linux",
        )
    }

    fn swap(&mut self) {
        std::mem::swap(&mut self.active, &mut self.inactive);
    }

    #[cfg(target_os = "linux")]
    fn file_path(file: &std::fs::File) -> PathBuf {
        use std::os::fd::AsRawFd;

        format!("/proc/self/fd/{}", file.as_raw_fd()).into()
    }

    #[cfg(not(target_os = "linux"))]
    fn file_path(_file: &std::fs::File) -> PathBuf {
        unimplemented!()
    }

    fn active_path(&self) -> PathBuf {
        Self::file_path(&self.active)
    }

    fn inactive_path(&self) -> PathBuf {
        Self::file_path(&self.inactive)
    }

    fn empty_inactive_file(&mut self) -> Result<(), io::Error> {
        // Empty the file, or create it for the first time if the profiler hasn't
        // started yet.
        File::create(Self::file_path(&self.inactive))?;
        tracing::debug!(message = "emptied the file");
        Ok(())
    }
}

/// Options for configuring the async-profiler behavior.
/// Currently supports:
/// - Native memory allocation tracking
#[derive(Debug, Default)]
#[non_exhaustive]
pub struct ProfilerOptions {
    /// If set, the profiler will collect information about
    /// native memory allocations.
    ///
    /// The value is the interval in bytes or in other units,
    /// if followed by k (kilobytes), m (megabytes), or g (gigabytes).
    /// For example, `"10m"` will sample an allocation for every
    /// 10 megabytes of memory allocated. Passing `"0"` will sample
    /// all allocations.
    ///
    /// See [ProfilingModes in the async-profiler docs] for more details.
    ///
    /// [ProfilingModes in the async-profiler docs]: https://github.com/async-profiler/async-profiler/blob/v4.0/docs/ProfilingModes.md#native-memory-leaks
    pub native_mem: Option<String>,
    cpu_interval: Option<u128>,
    wall_clock_millis: Option<u128>,
}

const DEFAULT_CPU_INTERVAL_NANOS: u128 = 100_000_000;
const DEFAULT_WALL_CLOCK_INTERVAL_MILLIS: u128 = 1_000;

impl ProfilerOptions {
    /// Convert the profiler options to a string of arguments for the async-profiler.
    pub fn to_args_string(&self, jfr_file_path: &std::path::Path) -> String {
        let mut args = format!(
            "start,event=cpu,interval={},wall={}ms,jfr,cstack=dwarf,file={}",
            self.cpu_interval.unwrap_or(DEFAULT_CPU_INTERVAL_NANOS),
            self.wall_clock_millis
                .unwrap_or(DEFAULT_WALL_CLOCK_INTERVAL_MILLIS),
            jfr_file_path.display()
        );
        if let Some(ref native_mem) = self.native_mem {
            args.push_str(&format!(",nativemem={native_mem}"));
        }
        args
    }
}

/// Builder for [`ProfilerOptions`].
#[derive(Debug, Default)]
pub struct ProfilerOptionsBuilder {
    native_mem: Option<String>,
    cpu_interval: Option<u128>,
    wall_clock_millis: Option<u128>,
}

impl ProfilerOptionsBuilder {
    /// Same as [ProfilerOptionsBuilder::with_native_mem_bytes], but pass
    /// the string input directly to async_profiler.
    ///
    /// The value is the interval in bytes or in other units,
    /// if followed by k (kilobytes), m (megabytes), or g (gigabytes).
    ///
    /// Prefer using [ProfilerOptionsBuilder::with_native_mem_bytes], since it's
    /// type-checked.
    ///
    /// ### Examples
    ///
    /// This will sample allocations for every 10 megabytes allocated:
    ///
    /// ```
    /// # use async_profiler_agent::profiler::{ProfilerBuilder, ProfilerOptionsBuilder};
    /// # use async_profiler_agent::profiler::SpawnError;
    /// # fn main() -> Result<(), SpawnError> {
    /// let opts = ProfilerOptionsBuilder::default().with_native_mem("10m".into()).build();
    /// let profiler = ProfilerBuilder::default()
    ///     .with_profiler_options(opts)
    ///     .with_local_reporter("/tmp/profiles")
    ///     .build();
    /// # if false { // don't spawn the profiler in doctests
    /// profiler.spawn()?;
    /// # }
    /// # Ok(())
    /// # }
    /// ```
    pub fn with_native_mem(mut self, native_mem_interval: String) -> Self {
        self.native_mem = Some(native_mem_interval);
        self
    }

    /// If set, the profiler will collect information about
    /// native memory allocations.
    ///
    /// The argument passed is the profiling interval - the profiler will
    /// sample allocations every about that many bytes.
    ///
    /// See [ProfilingModes in the async-profiler docs] for more details.
    ///
    /// [ProfilingModes in the async-profiler docs]: https://github.com/async-profiler/async-profiler/blob/v4.0/docs/ProfilingModes.md#native-memory-leaks
    ///
    /// ### Examples
    ///
    /// This will sample allocations for every 10 megabytes allocated:
    ///
    /// ```
    /// # use async_profiler_agent::profiler::{ProfilerBuilder, ProfilerOptionsBuilder};
    /// # use async_profiler_agent::profiler::SpawnError;
    /// # fn main() -> Result<(), SpawnError> {
    /// let opts = ProfilerOptionsBuilder::default().with_native_mem_bytes(10_000_000).build();
    /// let profiler = ProfilerBuilder::default()
    ///     .with_profiler_options(opts)
    ///     .with_local_reporter("/tmp/profiles")
    ///     .build();
    /// # if false { // don't spawn the profiler in doctests
    /// profiler.spawn()?;
    /// # }
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// This will sample every allocation (potentially slow):
    /// ```
    /// # use async_profiler_agent::profiler::{ProfilerBuilder, ProfilerOptionsBuilder};
    /// # use async_profiler_agent::profiler::SpawnError;
    /// # fn main() -> Result<(), SpawnError> {
    /// let opts = ProfilerOptionsBuilder::default().with_native_mem_bytes(0).build();
    /// let profiler = ProfilerBuilder::default()
    ///     .with_profiler_options(opts)
    ///     .with_local_reporter("/tmp/profiles")
    ///     .build();
    /// # if false { // don't spawn the profiler in doctests
    /// profiler.spawn()?;
    /// # }
    /// # Ok(())
    /// # }
    /// ```
    pub fn with_native_mem_bytes(mut self, native_mem_interval: usize) -> Self {
        self.native_mem = Some(native_mem_interval.to_string());
        self
    }

    /// Sets the interval in which the profiler will collect
    /// CPU-time samples, via the [async-profiler `interval` option].
    ///
    /// CPU-time samples (JFR `jdk.ExecutionSample`) sample only threads that
    /// are currently running on a CPU, not threads that are sleeping.
    ///
    /// It can use a higher frequency than wall-clock sampling since the
    /// number of the threads that are running on a CPU at a given time is
    /// naturally limited by the number of CPUs, while the number of sleeping
    /// threads can be much larger.
    ///
    /// The default is to do a CPU-time sample every 100 milliseconds.
    ///
    /// The async-profiler agent collects both CPU time and wall-clock time
    /// samples, so this function should normally be used along with
    /// [ProfilerOptionsBuilder::with_wall_clock_interval].
    ///
    /// [async-profiler `interval` option]: https://github.com/async-profiler/async-profiler/blob/v4.0/docs/ProfilerOptions.md#options-applicable-to-any-output-format
    ///
    /// ### Examples
    ///
    /// This will sample allocations for every 10 CPU milliseconds (when running)
    /// and 100 wall-clock milliseconds (running or sleeping):
    ///
    /// ```
    /// # use async_profiler_agent::profiler::{ProfilerBuilder, ProfilerOptionsBuilder};
    /// # use async_profiler_agent::profiler::SpawnError;
    /// # use std::time::Duration;
    /// # fn main() -> Result<(), SpawnError> {
    /// let opts = ProfilerOptionsBuilder::default()
    ///     .with_cpu_interval(Duration::from_millis(10))
    ///     .with_wall_clock_interval(Duration::from_millis(100))
    ///     .build();
    /// let profiler = ProfilerBuilder::default()
    ///     .with_profiler_options(opts)
    ///     .with_local_reporter("/tmp/profiles")
    ///     .build();
    /// # if false { // don't spawn the profiler in doctests
    /// profiler.spawn()?;
    /// # }
    /// # Ok(())
    /// # }
    /// ```
    pub fn with_cpu_interval(mut self, cpu_interval: Duration) -> Self {
        self.cpu_interval = Some(cpu_interval.as_nanos());
        self
    }

    /// Sets the interval, in milliseconds, in which the profiler will collect
    /// wall-clock samples, via the [async-profiler `wall` option].
    ///
    /// Wall-clock samples (JFR `profiler.WallClockSample`) sample threads
    /// whether they are sleeping or running, and can therefore be
    /// very useful for finding threads that are blocked, for example
    /// on a synchronous lock or a slow system call.
    ///
    /// When using Tokio, since tasks are not threads, tasks that are not
    /// currently running will not be sampled by a wall clock sample. However,
    /// a wall clock sample is still very useful in Tokio, since it is what
    /// you want to catch tasks that are blocking a thread by waiting on
    /// synchronous operations.
    ///
    /// The default is to do a wall-clock sample every second.
    ///
    /// The async-profiler agent collects both CPU time and wall-clock time
    /// samples, so this function should normally be used along with
    /// [ProfilerOptionsBuilder::with_cpu_interval].
    ///
    /// [async-profiler `wall` option]: https://github.com/async-profiler/async-profiler/blob/v4.0/docs/ProfilerOptions.md#options-applicable-to-any-output-format
    ///
    /// ### Examples
    ///
    /// This will sample allocations for every 10 CPU milliseconds (when running)
    /// and 100 wall-clock milliseconds (running or sleeping):
    ///
    /// ```
    /// # use async_profiler_agent::profiler::{ProfilerBuilder, ProfilerOptionsBuilder};
    /// # use async_profiler_agent::profiler::SpawnError;
    /// # use std::time::Duration;
    /// # fn main() -> Result<(), SpawnError> {
    /// let opts = ProfilerOptionsBuilder::default()
    ///     .with_cpu_interval(Duration::from_millis(10))
    ///     .with_wall_clock_interval(Duration::from_millis(10))
    ///     .build();
    /// let profiler = ProfilerBuilder::default()
    ///     .with_profiler_options(opts)
    ///     .with_local_reporter("/tmp/profiles")
    ///     .build();
    /// # if false { // don't spawn the profiler in doctests
    /// profiler.spawn()?;
    /// # }
    /// # Ok(())
    /// # }
    /// ```
    pub fn with_wall_clock_interval(mut self, wall_clock: Duration) -> Self {
        self.wall_clock_millis = Some(wall_clock.as_millis());
        self
    }

    /// Build the [`ProfilerOptions`] from the builder.
    pub fn build(self) -> ProfilerOptions {
        ProfilerOptions {
            native_mem: self.native_mem,
            wall_clock_millis: self.wall_clock_millis,
            cpu_interval: self.cpu_interval,
        }
    }
}

/// Builds a [`Profiler`], panicking if any required fields were not set by the
/// time `build` is called.
#[derive(Debug, Default)]
pub struct ProfilerBuilder {
    reporting_interval: Option<Duration>,
    reporter: Option<Box<dyn Reporter + Send + Sync>>,
    agent_metadata: Option<AgentMetadata>,
    profiler_options: Option<ProfilerOptions>,
}

impl ProfilerBuilder {
    /// Sets the reporting interval (default: 30 seconds).
    ///
    /// This is the interval that samples are *reported* to the backend,
    /// and is unrelated to the interval at which the application
    /// is *sampled* by async profiler, which is controlled by
    /// [ProfilerOptionsBuilder::with_cpu_interval] and
    /// [ProfilerOptionsBuilder::with_wall_clock_interval].
    ///
    /// Most users should not change this setting.
    ///
    /// ## Example
    ///
    /// ```no_run
    /// # use std::path::PathBuf;
    /// # use std::time::Duration;
    /// # use async_profiler_agent::profiler::{ProfilerBuilder, SpawnError};
    /// # let path = PathBuf::from(".");
    /// let agent = ProfilerBuilder::default()
    ///     .with_local_reporter(path)
    ///     .with_reporting_interval(Duration::from_secs(15))
    ///     .build()
    ///     .spawn()?;
    /// # Ok::<_, SpawnError>(())
    /// ```
    pub fn with_reporting_interval(mut self, i: Duration) -> ProfilerBuilder {
        self.reporting_interval = Some(i);
        self
    }

    /// Sets the [`Reporter`], which is used to upload the collected profiling
    /// data. Common reporters are [`LocalReporter`], and, with the `s3-no-defaults`
    /// feature enabled,
    #[cfg_attr(not(feature = "s3-no-defaults"), doc = "`S3Reporter`.")]
    #[cfg_attr(feature = "s3-no-defaults", doc = "[`S3Reporter`].")]
    /// It is also possible to write your own [`Reporter`].
    ///
    /// It's normally easier to use [`LocalReporter`] directly via
    /// [`ProfilerBuilder::with_local_reporter`].
    ///
    /// If you want to output to multiple reporters, you can use
    /// [`MultiReporter`].
    ///
    /// [`LocalReporter`]: crate::reporter::local::LocalReporter
    /// [`MultiReporter`]: crate::reporter::multi::MultiReporter
    #[cfg_attr(
        feature = "s3-no-defaults",
        doc = "[`S3Reporter`]: crate::reporter::s3::S3Reporter"
    )]
    ///
    #[cfg_attr(feature = "s3-no-defaults", doc = include_str!("s3-example.md"))]
    pub fn with_reporter(mut self, r: impl Reporter + Send + Sync + 'static) -> ProfilerBuilder {
        self.reporter = Some(Box::new(r));
        self
    }

    /// Sets the profiler to ues [LocalReporter], which will write `.jfr` files to `path`,
    /// and disables metadata auto-detection (see [`ProfilerBuilder::with_custom_agent_metadata`])
    /// since the [LocalReporter] does not need that.
    ///
    /// This is useful for testing, since metadata auto-detection currently only works
    /// on [Amazon EC2] or [Amazon Fargate] instances.
    ///
    /// The local reporter should normally not be used in production, since it will
    /// not clean up JFR files. Instead, you can use a pre-existing [`Reporter`]
    /// or write your own (see [`ProfilerBuilder::with_reporter`]).
    ///
    /// [Amazon EC2]: https://aws.amazon.com/ec2
    /// [Amazon Fargate]: https://aws.amazon.com/fargate
    ///
    /// ## Example
    ///
    /// This will write profiles as `.jfr` files to `./path-to-profiles`:
    ///
    /// ```no_run
    /// # use std::path::PathBuf;
    /// # use async_profiler_agent::profiler::{ProfilerBuilder, SpawnError};
    /// # use async_profiler_agent::reporter::local::LocalReporter;
    /// # use async_profiler_agent::metadata::AgentMetadata;
    /// let path = PathBuf::from("./path-to-profiles");
    /// let agent = ProfilerBuilder::default()
    ///     .with_local_reporter(path)
    ///     .build()
    ///     .spawn()?;
    /// # Ok::<_, SpawnError>(())
    /// ```
    pub fn with_local_reporter(mut self, path: impl Into<PathBuf>) -> ProfilerBuilder {
        self.reporter = Some(Box::new(LocalReporter::new(path.into())));
        self.with_custom_agent_metadata(AgentMetadata::NoMetadata)
    }

    /// Provide custom agent metadata.
    ///
    /// The async-profiler Rust agent sends metadata to the [Reporter] with
    /// the identity of the current host and process, which is normally
    /// transmitted as `metadata.json` within the generated `.zip` file,
    /// using the schema format [`reporter::s3::MetadataJson`].
    ///
    /// That metadata can later be used by tooling to be able to sort
    /// profiling reports by host.
    ///
    /// async-profiler Rust agent will by default try to fetch the metadata
    /// using [IMDS] when running on [Amazon EC2] or [Amazon Fargate], and
    /// will error if it's unable to find it. If you are running the
    /// async-profiler agent on any other form of compute,
    /// you will need to create and attach your own metadata
    /// by calling this function.
    ///
    #[cfg_attr(feature = "s3-no-defaults", doc = include_str!("s3-example-custom-metadata.md"))]
    /// [`reporter::s3::MetadataJson`]: crate::reporter::s3::MetadataJson
    /// [Amazon EC2]: https://aws.amazon.com/ec2
    /// [Amazon Fargate]: https://aws.amazon.com/fargate
    /// [IMDS]: https://docs.aws.amazon.com/AWSEC2/latest/UserGuide/ec2-instance-metadata.html
    pub fn with_custom_agent_metadata(mut self, j: AgentMetadata) -> ProfilerBuilder {
        self.agent_metadata = Some(j);
        self
    }

    /// Provide custom profiler options.
    ///
    /// ### Example
    ///
    /// This will sample allocations for every 10 megabytes allocated:
    ///
    /// ```
    /// # use async_profiler_agent::profiler::{ProfilerBuilder, ProfilerOptionsBuilder};
    /// # use async_profiler_agent::profiler::SpawnError;
    /// # fn main() -> Result<(), SpawnError> {
    /// let opts = ProfilerOptionsBuilder::default().with_native_mem("10m".into()).build();
    /// let profiler = ProfilerBuilder::default()
    ///     .with_profiler_options(opts)
    ///     .with_local_reporter("/tmp/profiles")
    ///     .build();
    /// # if false { // don't spawn the profiler in doctests
    /// profiler.spawn()?;
    /// # }
    /// # Ok(())
    /// # }
    /// ```
    pub fn with_profiler_options(mut self, c: ProfilerOptions) -> ProfilerBuilder {
        self.profiler_options = Some(c);
        self
    }

    /// Turn this builder into a profiler!
    pub fn build(self) -> Profiler {
        Profiler {
            reporting_interval: self.reporting_interval.unwrap_or(Duration::from_secs(30)),
            reporter: self.reporter.expect("reporter is required"),
            agent_metadata: self.agent_metadata,
            profiler_options: self.profiler_options.unwrap_or_default(),
        }
    }
}

enum Status {
    Idle,
    Starting,
    Running(SystemTime),
}

/// This type provides wrapper APIs over [`asprof::AsProf`], to allow tracking
/// of the state of the profiler. The primary benefit of this is RAII - when
/// this type drops, it will stop the profiler if it's running.
struct ProfilerState<E: ProfilerEngine> {
    // this is only None in the destructor when stopping the async-profiler fails
    jfr_file: Option<JfrFile>,
    asprof: E,
    status: Status,
    profiler_options: ProfilerOptions,
}

impl<E: ProfilerEngine> ProfilerState<E> {
    fn new(asprof: E, profiler_options: ProfilerOptions) -> Result<Self, io::Error> {
        Ok(Self {
            jfr_file: Some(JfrFile::new()?),
            asprof,
            status: Status::Idle,
            profiler_options,
        })
    }

    fn jfr_file_mut(&mut self) -> &mut JfrFile {
        self.jfr_file.as_mut().unwrap()
    }

    async fn start(&mut self) -> Result<(), AsProfError> {
        let active = self.jfr_file.as_ref().unwrap().active_path();
        // drop guard - make sure the files are leaked if the profiler might have started
        self.status = Status::Starting;
        E::start_async_profiler(&self.asprof, &active, &self.profiler_options)?;
        self.status = Status::Running(SystemTime::now());
        Ok(())
    }

    fn stop(&mut self) -> Result<Option<SystemTime>, AsProfError> {
        E::stop_async_profiler()?;
        let status = std::mem::replace(&mut self.status, Status::Idle);
        Ok(match status {
            Status::Idle | Status::Starting => None,
            Status::Running(since) => Some(since),
        })
    }

    fn is_started(&self) -> bool {
        matches!(self.status, Status::Running(_))
    }
}

impl<E: ProfilerEngine> Drop for ProfilerState<E> {
    fn drop(&mut self) {
        match self.status {
            Status::Running(_) => {
                if let Err(err) = self.stop() {
                    // SECURITY: avoid removing the JFR file if stopping the profiler fails,
                    // to avoid symlink races
                    std::mem::forget(self.jfr_file.take());
                    // XXX: Rust defines leaking resources during drop as safe.
                    tracing::warn!(?err, "unable to stop profiler during drop glue");
                }
            }
            Status::Idle => {}
            Status::Starting => {
                // SECURITY: avoid removing the JFR file if stopping the profiler fails,
                // to avoid symlink races
                std::mem::forget(self.jfr_file.take());
            }
        }
    }
}

pub(crate) trait ProfilerEngine: Send + Sync + 'static {
    fn init_async_profiler() -> Result<(), asprof::AsProfError>;
    fn start_async_profiler(
        &self,
        jfr_file_path: &Path,
        options: &ProfilerOptions,
    ) -> Result<(), asprof::AsProfError>;
    fn stop_async_profiler() -> Result<(), asprof::AsProfError>;
}

#[derive(Debug, Error)]
#[non_exhaustive]
enum TickError {
    #[error(transparent)]
    AsProf(#[from] AsProfError),
    #[error(transparent)]
    #[cfg(feature = "aws-metadata-no-defaults")]
    Metadata(#[from] crate::metadata::aws::AwsProfilerMetadataError),
    #[error("reporter: {0}")]
    Reporter(Box<dyn std::error::Error + Send>),
    #[error("broken clock: {0}")]
    BrokenClock(#[from] SystemTimeError),
    #[error("jfr read error: {0}")]
    JfrRead(io::Error),
    #[error("empty inactive file error: {0}")]
    EmptyInactiveFile(io::Error),
}

#[derive(Debug, Error)]
#[non_exhaustive]
/// An error that happened spawning a profiler
pub enum SpawnError {
    /// Error from async-profiler
    #[error(transparent)]
    AsProf(#[from] asprof::AsProfError),
    /// Error writing to a tempfile
    #[error("tempfile error")]
    TempFile(#[source] io::Error),
}

#[derive(Debug, Error)]
#[non_exhaustive]
/// An error from [`Profiler::spawn_thread`]
pub enum SpawnThreadError {
    /// Error from async-profiler
    #[error(transparent)]
    AsProf(#[from] SpawnError),
    /// Error constructing Tokio runtime
    #[error("constructing Tokio runtime")]
    ConstructRt(#[source] io::Error),
}

// no control messages currently
enum Control {}

/// A handle to a running profiler
///
/// Currently just allows for stopping the profiler.
///
/// Dropping this handle will request that the profiler will stop.
#[must_use = "dropping this stops the profiler, call .detach() to detach"]
pub struct RunningProfiler {
    stop_channel: tokio::sync::oneshot::Sender<Control>,
    join_handle: tokio::task::JoinHandle<()>,
}

impl RunningProfiler {
    /// Request that the current profiler stops and wait until it exits.
    ///
    /// This will cause the currently-pending profile information to be flushed.
    ///
    /// After this function returns, it is correct and safe to [spawn] a new
    /// [Profiler], possibly with a different configuration. Therefore,
    /// this function can be used to "reconfigure" a profiler by stopping
    /// it and then starting a new one with a different configuration.
    ///
    /// [spawn]: Profiler::spawn_controllable
    pub async fn stop(self) {
        drop(self.stop_channel);
        let _ = self.join_handle.await;
    }

    /// Like [Self::detach], but returns a JoinHandle. This is currently not a public API.
    fn detach_inner(self) -> tokio::task::JoinHandle<()> {
        tokio::task::spawn(async move {
            // move the control channel to the spawned task. this way, it will be dropped
            // just when the task is aborted.
            let _abort_channel = self.stop_channel;
            self.join_handle.await.ok();
        })
    }

    /// Detach this profiler. This will prevent the profiler from being stopped
    /// when this handle is dropped. You should call this (or [Profiler::spawn]
    /// instead of [Profiler::spawn_controllable], which does the same thing)
    /// if you don't intend to reconfigure your profiler at runtime.
    pub fn detach(self) {
        self.detach_inner();
    }

    /// Spawns this [RunningProfiler] into a separate thread within a new Tokio runtime,
    /// and returns a [RunningProfilerThread] attached to it.
    fn spawn_attached(
        self,
        runtime: tokio::runtime::Runtime,
        spawn_fn: impl FnOnce(Box<dyn FnOnce() + Send>) -> std::thread::JoinHandle<()>,
    ) -> RunningProfilerThread {
        RunningProfilerThread {
            stop_channel: self.stop_channel,
            join_handle: spawn_fn(Box::new(move || {
                let _ = runtime.block_on(self.join_handle);
            })),
        }
    }

    /// Spawns this [RunningProfiler] into a separate thread within a new Tokio runtime,
    /// and detaches it.
    fn spawn_detached(
        self,
        runtime: tokio::runtime::Runtime,
        spawn_fn: impl FnOnce(Box<dyn FnOnce() + Send>) -> std::thread::JoinHandle<()>,
    ) {
        spawn_fn(Box::new(move || {
            let _stop_channel = self.stop_channel;
            let _ = runtime.block_on(self.join_handle);
        }));
    }
}

/// A handle to a running profiler, running on a separate thread.
///
/// Currently just allows for stopping the profiler.
///
/// Dropping this handle will request that the profiler will stop.
#[must_use = "dropping this stops the profiler, call .detach() to detach"]
pub struct RunningProfilerThread {
    stop_channel: tokio::sync::oneshot::Sender<Control>,
    join_handle: std::thread::JoinHandle<()>,
}

impl RunningProfilerThread {
    /// Request that the current profiler stops and wait until it exits.
    ///
    /// This will cause the currently-pending profile information to be flushed.
    ///
    /// After this function returns, it is correct and safe to [spawn] a new
    /// [Profiler], possibly with a different configuration. Therefore,
    /// this function can be used to "reconfigure" a profiler by stopping
    /// it and then starting a new one with a different configuration.
    ///
    /// [spawn]: Profiler::spawn_controllable
    pub fn stop(self) {
        drop(self.stop_channel);
        let _ = self.join_handle.join();
    }
}

/// Rust profiler based on [async-profiler].
///
/// Spawning a profiler can be done either in an attached (controllable)
/// mode, which allows for stopping the profiler (and, in fact, stops
/// it when the relevant handle is dropped), or in detached mode,
/// in which the profiler keeps running forever. Applications that can
/// shut down the profiler at run-time, for example applications that
/// support reconfiguration of a running profiler, generally want to use
/// controllable mode. Other applications (most of them) should use
/// detached mode.
///
/// In addition, the profiler can either be spawned into the current Tokio
/// runtime, or into a new one. Normally, applications should spawn
/// the profiler into their own Tokio runtime, but applications that
/// don't have a default Tokio runtime should spawn it into a
/// different one
///
/// This leaves 4 functions:
/// 1. [Self::spawn] - detached, same runtime
/// 2. [Self::spawn_thread_to_runtime] - detached, different runtime
/// 3. [Self::spawn_controllable] - controllable, same runtime
/// 4. [Self::spawn_controllable_thread_to_runtime] - controllable, different runtime
///
/// In addition, there's a helper function that just spawns the profiler
/// to a new runtime in a new thread, for applications that don't have
/// a Tokio runtime and don't need complex control:
///
/// 5. [Self::spawn_thread] - detached, new runtime in a new thread
///
/// [async-profiler]: https://github.com/async-profiler/async-profiler
pub struct Profiler {
    reporting_interval: Duration,
    reporter: Box<dyn Reporter + Send + Sync>,
    agent_metadata: Option<AgentMetadata>,
    profiler_options: ProfilerOptions,
}

impl Profiler {
    /// Start profiling. The profiler will run in a tokio task at the configured interval.
    ///
    /// This is the same as calling [Profiler::spawn_controllable] followed by
    /// [RunningProfiler::detach], except it returns a [JoinHandle].
    ///
    /// The returned [JoinHandle] can be used to detect if the profiler has exited
    /// due to a fatal error.
    ///
    /// This function will fail if it is unable to start async-profiler, for example
    /// if it can't find or load `libasyncProfiler.so`.
    ///
    /// [JoinHandle]: tokio::task::JoinHandle
    ///
    /// ### Tokio Runtime
    ///
    /// This function must be run within a Tokio runtime, otherwise it will panic. If
    /// your application does not have a `main` Tokio runtime, see
    /// [Profiler::spawn_thread].
    ///
    /// ### Example
    ///
    /// This example uses [ProfilerBuilder::with_local_reporter] which reports the profiles to
    /// a directory. It works with any other [Reporter] using [ProfilerBuilder::with_reporter].
    ///
    /// ```
    /// # use async_profiler_agent::profiler::{ProfilerBuilder, SpawnError};
    /// # #[tokio::main]
    /// # async fn main() -> Result<(), SpawnError> {
    /// let profiler = ProfilerBuilder::default()
    ///    .with_local_reporter("/tmp/profiles")
    ///    .build();
    /// # if false { // don't spawn the profiler in doctests
    /// profiler.spawn()?;
    /// # }
    /// # Ok(())
    /// # }
    /// ```
    pub fn spawn(self) -> Result<tokio::task::JoinHandle<()>, SpawnError> {
        self.spawn_controllable().map(RunningProfiler::detach_inner)
    }

    /// Like [Self::spawn], but instead of spawning within the current Tokio
    /// runtime, spawns within a set Tokio runtime and then runs a thread that calls
    /// [block_on](tokio::runtime::Runtime::block_on) on that runtime.
    ///
    /// If your configuration is standard, use [Profiler::spawn_thread].
    ///
    /// If you want to be able to stop the resulting profiler, use
    /// [Profiler::spawn_controllable_thread_to_runtime].
    ///
    /// `spawn_fn` should be [`std::thread::spawn`], or some function that behaves like it (to
    /// allow for configuring thread properties, for example thread names).
    ///
    /// This is to be used when your program does not have a "main" Tokio runtime already set up.
    ///
    /// ### Example
    ///
    /// This example uses [ProfilerBuilder::with_local_reporter] which reports the profiles to
    /// a directory. It works with any other [Reporter] using [ProfilerBuilder::with_reporter].
    ///
    /// ```no_run
    /// # use async_profiler_agent::profiler::{ProfilerBuilder, SpawnError};
    /// let rt = tokio::runtime::Builder::new_current_thread()
    ///     .enable_all()
    ///     .build()?;
    /// let profiler = ProfilerBuilder::default()
    ///    .with_local_reporter("/tmp/profiles")
    ///    .build();
    ///
    /// profiler.spawn_thread_to_runtime(
    ///     rt,
    ///     |t| {
    ///         std::thread::Builder::new()
    ///             .name("asprof-agent".to_owned())
    ///             .spawn(t)
    ///             .expect("thread name contains nuls")
    ///     }
    /// )?;
    /// # Ok::<_, anyhow::Error>(())
    /// ```
    pub fn spawn_thread_to_runtime(
        self,
        runtime: tokio::runtime::Runtime,
        spawn_fn: impl FnOnce(Box<dyn FnOnce() + Send>) -> std::thread::JoinHandle<()>,
    ) -> Result<(), SpawnError> {
        self.spawn_thread_inner(asprof::AsProf::builder().build(), runtime, spawn_fn)
    }

    /// Like [Self::spawn], but instead of spawning within the current Tokio
    /// runtime, spawns within a new Tokio runtime and then runs a thread that calls
    /// [block_on](tokio::runtime::Runtime::block_on) on that runtime, setting up the runtime
    /// by itself.
    ///
    /// If your configuration is less standard, use [Profiler::spawn_thread_to_runtime]. Calling
    /// [Profiler::spawn_thread] is equivalent to calling [Profiler::spawn_thread_to_runtime]
    /// with the following:
    /// 1. a current thread runtime with background worker threads (these exist
    ///    for blocking IO) named "asprof-worker"
    /// 2. a controller thread (the "main" thread of the runtime) named "asprof-agent"
    ///
    /// If you want to be able to stop the resulting profiler, use
    /// [Profiler::spawn_controllable_thread_to_runtime].
    ///
    /// This is to be used when your program does not have a "main" Tokio runtime already set up.
    ///
    /// ### Example
    ///
    /// This example uses [ProfilerBuilder::with_local_reporter] which reports the profiles to
    /// a directory. It works with any other [Reporter] using [ProfilerBuilder::with_reporter].
    ///
    /// ```no_run
    /// # use async_profiler_agent::profiler::{ProfilerBuilder, SpawnError};
    /// # use async_profiler_agent::reporter::local::LocalReporter;
    /// let profiler = ProfilerBuilder::default()
    ///    .with_local_reporter("/tmp/profiles")
    ///    .build();
    ///
    /// profiler.spawn_thread()?;
    /// # Ok::<_, anyhow::Error>(())
    /// ```
    pub fn spawn_thread(self) -> Result<(), SpawnThreadError> {
        // using "asprof" in thread name to deal with 15 character + \0 length limit
        let rt = tokio::runtime::Builder::new_current_thread()
            .thread_name("asprof-worker".to_owned())
            .enable_all()
            .build()
            .map_err(SpawnThreadError::ConstructRt)?;
        let builder = std::thread::Builder::new().name("asprof-agent".to_owned());
        self.spawn_thread_to_runtime(rt, |t| builder.spawn(t).expect("thread name contains nuls"))
            .map_err(SpawnThreadError::AsProf)
    }

    fn spawn_thread_inner<E: ProfilerEngine>(
        self,
        asprof: E,
        runtime: tokio::runtime::Runtime,
        spawn_fn: impl FnOnce(Box<dyn FnOnce() + Send>) -> std::thread::JoinHandle<()>,
    ) -> Result<(), SpawnError> {
        let handle: RunningProfiler = runtime.block_on(async move { self.spawn_inner(asprof) })?;
        handle.spawn_detached(runtime, spawn_fn);
        Ok(())
    }

    /// Like [Self::spawn], but returns a [RunningProfiler] that allows for controlling
    /// (currently only stopping) the profiler.
    ///
    /// This allows for changing the configuration of the profiler at runtime, by
    /// stopping it and then starting a new Profiler with a new configuration. It
    /// also allows for stopping profiling in case the profiler is suspected to
    /// cause operational issues.
    ///
    /// Dropping the returned [RunningProfiler] will cause the profiler to quit,
    /// so if your application doen't need to change the profiler's configuration at runtime,
    /// it will be easier to use [Profiler::spawn].
    ///
    /// This function will fail if it is unable to start async-profiler, for example
    /// if it can't find or load `libasyncProfiler.so`.
    ///
    /// ### Tokio Runtime
    ///
    /// This function must be run within a Tokio runtime, otherwise it will panic. If
    /// your application does not have a `main` Tokio runtime, see
    /// [Profiler::spawn_controllable_thread_to_runtime].
    ///
    /// ### Example
    ///
    /// This example uses [ProfilerBuilder::with_local_reporter] which reports the profiles to
    /// a directory. It works with any other [Reporter] using [ProfilerBuilder::with_reporter].
    ///
    /// ```no_run
    /// # use async_profiler_agent::profiler::{ProfilerBuilder, SpawnError};
    /// # #[tokio::main]
    /// # async fn main() -> Result<(), SpawnError> {
    /// let profiler = ProfilerBuilder::default()
    ///    .with_local_reporter("/tmp/profiles")
    ///    .build();
    ///
    /// let profiler = profiler.spawn_controllable()?;
    ///
    /// // [insert your signaling/monitoring mechanism to have a request to disable
    /// // profiling in case of a problem]
    /// let got_request_to_disable_profiling = async move {
    ///     // ...
    /// #   false
    /// };
    /// // spawn a task that will disable profiling if requested
    /// tokio::task::spawn(async move {
    ///     if got_request_to_disable_profiling.await {
    ///         profiler.stop().await;
    ///     }
    /// });
    /// # Ok(())
    /// # }
    /// ```
    pub fn spawn_controllable(self) -> Result<RunningProfiler, SpawnError> {
        self.spawn_inner(asprof::AsProf::builder().build())
    }

    /// Like [Self::spawn_controllable], but instead of spawning within the current Tokio
    /// runtime, spawns within a set Tokio runtime and then runs a thread that calls
    /// [block_on](tokio::runtime::Runtime::block_on) on that runtime.
    ///
    /// `spawn_fn` should be [`std::thread::spawn`], or some function that behaves like it (to
    /// allow for configuring thread properties, for example thread names).
    ///
    /// This is to be used when your program does not have a "main" Tokio runtime already set up.
    ///
    /// ### Example
    ///
    /// This example uses [ProfilerBuilder::with_local_reporter] which reports the profiles to
    /// a directory. It works with any other [Reporter] using [ProfilerBuilder::with_reporter].
    ///
    /// ```no_run
    /// # use async_profiler_agent::profiler::{ProfilerBuilder, SpawnError};
    /// let rt = tokio::runtime::Builder::new_current_thread()
    ///     .enable_all()
    ///     .build()?;
    /// let profiler = ProfilerBuilder::default()
    ///    .with_local_reporter("/tmp/profiles")
    ///    .build();
    ///
    /// let profiler = profiler.spawn_controllable_thread_to_runtime(
    ///     rt,
    ///     |t| {
    ///         std::thread::Builder::new()
    ///             .name("asprof-agent".to_owned())
    ///             .spawn(t)
    ///             .expect("thread name contains nuls")
    ///     }
    /// )?;
    ///
    /// # fn got_request_to_disable_profiling() -> bool { false }
    /// // spawn a task that will disable profiling if requested
    /// std::thread::spawn(move || {
    ///     if got_request_to_disable_profiling() {
    ///         profiler.stop();
    ///     }
    /// });
    /// # Ok::<_, anyhow::Error>(())
    /// ```
    pub fn spawn_controllable_thread_to_runtime(
        self,
        runtime: tokio::runtime::Runtime,
        spawn_fn: impl FnOnce(Box<dyn FnOnce() + Send>) -> std::thread::JoinHandle<()>,
    ) -> Result<RunningProfilerThread, SpawnError> {
        self.spawn_controllable_thread_inner(asprof::AsProf::builder().build(), runtime, spawn_fn)
    }

    fn spawn_controllable_thread_inner<E: ProfilerEngine>(
        self,
        asprof: E,
        runtime: tokio::runtime::Runtime,
        spawn_fn: impl FnOnce(Box<dyn FnOnce() + Send>) -> std::thread::JoinHandle<()>,
    ) -> Result<RunningProfilerThread, SpawnError> {
        let handle = runtime.block_on(async move { self.spawn_inner(asprof) })?;
        Ok(handle.spawn_attached(runtime, spawn_fn))
    }

    fn spawn_inner<E: ProfilerEngine>(self, asprof: E) -> Result<RunningProfiler, SpawnError> {
        // Initialize async profiler - needs to be done once.
        E::init_async_profiler()?;
        tracing::info!("successfully initialized async profiler.");

        let mut sampling_ticker = tokio::time::interval(self.reporting_interval);
        let (stop_channel, mut stop_rx) = tokio::sync::oneshot::channel();

        // Get profiles at the configured interval rate.
        let join_handle = tokio::spawn(async move {
            let mut state = match ProfilerState::new(asprof, self.profiler_options) {
                Ok(state) => state,
                Err(err) => {
                    tracing::error!(?err, "unable to create profiler state");
                    return;
                }
            };

            // Lazily-loaded if not specified up front.
            let mut agent_metadata = self.agent_metadata;
            let mut done = false;

            while !done {
                // Wait until a timer or exit event
                tokio::select! {
                    biased;

                    r = &mut stop_rx, if !stop_rx.is_terminated() => {
                        match r {
                            Err(_) => {
                                tracing::info!("profiler stop requested, doing a final tick");
                                done = true;
                            }
                        }
                    }
                    _ = sampling_ticker.tick() => {
                        tracing::debug!("profiler timer woke up");
                    }
                }

                if let Err(err) = profiler_tick(
                    &mut state,
                    &mut agent_metadata,
                    &*self.reporter,
                    self.reporting_interval,
                )
                .await
                {
                    match &err {
                        TickError::Reporter(_) => {
                            // don't stop on IO errors
                            tracing::error!(?err, "error during profiling, continuing");
                        }
                        _stop => {
                            tracing::error!(?err, "error during profiling, stopping");
                            break;
                        }
                    }
                }
            }

            tracing::info!("profiling task finished");
        });

        Ok(RunningProfiler {
            stop_channel,
            join_handle,
        })
    }
}

async fn profiler_tick<E: ProfilerEngine>(
    state: &mut ProfilerState<E>,
    agent_metadata: &mut Option<AgentMetadata>,
    reporter: &(dyn Reporter + Send + Sync),
    reporting_interval: Duration,
) -> Result<(), TickError> {
    if !state.is_started() {
        state.start().await?;
        return Ok(());
    }

    let Some(start) = state.stop()? else {
        tracing::warn!("stopped the profiler but it wasn't running?");
        return Ok(());
    };
    let start = start.duration_since(UNIX_EPOCH)?;
    let end = SystemTime::now().duration_since(UNIX_EPOCH)?;

    // Start it up immediately, writing to the "other" file, so that we keep
    // profiling the application while we're reporting data.
    state
        .jfr_file_mut()
        .empty_inactive_file()
        .map_err(TickError::EmptyInactiveFile)?;
    state.jfr_file_mut().swap();
    state.start().await?;

    // Lazily load the agent metadata if it was not provided in
    // the constructor. See the struct comments for why this is.
    // This code runs at most once.
    if agent_metadata.is_none() {
        #[cfg(feature = "aws-metadata-no-defaults")]
        let md = crate::metadata::aws::load_agent_metadata().await?;
        #[cfg(not(feature = "aws-metadata-no-defaults"))]
        let md = crate::metadata::AgentMetadata::NoMetadata;
        tracing::debug!("loaded metadata");
        agent_metadata.replace(md);
    }

    let report_metadata = ReportMetadata {
        instance: agent_metadata.as_ref().unwrap(),
        start,
        end,
        reporting_interval,
    };

    let jfr = tokio::fs::read(state.jfr_file_mut().inactive_path())
        .await
        .map_err(TickError::JfrRead)?;

    reporter
        .report(jfr, &report_metadata)
        .await
        .map_err(TickError::Reporter)?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{self, AtomicBool, AtomicU32};
    use std::sync::Arc;

    use test_case::test_case;

    use super::*;

    #[test]
    fn test_jfr_file_drop() {
        let mut jfr = JfrFile::new().unwrap();

        std::fs::write(jfr.active_path(), b"Hello, 2!").unwrap();
        jfr.swap();
        assert_eq!(std::fs::read(jfr.inactive_path()).unwrap(), b"Hello, 2!");
        jfr.empty_inactive_file().unwrap();
        assert_eq!(std::fs::read(jfr.inactive_path()).unwrap(), b"");
    }

    struct MockProfilerEngine {
        counter: AtomicU32,
    }
    impl ProfilerEngine for MockProfilerEngine {
        fn init_async_profiler() -> Result<(), asprof::AsProfError> {
            Ok(())
        }

        fn start_async_profiler(
            &self,
            jfr_file_path: &Path,
            _options: &ProfilerOptions,
        ) -> Result<(), asprof::AsProfError> {
            let contents = format!(
                "JFR{}",
                self.counter.fetch_add(1, atomic::Ordering::Relaxed)
            );
            std::fs::write(jfr_file_path, contents.as_bytes()).unwrap();
            Ok(())
        }

        fn stop_async_profiler() -> Result<(), asprof::AsProfError> {
            Ok(())
        }
    }

    struct MockReporter(tokio::sync::mpsc::Sender<(String, AgentMetadata)>);
    impl std::fmt::Debug for MockReporter {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.debug_struct("MockReporter").finish()
        }
    }

    #[async_trait::async_trait]
    impl Reporter for MockReporter {
        async fn report(
            &self,
            jfr: Vec<u8>,
            metadata: &ReportMetadata,
        ) -> Result<(), Box<dyn std::error::Error + Send>> {
            self.0
                .send((String::from_utf8(jfr).unwrap(), metadata.instance.clone()))
                .await
                .unwrap();
            Ok(())
        }
    }

    fn make_mock_profiler() -> (
        Profiler,
        tokio::sync::mpsc::Receiver<(String, AgentMetadata)>,
    ) {
        let (tx, rx) = tokio::sync::mpsc::channel(1);
        let agent = ProfilerBuilder::default()
            .with_reporter(MockReporter(tx))
            .with_custom_agent_metadata(AgentMetadata::Ec2AgentMetadata {
                aws_account_id: "0".into(),
                aws_region_id: "us-east-1".into(),
                ec2_instance_id: "i-fake".into(),
            })
            .build();
        (agent, rx)
    }

    #[tokio::test(start_paused = true)]
    async fn test_profiler_agent() {
        let e_md = AgentMetadata::Ec2AgentMetadata {
            aws_account_id: "0".into(),
            aws_region_id: "us-east-1".into(),
            ec2_instance_id: "i-fake".into(),
        };
        let (agent, mut rx) = make_mock_profiler();
        agent
            .spawn_inner::<MockProfilerEngine>(MockProfilerEngine {
                counter: AtomicU32::new(0),
            })
            .unwrap()
            .detach();
        let (jfr, md) = rx.recv().await.unwrap();
        assert_eq!(jfr, "JFR0");
        assert_eq!(e_md, md);
        let (jfr, md) = rx.recv().await.unwrap();
        assert_eq!(jfr, "JFR1");
        assert_eq!(e_md, md);
    }

    #[test_case(false; "uncontrollable")]
    #[test_case(true; "controllable")]
    fn test_profiler_local_rt(controllable: bool) {
        let e_md = AgentMetadata::Ec2AgentMetadata {
            aws_account_id: "0".into(),
            aws_region_id: "us-east-1".into(),
            ec2_instance_id: "i-fake".into(),
        };
        let (agent, mut rx) = make_mock_profiler();
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .start_paused(true)
            .build()
            .unwrap();
        // spawn the profiler, doing this before spawning a thread to allow
        // capturing errors from `spawn`
        let handle = if controllable {
            Some(
                agent
                    .spawn_controllable_thread_inner::<MockProfilerEngine>(
                        MockProfilerEngine {
                            counter: AtomicU32::new(0),
                        },
                        rt,
                        std::thread::spawn,
                    )
                    .unwrap(),
            )
        } else {
            agent
                .spawn_thread_inner::<MockProfilerEngine>(
                    MockProfilerEngine {
                        counter: AtomicU32::new(0),
                    },
                    rt,
                    std::thread::spawn,
                )
                .unwrap();
            None
        };

        let (jfr, md) = rx.blocking_recv().unwrap();
        assert_eq!(jfr, "JFR0");
        assert_eq!(e_md, md);
        let (jfr, md) = rx.blocking_recv().unwrap();
        assert_eq!(jfr, "JFR1");
        assert_eq!(e_md, md);

        if let Some(handle) = handle {
            let drain_thread =
                std::thread::spawn(move || while let Some(_) = rx.blocking_recv() {});
            // request a stop
            handle.stop();
            // the drain thread should be done
            drain_thread.join().unwrap();
        }
    }

    enum StopKind {
        Delibrate,
        Drop,
        Abort,
    }

    #[tokio::test(start_paused = true)]
    #[test_case(StopKind::Delibrate; "deliberate stop")]
    #[test_case(StopKind::Drop; "drop stop")]
    #[test_case(StopKind::Abort; "abort stop")]
    async fn test_profiler_stop(stop_kind: StopKind) {
        let e_md = AgentMetadata::Ec2AgentMetadata {
            aws_account_id: "0".into(),
            aws_region_id: "us-east-1".into(),
            ec2_instance_id: "i-fake".into(),
        };
        let (agent, mut rx) = make_mock_profiler();
        let profiler_ref = agent
            .spawn_inner::<MockProfilerEngine>(MockProfilerEngine {
                counter: AtomicU32::new(0),
            })
            .unwrap();
        let (jfr, md) = rx.recv().await.unwrap();
        assert_eq!(jfr, "JFR0");
        assert_eq!(e_md, md);
        let (jfr, md) = rx.recv().await.unwrap();
        assert_eq!(jfr, "JFR1");
        assert_eq!(e_md, md);
        // check that stop is faster than an interval and returns an "immediate" next jfr
        match stop_kind {
            StopKind::Drop => drop(profiler_ref),
            StopKind::Delibrate => {
                tokio::time::timeout(Duration::from_millis(1), profiler_ref.stop())
                    .await
                    .unwrap();
            }
            StopKind::Abort => {
                // You can call Abort on the JoinHandle. make sure that is not buggy.
                profiler_ref.detach_inner().abort();
            }
        }
        // check that we get the next JFR "quickly", and the JFR after that is empty.
        let (jfr, md) = tokio::time::timeout(Duration::from_millis(1), rx.recv())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(jfr, "JFR2");
        assert_eq!(e_md, md);
        assert!(rx.recv().await.is_none());
    }

    // simulate a badly-behaved profiler that errors on start/stop and then
    // tries to access the JFR file
    struct StopErrorProfilerEngine {
        start_error: bool,
        counter: Arc<AtomicBool>,
    }
    impl ProfilerEngine for StopErrorProfilerEngine {
        fn init_async_profiler() -> Result<(), asprof::AsProfError> {
            Ok(())
        }

        fn start_async_profiler(
            &self,
            jfr_file_path: &Path,
            _options: &ProfilerOptions,
        ) -> Result<(), asprof::AsProfError> {
            let jfr_file_path = jfr_file_path.to_owned();
            std::fs::write(&jfr_file_path, "JFR").unwrap();
            let counter = self.counter.clone();
            tokio::task::spawn(async move {
                tokio::time::sleep(Duration::from_secs(5)).await;
                assert_eq!(std::fs::read_to_string(jfr_file_path).unwrap(), "JFR");
                counter.store(true, atomic::Ordering::Release);
            });
            if self.start_error {
                Err(asprof::AsProfError::AsyncProfilerError("error".into()))
            } else {
                Ok(())
            }
        }

        fn stop_async_profiler() -> Result<(), asprof::AsProfError> {
            Err(asprof::AsProfError::AsyncProfilerError("error".into()))
        }
    }

    #[tokio::test(start_paused = true)]
    #[test_case(false; "error on stop")]
    #[test_case(true; "error on start")]
    async fn test_profiler_error(start_error: bool) {
        let (agent, mut rx) = make_mock_profiler();
        let counter = Arc::new(AtomicBool::new(false));
        let engine = StopErrorProfilerEngine {
            start_error,
            counter: counter.clone(),
        };
        let handle = agent.spawn_inner(engine).unwrap().detach_inner();
        assert!(rx.recv().await.is_none());
        // check that the "sleep 5" step in start_async_profiler succeeds
        for _ in 0..100 {
            tokio::time::sleep(Duration::from_secs(1)).await;
            if counter.load(atomic::Ordering::Acquire) {
                handle.await.unwrap(); // Check that the JoinHandle is done
                return;
            }
        }
        panic!("didn't read from file");
    }

    #[test]
    fn test_profiler_options_to_args_string_default() {
        let opts = ProfilerOptions::default();
        let dummy_path = Path::new("/tmp/test.jfr");
        let args = opts.to_args_string(dummy_path);
        assert!(
            args.contains("start,event=cpu,interval=100000000,wall=1000ms,jfr,cstack=dwarf"),
            "Default args string not constructed correctly"
        );
        assert!(args.contains("file=/tmp/test.jfr"));
        assert!(!args.contains("nativemem="));
    }

    #[test]
    fn test_profiler_options_to_args_string_with_native_mem() {
        let opts = ProfilerOptions {
            native_mem: Some("10m".to_string()),
            wall_clock_millis: None,
            cpu_interval: None,
        };
        let dummy_path = Path::new("/tmp/test.jfr");
        let args = opts.to_args_string(dummy_path);
        assert!(args.contains("nativemem=10m"));
    }

    #[test]
    fn test_profiler_options_builder() {
        let opts = ProfilerOptionsBuilder::default()
            .with_native_mem_bytes(5000000)
            .build();

        assert_eq!(opts.native_mem, Some("5000000".to_string()));
    }

    #[test]
    fn test_profiler_options_builder_all_options() {
        let opts = ProfilerOptionsBuilder::default()
            .with_native_mem_bytes(5000000)
            .with_cpu_interval(Duration::from_secs(1))
            .with_wall_clock_interval(Duration::from_secs(10))
            .build();

        let dummy_path = Path::new("/tmp/test.jfr");
        let args = opts.to_args_string(dummy_path);
        assert_eq!(args, "start,event=cpu,interval=1000000000,wall=10000ms,jfr,cstack=dwarf,file=/tmp/test.jfr,nativemem=5000000");
    }

    #[test]
    fn test_local_reporter_has_no_metadata() {
        // Check that with_local_reporter sets some configuration
        let reporter = ProfilerBuilder::default().with_local_reporter(".");
        assert_eq!(
            format!("{:?}", reporter.reporter),
            r#"Some(LocalReporter { directory: "." })"#
        );
        match reporter.agent_metadata {
            Some(AgentMetadata::NoMetadata) => {}
            bad => panic!("{bad:?}"),
        };
    }
}
