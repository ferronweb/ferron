pub fn match_location(path: &str, req_path: &str) -> bool {
  let mut path_without_trailing_slashes = path;
  while path_without_trailing_slashes.ends_with("/") {
    path_without_trailing_slashes =
      &path_without_trailing_slashes[..(path_without_trailing_slashes.len() - 1)];
  }

  let mut path_prepared = path_without_trailing_slashes.to_owned();
  let mut req_path_prepared = req_path.to_owned();

  while path_prepared.contains("//") {
    path_prepared = path_prepared.replace("//", "/");
  }

  while req_path_prepared.contains("//") {
    req_path_prepared = path_prepared.replace("//", "/");
  }

  if cfg!(windows) {
    path_prepared = path_prepared.to_lowercase();
    req_path_prepared = req_path_prepared.to_lowercase();
  }

  path_prepared == req_path_prepared
    || req_path_prepared.starts_with(&format!("{}/", path_prepared))
}

#[cfg(test)]
mod tests {
  use super::match_location;

  #[test]
  fn test_exact_match() {
    assert!(match_location("/home", "/home"));
    assert!(match_location("/api/v1", "/api/v1"));
  }

  #[test]
  fn test_trailing_slash() {
    assert!(match_location("/home/", "/home"));
    assert!(match_location("/home", "/home/"));
    assert!(match_location("/api/v1/", "/api/v1"));
  }

  #[test]
  fn test_subpath_match() {
    assert!(match_location("/api", "/api/v1"));
    assert!(match_location("/users", "/users/profile"));
  }

  #[test]
  fn test_non_matching_paths() {
    assert!(!match_location("/home", "/dashboard"));
    assert!(!match_location("/api", "/user"));
  }

  #[test]
  fn test_multiple_slashes() {
    assert!(match_location("/api//v1", "/api/v1"));
    assert!(match_location("//home///", "/home"));
  }

  #[test]
  fn test_case_insensitivity_on_windows() {
    #[cfg(windows)]
    {
      assert!(match_location("/API", "/api"));
      assert!(match_location("/Home", "/home"));
    }
  }
}
