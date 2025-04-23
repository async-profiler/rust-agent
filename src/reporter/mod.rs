// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

use std::fmt;

use async_trait::async_trait;

use crate::metadata::ReportMetadata;

pub mod multi;
#[cfg(feature = "s3")]
pub mod s3;

/// Abstraction around reporting profiler data.
#[async_trait]
pub trait Reporter: fmt::Debug {
    async fn report(
        &self,
        jfr: Vec<u8>,
        metadata: &ReportMetadata,
    ) -> Result<(), Box<dyn std::error::Error + Send>>;
}
