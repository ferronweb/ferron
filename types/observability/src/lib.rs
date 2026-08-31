//! Observability event types and sinks for Ferron modules.
//!
//! This crate defines the common event model used by all observability
//! backends (console, file, OTLP, Prometheus, StatsD). Modules emit
//! events through a [`CompositeEventSink`], which dispatches them to
//! configured sinks.
//!
//! # Key types
//!
//! - [`Event`] — the top-level enum that wraps access, log, metric, and trace events.
//! - [`CompositeEventSink`] — the per-host event dispatcher that routes events to sinks.
//! - [`ObservabilityContext`] — passed to observability [`Provider`](ferron_core::providers::Provider) implementations.
//!
//! # For module authors
//!
//! Most modules interact with observability through the [`CompositeEventSink`]
//! stored in `HttpContext::events`. To emit
//! structured log or metric events, construct the appropriate event variant and
//! call `sink.emit(event)`.

mod access;
/// W3C Baggage header parsing and key promotion into telemetry attributes.
pub mod baggage;
mod config;
/// Control plane metadata and span link configuration.
pub mod control_plane;
mod event;
#[cfg(feature = "module")]
pub mod module;
mod provider;
/// Trace sampling configuration and evaluation.
pub mod sampler;
mod sink;
mod sink_builder;

pub use config::*;
pub use event::*;
pub use provider::*;
pub use sink::*;
pub use sink_builder::*;
