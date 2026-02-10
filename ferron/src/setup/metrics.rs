use std::time::Duration;

use async_channel::Sender;
use ferron_common::observability::{Metric, MetricAttributeValue, MetricType, MetricValue};

/// Performs background periodic metrics collection.
pub async fn background_metrics(metrics_channels: Vec<Sender<Metric>>, parallelism: usize) {
  let mut previous_instant = std::time::Instant::now();
  let mut previous_cpu_user_time = 0.0;
  let mut previous_cpu_system_time = 0.0;
  let mut previous_rss = 0;
  let mut previous_vms = 0;
  loop {
    // Sleep for 1 second
    tokio::time::sleep(Duration::from_secs(1)).await;

    if let Ok(Ok(stat)) =
      tokio::task::spawn_blocking(|| procfs::process::Process::myself().and_then(|p| p.stat())).await
    {
      let cpu_user_time = stat.utime as f64 / procfs::ticks_per_second() as f64;
      let cpu_system_time = stat.stime as f64 / procfs::ticks_per_second() as f64;
      let cpu_user_time_increase = cpu_user_time - previous_cpu_user_time;
      let cpu_system_time_increase = cpu_system_time - previous_cpu_system_time;
      previous_cpu_user_time = cpu_user_time;
      previous_cpu_system_time = cpu_system_time;

      let rss = stat.rss * procfs::page_size();
      let rss_diff = rss as i64 - previous_rss as i64;
      let vms_diff = stat.vsize as i64 - previous_vms as i64;
      previous_rss = rss;
      previous_vms = stat.vsize;

      let elapsed = previous_instant.elapsed().as_secs_f64();
      previous_instant = std::time::Instant::now();

      let cpu_user_utilization = cpu_user_time_increase / (elapsed * parallelism as f64);
      let cpu_system_utilization = cpu_system_time_increase / (elapsed * parallelism as f64);

      for metrics_sender in &metrics_channels {
        metrics_sender
          .send(Metric::new(
            "process.cpu.time",
            vec![("cpu.mode", MetricAttributeValue::String("user".to_string()))],
            MetricType::Counter,
            MetricValue::F64(cpu_user_time_increase),
            Some("s"),
            Some("Total CPU seconds broken down by different states."),
          ))
          .await
          .unwrap_or_default();

        metrics_sender
          .send(Metric::new(
            "process.cpu.time",
            vec![("cpu.mode", MetricAttributeValue::String("system".to_string()))],
            MetricType::Counter,
            MetricValue::F64(cpu_system_time_increase),
            Some("s"),
            Some("Total CPU seconds broken down by different states."),
          ))
          .await
          .unwrap_or_default();

        metrics_sender
          .send(Metric::new(
            "process.cpu.utilization",
            vec![("cpu.mode", MetricAttributeValue::String("user".to_string()))],
            MetricType::Gauge,
            MetricValue::F64(cpu_user_utilization),
            Some("1"),
            Some(
              "Difference in process.cpu.time since the last measurement, \
               divided by the elapsed time and number of CPUs available to the process.",
            ),
          ))
          .await
          .unwrap_or_default();

        metrics_sender
          .send(Metric::new(
            "process.cpu.utilization",
            vec![("cpu.mode", MetricAttributeValue::String("system".to_string()))],
            MetricType::Gauge,
            MetricValue::F64(cpu_system_utilization),
            Some("1"),
            Some(
              "Difference in process.cpu.time since the last measurement, \
              divided by the elapsed time and number of CPUs available to the process.",
            ),
          ))
          .await
          .unwrap_or_default();

        metrics_sender
          .send(Metric::new(
            "process.memory.usage",
            vec![],
            MetricType::UpDownCounter,
            MetricValue::I64(rss_diff),
            Some("By"),
            Some("The amount of physical memory in use."),
          ))
          .await
          .unwrap_or_default();

        metrics_sender
          .send(Metric::new(
            "process.memory.virtual",
            vec![],
            MetricType::UpDownCounter,
            MetricValue::I64(vms_diff),
            Some("By"),
            Some("The amount of committed virtual memory."),
          ))
          .await
          .unwrap_or_default();
      }
    }
  }
}
