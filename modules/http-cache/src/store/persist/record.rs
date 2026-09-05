//! Binary record format for the persistent HTTP cache.
//!
//! Both the snapshot and the journal are sequences of framed records:
//!
//! ```text
//! u32 total_len      length in bytes of everything after this field
//! u32 crc32          CRC32 of `kind` + `payload`
//! u8  kind           0x01 = Put (full entry), 0x02 = Delete (key)
//! payload            record-specific body
//! ```
//!
//! A `Put` payload carries every `StoredEntry` field. `created_at: Instant`
//! is stored as a Unix epoch millisecond timestamp and re-derived on decode.
//! All integers are big-endian. All strings/byte blobs are length-prefixed.
//! Field values are stored as raw bytes (`http::HeaderValue` is not
//! necessarily UTF-8).
//!
//! Decoding is deliberately tolerant of a truncated final record (an
//! incomplete frame is reported as [`DecodeError::Eof`] and callers stop
//! there), but a checksum mismatch or implausible length field inside the
//! file is reported as corruption.

use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use bytes::Bytes;
use http::header::{HeaderMap, HeaderName, HeaderValue};
use http::StatusCode;

use crate::lscache::ScopedTag;
use crate::policy::CacheScope;
use crate::store::types::{StoredEntry, VaryRule};

pub const RECORD_PUT: u8 = 0x01;
pub const RECORD_DELETE: u8 = 0x02;

/// Hard sanity cap for a single record's total length. Legitimate records
/// are bounded by the configured `max_response_size` (default 2 MiB); this
/// cap only guards against corrupted length fields.
const MAX_RECORD_LEN: u32 = 512 * 1024 * 1024;

#[derive(Debug, PartialEq, Eq)]
pub enum DecodeError {
    /// An incomplete frame at the end of the data (truncated tail).
    Eof,
    /// The length field claims more bytes than the file contains or exceeds
    /// the [`MAX_RECORD_LEN`] sanity cap.
    BadLength,
    /// The recorded CRC32 does not match the payload.
    Crc,
    /// The payload structure is invalid (unknown kind, bad prefix, ...).
    Payload,
}

#[derive(Debug, Clone, PartialEq)]
pub enum DecodedRecord {
    Put {
        key: String,
        entry: Box<StoredEntry>,
    },
    Delete {
        key: String,
    },
}

/// Serialize a `Put` record for `entry` under cache key `key`.
pub fn encode_put(key: &str, entry: &StoredEntry) -> Vec<u8> {
    let mut payload = Vec::with_capacity(128 + estimate_entry_bytes(entry));
    encode_entry_payload(&mut payload, key, entry);
    frame(RECORD_PUT, &payload)
}

/// Serialize a `Delete` record for cache key `key`.
pub fn encode_delete(key: &str) -> Vec<u8> {
    let mut payload = Vec::with_capacity(2 + key.len());
    put_str(&mut payload, key);
    frame(RECORD_DELETE, &payload)
}

/// Approximate serialized size of a `Put` record for `entry`. Used to bound
/// the in-memory mutation queue rather than the true serialized length.
pub fn estimate_entry_bytes(entry: &StoredEntry) -> usize {
    let mut size = 2 + entry.base_key.len() + 256;
    for (name, value) in &*entry.headers {
        size += 1 + name.as_str().len() + 2 + value.len();
    }
    if let Some(body) = &entry.body {
        size += body.len();
    }
    for cookie in &*entry.lsc_cookies {
        size += cookie.len();
    }
    for tag in &entry.tags {
        size += tag.name.len();
    }
    size += entry.purge_url.len() + entry.purge_host.len();
    if let Some(private_key) = &entry.private_key {
        size += private_key.len();
    }
    if let Some(etag) = &entry.etag {
        size += etag.len();
    }
    if let Some(last_modified) = &entry.last_modified {
        size += last_modified.len();
    }
    for name in &entry.vary.header_names {
        size += name.as_str().len();
    }
    for cookie in &entry.vary.cookie_names {
        size += cookie.len();
    }
    if let Some(value) = &entry.vary.value {
        size += value.len();
    }
    size += 1;
    size
}

