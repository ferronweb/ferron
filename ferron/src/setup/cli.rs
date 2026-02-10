use std::path::PathBuf;

use clap::{Args, Parser, Subcommand, ValueEnum};

#[derive(Debug, Clone, PartialEq, ValueEnum)]
pub enum ConfigAdapter {
  Kdl,
  #[cfg(feature = "config-yaml-legacy")]
  YamlLegacy,
  #[cfg(feature = "config-docker-auto")]
  DockerAuto,
}

#[derive(ValueEnum, Debug, Clone, PartialEq)]
pub enum LogOutput {
  Stdout,
  Stderr,
  Off,
}

#[derive(Args, Debug, Clone, PartialEq)]
pub struct ServeArgs {
  /// The listening IP to use.
  #[arg(short, long, default_value = "127.0.0.1")]
  pub listen_ip: String,

  /// The port to use.
  #[arg(short, long, default_value = "3000")]
  pub port: u16,

  /// The root directory to serve.
  #[arg(short, long, default_value = ".")]
  pub root: PathBuf,

  /// Basic authentication credentials for authorized users. The credential value must
  /// be in the form "${user}:${hashed_password}" where the "${hashed_password}" is from
  /// the ferron-passwd program or from any program using the password-auth generate_hash()
  /// macro (see https://docs.rs/password-auth/latest/password_auth/fn.generate_hash.html).
  #[arg(short, long)]
  pub credential: Vec<String>,

  /// Whether to disable brute-force password protection.
  #[arg(long)]
  pub disable_brute_protection: bool,

  /// Whether to start the server as a forward proxy.
  #[arg(long)]
  pub forward_proxy: bool,

  /// Where to output logs.
  #[arg(long, default_value = "stdout")]
  pub log: LogOutput,

  /// Where to output error logs.
  #[arg(long, default_value = "stderr")]
  pub error_log: LogOutput,
}

#[derive(Subcommand, Debug, Clone, PartialEq)]
pub enum Command {
  /// Utility command to start up a basic HTTP server.
  Serve(ServeArgs),
}

/// A fast, memory-safe web server written in Rust
#[derive(Parser, Debug, PartialEq)]
#[command(about, long_about = None)]
pub struct FerronArgs {
  /// The path to the server configuration file
  #[arg(short, long, default_value = "./ferron.kdl")]
  pub config: PathBuf,

  /// The string containing the server configuration
  #[arg(long)]
  pub config_string: Option<String>,

  /// The configuration adapter to use
  #[arg(long, value_enum)]
  pub config_adapter: Option<ConfigAdapter>,

  /// Prints the used compile-time module configuration (`ferron-build.yaml` or `ferron-build-override.yaml` in the Ferron source) and exits
  #[arg(long)]
  pub module_config: bool,

  /// Print version and build information
  #[arg(short = 'V', long)]
  pub version: bool,

  #[command(subcommand)]
  pub command: Option<Command>,
}

#[cfg(test)]
mod tests {
  use super::*;

  // The hash here is for the password '123?45>6'.
  const COMMON_TEST_PASSWORD: &str =
    "$argon2id$v=19$m=19456,t=2,p=1$emTillHaS3OqFuvITdXxzg$G00heP8QSXk5H/ruTiLt302Xk3uETfU5QO8hBIwUq08";

  #[test]
  fn test_supported_args() {
    let args = FerronArgs::parse_from(vec![
      "ferron",
      "--config",
      "/dev/null",
      "--config-adapter",
      "kdl",
      "--module-config",
      "--version",
    ]);
    assert!(args.module_config);
    assert!(args.version);
    assert_eq!(PathBuf::from("/dev/null"), args.config);
    assert_eq!(Some(ConfigAdapter::Kdl), args.config_adapter);
    assert_eq!(None, args.command);
  }

