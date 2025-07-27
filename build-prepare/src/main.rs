use std::collections::HashMap;
use std::error::Error;
use std::fmt;
use std::path::Path;
use std::{fs, io};

use toml::{Table, Value};
use yaml_rust2::YamlLoader;

#[derive(Debug)]
enum BuildError {
  IoError(io::Error),
  TomlError(toml::de::Error),
  YamlError(yaml_rust2::ScanError),
  ConfigError(String),
  MissingFile(String),
}

impl fmt::Display for BuildError {
  fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
    match self {
      BuildError::IoError(e) => write!(f, "IO error: {e}"),
      BuildError::TomlError(e) => write!(f, "TOML parsing error: {e}"),
      BuildError::YamlError(e) => write!(f, "YAML parsing error: {e}"),
      BuildError::ConfigError(msg) => write!(f, "Configuration error: {msg}"),
      BuildError::MissingFile(path) => write!(f, "Missing required file: {path}"),
    }
  }
}

impl Error for BuildError {}

impl From<io::Error> for BuildError {
  fn from(error: io::Error) -> Self {
    BuildError::IoError(error)
  }
}

impl From<toml::de::Error> for BuildError {
  fn from(error: toml::de::Error) -> Self {
    BuildError::TomlError(error)
  }
}

impl From<yaml_rust2::ScanError> for BuildError {
  fn from(error: yaml_rust2::ScanError) -> Self {
    BuildError::YamlError(error)
  }
}

type Result<T> = std::result::Result<T, BuildError>;

fn copy_dir_all(src: impl AsRef<Path>, dst: impl AsRef<Path>) -> io::Result<()> {
  fs::create_dir_all(&dst)?;
  for entry in fs::read_dir(src)? {
    let entry = entry?;
    let ty = entry.file_type()?;
    if ty.is_dir() {
      copy_dir_all(entry.path(), dst.as_ref().join(entry.file_name()))?;
    } else {
      fs::copy(entry.path(), dst.as_ref().join(entry.file_name()))?;
    }
  }
  Ok(())
}

fn setup_workspace() -> Result<()> {
  println!("Setting up temporary workspace...");

  // Clean up any existing workspace
  if Path::new("build-workspace").exists() {
    fs::remove_dir_all("build-workspace").map_err(BuildError::IoError)?;
  }

  fs::create_dir("build-workspace").map_err(BuildError::IoError)?;

  Ok(())
}

fn process_cargo_workspace() -> Result<Vec<String>> {
  println!("Processing Cargo workspace...");

  let cargo_workspace_contents =
    fs::read_to_string("Cargo.toml").map_err(|_| BuildError::MissingFile("Cargo.toml".to_string()))?;

  let mut cargo_workspace: Table = cargo_workspace_contents.parse().map_err(BuildError::TomlError)?;

  let workspace = cargo_workspace
    .get_mut("workspace")
    .ok_or_else(|| BuildError::ConfigError("No [workspace] section found in Cargo.toml".to_string()))?
    .as_table_mut()
    .ok_or_else(|| BuildError::ConfigError("[workspace] is not a table".to_string()))?;

  let workspace_members = workspace
    .get_mut("members")
    .ok_or_else(|| BuildError::ConfigError("No workspace members found".to_string()))?
    .as_array_mut()
    .ok_or_else(|| BuildError::ConfigError("Workspace members is not an array".to_string()))?;

  // Remove ferron-build-internal from workspace members
  if let Some(index) = workspace_members
    .iter()
    .position(|x| *x == "ferron-build-internal".into())
  {
    workspace_members.remove(index);
    println!("Removed ferron-build-internal from workspace members");
  }

  let mut copied_members = Vec::new();

  for workspace_member in workspace_members.iter() {
    if let Some(workspace_member_str) = workspace_member.as_str() {
      if workspace_member_str != "ferron-load-modules" {
        let src_path = Path::new(workspace_member_str);
        if src_path.exists() {
          let dst_path = format!("build-workspace/{workspace_member_str}");
          copy_dir_all(src_path, &dst_path).map_err(BuildError::IoError)?;
          copied_members.push(workspace_member_str.to_string());
          println!("Copied workspace member: {workspace_member_str}");
        } else {
          eprintln!(
            "Warning: Workspace member '{workspace_member_str}' does not exist, skipping"
          );
        }
      }
    }
  }

  // Write updated Cargo.toml
  fs::write("build-workspace/Cargo.toml", cargo_workspace.to_string().as_bytes()).map_err(BuildError::IoError)?;

  // Copy optional files
  copy_optional_file("Cargo.lock", "build-workspace/Cargo.lock");
  copy_optional_file("Cross.toml", "build-workspace/Cross.toml");
  copy_optional_directory("assets", "build-workspace/assets");

  Ok(copied_members)
}

