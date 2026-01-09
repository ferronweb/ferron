use std::collections::HashSet;
use std::error::Error;
use std::sync::Arc;

use async_channel::{Receiver, Sender};

use crate::config::ServerConfiguration;
use crate::logging::LogMessage;

/// A trait that defines an observability backend loader
pub trait ObservabilityBackendLoader {
  /// Loads an observability backend according to specific configuration
  fn load_observability_backend(
    &mut self,
    config: &ServerConfiguration,
    global_config: Option<&ServerConfiguration>,
    secondary_runtime: &tokio::runtime::Runtime,
  ) -> Result<Arc<dyn ObservabilityBackend + Send + Sync>, Box<dyn Error + Send + Sync>>;

  /// Determines configuration properties required to load an observability backend
  fn get_requirements(&self) -> Vec<&'static str> {
    vec![]
  }

  /// Validates the server configuration
  #[allow(unused_variables)]
  fn validate_configuration(
    &self,
    config: &ServerConfiguration,
    used_properties: &mut HashSet<String>,
  ) -> Result<(), Box<dyn Error + Send + Sync>> {
    Ok(())
  }
}

/// A trait that defines an observability backend
pub trait ObservabilityBackend {
  /// Obtains the channel for logging
  fn get_log_channel(&self) -> Option<Sender<LogMessage>> {
    None
  }

  /// Obtains the channel for metrics
  fn get_metric_channel(&self) -> Option<Sender<Metric>> {
    None
  }

  /// Obtains the channel for traces
  fn get_trace_channel(&self) -> Option<(Sender<()>, Receiver<Sender<TraceSignal>>)> {
    None
  }
}

/// Observability backend channels inside configurations
#[derive(Clone)]
pub struct ObservabilityBackendChannels {
  /// Log channels
  pub log_channels: Vec<Sender<LogMessage>>,
  /// Metric channels
  pub metric_channels: Vec<Sender<Metric>>,
  /// Trace channels
  pub trace_channels: Vec<(Sender<()>, Receiver<Sender<TraceSignal>>)>,
}

impl Default for ObservabilityBackendChannels {
  fn default() -> Self {
    Self::new()
  }
}

impl ObservabilityBackendChannels {
  /// Creates an empty instance of `ObservabilityBackendChannels`
  pub fn new() -> Self {
    Self {
      log_channels: Vec::new(),
      metric_channels: Vec::new(),
      trace_channels: Vec::new(),
    }
  }

  /// Adds a log channel to the observability backend channels
  pub fn add_log_channel(&mut self, channel: Sender<LogMessage>) {
    self.log_channels.push(channel);
  }

  /// Adds a metric channel to the observability backend channels
  pub fn add_metric_channel(&mut self, channel: Sender<Metric>) {
    self.metric_channels.push(channel);
  }

  /// Adds a trace channel to the observability backend channels
  pub fn add_trace_channel(&mut self, channel: (Sender<()>, Receiver<Sender<TraceSignal>>)) {
    self.trace_channels.push(channel);
  }
}

/// Represents a metric with its name, attributes, and value.
#[derive(Clone)]
pub struct Metric {
  /// Name of the metric
  pub name: &'static str,
  /// Attributes of the metric
  pub attributes: Vec<(&'static str, MetricAttributeValue)>,
  /// Type of the metric
  pub ty: MetricType,
  /// Value of the metric
  pub value: MetricValue,
  /// Optional unit of the metric
  pub unit: Option<&'static str>,
  /// Optional description of the metric
  pub description: Option<&'static str>,
}

impl Metric {
  /// Creates a new instance of `Metric`
  pub fn new(
    name: &'static str,
    attributes: Vec<(&'static str, MetricAttributeValue)>,
    ty: MetricType,
    value: MetricValue,
    unit: Option<&'static str>,
    description: Option<&'static str>,
  ) -> Self {
    Self {
      name,
      attributes,
      ty,
      value,
      unit,
      description,
    }
  }
}

/// Represents a type of metric.
#[derive(Clone, Debug, PartialEq)]
pub enum MetricType {
  /// Increasing counter
  Counter,

  /// Gauge
  Gauge,

  /// Increasing or decreasing counter
  UpDownCounter,

  /// Histogram with optional buckets
  Histogram(Option<Vec<f64>>),
}

/// Represents a value for a metric.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub enum MetricValue {
  F64(f64),
  U64(u64),
  I64(i64),
}

/// Represents an attribute value for a metric.
#[derive(Clone, Debug, PartialEq)]
pub enum MetricAttributeValue {
  /// String value
  String(String),

  /// Boolean value
  Bool(bool),

  /// Integer value
  I64(i64),

  /// Floating-point value
  F64(f64),
}

/// Facilitates logging of error messages through a provided sender.
pub struct MetricsMultiSender {
  senders: Vec<Sender<Metric>>,
}

impl MetricsMultiSender {
  /// Creates a new `MetricsMultiSender` instance.
  ///
  /// # Parameters
  ///
  /// - `sender`: A `Sender<Metric>` used for sending metric data.
  ///
  /// # Returns
  ///
  /// A new `MetricsMultiSender` instance associated with the provided sender.
  pub fn new(sender: Sender<Metric>) -> Self {
    Self { senders: vec![sender] }
  }

  /// Creates a new `MetricsMultiSender` instance with multiple senders.
  ///
  /// # Parameters
  ///
  /// - `senders`: A vector of `Sender<Metric>` used for sending metric data.
  ///
  /// # Returns
  ///
  /// A new `MetricsMultiSender` instance associated with multiple provided senders.
  pub fn new_multiple(senders: Vec<Sender<Metric>>) -> Self {
    Self { senders }
  }

  /// Creates a new `MetricsMultiSender` instance without any underlying sender.
  ///
  /// # Returns
  ///
  /// A new `MetricsMultiSender` instance not associated with any sender.
  pub fn without_sender() -> Self {
    Self { senders: vec![] }
  }

  /// Sends metric data asynchronously.
  ///
  /// # Parameters
  ///
  /// - `metric_data`: A `Metric` containing the metric data to be sent.
  ///
  pub async fn send(&self, metric_data: Metric) {
    for sender in &self.senders {
      sender.send(metric_data.clone()).await.unwrap_or_default();
    }
  }
}

impl Clone for MetricsMultiSender {
  /// Clone a `MetricsMultiSender`.
  ///
  /// # Returns
  ///
  /// A cloned `MetricsMultiSender` instance
  fn clone(&self) -> Self {
    Self {
      senders: self.senders.clone(),
    }
  }
}

/// Represents a trace signal with a Ferron module name and attributes.
#[derive(Clone)]
#[non_exhaustive]
pub enum TraceSignal {
  /// Start a new span with the given module name.
  StartSpan(String),
  /// End the span with the given module name and optional error description.
  EndSpan(String, Option<String>),
}
