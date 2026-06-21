use std::collections::HashMap;

use typemap_rev::TypeMapKey;

use crate::HttpContext;

/// Represents a field for custom access logging.
pub enum CustomAccessLogField {
    String(String),
    U64(u64),
    F64(f64),
    Bool(bool),
}

impl From<String> for CustomAccessLogField {
    #[inline]
    fn from(value: String) -> Self {
        Self::String(value)
    }
}

impl From<u64> for CustomAccessLogField {
    #[inline]
    fn from(value: u64) -> Self {
        Self::U64(value)
    }
}

impl From<f64> for CustomAccessLogField {
    #[inline]
    fn from(value: f64) -> Self {
        Self::F64(value)
    }
}

impl From<bool> for CustomAccessLogField {
    #[inline]
    fn from(value: bool) -> Self {
        Self::Bool(value)
    }
}

/// A mutable reference to the custom access log fields in an [`HttpContext`].
pub struct CustomAccessLogFields;

impl TypeMapKey for CustomAccessLogFields {
    type Value = HashMap<String, CustomAccessLogField>;
}

/// Returns a mutable reference to the custom access log fields in an [`HttpContext`].
#[inline]
pub fn custom_access_log_fields(
    ctx: &mut HttpContext,
) -> &mut <CustomAccessLogFields as typemap_rev::TypeMapKey>::Value {
    ctx.extensions.entry::<CustomAccessLogFields>().or_default()
}
