use std::error::Error;

use chrono::{DateTime, Local};
use tokio::fs::ReadDir;

use crate::ferron_util::anti_xss::anti_xss;
use crate::ferron_util::sizify::sizify;

pub async fn generate_directory_listing(
  mut directory: ReadDir,
  request_path: &str,
  description: Option<String>,
) -> Result<String, Box<dyn Error + Send + Sync>> {
  let mut request_path_without_trailing_slashes = request_path;
  while request_path_without_trailing_slashes.ends_with("/") {
    request_path_without_trailing_slashes =
      &request_path_without_trailing_slashes[..(request_path_without_trailing_slashes.len() - 1)];
  }

  // Return path
  let mut return_path_vec: Vec<&str> = request_path_without_trailing_slashes.split("/").collect();
  return_path_vec.pop();
  return_path_vec.push("");
  let return_path = &return_path_vec.join("/") as &str;

  let mut table_rows = Vec::new();
  if !request_path_without_trailing_slashes.is_empty() {
    table_rows.push(format!(
      "<tr><td><a href=\"{}\">Return</a></td><td></td><td></td></tr>",
      anti_xss(return_path)
    ));
  } else {
    request_path_without_trailing_slashes = "/";
  }
  let min_table_rows_length = table_rows.len();

  // Create a vector containing entries, then sort them by file name.
  let mut entries = Vec::new();
  while let Some(entry) = directory.next_entry().await? {
    entries.push(entry);
  }
  entries.sort_by_cached_key(|entry| entry.file_name().to_string_lossy().to_string());

  for entry in entries.iter() {
    let filename = entry.file_name().to_string_lossy().to_string();
    if filename.starts_with('.') {
      // Don't add files nor directories with "." at the beginning of their names
      continue;
    }
    match entry.metadata().await {
      Ok(metadata) => {
        let filename_link = format!(
          "<a href=\"{}{}{}\">{}</a>",
          request_path_without_trailing_slashes,
          anti_xss(urlencoding::encode(&filename).as_ref()),
          match metadata.is_dir() {
            true => "/",
            false => "",
          },
          anti_xss(&filename)
        );

        let row = format!(
          "<tr><td>{}</td><td>{}</td><td>{}</td></tr>",
          filename_link,
          match metadata.is_file() {
            true => anti_xss(&sizify(metadata.len(), false)),
            false => "-".to_string(),
          },
          anti_xss(
            &(match metadata.modified() {
              Ok(mtime) => {
                let datetime: DateTime<Local> = mtime.into();
                datetime.format("%a %b %d %Y").to_string()
              }
              Err(_) => "-".to_string(),
            })
          )
        );
        table_rows.push(row);
      }
      Err(_) => {
        let filename_link = format!(
          "<a href=\"{}{}{}\">{}</a>",
          "{}{}",
          request_path_without_trailing_slashes,
          anti_xss(urlencoding::encode(&filename).as_ref()),
          anti_xss(&filename)
        );
        let row = format!("<tr><td>{}</td><td>-</td><td>-</td></tr>", filename_link);
        table_rows.push(row);
      }
    };
  }

  if table_rows.len() < min_table_rows_length {
    table_rows.push("<tr><td>No files found</td><td></td><td></td></tr>".to_string());
  }

  Ok(format!(
    "<!DOCTYPE html>
<html lang=\"en\">
<head>
    <meta charset=\"UTF-8\">
    <meta name=\"viewport\" content=\"width=device-width, initial-scale=1.0\">
    <title>Directory: {}</title>
</head>
<body>
    <h1>Directory: {}</h1>
    <table>
      <tr><th>Filename</th><th>Size</th><th>Date</th></tr>
      {}
      {}
    </table>
</body>
</html>",
    anti_xss(request_path),
    anti_xss(request_path),
    table_rows.join(""),
    match description {
      Some(description) => format!(
        "<hr>{}",
        anti_xss(&description)
          .replace("\r\n", "\n")
          .replace("\r", "\n")
          .replace("\n", "<br>")
      ),
      None => "".to_string(),
    }
  ))
}
