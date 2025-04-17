use std::{ffi::OsString, fs::File, io::Cursor};

use clap::{Parser, Subcommand};
use jfrs::reader::{
    event::Accessor,
    type_descriptor::TypeDescriptor,
    value_descriptor::{Primitive, ValueDescriptor},
    Chunk, JfrReader,
};
use std::io::{Read, Seek};
use std::time::Duration;

#[derive(Debug, Parser)]
#[command(name = "pollcatch-decoder")]
#[command(about = "Find slow polls from a JFR")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Debug, Subcommand)]
enum Commands {
    /// Print long polls from a JFR file
    Longpolls {
        /// JFR file to read from
        jfr_file: OsString,
        /// If true, unzip first
        #[arg(long)]
        zip: bool,
        /// Poll duration to mark from
        #[clap(value_parser = humantime::parse_duration)]
        min_length: Duration,
        #[arg(long, default_value = "5")]
        stack_depth: usize,
    },
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct PollEventKey {
    tid: u32,
    clock_start: u64,
    duration: u64,
}

fn extract_async_profiler_jfr_from_zip(file: File) -> anyhow::Result<Option<Vec<u8>>> {
    let mut zip = zip::ZipArchive::new(file)?;

    for i in 0..zip.len() {
        let mut file = zip.by_index(i)?;
        if file.name() == "async_profiler_dump_0.jfr" {
            let mut buf = vec![];
            std::io::copy(&mut file, &mut buf)?;
            return Ok(Some(buf));
        }
    }

    Ok(None)
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    tracing_subscriber::fmt::init();
    match cli.command {
        Commands::Longpolls {
            jfr_file,
            min_length,
            stack_depth,
            zip,
        } => {
            let mut jfr_file = std::fs::File::open(jfr_file)?;
            let samples = match zip {
                false => jfr_samples(&mut jfr_file, min_length)?,
                true => {
                    if let Some(data) = extract_async_profiler_jfr_from_zip(jfr_file)? {
                        jfr_samples(&mut Cursor::new(&data), min_length)?
                    } else {
                        anyhow::bail!("no async_profiler_dump_0.jfr file found");
                    }
                }
            };
            print_samples(samples, stack_depth);
            Ok(())
        }
    }
}

fn symbol_to_string(s: Accessor<'_>) -> Option<&str> {
    if let Some(sym) = s.get_field("string") {
        if let Ok(val) = sym.value.try_into() {
            return Some(val);
        }
    }

    None
}

fn print_samples(samples: Vec<Sample>, stack_depth: usize) {
    for sample in samples {
        if sample.frames.iter().any(|f| {
            f.name.as_ref().is_some_and(|n| {
                n.contains(
                    "<tokio::runtime::scheduler::multi_thread::worker::Context>::park_timeout",
                )
            })
        }) {
            // skip samples that are of sleeps
            continue;
        }
        println!(
            "[{:.6}] thread {} - poll of {}us",
            sample.start_time.as_secs_f64(),
            sample.thread_id,
            sample.delta_t.as_micros()
        );
        for (i, frame) in sample.frames.iter().enumerate() {
            if i == stack_depth {
                println!(
                    " - {:3} more frame(s) (pass --stack-depth={} to show)",
                    sample.frames.len() - stack_depth,
                    sample.frames.len()
                );
                break;
            }
            println!(
                " - {:3}: {}.{}",
                i + 1,
                frame.class_name.as_deref().unwrap_or("<unknown>"),
                frame.name.as_deref().unwrap_or("<unknown>")
            );
        }
        println!();
    }
}

struct Sample {
    delta_t: Duration,
    start_time: Duration,
    thread_id: i64,
    frames: Vec<StackFrame>,
}

struct StackFrame {
    class_name: Option<String>,
    name: Option<String>,
}

fn resolve_stack_trace(trace: Accessor<'_>) -> Vec<StackFrame> {
    let mut res = vec![];
    if let Some(frames) = trace.get_field("frames") {
        if let Some(frames) = frames.as_iter() {
            for frame in frames {
                let mut class_name_s = None;
                let mut name_s = None;
                if let Some(method) = frame.get_field("method") {
                    if let Some(class) = method.get_field("type") {
                        if let Some(class_name) = class.get_field("name") {
                            class_name_s = symbol_to_string(class_name).map(|x| x.to_owned());
                        }
                    }
                    if let Some(name) = method.get_field("name") {
                        name_s = symbol_to_string(name).map(|x| x.to_owned());
                    }
                }
                res.push(StackFrame {
                    class_name: class_name_s,
                    name: name_s,
                });
            }
        }
    }
    res
}

fn find_delta_t_from_clock(pr_map: &[PollEventKey], tid: i64, clock_start: i64) -> Option<u64> {
    if let (Ok(tid), Ok(clock_start)) = (tid.try_into(), clock_start.try_into()) {
        let partition_point = pr_map
            .partition_point(|x| x.tid < tid || (tid == x.tid && x.clock_start <= clock_start));
        if let Some(index) = partition_point.checked_sub(1) {
            let bound = pr_map[index];
            let inside = tid == bound.tid
                && bound.clock_start < clock_start
                && clock_start - bound.clock_start < bound.duration;
            if inside {
                return Some(clock_start - bound.clock_start);
            }
        }
        None
    } else {
        None
    }
}

fn process_sample(
    chunk: &Chunk,
    tys: &JfrTypeInfo,
    pr_map: &[PollEventKey],
    sampled_thread: Option<&ValueDescriptor>,
    stacktrace: Option<&ValueDescriptor>,
    start_time_ticks: i64,
    long_poll_duration: u128,
) -> Option<Sample> {
    let mut delta_t = 0;
    let mut thread_id = !0;
    if let Some(ValueDescriptor::Object(st)) = sampled_thread {
        if let Some(tid) = st.fields.get(tys.os_thread_index).and_then(as_long) {
            thread_id = tid;
        }
    }
    if delta_t == 0 {
        if let Some(delta_t_) = find_delta_t_from_clock(pr_map, thread_id, start_time_ticks) {
            delta_t = delta_t_;
        }
    }

    let delta_t_micros = (delta_t as u128) * 1000000 / (chunk.header.ticks_per_second as u128);
    if delta_t_micros < long_poll_duration {
        return None;
    }
    stacktrace.map(|trace| Sample {
        thread_id,
        start_time: Duration::from_nanos(
            ((start_time_ticks as u128) * 1_000_000_000 / (chunk.header.ticks_per_second as u128))
                as u64,
        ),
        delta_t: Duration::from_micros(delta_t_micros as u64),
        frames: resolve_stack_trace(Accessor::new(chunk, trace)),
    })
}

struct JfrTypeInfo {
    // profiler.WallClockSample
    wall_clock_sample: Option<i64>,
    wcs_start_time_index: usize,
    wcs_stacktrace_index: usize,
    wcs_sampled_thread_index: usize,

