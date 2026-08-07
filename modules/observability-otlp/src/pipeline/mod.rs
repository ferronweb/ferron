//! Batching pipelines: buffered, batched export of OTLP signals.
//!
//! Each pipeline owns a bounded buffer of finished items and a background
//! task that flushes the buffer on batch size or interval and drains it on
//! shutdown. Signals are wired in one pipeline per step (traces first).

pub mod traces;
