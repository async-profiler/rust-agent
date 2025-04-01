// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

use std::{
    ffi::{c_char, CStr, CString},
    sync::Arc,
};

use thiserror::Error;

pub(crate) mod raw;

#[derive(Debug, Default)]
pub struct AsProfBuilder {}

#[derive(Error, Debug)]
#[error("async-profiler Library Error: {0}")]
#[non_exhaustive]
pub enum AsProfError {
    #[error("async-profiler error: {0}")]
    AsProfError(String),
    #[error("async-profiler i/o error: {0}")]
    Io(#[from] std::io::Error),
    #[error("error loading libasyncProfiler: {0}")]
    LibraryError(#[from] Arc<libloading::Error>),
}

impl AsProfBuilder {
    pub fn build(self) -> AsProf {
        AsProf {}
    }
}

pub struct AsProf {}

impl AsProf {
    pub fn builder() -> AsProfBuilder {
        AsProfBuilder::default()
    }
}

impl super::profiler::ProfilerEngine for AsProf {
    fn init_async_profiler() -> Result<(), self::AsProfError> {
        unsafe {
            (raw::async_profiler()?.asprof_init)();
        };
        Ok(())
    }

    fn start_async_profiler(
        &self,
        jfr_file_path: &std::path::PathBuf,
    ) -> Result<(), self::AsProfError> {
        tracing::debug!("starting the async-profiler and giving JFR file path: {jfr_file_path:?}");

        let args = format!(
            "start,event=cpu,interval=100000000,wall=1000ms,jfr,cstack=dwarf,file={}",
            jfr_file_path.display()
        );

        Self::asprof_execute(&args)?;
        tracing::debug!("async-profiler started successfully");
        Ok(())
    }

    fn stop_async_profiler() -> Result<(), self::AsProfError> {
        Self::asprof_execute("stop")?;
        tracing::debug!("async-profiler stopped successfully");
        Ok(())
    }
}

impl AsProf {
    fn asprof_execute(args: &str) -> Result<(), AsProfError> {
        unsafe extern "C" fn callback(buf: *const c_char, size: usize) {
            unsafe {
                if !buf.is_null() {
                    let parts = std::slice::from_raw_parts(buf as *const u8, size);
                    tracing::debug!(
                        "response from async-profiler: {}",
                        String::from_utf8_lossy(parts)
                    );
                } else {
                    tracing::debug!("invalid pointer or size");
                }
            }
        }

        let args_compatible = CString::new(args).unwrap();
        let response = unsafe {
            (raw::async_profiler()?.asprof_execute)(args_compatible.as_ptr(), Some(callback))
        };
        if !response.is_null() {
            let response = unsafe { CStr::from_ptr(response) };
            let response_str = response.to_string_lossy();
            tracing::error!("received error from async-profiler: {}", response_str);
            Err(AsProfError::AsProfError(response_str.to_string()))
            // TODO: stop the background thread in case there is an error
        } else {
            Ok(())
        }
    }
}