    // jdk.ExecutionSample
    execution_sample: Option<i64>,
    exs_start_time_index: usize,
    exs_stacktrace_index: usize,
    exs_sampled_thread_index: usize,

    // jdk.ActiveSetting
    active_setting: Option<i64>,
    active_setting_name_index: usize,
    active_setting_value_index: usize,

    // profiler.UserEvent
    user_event: Option<i64>,
    user_event_type_index: usize,
    user_event_start_time_index: usize,
    user_event_event_thread_index: usize,
    user_event_data_index: usize,

    // profiler.UserEventType
    user_event_type_name: usize,

    // java.lang.Thread
    os_thread_index: usize,
}

impl JfrTypeInfo {
    fn new() -> Self {
        JfrTypeInfo {
            wall_clock_sample: None,
            wcs_start_time_index: !0,
            wcs_stacktrace_index: !0,
            wcs_sampled_thread_index: !0,
            execution_sample: None,
            exs_start_time_index: !0,
            exs_stacktrace_index: !0,
            exs_sampled_thread_index: !0,
            active_setting: None,
            active_setting_name_index: !0,
            active_setting_value_index: !0,
            user_event: None,
            user_event_type_index: !0,
            user_event_start_time_index: !0,
            user_event_event_thread_index: !0,
            user_event_data_index: !0,
            user_event_type_name: !0,
            os_thread_index: !0,
        }
    }

