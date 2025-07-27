use shadow_rs::{BuildPattern, ShadowBuilder};
use std::{env, io};
use winresource::WindowsResource;

fn main() -> io::Result<()> {
  if env::var_os("CARGO_CFG_WINDOWS").is_some() {
    WindowsResource::new().set_icon("assets/icon.ico").compile()?;
  }

  ShadowBuilder::builder()
    .build_pattern(BuildPattern::RealTime)
    .build()
    .unwrap();

  Ok(())
}