/// Decode the next record from `data` starting at `pos`.
///
/// Returns `Ok(None)` at a clean end of stream, `Ok(Some((record, next_pos)))`
/// after a full record, or an error that describes why decoding must stop.
pub fn decode_next(data: &[u8], pos: usize) -> Result<Option<(DecodedRecord, usize)>, DecodeError> {
    let remaining = data.len() - pos;
    // Shortest valid frame: 4 (len) + 4 (crc) + 1 (kind) + 2 (empty key).
    if remaining < 11 {
        return if remaining == 0 {
            Ok(None)
        } else {
            Err(DecodeError::Eof)
        };
    }

    let total_len = u32::from_be_bytes(
        data[pos..pos + 4]
            .try_into()
            .map_err(|_| DecodeError::Eof)?,
    ) as usize;
    // Field must cover at least crc(4) + kind(1) + key prefix(2).
    if total_len < 7 || total_len > MAX_RECORD_LEN as usize {
        return Err(DecodeError::BadLength);
    }
    let end = pos
        .checked_add(total_len + 4)
        .filter(|end| *end <= data.len())
        .ok_or(DecodeError::Eof)?;

    let crc_start = pos + 4;
    let payload_start = pos + 8;
    let expected_crc = u32::from_be_bytes(
        data[crc_start..payload_start]
            .try_into()
            .map_err(|_| DecodeError::Eof)?,
    );
    let kind = data[payload_start];
    let payload = &data[payload_start + 1..end];
    if crc32fast::hash(&data[payload_start..end]) != expected_crc {
        return Err(DecodeError::Crc);
    }

    let record = match kind {
        RECORD_PUT => {
            let mut dec = Decoder::new(payload);
            let key = dec.str()?;
            let entry = decode_entry(&mut dec)?;
            if !dec.is_done() {
                return Err(DecodeError::Payload);
            }
            DecodedRecord::Put {
                key,
                entry: Box::new(entry),
            }
        }
        RECORD_DELETE => {
            let mut dec = Decoder::new(payload);
            let key = dec.str()?;
            if !dec.is_done() {
                return Err(DecodeError::Payload);
            }
            DecodedRecord::Delete { key }
        }
        _ => return Err(DecodeError::Payload),
    };
    Ok(Some((record, end)))
}

/// Wrap `payload` in a framed record.
fn frame(kind: u8, payload: &[u8]) -> Vec<u8> {
    let total_len = 4 + 1 + payload.len();
    let mut buf = Vec::with_capacity(8 + payload.len());
    buf.extend_from_slice(&(total_len as u32).to_be_bytes());
    let crc = crc32fast::hash(&{
        let mut body = Vec::with_capacity(1 + payload.len());
        body.push(kind);
        body.extend_from_slice(payload);
        body
    });
    buf.extend_from_slice(&crc.to_be_bytes());
    buf.push(kind);
    buf.extend_from_slice(payload);
    buf
}

fn encode_entry_payload(out: &mut Vec<u8>, key: &str, entry: &StoredEntry) {
    put_str(out, key);
    put_str(out, &entry.base_key);
    out.push(match entry.scope {
        CacheScope::Public => 0,
        CacheScope::Private => 1,
    });
    put_u16(out, entry.status.as_u16());
    put_u16(out, entry.headers.len() as u16);
    for (name, value) in &*entry.headers {
        put_name(out, name);
        put_bytes(out, value.as_bytes());
    }
    match &entry.body {
        Some(body) => {
            put_u32(out, body.len() as u32);
            out.extend_from_slice(body);
        }
        None => put_u32(out, 0),
    }
    put_i64(out, created_at_epoch_ms(entry.created_at));
    put_u64(out, millis(entry.ttl));
    put_opt_duration(out, entry.stale_while_revalidate);
    put_opt_duration(out, entry.stale_if_error);
    out.push(entry.must_revalidate as u8);
    put_u64(out, entry.access_at);
    put_opt_str(out, entry.private_key.as_deref());
    put_u16(out, entry.tags.len() as u16);
    for tag in &entry.tags {
        out.push(match tag.scope {
            CacheScope::Public => 0,
            CacheScope::Private => 1,
        });
        put_str(out, &tag.name);
    }
    put_str(out, &entry.purge_url);
    put_str(out, &entry.purge_host);
    put_opt_bytes(out, entry.etag.as_ref().map(HeaderValue::as_bytes));
    put_opt_bytes(out, entry.last_modified.as_ref().map(HeaderValue::as_bytes));
    encode_vary(out, &entry.vary);
}