fn copy_optional_file(src: &str, dst: &str) {
  match fs::copy(src, dst) {
    Ok(_) => println!("Copied optional file: {src}"),
    Err(_) => println!("Optional file '{src}' not found, skipping"),
  }
}

fn copy_optional_directory(src: &str, dst: &str) {
  match copy_dir_all(src, dst) {
    Ok(_) => println!("Copied optional directory: {src}"),
    Err(_) => println!("Optional directory '{src}' not found, skipping"),
  }
}

fn load_build_config() -> Result<yaml_rust2::Yaml> {
  println!("Loading build configuration...");

  let config_contents = fs::read_to_string("ferron-build-override.yaml")
    .or_else(|_| fs::read_to_string("ferron-build.yaml"))
    .map_err(|_| BuildError::MissingFile("ferron-build.yaml or ferron-build-override.yaml".to_string()))?;

  let docs = YamlLoader::load_from_str(&config_contents).map_err(BuildError::YamlError)?;

  if docs.is_empty() {
    return Err(BuildError::ConfigError("Build configuration file is empty".to_string()));
  }

  Ok(docs[0].clone())
}

fn process_ferron_load_modules(build_config: &yaml_rust2::Yaml) -> Result<()> {
  println!("Processing ferron-load-modules...");

  // Copy ferron-load-modules directory
  copy_dir_all("ferron-load-modules", "build-workspace/ferron-load-modules")
    .map_err(|_| BuildError::MissingFile("ferron-load-modules directory".to_string()))?;

  // Load and modify Cargo.toml
  let manifest_path = "build-workspace/ferron-load-modules/Cargo.toml";
  let crate_manifest_contents =
    fs::read_to_string(manifest_path).map_err(|_| BuildError::MissingFile(manifest_path.to_string()))?;

  let mut crate_manifest: Table = crate_manifest_contents.parse().map_err(BuildError::TomlError)?;

  // Initialize builtin features
  let dependencies = crate_manifest
    .get_mut("dependencies")
    .ok_or_else(|| BuildError::ConfigError("No dependencies section in ferron-load-modules/Cargo.toml".to_string()))?
    .as_table_mut()
    .ok_or_else(|| BuildError::ConfigError("Dependencies section is not a table".to_string()))?;

  dependencies
    .get_mut("ferron-modules-builtin")
    .ok_or_else(|| BuildError::ConfigError("ferron-modules-builtin dependency not found".to_string()))?
    .as_table_mut()
    .ok_or_else(|| BuildError::ConfigError("ferron-modules-builtin dependency is not a table".to_string()))?
    .insert("features".to_string(), Value::Array(Vec::new()));

  let mut additional_crates = Vec::new();

  // Process modules from build config
  let modules = build_config["modules"]
    .as_vec()
    .ok_or_else(|| BuildError::ConfigError("'modules' is missing or not an array".to_string()))?;

  for module in modules.iter() {
    if let Some(loader) = module["loader"].as_str() {
      println!("Processing module with '{loader}' loader");
    }

    let is_builtin = module["builtin"].as_bool().unwrap_or(false);

    if is_builtin {
      if let Some(cargo_feature) = module["cargo_feature"].as_str() {
        dependencies
          .get_mut("ferron-modules-builtin")
          .ok_or_else(|| BuildError::ConfigError("ferron-modules-builtin dependency not found".to_string()))?
          .as_table_mut()
          .ok_or_else(|| BuildError::ConfigError("ferron-modules-builtin dependency is not a table".to_string()))?
          .get_mut("features")
          .unwrap()
          .as_array_mut()
          .unwrap()
          .push(cargo_feature.into());
        println!("  Added builtin feature: {cargo_feature}");
      }
    } else if let Some(git_url) = module["git"].as_str() {
      let crate_name = module["crate"]
        .as_str()
        .ok_or_else(|| BuildError::ConfigError("Module missing 'crate' field".to_string()))?;

      let mut property: HashMap<String, Value> = HashMap::new();
      property.insert("git".to_string(), git_url.into());
      property.insert("default-features".to_string(), Value::Boolean(false));

      dependencies.insert(crate_name.to_string(), property.into());
      additional_crates.push(crate_name.to_string());
      println!("  Added git dependency: {crate_name} from {git_url}");
    } else if let Some(path) = module["path"].as_str() {
      let crate_name = module["crate"]
        .as_str()
        .ok_or_else(|| BuildError::ConfigError("Module missing 'crate' field".to_string()))?;

      let mut property: HashMap<String, Value> = HashMap::new();
      property.insert("path".to_string(), path.into());
      property.insert("default-features".to_string(), Value::Boolean(false));

      dependencies.insert(crate_name.to_string(), property.into());
      additional_crates.push(crate_name.to_string());
      println!("  Added path dependency: {crate_name} from {path}");
    } else {
      eprintln!("Warning: A module has no valid source (builtin, git, or path)");
    }
  }

  // Update features for additional crates
  if let Some(features) = crate_manifest.get_mut("features").and_then(|f| f.as_table_mut()) {
    for additional_crate in &additional_crates {
      for (feature_name, feature_dependencies) in features.iter_mut() {
        if feature_name.starts_with("runtime-") {
          if let Some(deps_array) = feature_dependencies.as_array_mut() {
            deps_array.push(format!("{additional_crate}/{feature_name}").into());
          }
        }
      }
    }
  }

  // Write updated Cargo.toml
  fs::write(manifest_path, crate_manifest.to_string().as_bytes()).map_err(BuildError::IoError)?;

  // Handle build.rs files
  let old_build_rs = "build-workspace/ferron-load-modules/build.rs";
  let new_build_rs = "build-workspace/ferron-load-modules/build_with_modules.rs";

  if Path::new(old_build_rs).exists() {
    fs::remove_file(old_build_rs).map_err(BuildError::IoError)?;
  }

  if Path::new(new_build_rs).exists() {
    fs::rename(new_build_rs, old_build_rs).map_err(BuildError::IoError)?;
    println!("Replaced build.rs with build_with_modules.rs");
  } else {
    eprintln!("Warning: build_with_modules.rs not found");
  }

  Ok(())
}

fn copy_build_config() -> Result<()> {
  println!("Copying build configuration...");

  // Try to copy override config first, then fallback to regular config
  if Path::new("ferron-build-override.yaml").exists() {
    fs::copy("ferron-build-override.yaml", "build-workspace/ferron-build.yaml").map_err(BuildError::IoError)?;
    println!("Copied ferron-build-override.yaml as ferron-build.yaml");
  } else if Path::new("ferron-build.yaml").exists() {
    fs::copy("ferron-build.yaml", "build-workspace/ferron-build.yaml").map_err(BuildError::IoError)?;
    println!("Copied ferron-build.yaml");
  } else {
    return Err(BuildError::MissingFile("ferron-build.yaml".to_string()));
  }

  Ok(())
}

fn main() -> Result<()> {
  println!("Preparing Ferron build environment...");

  setup_workspace()?;

  let _workspace_members = process_cargo_workspace()?;

  let build_config = load_build_config()?;

  process_ferron_load_modules(&build_config)?;

  copy_build_config()?;

  println!("✓ Ferron build environment prepared successfully!");

  Ok(())
}
