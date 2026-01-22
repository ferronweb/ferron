use std::path::PathBuf;
use std::str::FromStr;

use clap::{crate_name, crate_version, Arg, ArgAction, ArgMatches, Command};
use mimalloc::MiMalloc;

#[global_allocator]
static GLOBAL: MiMalloc = MiMalloc;

/// Parses the command-line arguments
fn parse_arguments() -> ArgMatches {
  Command::new(crate_name!())
    .version(crate_version!())
    .about("A utility that precompresses static files for Ferron")
    .arg(
      Arg::new("assets")
        .help("The path to the static assets (it can be a directory or a file)")
        .required(true)
        .action(ArgAction::Append)
        .value_parser(PathBuf::from_str),
    )
    .arg(
      Arg::new("threads")
        .long("threads")
        .short('t')
        .help("The number of threads to use for compression")
        .default_value("64")
        .action(ArgAction::Append)
        .value_parser(usize::from_str),
    )
    .get_matches()
}

/// Obtains the paths of the assets
fn get_paths(assets_pathbuf: &PathBuf) -> Result<Vec<PathBuf>, std::io::Error> {
  if assets_pathbuf.is_dir() {
    let metadata = std::fs::read_dir(assets_pathbuf)?;
    let mut paths = Vec::new();
    for entry in metadata {
      let path = entry?.path();
      let extension = path.extension().and_then(|ext| ext.to_str());

      // Compressed files
      match extension {
        Some("gz") | Some("deflate") | Some("br") | Some("zst") => continue,
        _ => {}
      };

      // Non-compressible extensions
      let non_compressible_file_extensions = vec![
        "7z",
        "air",
        "amlx",
        "apk",
        "apng",
        "appinstaller",
        "appx",
        "appxbundle",
        "arj",
        "au",
        "avif",
        "bdoc",
        "boz",
        "br",
        "bz",
        "bz2",
        "caf",
        "class",
        "doc",
        "docx",
        "dot",
        "dvi",
        "ear",
        "epub",
        "flv",
        "gdoc",
        "gif",
        "gsheet",
        "gslides",
        "gz",
        "iges",
        "igs",
        "jar",
        "jnlp",
        "jp2",
        "jpe",
        "jpeg",
        "jpf",
        "jpg",
        "jpg2",
        "jpgm",
        "jpm",
        "jpx",
        "kmz",
        "latex",
        "m1v",
        "m2a",
        "m2v",
        "m3a",
        "m4a",
        "mesh",
        "mk3d",
        "mks",
        "mkv",
        "mov",
        "mp2",
        "mp2a",
        "mp3",
        "mp4",
        "mp4a",
        "mp4v",
        "mpe",
        "mpeg",
        "mpg",
        "mpg4",
        "mpga",
        "msg",
        "msh",
        "msix",
        "msixbundle",
        "odg",
        "odp",
        "ods",
        "odt",
        "oga",
        "ogg",
        "ogv",
        "ogx",
        "opus",
        "p12",
        "pdf",
        "pfx",
        "pgp",
        "pkpass",
        "png",
        "pot",
        "pps",
        "ppt",
        "pptx",
        "qt",
        "ser",
        "silo",
        "sit",
        "snd",
        "spx",
        "stpxz",
        "stpz",
        "swf",
        "tif",
        "tiff",
        "ubj",
        "usdz",
        "vbox-extpack",
        "vrml",
        "war",
        "wav",
        "weba",
        "webm",
        "wmv",
        "wrl",
        "x3dbz",
        "x3dvz",
        "xla",
        "xlc",
        "xlm",
        "xls",
        "xlsx",
        "xlt",
        "xlw",
        "xpi",
        "xps",
        "zip",
        "zst",
      ];
      if extension.is_some_and(|ext| non_compressible_file_extensions.contains(&ext)) {
        continue;
      }

      if path.is_file() {
        paths.push(path);
      } else if path.is_dir() {
        paths.extend(get_paths(&path)?);
      }
    }
    Ok(paths)
  } else if assets_pathbuf.is_file() {
    Ok(vec![assets_pathbuf.clone()])
  } else {
    Err(std::io::Error::new(std::io::ErrorKind::InvalidInput, "Invalid path"))
  }
}

