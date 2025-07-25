// const DIRECTORY_CSS: &'static str = include_str!("../res/directory.css");

/// Formats a page with the given contents, title, and CSS stylesheets.
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
               {}
           </body>
       </html>
",
      crate::util::anti_xss($title),
      css,
      $contents
    )
  }};
}

pub(crate) use format_page;
