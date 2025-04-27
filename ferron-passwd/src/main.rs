use clap::Parser;
use mimalloc::MiMalloc;
use password_auth::generate_hash;
use rpassword::prompt_password;
use std::process;
use yaml_rust2::{yaml, Yaml, YamlEmitter};

#[global_allocator]
static GLOBAL: MiMalloc = MiMalloc;

/// A password tool for Ferron
#[derive(Parser, Debug)]
#[command(version, about, long_about = None)]
struct Args {
  /// The username, for which you want to generate an user entry
  #[arg()]
  username: String,
}

fn main() {
  let args = Args::parse();

  let password = match prompt_password("Password: ") {
    Ok(pass) => pass,
    Err(e) => {
      eprintln!("Error reading password: {}", e);
      process::exit(1);
    }
  };
  let password2 = match prompt_password("Confirm password: ") {
    Ok(pass) => pass,
    Err(e) => {
      eprintln!("Error reading password confirmation: {}", e);
      process::exit(1);
    }
  };

  if password != password2 {
    eprintln!("Passwords don't match!");
    process::exit(1);
  }

  let password_hash = generate_hash(password);

  let mut yaml_user_hashmap = yaml::Hash::new();
  yaml_user_hashmap.insert(
    Yaml::String("name".to_string()),
    Yaml::String(args.username),
  );
  yaml_user_hashmap.insert(
    Yaml::String("pass".to_string()),
    Yaml::String(password_hash),
  );

  let yaml_data = Yaml::Array(vec![Yaml::Hash(yaml_user_hashmap)]);

  let mut output = String::new();
  if let Err(e) = YamlEmitter::new(&mut output).dump(&yaml_data) {
      eprintln!("Error generating YAML output: {}", e);
      process::exit(1);
  }

  println!("Copy the user object below into \"users\" property of either global configuration or a virtual host in the \"ferron.yaml\" file. Remember about the indentation in the server configuration.");
  println!("{}", output);
}
