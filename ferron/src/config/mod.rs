pub mod adapters;
mod lookup;
pub mod processing;

pub use ferron_common::config::*;

use std::error::Error;
use std::sync::Arc;

use fancy_regex::RegexBuilder;

pub use self::lookup::*;
use crate::util::IpBlockList;

/// Parses conditional data
pub fn parse_conditional_data(
  name: &str,
  value: ServerConfigurationEntry,
) -> Result<ConditionalData, Box<dyn Error + Send + Sync>> {
  Ok(match name {
    "is_remote_ip" => {
      let mut list = IpBlockList::new();
      list.load_from_vec(value.values.iter().filter_map(|v| v.as_str()).collect());
      ConditionalData::IsRemoteIp(list)
    }
    "is_forwarded_for" => {
      let mut list = IpBlockList::new();
      list.load_from_vec(value.values.iter().filter_map(|v| v.as_str()).collect());
      ConditionalData::IsForwardedFor(list)
    }
    "is_not_remote_ip" => {
      let mut list = IpBlockList::new();
      list.load_from_vec(value.values.iter().filter_map(|v| v.as_str()).collect());
      ConditionalData::IsNotRemoteIp(list)
    }
    "is_not_forwarded_for" => {
      let mut list = IpBlockList::new();
      list.load_from_vec(value.values.iter().filter_map(|v| v.as_str()).collect());
      ConditionalData::IsNotForwardedFor(list)
    }
    "is_equal" => ConditionalData::IsEqual(
      value
        .values
        .first()
        .and_then(|v| v.as_str())
        .ok_or(anyhow::anyhow!(
          "Missing or invalid left side of a \"is_equal\" subcondition"
        ))?
        .to_string(),
      value
        .values
        .get(1)
        .and_then(|v| v.as_str())
        .ok_or(anyhow::anyhow!(
          "Missing or invalid right side of a \"is_equal\" subcondition"
        ))?
        .to_string(),
    ),
    "is_not_equal" => ConditionalData::IsNotEqual(
      value
        .values
        .first()
        .and_then(|v| v.as_str())
        .ok_or(anyhow::anyhow!(
          "Missing or invalid left side of a \"is_not_equal\" subcondition"
        ))?
        .to_string(),
      value
        .values
        .get(1)
        .and_then(|v| v.as_str())
        .ok_or(anyhow::anyhow!(
          "Missing or invalid right side of a \"is_not_equal\" subcondition"
        ))?
        .to_string(),
    ),
    "is_regex" => {
      let left_side = value.values.first().and_then(|v| v.as_str()).ok_or(anyhow::anyhow!(
        "Missing or invalid left side of a \"is_regex\" subcondition"
      ))?;
      let right_side = value.values.get(1).and_then(|v| v.as_str()).ok_or(anyhow::anyhow!(
        "Missing or invalid right side of a \"is_regex\" subcondition"
      ))?;
      ConditionalData::IsRegex(
        left_side.to_string(),
        RegexBuilder::new(right_side)
          .case_insensitive(
            value
              .props
              .get("case_insensitive")
              .and_then(|p| p.as_bool())
              .unwrap_or(false),
          )
          .build()?,
      )
    }
    "is_not_regex" => {
      let left_side = value.values.first().and_then(|v| v.as_str()).ok_or(anyhow::anyhow!(
        "Missing or invalid left side of a \"is_not_regex\" subcondition"
      ))?;
      let right_side = value.values.get(1).and_then(|v| v.as_str()).ok_or(anyhow::anyhow!(
        "Missing or invalid right side of a \"is_not_regex\" subcondition"
      ))?;
      ConditionalData::IsNotRegex(
        left_side.to_string(),
        RegexBuilder::new(right_side)
          .case_insensitive(
            value
              .props
              .get("case_insensitive")
              .and_then(|p| p.as_bool())
              .unwrap_or(false),
          )
          .build()?,
      )
    }
    "is_rego" => {
      let rego_policy = value
        .values
        .first()
        .and_then(|v| v.as_str())
        .ok_or(anyhow::anyhow!("Missing or invalid Rego policy"))?;
      let mut rego_engine = regorus::Engine::new();
      rego_engine.add_policy("ferron.rego".to_string(), rego_policy.to_string())?;
      ConditionalData::IsRego(Arc::new(rego_engine))
    }
    "set_constant" => ConditionalData::SetConstant(
      value
        .values
        .first()
        .and_then(|v| v.as_str())
        .ok_or(anyhow::anyhow!(
          "Missing or invalid constant name in a \"set_constant\" subcondition"
        ))?
        .to_string(),
      value
        .values
        .get(1)
        .and_then(|v| v.as_str())
        .ok_or(anyhow::anyhow!(
          "Missing or invalid constant value in a \"set_constant\" subcondition"
        ))?
        .to_string(),
    ),
    "is_language" => ConditionalData::IsLanguage(
      value
        .values
        .first()
        .and_then(|v| v.as_str())
        .ok_or(anyhow::anyhow!(
          "Missing or invalid desired language in a \"is_language\" subcondition"
        ))?
        .to_string(),
    ),
    _ => Err(anyhow::anyhow!("Unrecognized subcondition: {name}"))?,
  })
}