  #[test]
  fn test_supported_args_short_options() {
    let args = FerronArgs::parse_from(vec![
      "ferron",
      "-c",
      "/dev/null",
      "--config-adapter",
      "kdl",
      "--module-config",
      "-V",
    ]);
    assert!(args.module_config);
    assert!(args.version);
    assert_eq!(PathBuf::from("/dev/null"), args.config);
    assert_eq!(None, args.config_string);
    assert_eq!(Some(ConfigAdapter::Kdl), args.config_adapter);
    assert_eq!(None, args.command);
  }

  #[test]
  fn test_supported_optional_args() {
    let args = FerronArgs::parse_from(vec!["ferron"]);
    assert!(!args.module_config);
    assert!(!args.version);
    assert_eq!(PathBuf::from("./ferron.kdl"), args.config);
    assert_eq!(None, args.config_string);
    assert_eq!(None, args.config_adapter);
    assert_eq!(None, args.command);
  }

  #[test]
  fn test_supported_config_string_arg() {
    let expected_string =
      String::from(":8080 {\n  log \"/dev/stderr\"\n  error_log \"/dev/stderr\"\n  root \"/mnt/www\"\n}");
    let args = FerronArgs::parse_from(vec!["ferron", "--config-string", &expected_string]);
    assert!(!args.module_config);
    assert!(!args.version);
    assert_eq!(PathBuf::from("./ferron.kdl"), args.config);
    assert_eq!(Some(expected_string), args.config_string);
    assert_eq!(None, args.config_adapter);
    assert_eq!(None, args.command);
  }

  #[test]
  fn test_supported_http_serve_default_args() {
    let args = FerronArgs::parse_from(vec!["ferron", "serve"]);
    assert!(!args.module_config);
    assert!(!args.version);
    assert_eq!(PathBuf::from("./ferron.kdl"), args.config);
    assert_eq!(None, args.config_string);
    assert_eq!(None, args.config_adapter);
    assert!(args.command.is_some());
    match args.command.unwrap() {
      Command::Serve(http_serve_args) => {
        assert_eq!(String::from("127.0.0.1"), http_serve_args.listen_ip);
        assert_eq!(3000, http_serve_args.port);
        assert_eq!(PathBuf::from("."), http_serve_args.root);
        assert_eq!(Vec::<String>::new(), http_serve_args.credential);
        assert!(!http_serve_args.disable_brute_protection);
        assert!(!http_serve_args.forward_proxy);
        assert_eq!(LogOutput::Stdout, http_serve_args.log);
        assert_eq!(LogOutput::Stderr, http_serve_args.error_log);
      }
    }
  }

  #[test]
  fn test_supported_http_serve_args() {
    let args = FerronArgs::parse_from(vec![
      "ferron",
      "serve",
      "--listen-ip",
      "0.0.0.0",
      "--port",
      "8080",
      "--root",
      "./wwwroot",
      "--credential",
      format!("test:{COMMON_TEST_PASSWORD}").as_str(),
      "--credential",
      format!("test2:{COMMON_TEST_PASSWORD}").as_str(),
      "--disable-brute-protection",
      "--forward-proxy",
      "--log",
      "off",
      "--error-log",
      "off",
    ]);
    assert!(!args.module_config);
    assert!(!args.version);
    assert_eq!(PathBuf::from("./ferron.kdl"), args.config);
    assert_eq!(None, args.config_string);
    assert_eq!(None, args.config_adapter);
    assert!(args.command.is_some());
    match args.command.unwrap() {
      Command::Serve(http_serve_args) => {
        assert_eq!(String::from("0.0.0.0"), http_serve_args.listen_ip);
        assert_eq!(8080, http_serve_args.port);
        assert_eq!(PathBuf::from("./wwwroot"), http_serve_args.root);
        assert_eq!(
          vec![
            format!("test:{COMMON_TEST_PASSWORD}"),
            format!("test2:{COMMON_TEST_PASSWORD}")
          ],
          http_serve_args.credential
        );
        assert!(http_serve_args.disable_brute_protection);
        assert!(http_serve_args.forward_proxy);
        assert_eq!(LogOutput::Off, http_serve_args.log);
        assert_eq!(LogOutput::Off, http_serve_args.error_log);
      }
    }
  }
}