fn decode_entry(dec: &mut Decoder<'_>) -> Result<StoredEntry, DecodeError> {
    let base_key = dec.str()?;
    let scope = match dec.u8()? {
        0 => CacheScope::Public,
        1 => CacheScope::Private,
        _ => return Err(DecodeError::Payload),
    };
    let status = StatusCode::from_u16(dec.u16()?).map_err(|_| DecodeError::Payload)?;

    let mut headers = HeaderMap::new();
    let header_count = dec.u16()?;
    for _ in 0..header_count {
        let name = HeaderName::from_bytes(dec.raw_bytes(1)?).map_err(|_| DecodeError::Payload)?;
        let value = HeaderValue::from_bytes(dec.raw_bytes(2)?).map_err(|_| DecodeError::Payload)?;
        headers.append(name, value);
    }

    let body_len = dec.u32()? as usize;
    let body = if body_len == 0 {
        None
    } else {
        Some(Bytes::copy_from_slice(dec.raw(body_len)?))
    };

    let created_at = instant_from_epoch_ms(dec.i64()?);
    let ttl = Duration::from_millis(dec.u64()?);
    let stale_while_revalidate = take_opt_duration(dec);
    let stale_if_error = take_opt_duration(dec);
    let must_revalidate = dec.u8()? != 0;
    let access_at = dec.u64()?;
    let private_key = take_opt_str(dec)?;

    let tag_count = dec.u16()?;
    let mut tags = Vec::with_capacity(tag_count as usize);
    for _ in 0..tag_count {
        let scope = match dec.u8()? {
            0 => CacheScope::Public,
            1 => CacheScope::Private,
            _ => return Err(DecodeError::Payload),
        };
        tags.push(ScopedTag {
            scope,
            name: dec.str()?,
        });
    }

    let purge_url = dec.str()?;
    let purge_host = dec.str()?;
    let etag = take_opt_bytes(dec)?.and_then(|b| HeaderValue::from_bytes(b).ok());
    let last_modified = take_opt_bytes(dec)?.and_then(|b| HeaderValue::from_bytes(b).ok());
    let vary = decode_vary(dec)?;

    Ok(StoredEntry {
        scope,
        base_key,
        vary,
        status,
        headers: Arc::new(headers),
        body,
        lsc_cookies: Arc::new(Vec::new()),
        created_at,
        ttl,
        access_at,
        private_key,
        tags,
        purge_url,
        purge_host,
        etag,
        last_modified,
        stale_while_revalidate,
        stale_if_error,
        must_revalidate,
    })
}

/// `lsc_cookies` are never written to disk: they are session-scoped `Set-Cookie`
/// rehydration metadata and cannot survive a process restart meaningfully.
fn encode_vary(out: &mut Vec<u8>, vary: &VaryRule) {
    put_u8(out, vary.header_names.len() as u8);
    for name in &vary.header_names {
        put_name(out, name);
    }
    put_u16(out, vary.cookie_names.len() as u16);
    for cookie in &vary.cookie_names {
        put_str(out, cookie);
    }
    put_opt_str(out, vary.value.as_deref());
    out.push(vary.no_vary as u8);
}

fn decode_vary(dec: &mut Decoder<'_>) -> Result<VaryRule, DecodeError> {
    let mut header_names = Vec::new();
    let header_count = dec.u8()?;
    for _ in 0..header_count {
        header_names
            .push(HeaderName::from_bytes(dec.raw_bytes(1)?).map_err(|_| DecodeError::Payload)?);
    }
    let mut cookie_names = Vec::new();
    let cookie_count = dec.u16()?;
    for _ in 0..cookie_count {
        cookie_names.push(dec.str()?);
    }
    let value = take_opt_str(dec)?;
    // Records written before the `no-vary` flag existed end here. They used
    // the current default behavior (automatic vary cookies apply), so a
    // missing flag decodes as `false`.
    let no_vary = if dec.is_done() { false } else { dec.u8()? != 0 };
    Ok(VaryRule {
        header_names,
        cookie_names,
        value,
        no_vary,
    })
}

