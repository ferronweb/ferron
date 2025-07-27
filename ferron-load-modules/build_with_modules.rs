use std::env;
use std::fs::File;
use std::io::Write;
use std::path::Path;

use toml::Table;
use yaml_rust2::YamlLoader;

fn main() {
  println!("cargo:rerun-if-changed=../ferron-build.yaml");
  println!("cargo:rerun-if-changed=../ferron-build-override.yaml");
  let crate_dir = env::var("CARGO_MANIFEST_DIR").unwrap();
  let ferron_build_yaml_dir = Path::new(&crate_dir).join("../ferron-build.yaml");
  let ferron_build_yaml_override_dir = Path::new(&crate_dir).join("../ferron-build-override.yaml");
  let ferron_build_yaml_contents = std::fs::read_to_string(ferron_build_yaml_override_dir)
    .or_else(|_| std::fs::read_to_string(ferron_build_yaml_dir))
    .unwrap();
  let ferron_build_yaml_docs = YamlLoader::load_from_str(&ferron_build_yaml_contents).unwrap();
  let ferron_build_yaml = &ferron_build_yaml_docs[0];

  let cargo_toml_contents = std::fs::read_to_string(Path::new(&crate_dir).join("Cargo.toml")).unwrap();
  let cargo_toml = cargo_toml_contents.parse::<Table>().unwrap();
  let ferron_modules_builtin_features =
    if let Some(features) = cargo_toml["dependencies"]["ferron-modules-builtin"]["features"].as_array() {
      features
        .iter()
        .filter_map(|feature| feature.as_str())
        .collect::<Vec<&str>>()
    } else {
      vec![]
    };

  let mut modules_block_inside = String::new();

  ferron_build_yaml["modules"]
    .as_vec()
    .unwrap()
    .iter()
    .for_each(|module| {
      let is_builtin = module["builtin"].as_bool().unwrap_or(false);
      if is_builtin
        && module["cargo_feature"]
          .as_str()
          .is_none_or(|f| ferron_modules_builtin_features.contains(&f))
      {
        let module_loader_name = module["loader"].as_str().unwrap();
        let module_loader = format!("ferron_modules_builtin::{}::new()", module_loader_name);
        modules_block_inside.push_str(&format!("register_module_loader!({});\n", module_loader));
      } else if let Some(crate_name) = module["crate"].as_str() {
        let module_loader_name = module["loader"].as_str().unwrap();
        let module_loader = format!("{}::{}::new()", crate_name.replace("-", "_"), module_loader_name);
        modules_block_inside.push_str(&format!("register_module_loader!({});\n", module_loader));
      } else {
        println!(
          "cargo:warning=Module with \"{}\" loader is not built-in",
          module["loader"].as_str().unwrap()
        );
      }
    });

  let out_dir = env::var("OUT_DIR").unwrap();
  let dest_path = Path::new(&out_dir).join("register_module_loaders.rs");
  let mut f = File::create(&dest_path).unwrap();

  f.write_all(format!("{{{}}}", modules_block_inside).as_bytes()).unwrap();
}
