//! Linux process metrics collection using procfs.
//!
//! Collects CPU time, CPU utilization, memory usage, and virtual memory
//! metrics for the current process by reading `/proc/self/stat`.

use std::sync::Arc;
use std::time::Duration;
use std::time::Instant;

use ferron_observability::{
    CompositeEventSink, Event, MetricAttributeValue, MetricEvent, MetricType, MetricValue,
};

/// Tracks the previous state of process metrics for delta calculations.
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

/// Reads the current process state from `/proc/self/stat`.
fn read_process_state() -> Option<ProcessStateSnapshot> {
    let stat = match procfs::process::Process::myself().and_then(|p| p.stat()) {
        Ok(s) => s,
        Err(e) => {
            // Log at debug level — this is expected to fail occasionally during startup
            ferron_core::log_debug!("Failed to read process stats: {}", e);
            return None;
        }
    };

    let tps = procfs::ticks_per_second() as f64;
    Some(ProcessStateSnapshot {
        cpu_user_time: stat.utime as f64 / tps,
        cpu_system_time: stat.stime as f64 / tps,
        rss: stat.rss * procfs::page_size(),
        vms: stat.vsize,
    })
}

/// A point-in-time snapshot of process metrics from `/proc/self/stat`.
struct ProcessStateSnapshot {
    cpu_user_time: f64,
    cpu_system_time: f64,
    rss: u64,
    vms: u64,
}

/// Runs the background process metrics collection loop.
///
/// Collects metrics every 1 second and emits them through the composite event sink.
pub async fn collect_process_metrics(
    event_sink: Arc<CompositeEventSink>,
    cancel_token: tokio_util::sync::CancellationToken,
) {
    // Get the number of logical CPUs for utilization normalization
    let parallelism = num_cpus::get();

    let mut state = ProcessState::default();

    // Initialize the baseline from current process state
    if let Some(snapshot) = read_process_state() {
        state.previous_cpu_user_time = snapshot.cpu_user_time;
        state.previous_cpu_system_time = snapshot.cpu_system_time;
        state.previous_rss = snapshot.rss;
        state.previous_vms = snapshot.vms;
    }

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

        let rss_diff = snapshot.rss as i64 - state.previous_rss as i64;
        let vms_diff = snapshot.vms as i64 - state.previous_vms as i64;
        state.previous_rss = snapshot.rss;
        state.previous_vms = snapshot.vms;

        let elapsed = state.instant.elapsed().as_secs_f64();
        state.instant = Instant::now();

        // Avoid division by zero (shouldn't happen with 1s interval, but be safe)
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
