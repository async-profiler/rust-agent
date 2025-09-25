## Example

```no_run
# use std::path::PathBuf;
# use async_profiler_agent::profiler::{ProfilerBuilder, SpawnError};
# use async_profiler_agent::reporter::s3::{S3Reporter, S3ReporterConfig};
# use async_profiler_agent::metadata::AgentMetadata;
# use aws_config::BehaviorVersion;
# #[tokio::main]
# async fn main() -> Result<(), SpawnError> {
# let path = PathBuf::from(".");
let bucket_owner = "<your account id>";
let bucket_name = "<your bucket name>";
let profiling_group = "a-name-to-give-the-uploaded-data";
let sdk_config = aws_config::defaults(BehaviorVersion::latest()).load().await;
let agent = ProfilerBuilder::default()
    .with_reporter(S3Reporter::new(S3ReporterConfig {
        sdk_config: &sdk_config,
        bucket_owner: bucket_owner.into(),
        bucket_name: bucket_name.into(),
        profiling_group_name: profiling_group.into(),
    }))
    .build()
    .spawn()?;
# Ok::<_, SpawnError>(())
# }
```
