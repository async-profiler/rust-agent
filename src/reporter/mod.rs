// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

//! This module contains [Reporter]s that upload profiling data to a destination.
//!
//! The following [Reporter]s are included:
//! 1. [local::LocalReporter], which uploads profiling data to a local directory
//! 2. [s3::S3Reporter], which uploads profiling data to an S3 bucket
//! 3. [multi::MultiReporter], which allows combining multiple reporters.

use std::fmt;

use async_trait::async_trait;

use crate::metadata::ReportMetadata;

pub mod local;
pub mod multi;
#[cfg(feature = "s3-no-defaults")]
pub mod s3;

/// Abstraction around reporting profiler data.
#[async_trait]
pub trait Reporter: fmt::Debug {
    /// Takes a profiling sample, including JFR data and sample metadata,
    /// and uploads it towards a destination.
    ///
    /// If this function returns an error, the sample will be dropped
    /// but profiling will continue, and this function will be called
    /// again for the next sample (or theoretically, a future version
    /// might have configuration that will an attempt to re-upload the
    /// current sample will be made - but today's [`Profiler`] does
    /// not make any such attempts).
    ///
    /// [`Profiler`]: crate::profiler::Profiler
    async fn report(
        &self,
        jfr: Vec<u8>,
        metadata: &ReportMetadata,
    ) -> Result<(), Box<dyn std::error::Error + Send>>;
}
