mod access_log;
mod cache;
mod context;
mod logs;
mod metrics;
mod traces;

pub(crate) use access_log::emit_access_log;
pub(crate) use cache::OtlpProviderCache;
pub(crate) use logs::emit_log;
pub(crate) use metrics::emit_metric;
pub(crate) use traces::emit_trace;

#[cfg(test)]
pub(crate) use {
    access_log::OtelAccessAttributeVisitor, context::CorrelationContext,
    metrics::sanitize_label_value,
};