    fn load_type_descriptor(&mut self, ty: &TypeDescriptor) {
        match ty.name() {
            "profiler.WallClockSample" => {
                self.wall_clock_sample = Some(ty.class_id);
                for (i, field) in ty.fields.iter().enumerate() {
                    match field.name() {
                        "startTime" => self.wcs_start_time_index = i,
                        "stackTrace" => self.wcs_stacktrace_index = i,
                        "sampledThread" => self.wcs_sampled_thread_index = i,
                        _ => {}
                    }
                }
            }
            "profiler.UserEvent" => {
                self.user_event = Some(ty.class_id);
                for (i, field) in ty.fields.iter().enumerate() {
                    match field.name() {
                        "type" => self.user_event_type_index = i,
                        "startTime" => self.user_event_start_time_index = i,
                        "eventThread" => self.user_event_event_thread_index = i,
                        "data" => self.user_event_data_index = i,
                        _ => {}
                    }
                }
            }
            "profiler.types.UserEventType" => {
                for (i, field) in ty.fields.iter().enumerate() {
                    #[allow(clippy::single_match)]
                    match field.name() {
                        "name" => self.user_event_type_name = i,
                        _ => {}
                    }
                }
            }
            "jdk.ExecutionSample" => {
                self.execution_sample = Some(ty.class_id);
                for (i, field) in ty.fields.iter().enumerate() {
                    match field.name() {
                        "startTime" => self.exs_start_time_index = i,
                        "stackTrace" => self.exs_stacktrace_index = i,
                        "sampledThread" => self.exs_sampled_thread_index = i,
                        _ => {}
                    }
                }
            }
            "java.lang.Thread" => {
                for (i, field) in ty.fields.iter().enumerate() {
                    #[allow(clippy::single_match)]
                    match field.name() {
                        "osThreadId" => self.os_thread_index = i,
                        _ => {}
                    }
                }
            }
            "jdk.ActiveSetting" => {
                self.active_setting = Some(ty.class_id);
                for (i, field) in ty.fields.iter().enumerate() {
                    match field.name() {
                        "name" => self.active_setting_name_index = i,
                        "value" => self.active_setting_value_index = i,
                        _ => {}
                    }
                }
            }
            _ => {}
        }
    }
}

fn as_object(x: &ValueDescriptor) -> Option<&jfrs::reader::value_descriptor::Object> {
    match x {
        ValueDescriptor::Object(o) => Some(o),
        _ => None,
    }
}

fn as_string(x: &ValueDescriptor) -> Option<&str> {
    match x {
        ValueDescriptor::Primitive(Primitive::String(s)) => Some(s),
        _ => None,
    }
}

fn as_long(x: &ValueDescriptor) -> Option<i64> {
    match x {
        ValueDescriptor::Primitive(Primitive::Long(i)) => Some(*i),
        _ => None,
    }
}

fn resolve_field<'a>(
    chunk: &'a Chunk,
    o: &'a jfrs::reader::value_descriptor::Object,
    index: usize,
) -> Option<&'a ValueDescriptor> {
    o.fields
        .get(index)
        .and_then(|st| Accessor::new(chunk, st).resolve())
        .map(|a| a.value)
}

fn poll_event_from_user_event(
    chunk: &Chunk,
    tys: &JfrTypeInfo,
    event: jfrs::reader::event::Event<'_>,
) -> Option<PollEventKey> {
    let event = as_object(event.value().value)?;
    let ty = as_object(resolve_field(chunk, event, tys.user_event_type_index)?)?;

    if as_string(ty.fields.get(tys.user_event_type_name)?) == Some("tokio.PollcatchV1") {
        let start_time_ticks = event
            .fields
            .get(tys.user_event_start_time_index)
            .and_then(as_long)
            .unwrap_or(0);
        let mut thread_id = 0;
        let event_thread =
            resolve_field(chunk, event, tys.user_event_event_thread_index).and_then(as_object);
        if let Some(et) = event_thread {
            if let Some(tid) = et.fields.get(tys.os_thread_index).and_then(as_long) {
                thread_id = tid;
            }
        }
        let data = if let Some(s) = event
            .fields
            .get(tys.user_event_data_index)
            .and_then(as_string)
        {
            // convert from "pseudo latin 1" to Vec<u8>
            s.chars().map(|c| c as u32 as u8).collect::<Vec<u8>>()
        } else {
            vec![]
        };

        let before = data
            .get(0..8)
            .map_or(0, |x| u64::from_le_bytes(x.try_into().unwrap()));
        let end = data
            .get(8..16)
            .map_or(0, |x| u64::from_le_bytes(x.try_into().unwrap()));
        let _clock_end = data
            .get(16..24)
            .map_or(0, |x| u64::from_le_bytes(x.try_into().unwrap()));
        let duration = end.saturating_sub(before);

        Some(PollEventKey {
            tid: thread_id as u32,
            clock_start: (start_time_ticks as u64).saturating_sub(duration),
            duration: duration as u64,
        })
    } else {
        None
    }
}

