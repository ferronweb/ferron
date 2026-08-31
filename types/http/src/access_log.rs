//! Custom access log field storage.
//!
//! Modules can attach typed custom fields to an [`HttpContext`] that
//! access log formatters retrieve when building log lines. Fields are
//! stored in the extensions type map via [`CustomAccessLogFields`].

use rustc_hash::FxHashMap;
use typemap_rev::TypeMapKey;

use crate::HttpContext;

/// A typed value for a custom access log field.
pub enum CustomAccessLogField {
    /// A string value.
    String(String),
    /// An unsigned integer value.
    U64(u64),
    /// A floating-point value.
    F64(f64),
    /// A boolean value.
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

/// TypeMap key for custom access log fields in [`HttpContext::extensions`](crate::HttpContext::extensions).
pub struct CustomAccessLogFields;

impl TypeMapKey for CustomAccessLogFields {
    type Value = FxHashMap<String, CustomAccessLogField>;
}

/// Returns a mutable reference to the custom access log fields in an [`HttpContext`].
#[inline]
pub fn custom_access_log_fields(
    ctx: &mut HttpContext,
) -> &mut <CustomAccessLogFields as typemap_rev::TypeMapKey>::Value {
    ctx.extensions.entry::<CustomAccessLogFields>().or_default()
}