/// Compresses an asset using gzip
fn compress_asset_gzip(path: &PathBuf) -> Result<(), std::io::Error> {
  let compressed_path = path.with_extension(
    path
      .extension()
      .and_then(|ext| ext.to_str())
      .map_or("gz".to_string(), |ext| format!("{}.gz", ext)),
  );
  let mut input = std::fs::File::open(path)?;
  let mut output = std::fs::File::create(&compressed_path)?;
  let mut encoder = flate2::write::GzEncoder::new(&mut output, flate2::Compression::default());
  std::io::copy(&mut input, &mut encoder)?;
  encoder.finish()?;
  Ok(())
}

/// Compresses an asset using deflate
fn compress_asset_deflate(path: &PathBuf) -> Result<(), std::io::Error> {
  let compressed_path = path.with_extension(
    path
      .extension()
      .and_then(|ext| ext.to_str())
      .map_or("deflate".to_string(), |ext| format!("{}.deflate", ext)),
  );
  let mut input = std::fs::File::open(path)?;
  let mut output = std::fs::File::create(&compressed_path)?;
  let mut encoder = flate2::write::DeflateEncoder::new(&mut output, flate2::Compression::default());
  std::io::copy(&mut input, &mut encoder)?;
  encoder.finish()?;
  Ok(())
}

/// Compresses an asset using Brotli
fn compress_asset_brotli(path: &PathBuf) -> Result<(), std::io::Error> {
  let compressed_path = path.with_extension(
    path
      .extension()
      .and_then(|ext| ext.to_str())
      .map_or("br".to_string(), |ext| format!("{}.br", ext)),
  );
  let mut input = std::fs::File::open(path)?;
  let mut output = std::fs::File::create(&compressed_path)?;
  brotli::enc::BrotliCompress(&mut input, &mut output, &brotli::enc::BrotliEncoderParams::default())?;
  Ok(())
}

/// Compresses an asset using Zstandard
fn compress_asset_zstd(path: &PathBuf) -> Result<(), std::io::Error> {
  let compressed_path = path.with_extension(
    path
      .extension()
      .and_then(|ext| ext.to_str())
      .map_or("zst".to_string(), |ext| format!("{}.zst", ext)),
  );
  let mut input = std::fs::File::open(path)?;
  let output = std::fs::File::create(&compressed_path)?;
  let mut encoder = zstd::Encoder::new(output, 0)?;
  encoder.window_log(17)?; // Limit the Zstandard window size to 128K (2^17 bytes) to support many HTTP clients
  std::io::copy(&mut input, &mut encoder)?;
  encoder.finish()?;
  Ok(())
}

/// Compresses an asset using multiple compression algorithms
fn compress_asset(path: &PathBuf) -> Result<(), std::io::Error> {
  compress_asset_gzip(path)?;
  compress_asset_deflate(path)?;
  compress_asset_brotli(path)?;
  compress_asset_zstd(path)?;
  Ok(())
}

/// The main entry point of the application
fn main() {
  // Parse command-line arguments
  let args = parse_arguments();

  // Obtain command-line arguments
  let assets_pathbufs: Vec<PathBuf> = match args.get_many::<PathBuf>("assets") {
    Some(paths) => paths.cloned().collect(),
    None => {
      eprintln!("Cannot obtain the assets paths");
      std::process::exit(1);
    }
  };

  // Obtain the number of threads
  let num_threads = match args.get_one::<usize>("threads") {
    Some(num) => *num,
    None => {
      eprintln!("Cannot obtain the number of threads");
      std::process::exit(1);
    }
  };

  let mut paths = Vec::new();

  // Obtain the paths
  for assets_pathbuf in assets_pathbufs {
    paths.extend(match get_paths(&assets_pathbuf) {
      Ok(paths) => paths,
      Err(err) => {
        eprintln!("Error obtaining paths at {}: {}", assets_pathbuf.display(), err);
        std::process::exit(1);
      }
    });
  }

  // Initialize the thread pool
  let thread_pool = match rayon::ThreadPoolBuilder::new().num_threads(num_threads).build() {
    Ok(pool) => pool,
    Err(err) => {
      eprintln!("Error initializing thread pool: {}", err);
      std::process::exit(1);
    }
  };

  // Compress the assets
  thread_pool.scope(move |scope| {
    for path in paths {
      println!("Compressing asset at {}...", path.display());
      scope.spawn(move |_| {
        if let Err(err) = compress_asset(&path) {
          eprintln!("Error compressing asset at {}: {}", path.display(), err);
          std::process::exit(1);
        }
      });
    }
  });
}
