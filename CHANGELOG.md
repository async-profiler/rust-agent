# Changelog

All notable changes to this project will be documented in this file.

## [unreleased]

## [0.1.8](https://github.com/async-profiler/rust-agent/compare/v0.1.7...v0.1.8) - 2025-07-14

### Added

- *(decoder)* improve handling of decoding non-pollcatch samples

### Other

- ci: set up trusted publishing
- fix clippy in new compilers
- update zip to 4.0

## [0.1.7](https://github.com/async-profiler/rust-agent/compare/v0.1.6...v0.1.7) - 2025-05-25

### Fixed

- load agent metadata even for aws-metadata-no-defaults

### Other

- changed the name format of the .zip files uploaded to S3 when using the Fargate integration
  to have less duplication

## [0.1.6](https://github.com/async-profiler/rust-agent/compare/v0.1.5...v0.1.6) - 2025-05-20

### Added

- support not enabling default features for AWS and Reqwest (`aws-metadata-no-defaults`, `s3-no-defaults`)

## [0.1.5](https://github.com/async-profiler/rust-agent/compare/v0.1.4...v0.1.5) - 2025-05-20

### Added

- add support for stopping the profiler
- make native memory profiling interval configurable

### Other

- ci: fix caching of decoder

## [0.1.4](https://github.com/arielb1/rust-agent/compare/v0.1.3...v0.1.4) - 2025-04-29

### Other

- fix links in docs

## [0.1.3](https://github.com/async-profiler/rust-agent/compare/v0.1.2...v0.1.3) - 2025-04-29

### Other

- add API documentation

## [0.1.2](https://github.com/async-profiler/rust-agent/compare/v0.1.1...v0.1.2) - 2025-04-24

### Added

- implement LocalReporter
- implement MultiReporter
- add JFR decoder implementation (not published in crates.io)

### Other

- add test for decoder against JFR file
- add integration tests

## [0.1.1](https://github.com/async-profiler/rust-agent/compare/v0.1.0...v0.1.1) - 2025-04-16

### 🚀 Features

- Add initial support for pollcatch

### ⚙️ Miscellaneous Tasks

- Fix release-plz.toml

## [0.1.0] - 2025-04-16

### ⚙️ Miscellaneous Tasks

- Add git-cliff
- Add release-plz
- Cargo audit on CI

<!-- generated by git-cliff -->