fn created_at_epoch_ms(created_at: Instant) -> i64 {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64;
    now.saturating_sub(created_at.elapsed().as_millis() as i64)
}

fn instant_from_epoch_ms(epoch_ms: i64) -> Instant {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64;
    let elapsed = now.saturating_sub(epoch_ms).max(0) as u64;
    Instant::now() - Duration::from_millis(elapsed)
}

fn millis(duration: Duration) -> u64 {
    duration.as_millis().min(u64::MAX as u128) as u64
}

fn put_opt_duration(out: &mut Vec<u8>, duration: Option<Duration>) {
    match duration {
        Some(duration) => {
            out.push(1);
            put_u64(out, millis(duration));
        }
        None => out.push(0),
    }
}

fn take_opt_duration(dec: &mut Decoder<'_>) -> Option<Duration> {
    match dec.u8().ok()? {
        0 => None,
        _ => Some(Duration::from_millis(dec.u64().ok()?)),
    }
}

fn put_opt_str(out: &mut Vec<u8>, value: Option<&str>) {
    match value {
        Some(value) => {
            out.push(1);
            put_str(out, value);
        }
        None => out.push(0),
    }
}

fn take_opt_str(dec: &mut Decoder<'_>) -> Result<Option<String>, DecodeError> {
    match dec.u8()? {
        0 => Ok(None),
        _ => Ok(Some(dec.str()?)),
    }
}

fn put_opt_bytes(out: &mut Vec<u8>, value: Option<&[u8]>) {
    match value {
        Some(value) => {
            out.push(1);
            put_bytes(out, value);
        }
        None => out.push(0),
    }
}

fn take_opt_bytes<'a>(dec: &mut Decoder<'a>) -> Result<Option<&'a [u8]>, DecodeError> {
    match dec.u8()? {
        0 => Ok(None),
        _ => Ok(Some(dec.raw_bytes(2)?)),
    }
}

fn put_u8(out: &mut Vec<u8>, value: u8) {
    out.push(value);
}

fn put_u16(out: &mut Vec<u8>, value: u16) {
    out.extend_from_slice(&value.to_be_bytes());
}

fn put_u32(out: &mut Vec<u8>, value: u32) {
    out.extend_from_slice(&value.to_be_bytes());
}

fn put_u64(out: &mut Vec<u8>, value: u64) {
    out.extend_from_slice(&value.to_be_bytes());
}

fn put_i64(out: &mut Vec<u8>, value: i64) {
    out.extend_from_slice(&value.to_be_bytes());
}

fn put_str(out: &mut Vec<u8>, value: &str) {
    debug_assert!(value.len() <= u16::MAX as usize);
    put_u16(out, value.len() as u16);
    out.extend_from_slice(value.as_bytes());
}

fn put_name(out: &mut Vec<u8>, name: &HeaderName) {
    debug_assert!(name.as_str().len() <= u8::MAX as usize);
    out.push(name.as_str().len() as u8);
    out.extend_from_slice(name.as_str().as_bytes());
}

fn put_bytes(out: &mut Vec<u8>, value: &[u8]) {
    debug_assert!(value.len() <= u16::MAX as usize);
    put_u16(out, value.len() as u16);
    out.extend_from_slice(value);
}

