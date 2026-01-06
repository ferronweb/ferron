use std::fs;

const CONFIG_TEMPLATE: &'static str = "%{FERRON_CONFIG}";
const UPDATER_TEMPLATE: &'static str = "%{FERRON_UPDATER}";

fn main() -> Result<(), std::io::Error> {
  println!("Building installer scripts...");

  fs::create_dir_all("dist")?;

  if let Ok(mut installer_script) = fs::read_to_string("installer/install-template.sh") {
    let mut updater_script = fs::read_to_string("installer/updater-template.sh").ok();
    if let Ok(config) = fs::read_to_string("ferron-packages.kdl") {
      updater_script = updater_script.map(|s| s.replace(CONFIG_TEMPLATE, &config.trim_end()));
      installer_script = installer_script.replace(CONFIG_TEMPLATE, &config.trim_end());
    }
    if let Some(updater_script) = updater_script {
      installer_script = installer_script.replace(UPDATER_TEMPLATE, &updater_script.trim_end());
    }
    fs::write("dist/install.sh", installer_script)?;
  } else {
    eprintln!("Warning: failed to read install-template.sh, not creating installer for Linux...");
  }

  if let Ok(installer_script) = fs::read_to_string("installer/install-template.ps1") {
    // For now, just copy the script from the template
    fs::write("dist/install.ps1", installer_script)?;
  } else {
    eprintln!("Warning: failed to read install-template.ps1, not creating installer for Windows...");
  }

  Ok(())
}
