use clap::Parser;
use mimalloc::MiMalloc;
use password_auth::generate_hash;
use rpassword::prompt_password;
use std::process;

#[global_allocator]
static GLOBAL: MiMalloc = MiMalloc;

/// A password tool for Ferron
#[derive(Parser, Debug)]
#[command(version, about, long_about = None)]
struct Args;

fn main() {
  Args::parse();

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

  println!("The generated password hash: {}", password_hash);
  println!("Refer to the Ferron configuration documentation for information on how to configure the users with passwords")
}