struct Decoder<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> Decoder<'a> {
    fn new(data: &'a [u8]) -> Self {
        Self { data, pos: 0 }
    }

    fn is_done(&self) -> bool {
        self.pos == self.data.len()
    }

    fn take(&mut self, len: usize) -> Result<&'a [u8], DecodeError> {
        let end = self
            .pos
            .checked_add(len)
            .filter(|end| *end <= self.data.len())
            .ok_or(DecodeError::Payload)?;
        let slice = &self.data[self.pos..end];
        self.pos = end;
        Ok(slice)
    }

    fn u8(&mut self) -> Result<u8, DecodeError> {
        Ok(self.take(1)?[0])
    }

    fn u16(&mut self) -> Result<u16, DecodeError> {
        Ok(u16::from_be_bytes(
            self.take(2)?.try_into().map_err(|_| DecodeError::Eof)?,
        ))
    }

    fn u32(&mut self) -> Result<u32, DecodeError> {
        Ok(u32::from_be_bytes(
            self.take(4)?.try_into().map_err(|_| DecodeError::Eof)?,
        ))
    }

    fn u64(&mut self) -> Result<u64, DecodeError> {
        Ok(u64::from_be_bytes(
            self.take(8)?.try_into().map_err(|_| DecodeError::Eof)?,
        ))
    }

    fn i64(&mut self) -> Result<i64, DecodeError> {
        Ok(i64::from_be_bytes(
            self.take(8)?.try_into().map_err(|_| DecodeError::Eof)?,
        ))
    }

    fn raw(&mut self, len: usize) -> Result<&'a [u8], DecodeError> {
        self.take(len)
    }

    fn raw_bytes(&mut self, prefix_len: u8) -> Result<&'a [u8], DecodeError> {
        let len = match prefix_len {
            1 => self.u8()? as usize,
            2 => self.u16()? as usize,
            _ => return Err(DecodeError::Payload),
        };
        self.take(len)
    }

    fn str(&mut self) -> Result<String, DecodeError> {
        let len = self.u16()? as usize;
        std::str::from_utf8(self.take(len)?)
            .map(str::to_string)
            .map_err(|_| DecodeError::Payload)
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use bytes::Bytes;
    use http::header::{HeaderName, HeaderValue, ACCEPT_ENCODING, CACHE_CONTROL};
    use http::HeaderMap;

    use crate::lscache::ScopedTag;
    use crate::policy::CacheScope;
    use crate::store::types::{StoredEntry, VaryRule};

    use super::{
        decode_next, encode_delete, encode_put, DecodeError, DecodedRecord, RECORD_DELETE,
        RECORD_PUT,
    };

    fn test_entry() -> StoredEntry {
        let mut headers = HeaderMap::new();
        headers.insert(
            CACHE_CONTROL,
            HeaderValue::from_static("public, max-age=60"),
        );
        headers.append(CACHE_CONTROL, HeaderValue::from_static("must-revalidate"));
        headers.insert(ACCEPT_ENCODING, HeaderValue::from_static("gzip, br"));
        // Non-UTF-8 header value: header values are opaque bytes.
        headers.append(
            HeaderName::from_static("x-binary"),
            HeaderValue::from_bytes(&[0xff, 0x80, b'a']).unwrap(),
        );

        let vary = VaryRule {
            header_names: vec![HeaderName::from_static("accept-language")],
            cookie_names: vec!["lang".to_string(), "bucket".to_string()],
            value: Some("rewritten".to_string()),
            no_vary: false,
        };

        StoredEntry {
            scope: CacheScope::Private,
            base_key: "https://example.com/page?q=1".to_string(),
            vary,
            status: http::StatusCode::OK,
            headers: std::sync::Arc::new(headers),
            body: Some(Bytes::from_static(b"response body bytes")),
            lsc_cookies: std::sync::Arc::new(vec![
                HeaderValue::from_static("lsc-cookie=1"),
                HeaderValue::from_static("lsc-cookie=2"),
            ]),
            created_at: std::time::Instant::now() - Duration::from_secs(42),
            ttl: Duration::from_secs(3600),
            access_at: 7,
            private_key: Some("user=alice".to_string()),
            tags: vec![
                ScopedTag {
                    scope: CacheScope::Public,
                    name: "blog".to_string(),
                },
                ScopedTag {
                    scope: CacheScope::Private,
                    name: "user=alice".to_string(),
                },
            ],
            purge_url: "/page?q=1".to_string(),
            purge_host: "example.com".to_string(),
            etag: Some(HeaderValue::from_static("\"abc\"")),
            last_modified: Some(HeaderValue::from_static("Wed, 21 Oct 2015 07:28:00 GMT")),
            stale_while_revalidate: Some(Duration::from_secs(300)),
            stale_if_error: Some(Duration::from_secs(600)),
            must_revalidate: true,
        }
    }

    #[test]
    fn put_roundtrip_preserves_all_fields() {
        let entry = test_entry();
        let bytes = encode_put("the-key", &entry);

        let mut pos = 0;
        let (record, next) = decode_next(&bytes, pos).unwrap().unwrap();
        pos = next;
        assert_eq!(pos, bytes.len());

        let DecodedRecord::Put {
            key,
            entry: decoded,
        } = record
        else {
            panic!("expected Put record");
        };
        assert_eq!(key, "the-key");
        assert_eq!(decoded.scope, entry.scope);
        assert_eq!(decoded.base_key, entry.base_key);
        assert_eq!(decoded.vary, entry.vary);
        assert_eq!(decoded.status, entry.status);
        assert_eq!(decoded.headers, entry.headers);
        assert_eq!(decoded.body, entry.body);
        // lsc_cookies are session-scoped and never serialized.
        assert!(decoded.lsc_cookies.is_empty());
        assert_eq!(decoded.ttl, entry.ttl);
        assert_eq!(decoded.access_at, entry.access_at);
        assert_eq!(decoded.private_key, entry.private_key);
        assert_eq!(decoded.tags, entry.tags);
        assert_eq!(decoded.purge_url, entry.purge_url);
        assert_eq!(decoded.purge_host, entry.purge_host);
        assert_eq!(decoded.etag, entry.etag);
        assert_eq!(decoded.last_modified, entry.last_modified);
        assert_eq!(decoded.stale_while_revalidate, entry.stale_while_revalidate);
        assert_eq!(decoded.stale_if_error, entry.stale_if_error);
        assert_eq!(decoded.must_revalidate, entry.must_revalidate);

        // created_at survives as a wall-clock age, not an exact Instant.
        let a = entry.created_at.elapsed();
        let b = decoded.created_at.elapsed();
        assert!(
            a.abs_diff(b) < Duration::from_secs(1),
            "age drifted: {a:?} vs {b:?}"
        );
    }

    #[test]
    fn put_roundtrip_with_absent_optionals() {
        let mut entry = test_entry();
        entry.body = None;
        entry.tags = Vec::new();
        entry.private_key = None;
        entry.stale_while_revalidate = None;
        entry.stale_if_error = None;
        entry.etag = None;
        entry.last_modified = None;
        entry.must_revalidate = false;
        entry.vary = VaryRule::default();

        let bytes = encode_put("k", &entry);
        let (record, _) = decode_next(&bytes, 0).unwrap().unwrap();
        let DecodedRecord::Put { entry: decoded, .. } = record else {
            panic!("expected Put record");
        };
        assert_eq!(decoded.body, None);
        assert!(decoded.tags.is_empty());
        assert_eq!(decoded.private_key, None);
        assert_eq!(decoded.stale_while_revalidate, None);
        assert_eq!(decoded.stale_if_error, None);
        assert_eq!(decoded.etag, None);
        assert_eq!(decoded.last_modified, None);
        assert!(!decoded.must_revalidate);
        assert_eq!(decoded.vary, VaryRule::default());
    }

    #[test]
    fn put_roundtrip_preserves_no_vary() {
        let mut entry = test_entry();
        entry.vary.no_vary = true;
        let bytes = encode_put("k", &entry);
        let (record, _) = decode_next(&bytes, 0).unwrap().unwrap();
        let DecodedRecord::Put { entry: decoded, .. } = record else {
            panic!("expected Put record");
        };
        assert!(decoded.vary.no_vary);
        assert_eq!(decoded.vary, entry.vary);
    }

    #[test]
    fn put_without_no_vary_flag_decodes_as_false() {
        // Records written before the `no-vary` flag existed end right after
        // the vary value. They must decode as `no_vary: false` (automatic
        // vary cookies apply) rather than failing.
        let entry = test_entry();
        let mut bytes = encode_put("k", &entry);
        bytes.truncate(bytes.len() - 1);
        // Fix the frame length and CRC for the truncated payload.
        let total_len = (bytes.len() - 4) as u32;
        bytes[0..4].copy_from_slice(&total_len.to_be_bytes());
        let crc = crc32fast::hash(&bytes[8..]);
        bytes[4..8].copy_from_slice(&crc.to_be_bytes());

        let (record, _) = decode_next(&bytes, 0).unwrap().unwrap();
        let DecodedRecord::Put { entry: decoded, .. } = record else {
            panic!("expected Put record");
        };
        assert!(!decoded.vary.no_vary);
        assert_eq!(decoded.vary.cookie_names, entry.vary.cookie_names);
    }

    #[test]
    fn delete_roundtrip() {
        let bytes = encode_delete("the-key");
        let (record, pos) = decode_next(&bytes, 0).unwrap().unwrap();
        assert_eq!(pos, bytes.len());
        assert_eq!(
            record,
            DecodedRecord::Delete {
                key: "the-key".into()
            }
        );
    }

    #[test]
    fn back_to_back_records_decode_in_order() {
        let mut blob = Vec::new();
        blob.extend_from_slice(&encode_put("k1", &test_entry()));
        blob.extend_from_slice(&encode_delete("k2"));
        blob.extend_from_slice(&encode_put("k3", &test_entry()));

        let mut pos = 0;
        let mut kinds = Vec::new();
        let mut keys = Vec::new();
        while let Some((record, next)) = decode_next(&blob, pos).unwrap() {
            match record {
                DecodedRecord::Put { key, .. } => {
                    kinds.push(RECORD_PUT);
                    keys.push(key);
                }
                DecodedRecord::Delete { key } => {
                    kinds.push(RECORD_DELETE);
                    keys.push(key);
                }
            }
            pos = next;
        }
        assert_eq!(kinds, vec![RECORD_PUT, RECORD_DELETE, RECORD_PUT]);
        assert_eq!(keys, vec!["k1", "k2", "k3"]);
        assert_eq!(pos, blob.len());
    }

    #[test]
    fn truncated_tail_is_reported_not_fatal() {
        let mut blob = encode_put("k1", &test_entry());
        blob.extend_from_slice(&encode_put("k2", &test_entry()));
        // Cut into the middle of the second record.
        blob.truncate(blob.len() - 5);

        let mut pos = 0;
        let (record, next) = decode_next(&blob, pos).unwrap().unwrap();
        match record {
            DecodedRecord::Put { key, .. } => assert_eq!(key, "k1"),
            _ => panic!("expected Put"),
        }
        pos = next;
        assert_eq!(decode_next(&blob, pos), Err(DecodeError::Eof));
    }

    #[test]
    fn truncated_header_is_eof() {
        let mut blob = encode_put("k1", &test_entry());
        blob.extend_from_slice(&[0u8; 7]); // length field only
        let (_, next) = decode_next(&blob, 0).unwrap().unwrap();
        assert_eq!(decode_next(&blob, next), Err(DecodeError::Eof));
    }

    #[test]
    fn crc_corruption_is_detected() {
        let mut blob = encode_put("k1", &test_entry());
        let last = blob.len() - 1;
        blob[last] ^= 0xff;
        assert_eq!(decode_next(&blob, 0), Err(DecodeError::Crc));
    }

    #[test]
    fn bad_kind_is_payload_error() {
        let mut blob = encode_put("k1", &test_entry());
        // Corrupt the kind byte and fix the CRC so the failure is structural.
        blob[8] ^= 0xff;
        let crc = crc32fast::hash(&blob[8..]);
        blob[4..8].copy_from_slice(&crc.to_be_bytes());
        assert_eq!(decode_next(&blob, 0), Err(DecodeError::Payload));
    }

    #[test]
    fn implausible_length_is_bad_length() {
        let mut blob = encode_put("k1", &test_entry());
        blob[0..4].copy_from_slice(&u32::MAX.to_be_bytes());
        assert_eq!(decode_next(&blob, 0), Err(DecodeError::BadLength));
    }

    #[test]
    fn trailing_garbage_after_records_is_reported() {
        let mut blob = encode_put("k1", &test_entry());
        blob.extend_from_slice(&[0xaa, 0xbb, 0xcc, 0xdd]); // >= minimal header
        let (_, pos) = decode_next(&blob, 0).unwrap().unwrap();
        assert!(matches!(
            decode_next(&blob, pos),
            Err(DecodeError::Eof) | Err(DecodeError::Crc) | Err(DecodeError::BadLength)
        ));
    }

    #[test]
    fn empty_input_is_clean_end() {
        assert_eq!(decode_next(&[], 0), Ok(None));
    }
}
