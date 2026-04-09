//! Directory listing generation stage

use std::io;
use std::path::PathBuf;
use std::time::SystemTime;

use async_trait::async_trait;
use bytes::Bytes;
use chrono::{DateTime, Local};
use ferron_core::pipeline::{PipelineError, Stage};
use ferron_core::StageConstraint;
use ferron_http::format_page;
use ferron_http::util::anti_xss::anti_xss;
use ferron_http::{HttpFileContext, HttpResponse};
use http::{header, HeaderValue};
use http::{Method, Response, StatusCode};
use http_body_util::{BodyExt, Empty, Full};

pub struct DirectoryListingStage;

struct DirectoryListingEntry {
    filename: String,
    extension: Option<String>,
    metadata: Option<DirectoryListingMetadata>,
}

struct DirectoryListingMetadata {
    is_dir: bool,
    size: Option<u64>,
    modified: Option<SystemTime>,
}

impl Default for DirectoryListingStage {
    #[inline]
    fn default() -> Self {
        Self
    }
}

#[async_trait(?Send)]
impl Stage<HttpFileContext> for DirectoryListingStage {
    #[inline]
    fn name(&self) -> &str {
        "directory_listing"
    }

    #[inline]
    fn constraints(&self) -> Vec<StageConstraint> {
        vec![StageConstraint::Before("static_file".to_string())]
    }

    #[inline]
    async fn run(&self, ctx: &mut HttpFileContext) -> Result<bool, PipelineError> {
        // Skip if root is not configured
        if ctx.http.configuration.get_value("root", true).is_none() {
            return Ok(true);
        }

        let Some(request) = ctx.http.req.take() else {
            return Ok(true);
        };

        // Only handle directories
        if ctx.path_info.is_some() || !ctx.metadata.is_dir() {
            ctx.http.req = Some(request);
            return Ok(true);
        }

        let method = request.method().clone();

        // Handle OPTIONS
        if method == Method::OPTIONS {
            let res = Response::builder()
                .status(StatusCode::NO_CONTENT)
                .header(header::ALLOW, "GET, HEAD, POST, OPTIONS")
                .body(Empty::new().map_err(|_| unreachable!()).boxed_unsync())
                .expect("failed to build OPTIONS response");
            ctx.http.req = Some(request);
            ctx.http.res = Some(HttpResponse::Custom(res));
            return Ok(false);
        }

        // Only handle GET and HEAD
        if method != Method::GET && method != Method::HEAD && method != Method::POST {
            let mut allow_headers = http::HeaderMap::new();
            allow_headers.insert(
                header::ALLOW,
                HeaderValue::from_static("GET, HEAD, POST, OPTIONS"),
            );
            ctx.http.req = Some(request);
            ctx.http.res = Some(HttpResponse::BuiltinError(405, Some(allow_headers)));
            return Ok(false);
        }

        // Check if directory listing is enabled
        if !ctx.http.configuration.get_flag("directory_listing", true) {
            ctx.http.req = Some(request);
            ctx.http.res = Some(HttpResponse::BuiltinError(403, None));
            return Ok(false);
        }

        let method = request.method().clone();

        // Read directory contents and metadata in one blocking pass.
        let entries = vibeio::spawn_blocking({
            let dir_path = ctx.file_path.clone();
            move || read_directory_entries(dir_path)
        })
        .await
        .map_err(|e| PipelineError::custom(format!("failed to spawn blocking task: {e}")))
        .and_then(|r| r.map_err(|e| PipelineError::custom(e.to_string())))?;

        // Read .maindesc if present
        let maindesc_path = ctx.file_path.join(".maindesc");
        let description = vibeio::fs::read_to_string(&maindesc_path).await.ok();

        // Get original request path for links
        let request_path = (ctx.http.original_uri.as_ref().unwrap_or(request.uri())).path();

        let html = generate_directory_listing(entries, request_path, description);

        let content_length = html.len() as u64;
        let body_bytes = Bytes::from(html);

        let builder = Response::builder()
            .status(StatusCode::OK)
            .header(header::CONTENT_TYPE, "text/html")
            .header(header::CONTENT_LENGTH, content_length);

        let response = if method == Method::HEAD {
            builder
                .body(Empty::new().map_err(|_| unreachable!()).boxed_unsync())
                .expect("failed to build HEAD response")
        } else {
            builder
                .body(
                    Full::new(body_bytes)
                        .map_err(|_| unreachable!())
                        .boxed_unsync(),
                )
                .expect("failed to build directory listing response")
        };

        ctx.http.req = Some(request);
        ctx.http.res = Some(HttpResponse::Custom(response));
        Ok(false)
    }
}

