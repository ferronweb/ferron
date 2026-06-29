use ferron_observability::TraceAttributeValue;
use rustc_hash::FxHashMap;

use crate::{HttpContext, HttpFileContext};

struct SpanAttributesKey;

impl typemap_rev::TypeMapKey for SpanAttributesKey {
    type Value = FxHashMap<&'static str, TraceAttributeValue>;
}

pub trait HttpContextSpanExt {
    fn get_span_attributes(&mut self) -> &mut FxHashMap<&'static str, TraceAttributeValue>;
    fn remove_span_attributes(&mut self) -> Option<FxHashMap<&'static str, TraceAttributeValue>>;
}

impl HttpContextSpanExt for HttpContext {
    #[inline]
    fn get_span_attributes(&mut self) -> &mut FxHashMap<&'static str, TraceAttributeValue> {
        self.extensions.entry::<SpanAttributesKey>().or_default()
    }

    #[inline]
    fn remove_span_attributes(&mut self) -> Option<FxHashMap<&'static str, TraceAttributeValue>> {
        self.extensions.remove::<SpanAttributesKey>()
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
    fn remove_span_attributes(&mut self) -> Option<FxHashMap<&'static str, TraceAttributeValue>> {
        self.http.extensions.remove::<SpanAttributesKey>()
    }
}
