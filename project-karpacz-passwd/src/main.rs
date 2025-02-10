use clap::Parser;
use password_auth::generate_hash;
use rpassword::prompt_password;
use yaml_rust2::{yaml, Yaml, YamlEmitter};

#[cfg(not(target_os = "freebsd"))]
use mimalloc::MiMalloc;

#[cfg(not(target_os = "freebsd"))]
#[global_allocator]
static GLOBAL: MiMalloc = MiMalloc;

#[derive(Parser, Debug)]
#[command(version, about, long_about = None)]
struct Args {
  /// The username, for which you want to generate an user entry
  #[arg()]
  username: String,
}

fn main() {
  let args = Args::parse();

  let password = prompt_password("Password: ").unwrap();
  let password2 = prompt_password("Confirm password: ").unwrap();

  if password != password2 {
    eprintln!("Passwords don't match!");
    std::process::exit(1);
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
  YamlEmitter::new(&mut output).dump(&yaml_data).unwrap();

  println!("Copy the user object below into \"users\" property of either global configuration or a virtual host in the \"project-karpacz.yaml\" file. Remember about the indentation in the server configuration.");
  println!("{}", output);
}
