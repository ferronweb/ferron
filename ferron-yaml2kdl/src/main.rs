use std::str::FromStr;
use std::{fs, path::PathBuf};

use clap::{crate_name, crate_version, Arg, ArgAction, ArgMatches, Command};
use ferron_yaml2kdl_core::convert_yaml_to_kdl;
use mimalloc::MiMalloc;

#[global_allocator]
static GLOBAL: MiMalloc = MiMalloc;

/// Parses the command-line arguments
fn parse_arguments() -> ArgMatches {
  Command::new(crate_name!())
        .version(crate_version!())
        .about("A utility that attempts to convert Ferron 1.x YAML configuration to Ferron 2.x KDL configuration")
        .arg(
            Arg::new("input")
                .help("The name of an input file, containing Ferron 1.x YAML configuration")
                .required(true)
                .action(ArgAction::Set)
                .value_parser(PathBuf::from_str),
        )
        .arg(
            Arg::new("output")
                .help("The name of an output file, containing Ferron 2.x KDL configuration")
                .required(true)
                .action(ArgAction::Set)
                .value_parser(PathBuf::from_str),
        )
        .get_matches()
}

/// The main entry point of the application
fn main() {
  // Parse command-line arguments
  let args = parse_arguments();

  // Obtain command-line arguments
  let input_pathbuf: PathBuf = match args.get_one::<PathBuf>("input") {
    Some(arg) => arg.to_owned(),
    None => {
      eprintln!("Cannot obtain the input file path");
      std::process::exit(1);
    }
  };
  let output_pathbuf: PathBuf = match args.get_one::<PathBuf>("output") {
    Some(arg) => arg.to_owned(),
    None => {
      eprintln!("Cannot obtain the output file path");
      std::process::exit(1);
    }
  };

  // Convert the server configuration
  let kdl_config = match convert_yaml_to_kdl(input_pathbuf) {
    Ok(config) => config,
    Err(err) => {
      eprintln!("Error converting the server configuration: {}", err);
      std::process::exit(1);
    }
  };

  // Write the converted server configuration
  if let Err(err) = fs::write(output_pathbuf, kdl_config.to_string()) {
    eprintln!("Error writing the server configuration: {}", err);
    std::process::exit(1);
  }
}