fn jfr_samples<T>(reader: &mut T, long_poll_duration: Duration) -> anyhow::Result<Vec<Sample>>
where
    T: Read + Seek,
{
    let mut jfr_reader = JfrReader::new(reader);
    let long_poll_duration = long_poll_duration.as_micros();

    let mut samples = vec![];
    let mut tys = JfrTypeInfo::new();
    for chunk in jfr_reader.chunks() {
        let (mut c_rdr, c) = chunk?;
        for ty in c.metadata.type_pool.get_types() {
            tys.load_type_descriptor(ty);
        }
        let mut pr_map = &const { Vec::new() };
        let mut jfr_pr_map = vec![];
        for event in c_rdr.events_from_offset(&c, 0) {
            let event: jfrs::reader::event::Event<'_> = event?;
            if Some(event.class.class_id) == tys.user_event {
                if let Some(event) = poll_event_from_user_event(&c, &tys, event) {
                    jfr_pr_map.push(event);
                }
            }
        }
        jfr_pr_map.sort();
        for event in c_rdr.events_from_offset(&c, 0) {
            let event: jfrs::reader::event::Event<'_> = event?;
            if Some(event.class.class_id) == tys.active_setting {
                if let ValueDescriptor::Object(o) = event.value().value {
                    let name =
                        resolve_field(&c, o, tys.active_setting_name_index).and_then(as_string);
                    let value =
                        resolve_field(&c, o, tys.active_setting_value_index).and_then(as_string);
                    if let (Some("clock"), Some(value)) = (name, value) {
                        if value == "tsc" {
                            pr_map = &jfr_pr_map;
                        } else {
                            anyhow::bail!("decoder only supports tsc profiles, not {value:?}");
                        }
                    }
                }
            }
            if Some(event.class.class_id) == tys.wall_clock_sample {
                if let Some(o) = as_object(event.value().value) {
                    let start_time_ticks = o
                        .fields
                        .get(tys.wcs_start_time_index)
                        .and_then(as_long)
                        .unwrap_or(0);
                    let sampled_thread = o
                        .fields
                        .get(tys.wcs_sampled_thread_index)
                        .and_then(|st| Accessor::new(&c, st).resolve())
                        .map(|a| a.value);
                    let stacktrace = o.fields.get(tys.wcs_stacktrace_index);
                    if let Some(sample) = process_sample(
                        &c,
                        &tys,
                        pr_map,
                        sampled_thread,
                        stacktrace,
                        start_time_ticks,
                        long_poll_duration,
                    ) {
                        samples.push(sample);
                    }
                }
            }
            if Some(event.class.class_id) == tys.execution_sample {
                if let Some(o) = as_object(event.value().value) {
                    let start_time_ticks = o
                        .fields
                        .get(tys.exs_start_time_index)
                        .and_then(as_long)
                        .unwrap_or(0);
                    let sampled_thread = o
                        .fields
                        .get(tys.exs_sampled_thread_index)
                        .and_then(|st| Accessor::new(&c, st).resolve())
                        .map(|a| a.value);
                    let stacktrace = o.fields.get(tys.exs_stacktrace_index);
                    if let Some(sample) = process_sample(
                        &c,
                        &tys,
                        pr_map,
                        sampled_thread,
                        stacktrace,
                        start_time_ticks,
                        long_poll_duration,
                    ) {
                        samples.push(sample);
                    }
                }
            }
        }
    }
    Ok(samples)
}