/// File type emoji icon based on extension
fn file_icon(ext: Option<&str>, is_dir: bool) -> &'static str {
    if is_dir {
        return "📁";
    }
    match ext {
        // Images
        Some("png") | Some("jpg") | Some("jpeg") | Some("gif") | Some("webp") | Some("svg")
        | Some("ico") | Some("bmp") | Some("tiff") | Some("tif") => "\u{1f5bc}\u{fe0f}",
        // Audio
        Some("mp3") | Some("wav") | Some("ogg") | Some("flac") | Some("aac") => "🎵",
        // Video
        Some("mp4") | Some("mkv") | Some("avi") | Some("mov") | Some("wmv") | Some("flv")
        | Some("webm") => "🎥",
        // Documents
        Some("pdf") | Some("doc") | Some("docx") | Some("xls") | Some("xlsx") | Some("ppt")
        | Some("pptx") | Some("txt") => "🧾",
        // Packages/archives
        Some("zip") | Some("rar") | Some("tar") | Some("gz") | Some("xz") | Some("bz2")
        | Some("7z") | Some("iso") | Some("msi") | Some("deb") | Some("rpm") => "📦",
        // Binaries
        Some("exe") | Some("dll") | Some("jar") | Some("cgi") | Some("com") => "⚙️",
        // Scripts
        Some("js") | Some("ts") | Some("py") | Some("php") | Some("pl") | Some("sh")
        | Some("rb") | Some("bat") | Some("ps1") => "📜",
        // Fonts
        Some("ttf") | Some("otf") | Some("woff") | Some("woff2") => "🅰️",
        // CSS
        Some("css") => "🎨",
        // Default
        _ => "📄",
    }
}

/// Generate an HTML directory listing
fn generate_directory_listing(
    entries: Vec<DirectoryListingEntry>,
    request_path: &str,
    description: Option<String>,
) -> String {
    // Strip trailing slashes
    let mut path_without_slashes = request_path;
    while path_without_slashes.ends_with('/') {
        path_without_slashes = &path_without_slashes[..path_without_slashes.len() - 1];
    }

    // Build return path
    let mut path_parts: Vec<&str> = path_without_slashes.split('/').collect();
    path_parts.pop();
    path_parts.push("");
    let return_path = path_parts.join("/");

    let mut rows = Vec::new();
    if !path_without_slashes.is_empty() {
        rows.push(format!(
            "<tr><td>⬆️ <a href=\"{}\">Return</a></td><td></td><td></td></tr>",
            anti_xss(&return_path)
        ));
    }
    let min_rows = rows.len();

    for entry in entries {
        let filename = entry.filename;
        let Some(metadata) = entry.metadata else {
            let link = format!(
                "⚠️ <a href=\"{}/{}\">{}</a>",
                path_without_slashes,
                anti_xss(urlencoding::encode(&filename).as_ref()),
                anti_xss(&filename)
            );
            rows.push(format!(
                "<tr><td class=\"directory-filename\">{link}</td>\
                 <td class=\"directory-size\">-</td><td class=\"directory-date\">-</td></tr>"
            ));
            continue;
        };

        let is_dir = metadata.is_dir;
        let icon = file_icon(entry.extension.as_deref(), is_dir);
        let suffix = if is_dir { "/" } else { "" };

        let link = format!(
            "{} <a href=\"{}/{}{}\">{}</a>",
            icon,
            path_without_slashes,
            anti_xss(urlencoding::encode(&filename).as_ref()),
            suffix,
            anti_xss(&filename)
        );

        let size = if let Some(size) = metadata.size {
            anti_xss(&sizify(size, false))
        } else {
            "-".to_string()
        };

        let date = if let Some(mtime) = metadata.modified {
            let dt: DateTime<Local> = mtime.into();
            anti_xss(&dt.format("%a %b %d %Y").to_string())
        } else {
            "-".to_string()
        };

        rows.push(format!(
            "<tr><td class=\"directory-filename\">{link}</td>\
             <td class=\"directory-size\">{size}</td><td class=\"directory-date\">{date}</td></tr>"
        ));
    }

    if rows.len() <= min_rows {
        rows.push(
            "<tr><td class=\"directory-filename\">🤷 No files found</td>\
             <td class=\"directory-size\"></td><td class=\"directory-date\"></td></tr>"
                .to_string(),
        );
    }

    let body = format!(
        "<h1>Directory: {}</h1>\n<table>\n\
         <tr><th class=\"directory-filename\">Filename</th>\
         <th class=\"directory-size\">Size</th>\
         <th class=\"directory-date\">Date</th></tr>\n\
         {}\n</table>{}",
        anti_xss(request_path),
        rows.join("\n"),
        description
            .map(|d| format!(
                "<hr><pre class=\"directory-description\">{}</pre>",
                anti_xss(&d)
            ))
            .unwrap_or_default()
    );

    let css_common = ferron_http::util::CSS_COMMON;
    let css_directory = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/assets/directory.css"));

    format_page!(
        body,
        &format!("Directory: {request_path}"),
        vec![css_common, css_directory]
    )
}

