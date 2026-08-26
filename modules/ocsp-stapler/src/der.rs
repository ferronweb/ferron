//! Minimal DER parsing helpers used to extract signed bytes from an OCSP response.

use anyhow::{bail, Result};

/// Extract the exact DER bytes of `tbsResponseData` (first element of
/// BasicOcspResponse SEQUENCE) from the raw BasicOcspResponse DER.
///
/// ```text
/// BasicOcspResponse ::= SEQUENCE {
///   tbsResponseData    ResponseData,
///   signatureAlgorithm AlgorithmIdentifier,
///   signature          BIT STRING,
///   certs          [0] EXPLICIT SEQUENCE OF Certificate OPTIONAL }
/// ```
///
/// The signature is computed over the DER encoding of `tbsResponseData` as it
/// appears on the wire, so we must slice the original bytes rather than
/// re-encoding via `rasn`.
pub(crate) fn extract_tbs_bytes(basic_der: &[u8]) -> Result<Vec<u8>> {
    // Parse outer SEQUENCE header at offset 0.
    let (tag, _len, header_len, _, _) = parse_tlv(basic_der, 0)?;
    if tag != 0x30 {
        bail!("expected outer SEQUENCE (0x30), got {:#x}", tag);
    }
    // First child is tbsResponseData at `header_len`.
    let (tbs_tag, _tbs_len, _tbs_header_len, _, tbs_end) = parse_tlv(basic_der, header_len)?;
    if tbs_tag != 0x30 {
        bail!(
            "expected tbsResponseData SEQUENCE (0x30), got {:#x}",
            tbs_tag
        );
    }
    // Include tag+length+value.
    Ok(basic_der[header_len..tbs_end].to_vec())
}

/// Minimal DER TLV parser returning
/// `(tag, length, header_len, content_start, content_end)`.
pub(crate) fn parse_tlv(data: &[u8], offset: usize) -> Result<(u8, usize, usize, usize, usize)> {
    if offset + 2 > data.len() {
        bail!("TLV out of bounds at offset {}", offset);
    }
    let tag = data[offset];
    let len_byte = data[offset + 1];
    let (length, header_len) = if len_byte & 0x80 == 0 {
        (len_byte as usize, 2)
    } else {
        let num_bytes = (len_byte & 0x7F) as usize;
        if num_bytes == 0 || num_bytes > 4 {
            bail!("unsupported DER length at offset {}", offset);
        }
        if offset + 2 + num_bytes > data.len() {
            bail!("TLV length bytes out of bounds");
        }
        let mut len = 0usize;
        for b in &data[offset + 2..offset + 2 + num_bytes] {
            len = (len << 8) | (*b as usize);
        }
        (len, 2 + num_bytes)
    };
    let content_start = offset + header_len;
    let content_end = content_start + length;
    if content_end > data.len() {
        bail!(
            "TLV content out of bounds: offset {} len {} data len {}",
            offset,
            length,
            data.len()
        );
    }
    Ok((tag, length, header_len, content_start, content_end))
}
