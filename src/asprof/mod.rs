// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

use crate::profiler::ProfilerOptions;
use std::{
    ffi::{c_char, CStr, CString},
    path::Path,
    ptr::{self, addr_of},
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
    AsyncProfilerError(String),
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

#[derive(Copy, Clone, Debug)]
pub struct UserJfrKey {
    key: i32,
}

pub struct AsProf {}

impl AsProf {
    pub fn builder() -> AsProfBuilder {
        AsProfBuilder::default()
    }

    /// Return the async-profiler's sample counter
    pub fn get_sample_counter() -> Option<u64> {
        unsafe {
            let prof = raw::async_profiler().ok()?;
            let thread_local_data = prof.asprof_get_thread_local_data.as_ref()?();
            if thread_local_data.is_null() {
                None
            } else {
                Some(ptr::read_volatile(addr_of!(
                    (*thread_local_data).sample_counter
                )))
            }
        }
    }

    /// Create a user JFR key from a given key
    pub fn create_user_jfr_key(key: &CStr) -> Result<UserJfrKey, AsProfError> {
        unsafe {
            let prof = raw::async_profiler()?;
            let asprof_register_jfr_event =
                prof.asprof_register_jfr_event.as_ref().ok_or_else(|| {
                    AsProfError::AsyncProfilerError(
                        "async-profiler does not support user JFR events".into(),
                    )
                })?;
            let res = asprof_register_jfr_event(key.as_ptr());
            if res < 0 {
                Err(AsProfError::AsyncProfilerError(
                    "unable to register JFR event".into(),
                ))
            } else {
                Ok(UserJfrKey { key: res })
            }
        }
    }

    pub fn emit_user_jfr(key: UserJfrKey, jfr: &[u8]) -> Result<(), AsProfError> {
        unsafe {
            let prof = raw::async_profiler()?;
            let asprof_emit_jfr_event = prof.asprof_emit_jfr_event.as_ref().ok_or_else(|| {
                AsProfError::AsyncProfilerError(
                    "async-profiler does not support user JFR events".into(),
                )
            })?;
            Self::asprof_error(asprof_emit_jfr_event(key.key, jfr.as_ptr(), jfr.len()))
        }
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
        jfr_file_path: &Path,
        options: &ProfilerOptions,
    ) -> Result<(), self::AsProfError> {
        tracing::debug!("starting the async-profiler and giving JFR file path: {jfr_file_path:?}");

        let args = options.to_args_string(jfr_file_path);

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
    /// convert an asprof_error_t to a Result
    ///
    /// SAFETY: response must be a valid asprof_error_t
    unsafe fn asprof_error(response: raw::asprof_error_t) -> Result<(), AsProfError> {
        if !response.is_null() {
            let response = (raw::async_profiler()?.asprof_error_str)(response);
            if response.is_null() {
                return Ok(());
            }
            let response = unsafe { CStr::from_ptr(response) };
            let response_str = response.to_string_lossy();
            tracing::error!("received error from async-profiler: {}", response_str);
            Err(AsProfError::AsyncProfilerError(response_str.to_string()))
            // TODO: stop the background thread in case there is an error
        } else {
            Ok(())
        }
    }

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
        unsafe {
            Self::asprof_error((raw::async_profiler()?.asprof_execute)(
                args_compatible.as_ptr(),
                Some(callback),
            ))
        }
    }
}