fn read_directory_entries(dir_path: PathBuf) -> io::Result<Vec<DirectoryListingEntry>> {
    let mut entries = Vec::new();
    for entry in std::fs::read_dir(dir_path)? {
        let Ok(entry) = entry else {
            continue;
        };
        let filename = entry.file_name().to_string_lossy().to_string();
        if filename.starts_with('.') {
            continue;
        }

        let path = entry.path();
        let extension = path
            .extension()
            .and_then(|ext| ext.to_str())
            .map(str::to_owned);
        let metadata = entry
            .metadata()
            .ok()
            .map(|metadata| DirectoryListingMetadata {
                is_dir: metadata.is_dir(),
                size: metadata.is_file().then_some(metadata.len()),
                modified: metadata.modified().ok(),
            });

        entries.push(DirectoryListingEntry {
            filename,
            extension,
            metadata,
        });
    }

    entries.sort_by(|left, right| left.filename.cmp(&right.filename));
    Ok(entries)
}

// Sizify function taken from SVR.JS and rewritten from JavaScript to Rust
// SVR.JS is licensed under MIT, so below is the copyright notice:
//
// Copyright (c) 2018-2025 SVR.JS
// Portions of this file are derived from SVR.JS (https://git.svrjs.org/svrjs/svrjs).
//
// Permission is hereby granted, free of charge, to any person obtaining a copy
// of this software and associated documentation files (the "Software"), to deal
// in the Software without restriction, including without limitation the rights
// to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
// copies of the Software, and to permit persons to whom the Software is
// furnished to do so, subject to the following conditions:
//
// The above copyright notice and this permission notice shall be included in all
// copies or substantial portions of the Software.
//

/// Converts the file size into a human-readable one
pub fn sizify(bytes: u64, add_i: bool) -> String {
    if bytes == 0 {
        return "0".to_string();
    }

    let prefixes = ["", "K", "M", "G", "T", "P", "E", "Z", "Y", "R", "Q"];
    let prefix_index = (bytes.ilog2() as usize / 10).min(prefixes.len() - 1);
    let prefix_index_translated = 2_u64.pow(10 * prefix_index as u32);
    let decimal_points = (2
        - (bytes / prefix_index_translated)
            .checked_ilog10()
            .unwrap_or(0) as i32)
        .max(0);

    let size = ((bytes as f64 / prefix_index_translated as f64) * 10_f64.powi(decimal_points))
        .ceil()
        / 10_f64.powi(decimal_points);

    let mut result = String::new();
    result.push_str(&size.to_string()); // Size
    result.push_str(prefixes[prefix_index]); // Prefix
    if prefix_index > 0 && add_i {
        result.push('i'); // "i" suffix
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sizify_zero_bytes() {
        assert_eq!(sizify(0, false), "0");
    }

    #[test]
    fn test_sizify_small_values() {
        assert_eq!(sizify(1000, false), "1000");
        assert_eq!(sizify(1024, false), "1K");
    }

    #[test]
    fn test_sizify_larger_values() {
        assert_eq!(sizify(1048576, false), "1M");
        assert_eq!(sizify(1073741824, false), "1G");
        assert_eq!(sizify(1099511627776, false), "1T");
        assert_eq!(sizify(1125899906842624, false), "1P");
        assert_eq!(sizify(1152921504606846976, false), "1E");
    }

    #[test]
    fn test_sizify_add_i_suffix() {
        assert_eq!(sizify(1024, true), "1Ki");
        assert_eq!(sizify(1048576, true), "1Mi");
        assert_eq!(sizify(1073741824, true), "1Gi");
    }

    #[test]
    fn test_sizify_no_i_suffix() {
        assert_eq!(sizify(1024, false), "1K");
        assert_eq!(sizify(1048576, false), "1M");
        assert_eq!(sizify(1073741824, false), "1G");
    }

    #[test]
    fn test_sizify_decimal_points() {
        assert_eq!(sizify(1500, false), "1.47K");
        assert_eq!(sizify(1500000, false), "1.44M");
        assert_eq!(sizify(1500000000, false), "1.4G");
    }

    #[test]
    fn test_sizify_edge_cases() {
        assert_eq!(sizify(1, false), "1");
        assert_eq!(sizify(1023, false), "1023");
        assert_eq!(sizify(1025, false), "1.01K");
    }
}
