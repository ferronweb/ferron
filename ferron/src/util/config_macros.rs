/// Gets configuration entries for validation
macro_rules! get_entries_for_validation {
  ($name:literal, $config:expr, $used:expr) => {{
    $config.entries.get($name).and_then(|value| {
      $used.insert($name.to_string());
      value.get_value().map(|_| value)
    })
  }};
}

/// Gets configuration values for validation
macro_rules! get_values_for_validation {
  ($name:literal, $config:expr, $used:expr) => {{
    $config
      .entries
      .get($name)
      .and_then(|value| {
        $used.insert($name.to_string());
        value.get_value().map(|_| value)
      })
      .map_or(Vec::new(), |value| value.get_values())
  }};
}

/// Gets configuration entries
macro_rules! get_entries {
  ($name:literal, $config:expr) => {{
    $config
      .entries
      .get($name)
      .and_then(|value| value.get_value().map(|_| value))
  }};
}

/// Gets a configuration entry
macro_rules! get_entry {
  ($name:literal, $config:expr) => {{
    $config
      .entries
      .get($name)
      .and_then(|value| value.get_value().map(|_| value))
      .and_then(|value| value.get_entry())
  }};
}

/// Gets a configuration value
macro_rules! get_value {
  ($name:literal, $config:expr) => {{
    $config
      .entries
      .get($name)
      .and_then(|value| value.get_value().map(|_| value))
      .and_then(|value| value.get_value())
  }};
}

/// Gets configuration values
macro_rules! get_values {
  ($name:literal, $config:expr) => {{
    $config
      .entries
      .get($name)
      .and_then(|value| value.get_value().map(|_| value))
      .map_or(Vec::new(), |value| value.get_values())
  }};
}

pub(crate) use get_entries;
pub(crate) use get_entries_for_validation;
pub(crate) use get_entry;
pub(crate) use get_value;
pub(crate) use get_values;
pub(crate) use get_values_for_validation;
