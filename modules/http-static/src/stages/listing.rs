//! Directory listing generation stage

use std::io;

use async_trait::async_trait;
use bytes::Bytes;
use chrono::{DateTime, Local};
use ferron_core::pipeline::{PipelineError, Stage};
use ferron_core::StageConstraint;
use ferron_http::format_page;
use ferron_http::util::anti_xss::anti_xss;
use ferron_http::{HttpFileContext, HttpResponse};
use http::header;
use http::{Method, Response, StatusCode};
use http_body_util::{BodyExt, Empty, Full};

pub struct DirectoryListingStage;

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

        // Check if directory listing is enabled
        if !ctx.http.configuration.get_flag("directory_listing", true) {
            ctx.http.req = Some(request);
            ctx.http.res = Some(HttpResponse::BuiltinError(403, None));
            return Ok(false);
        }

        let method = request.method().clone();

        // Read directory contents
        let read_dir = vibeio::spawn_blocking({
            let dir_path = ctx.file_path.clone();
            move || std::fs::read_dir(&dir_path)
        })
        .await
        .map_err(|e| PipelineError::custom(format!("failed to spawn blocking task: {e}")))
        .and_then(|r| r.map_err(|e| PipelineError::custom(e.to_string())))?;

        // Read .maindesc if present
        let maindesc_path = ctx.file_path.join(".maindesc");
        let description = vibeio::fs::read_to_string(&maindesc_path).await.ok();

        // Get original request path for links
        let request_path = request.uri().path();

        let html = generate_directory_listing(read_dir, request_path, description)
            .await
            .map_err(|e| PipelineError::custom(e.to_string()))?;

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

/// Human-readable file size
fn sizify(bytes: u64, _iec: bool) -> String {
    if bytes == 0 {
        return "0 B".to_string();
    }
    let units = ["B", "KiB", "MiB", "GiB", "TiB"];
    let base = 1024.0;
    let exponent = (bytes as f64).log(base).floor() as usize;
    let exponent = exponent.min(units.len() - 1);
    let value = bytes as f64 / base.powi(exponent as i32);
    if exponent == 0 {
        format!("{value:.0} {}", units[exponent])
    } else {
        format!("{value:.1} {}", units[exponent])
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
async fn generate_directory_listing(
    directory: std::fs::ReadDir,
    request_path: &str,
    description: Option<String>,
) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
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

    // Collect and sort entries by filename
    let mut entries: Vec<_> = directory.filter_map(|e| e.ok()).collect();
    entries.sort_by_cached_key(|e| e.file_name().to_string_lossy().to_string());

    for entry in &entries {
        let filename = entry.file_name().to_string_lossy().to_string();
        if filename.starts_with('.') {
            continue;
        }

        let entry_path = entry.path();
        let entry_path_clone = entry_path.clone();
        let metadata = match vibeio::spawn_blocking(move || std::fs::metadata(&entry_path_clone))
            .await
            .map_err(|e| -> io::Error { io::Error::other(format!("spawn blocking failed: {e}")) })
            .and_then(|r| r)
        {
            Ok(m) => m,
            Err(_) => {
                // Can't read metadata, show warning
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
            }
        };

        let is_dir = metadata.is_dir();
        let icon = file_icon(entry_path.extension().and_then(|e| e.to_str()), is_dir);
        let suffix = if is_dir { "/" } else { "" };

        let link = format!(
            "{} <a href=\"{}/{}{}\">{}</a>",
            icon,
            path_without_slashes,
            anti_xss(urlencoding::encode(&filename).as_ref()),
            suffix,
            anti_xss(&filename)
        );

        let size = if metadata.is_file() {
            anti_xss(&sizify(metadata.len(), false))
        } else {
            "-".to_string()
        };

        let date = if let Ok(mtime) = metadata.modified() {
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

    Ok(format_page!(
        body,
        &format!("Directory: {request_path}"),
        vec![css_common, css_directory]
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sizify_zero_bytes() {
        assert_eq!(sizify(0, false), "0 B");
    }

    #[test]
    fn sizify_one_byte() {
        assert_eq!(sizify(1, false), "1 B");
    }

    #[test]
    fn sizify_kilobytes() {
        assert_eq!(sizify(1024, false), "1.0 KiB");
        assert_eq!(sizify(1536, false), "1.5 KiB");
    }

    #[test]
    fn sizify_megabytes() {
        assert_eq!(sizify(1_048_576, false), "1.0 MiB");
        assert_eq!(sizify(2_621_440, false), "2.5 MiB");
    }

    #[test]
    fn sizify_gigabytes() {
        assert_eq!(sizify(1_073_741_824, false), "1.0 GiB");
    }

    #[test]
    fn sizify_terabytes() {
        assert_eq!(sizify(1_099_511_627_776, false), "1.0 TiB");
    }

    #[test]
    fn sizify_caps_at_tib() {
        // Very large files should cap at TiB
        assert!(sizify(u64::MAX, false).contains("TiB"));
    }
}
