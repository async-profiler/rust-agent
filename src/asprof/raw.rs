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

#[allow(non_camel_case_types)]
#[repr(C)]
pub struct asprof_thread_local_data {
    pub sample_counter: u64,
}

#[allow(non_camel_case_types)]
pub type asprof_jfr_event_key = std::ffi::c_int;

pub(crate) struct AsyncProfiler {
    pub(crate) asprof_init: libloading::Symbol<'static, unsafe extern "C" fn()>,
    pub(crate) asprof_execute: libloading::Symbol<
        'static,
        unsafe extern "C" fn(
            command: *const std::ffi::c_char,
            output_callback: asprof_writer_t,
        ) -> asprof_error_t,
    >,
    pub(crate) asprof_error_str: libloading::Symbol<
        'static,
        unsafe extern "C" fn(asprof_error_t) -> *const std::ffi::c_char,
    >,
    pub(crate) asprof_get_thread_local_data: Option<
        libloading::Symbol<'static, unsafe extern "C" fn() -> *mut asprof_thread_local_data>,
    >,
    pub(crate) asprof_register_jfr_event: Option<
        libloading::Symbol<
            'static,
            unsafe extern "C" fn(*const std::ffi::c_char) -> asprof_jfr_event_key,
        >,
    >,
    pub(crate) asprof_emit_jfr_event: Option<
        libloading::Symbol<
            'static,
            unsafe extern "C" fn(asprof_jfr_event_key, *const u8, usize) -> asprof_error_t,
        >,
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
                asprof_error_str: lib.get(b"asprof_error_str")?,
                asprof_get_thread_local_data: lib.get(b"asprof_get_thread_local_data").ok(),
                asprof_register_jfr_event: lib.get(b"asprof_register_jfr_event").ok(),
                asprof_emit_jfr_event: lib.get(b"asprof_emit_jfr_event").ok(),
            })
        }
    });

pub fn async_profiler() -> Result<&'static AsyncProfiler, Arc<libloading::Error>> {
    ASYNC_PROFILER.as_ref().map_err(|e| e.clone())
}
