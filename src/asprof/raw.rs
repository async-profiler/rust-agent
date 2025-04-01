// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

use std::sync::{Arc, LazyLock};

// these bindings copied from asprof.h
// in sync with
// https://github.com/async-profiler/async-profiler/blob/bd439d8a0421a821b0c17e5ca74e363103c9cf67/src/asprof.h

#[allow(non_camel_case_types)]
pub type asprof_error_t = *const std::ffi::c_char;
#[allow(non_camel_case_types)]
pub type asprof_writer_t = Option<unsafe extern "C" fn(buf: *const std::ffi::c_char, size: usize)>;

pub(crate) struct AsyncProfiler {
    pub(crate) asprof_init: libloading::Symbol<'static, unsafe extern "C" fn()>,
    pub(crate) asprof_execute: libloading::Symbol<
        'static,
        unsafe extern "C" fn(
            command: *const std::ffi::c_char,
            output_callback: asprof_writer_t,
        ) -> asprof_error_t,
    >,
}

// make sure libasyncProfiler.so is dlopen'd from a static, to avoid it being dlclose'd.
// Since async-profiler starts threads and installs signal handlers, dlclosing it is probably
// a bad idea.
static ASYNC_PROFILER_LIB: LazyLock<Result<libloading::Library, Arc<libloading::Error>>> =
    LazyLock::new(|| Ok(unsafe { libloading::Library::new("libasyncProfiler.so")? }));

// this needs to be a separate static from ASYNC_PROFILER_LIB to avoid
// lifetime issues.
static ASYNC_PROFILER: LazyLock<Result<AsyncProfiler, Arc<libloading::Error>>> =
    LazyLock::new(|| {
        // safety: correct use of dlopen
        unsafe {
            let lib = ASYNC_PROFILER_LIB.as_ref().map_err(|e| e.clone())?;
            Ok(AsyncProfiler {
                asprof_init: lib.get(b"asprof_init")?,
                asprof_execute: lib.get(b"asprof_execute")?,
            })
        }
    });

pub fn async_profiler() -> Result<&'static AsyncProfiler, Arc<libloading::Error>> {
    ASYNC_PROFILER.as_ref().map_err(|e| e.clone())
}
