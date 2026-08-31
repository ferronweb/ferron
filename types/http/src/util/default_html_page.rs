//! HTML page formatting macro for default error and directory listing pages.

/// Formats an HTML page with the given contents, title, and CSS stylesheets.
///
/// Produces a complete `<!doctype html>` document with the title HTML-escaped
/// via [`anti_xss`](crate::util::anti_xss::anti_xss).
///
/// # Usage
///
/// ```ignore
/// let html = format_page!("<h1>Not Found</h1>", "404", [CSS_COMMON]);
/// ```
#[macro_export]
macro_rules! format_page {
    ($contents:expr, $title:expr, $css:expr) => {{
        let css = $css
            .into_iter()
            .map(|css| format!("<style>{}</style>", css))
            .collect::<Vec<_>>()
            .join("\n");
        format!(
            "<!doctype html>
<html lang=\"en\">
<head>
<meta charset=\"UTF-8\" />
<meta name=\"viewport\" content=\"width=device-width, initial-scale=1.0\" />
<title>{}</title>
{}
</head>
<body>
<div class=\"body-bg\"></div>
<div class=\"body-main\">
{}
</div>
</body>
</html>
",
            $crate::util::anti_xss::anti_xss($title),
            css,
            $contents
        )
    }};
}
