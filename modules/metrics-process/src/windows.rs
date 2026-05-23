use std::sync::Arc;
use std::time::Duration;
use std::time::Instant;

use ferron_observability::{
    CompositeEventSink, Event, MetricAttributeValue, MetricEvent, MetricType, MetricValue,
};
use windows::Win32::Foundation::{FILETIME, HANDLE};
use windows::Win32::System::ProcessStatus::{GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS};
use windows::Win32::System::Threading::{GetCurrentProcess, GetProcessTimes};

struct ProcessState {
    instant: Instant,
    previous_cpu_user_time: f64,
    previous_cpu_system_time: f64,
    previous_rss: u64,
    previous_vms: u64,
}

impl Default for ProcessState {
    fn default() -> Self {
        Self {
            instant: Instant::now(),
            previous_cpu_user_time: 0.0,
            previous_cpu_system_time: 0.0,
            previous_rss: 0,
            previous_vms: 0,
        }
    }
}

struct ProcessStateSnapshot {
    cpu_user_time: f64,
    cpu_system_time: f64,
    working_set: u64,
    pagefile_usage: u64,
}

fn filetime_to_seconds(ft: &FILETIME) -> f64 {
    let total_100ns = (ft.dwHighDateTime as u64) << 32 | ft.dwLowDateTime as u64;
    total_100ns as f64 / 10_000_000.0
}

fn read_process_state() -> Option<ProcessStateSnapshot> {
    let process_handle: HANDLE = unsafe { GetCurrentProcess() };

    let mut creation_time = FILETIME::default();
    let mut exit_time = FILETIME::default();
    let mut kernel_time = FILETIME::default();
    let mut user_time = FILETIME::default();

    if let Err(e) = unsafe {
        GetProcessTimes(
            process_handle,
            &mut creation_time,
            &mut exit_time,
            &mut kernel_time,
            &mut user_time,
        )
    } {
        ferron_core::log_debug!("GetProcessTimes failed: {}", e);
        return None;
    }

    let user_seconds = filetime_to_seconds(&user_time);
    let kernel_seconds = filetime_to_seconds(&kernel_time);

    let mut pmc = PROCESS_MEMORY_COUNTERS::default();
    if let Err(e) = unsafe {
        GetProcessMemoryInfo(
            process_handle,
            &mut pmc,
            std::mem::size_of::<PROCESS_MEMORY_COUNTERS>() as u32,
        )
    } {
        ferron_core::log_debug!("GetProcessMemoryInfo failed: {}", e);
        return None;
    }

    Some(ProcessStateSnapshot {
        cpu_user_time: user_seconds,
        cpu_system_time: kernel_seconds,
        working_set: pmc.WorkingSetSize as u64,
        pagefile_usage: pmc.PagefileUsage as u64,
    })
}

pub async fn collect_process_metrics(
    event_sink: Arc<CompositeEventSink>,
    cancel_token: tokio_util::sync::CancellationToken,
) {
    let parallelism = num_cpus::get();

    let mut state = ProcessState::default();

    loop {
        tokio::select! {
            _ = cancel_token.cancelled() => break,
            _ = tokio::time::sleep(Duration::from_secs(1)) => {}
        }

        let Some(snapshot) = read_process_state() else {
            continue;
        };

        let cpu_user_time_increase = snapshot.cpu_user_time - state.previous_cpu_user_time;
        let cpu_system_time_increase = snapshot.cpu_system_time - state.previous_cpu_system_time;
        state.previous_cpu_user_time = snapshot.cpu_user_time;
        state.previous_cpu_system_time = snapshot.cpu_system_time;

        let rss_diff = snapshot.working_set as i64 - state.previous_rss as i64;
        let vms_diff = snapshot.pagefile_usage as i64 - state.previous_vms as i64;
        state.previous_rss = snapshot.working_set;
        state.previous_vms = snapshot.pagefile_usage;

        let elapsed = state.instant.elapsed().as_secs_f64();
        state.instant = Instant::now();

        if elapsed <= 0.0 {
            continue;
        }

        let cpu_user_utilization = cpu_user_time_increase / (elapsed * parallelism as f64);
        let cpu_system_utilization = cpu_system_time_increase / (elapsed * parallelism as f64);

        emit_metrics(
            &event_sink,
            cpu_user_time_increase,
            cpu_system_time_increase,
            cpu_user_utilization,
            cpu_system_utilization,
            rss_diff,
            vms_diff,
        );
    }
}

fn emit_metrics(
    event_sink: &CompositeEventSink,
    cpu_user_time_increase: f64,
    cpu_system_time_increase: f64,
    cpu_user_utilization: f64,
    cpu_system_utilization: f64,
    rss_diff: i64,
    vms_diff: i64,
) {
    event_sink.emit(Event::Metric(MetricEvent {
        name: "process.cpu.time",
        attributes: vec![("cpu.mode", MetricAttributeValue::String("user".to_string()))],
        ty: MetricType::Counter,
        value: MetricValue::F64(cpu_user_time_increase),
        unit: Some("s"),
        description: Some("Total CPU seconds broken down by different states."),
    }));

    event_sink.emit(Event::Metric(MetricEvent {
        name: "process.cpu.time",
        attributes: vec![(
            "cpu.mode",
            MetricAttributeValue::String("system".to_string()),
        )],
        ty: MetricType::Counter,
        value: MetricValue::F64(cpu_system_time_increase),
        unit: Some("s"),
        description: Some("Total CPU seconds broken down by different states."),
    }));

    event_sink.emit(Event::Metric(MetricEvent {
        name: "process.cpu.utilization",
        attributes: vec![("cpu.mode", MetricAttributeValue::String("user".to_string()))],
        ty: MetricType::Gauge,
        value: MetricValue::F64(cpu_user_utilization),
        unit: Some("1"),
        description: Some(
            "Difference in process.cpu.time since the last measurement, \
             divided by the elapsed time and number of CPUs available to the process.",
        ),
    }));

    event_sink.emit(Event::Metric(MetricEvent {
        name: "process.cpu.utilization",
        attributes: vec![(
            "cpu.mode",
            MetricAttributeValue::String("system".to_string()),
        )],
        ty: MetricType::Gauge,
        value: MetricValue::F64(cpu_system_utilization),
        unit: Some("1"),
        description: Some(
            "Difference in process.cpu.time since the last measurement, \
             divided by the elapsed time and number of CPUs available to the process.",
        ),
    }));

    event_sink.emit(Event::Metric(MetricEvent {
        name: "process.memory.usage",
        attributes: vec![],
        ty: MetricType::UpDownCounter,
        value: MetricValue::I64(rss_diff),
        unit: Some("By"),
        description: Some("The amount of physical memory in use."),
    }));

    event_sink.emit(Event::Metric(MetricEvent {
        name: "process.memory.virtual",
        attributes: vec![],
        ty: MetricType::UpDownCounter,
        value: MetricValue::I64(vms_diff),
        unit: Some("By"),
        description: Some("The amount of committed virtual memory."),
    }));
}
