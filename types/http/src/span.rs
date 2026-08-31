//! Per-request span attribute management.
//!
//! This module provides the [`HttpContextSpanExt`] trait for attaching
//! custom attributes to the current trace span. Attributes are collected
//! during request processing and included when the span is emitted.

use ferron_observability::TraceAttributeValue;
use rustc_hash::FxHashMap;

use crate::{HttpContext, HttpFileContext};

struct SpanAttributesKey;

impl typemap_rev::TypeMapKey for SpanAttributesKey {
    type Value = FxHashMap<&'static str, TraceAttributeValue>;
}

/// Extension trait for managing span attributes on HTTP contexts.
///
/// Modules use this trait to attach custom attributes to the current
/// trace span. Attributes are collected during request processing and
/// included when the span is emitted.
pub trait HttpContextSpanExt {
    /// Get or create the span attributes map for this request.
    ///
    /// Returns a mutable reference to the map of span attributes. Callers
    /// can insert attributes that will be included when the trace span is emitted.
    fn get_span_attributes(&mut self) -> &mut FxHashMap<&'static str, TraceAttributeValue>;
    /// Drain all span attributes from this request and return them as a vector.
    ///
    /// This is typically called once when building the `StartSpan` event to
    /// transfer attributes from the context to the trace event.
    fn remove_span_attributes(&mut self) -> Vec<(&'static str, TraceAttributeValue)>;
}

impl HttpContextSpanExt for HttpContext {
    #[inline]
    fn get_span_attributes(&mut self) -> &mut FxHashMap<&'static str, TraceAttributeValue> {
        self.extensions.entry::<SpanAttributesKey>().or_default()
    }

    #[inline]
    fn remove_span_attributes(&mut self) -> Vec<(&'static str, TraceAttributeValue)> {
        self.extensions
            .get_mut::<SpanAttributesKey>()
            .map(|map| map.drain().collect())
            .unwrap_or_default()
    }
}

impl HttpContextSpanExt for HttpFileContext {
    #[inline]
    fn get_span_attributes(&mut self) -> &mut FxHashMap<&'static str, TraceAttributeValue> {
        self.http
            .extensions
            .entry::<SpanAttributesKey>()
            .or_default()
    }

    #[inline]
    fn remove_span_attributes(&mut self) -> Vec<(&'static str, TraceAttributeValue)> {
        self.http
            .extensions
            .get_mut::<SpanAttributesKey>()
            .map(|map| map.drain().collect())
            .unwrap_or_default()
    }
}
