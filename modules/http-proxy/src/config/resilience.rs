use std::error::Error;

use ferron_core::config::{ServerConfigurationBlock, ServerConfigurationValue};

use super::types::{CircuitBreakerConfig, RetryBudgetConfig};
use crate::types::health::{ExpectedStatusCodes, HealthCheckMethod, UpstreamHealthCheckConfig};

pub(super) fn parse_expected_status(
    s: &str,
) -> Result<ExpectedStatusCodes, Box<dyn Error + Send + Sync>> {
    let s = s.trim();

    if s == "2xx" {
        return Ok(ExpectedStatusCodes::Successful);
    }
    if s == "2xx,3xx" || s == "3xx,2xx" {
        return Ok(ExpectedStatusCodes::SuccessfulOrRedirect);
    }

    if let Some(idx) = s.find('-') {
        let start_str = &s[..idx].trim();
        let end_str = &s[idx + 1..].trim();
        if let (Ok(start), Ok(end)) = (start_str.parse::<u16>(), end_str.parse::<u16>()) {
            if start <= end && start >= 100 && end < 600 {
                return Ok(ExpectedStatusCodes::Range(start, end));
            }
        }
    }

    if s.contains(',') {
        let mut codes = Vec::new();
        for part in s.split(',') {
            let code: u16 = part.trim().parse()?;
            codes.push(code);
        }
        if codes.len() == 1 {
            return Ok(ExpectedStatusCodes::Specific(codes[0]));
        }
        return Ok(ExpectedStatusCodes::Any(codes));
    }

    let code: u16 = s.parse()?;
    Ok(ExpectedStatusCodes::Specific(code))
}

pub(super) fn parse_active_health_check(
    entries: &ServerConfigurationBlock,
    health_check_config: &mut UpstreamHealthCheckConfig,
) -> Result<(), Box<dyn Error + Send + Sync>> {
    for (name, entries) in entries.directives.iter() {
        match name.as_str() {
            "uri" => {
                if let Some(val) = entries
                    .first()
                    .and_then(|e| e.args.first())
                    .and_then(|v| v.as_str())
                {
                    health_check_config.uri = val.to_string();
                }
            }
            "method" => {
                if let Some(val) = entries
                    .first()
                    .and_then(|e| e.args.first())
                    .and_then(|v| v.as_str())
                {
                    health_check_config.method = match val.to_uppercase().as_str() {
                        "GET" => HealthCheckMethod::Get,
                        "HEAD" => HealthCheckMethod::Head,
                        _ => {
                            return Err(format!(
                                "Invalid health_check_method: {val}, must be GET or HEAD"
                            )
                            .into())
                        }
                    };
                }
            }
            "interval" => {
                if let Some(val) = entries
                    .first()
                    .and_then(|e| e.args.first())
                    .and_then(|v| v.as_duration())
                {
                    health_check_config.interval = val;
                }
            }
            "timeout" => {
                if let Some(val) = entries
                    .first()
                    .and_then(|e| e.args.first())
                    .and_then(|v| v.as_duration())
                {
                    health_check_config.timeout = val;
                }
            }
            "expect_status" => {
                if let Some(val) = entries
                    .first()
                    .and_then(|e| e.args.first())
                    .and_then(|v| v.as_str())
                {
                    health_check_config.expect_status = parse_expected_status(val)?;
                }
            }
            "response_time_threshold" => {
                if let Some(val) = entries
                    .first()
                    .and_then(|e| e.args.first())
                    .and_then(|v| v.as_duration())
                {
                    health_check_config.response_time_threshold = Some(val);
                }
            }
            "body_match" => {
                if let Some(val) = entries
                    .first()
                    .and_then(|e| e.args.first())
                    .and_then(|v| v.as_str())
                {
                    health_check_config.body_match = Some(val.to_string());
                }
            }
            "consecutive_fails" => {
                if let Some(val) = entries
                    .first()
                    .and_then(|e| e.args.first())
                    .and_then(|v: &ServerConfigurationValue| v.as_number())
                {
                    if val > 0 {
                        health_check_config.consecutive_fails = val as u64;
                    }
                }
            }
            "consecutive_passes" => {
                if let Some(val) = entries
                    .first()
                    .and_then(|e| e.args.first())
                    .and_then(|v: &ServerConfigurationValue| v.as_number())
                {
                    if val > 0 {
                        health_check_config.consecutive_passes = val as u64;
                    }
                }
            }
            "no_verification" => {
                if let Some(val) = entries.first().map(|e| e.get_flag()) {
                    health_check_config.no_verification = val;
                }
            }
            _ => {}
        }
    }

    Ok(())
}

pub(super) fn parse_retry_budget(
    entries: &ServerConfigurationBlock,
    retry_budget_config: &mut RetryBudgetConfig,
) -> Result<(), Box<dyn Error + Send + Sync>> {
    for (name, entries) in entries.directives.iter() {
        match name.as_str() {
            "max_retry_rate" => {
                if let Some(val) = entries
                    .first()
                    .and_then(|e| e.args.first())
                    .and_then(|v| v.as_number())
                {
                    let rate = val as f64 / 100.0;
                    if (0.0..=1.0).contains(&rate) {
                        retry_budget_config.max_retry_rate = rate;
                    }
                } else if let Some(val) = entries.first().and_then(|e| e.args.first()) {
                    if let Some(rate) = val.as_float() {
                        if (0.0..=1.0).contains(&rate) {
                            retry_budget_config.max_retry_rate = rate;
                        }
                    }
                }
            }
            "max_tokens" => {
                if let Some(val) = entries
                    .first()
                    .and_then(|e| e.args.first())
                    .and_then(|v: &ServerConfigurationValue| v.as_number())
                {
                    if val > 0 {
                        retry_budget_config.max_tokens = val as u64;
                    }
                }
            }
            "refill_rate" => {
                if let Some(val) = entries
                    .first()
                    .and_then(|e| e.args.first())
                    .and_then(|v| v.as_number())
                {
                    if val >= 0 {
                        retry_budget_config.refill_rate = val as f64;
                    }
                } else if let Some(val) = entries.first().and_then(|e| e.args.first()) {
                    if let Some(rate) = val.as_float() {
                        if rate >= 0.0 {
                            retry_budget_config.refill_rate = rate;
                        }
                    }
                }
            }
            _ => {}
        }
    }

    Ok(())
}

pub(super) fn parse_circuit_breaker(
    entries: &ServerConfigurationBlock,
    circuit_breaker_config: &mut CircuitBreakerConfig,
) -> Result<(), Box<dyn Error + Send + Sync>> {
    for (name, entries) in entries.directives.iter() {
        match name.as_str() {
            "max_fails" => {
                if let Some(val) = entries
                    .first()
                    .and_then(|e| e.args.first())
                    .and_then(|v: &ServerConfigurationValue| v.as_number())
                {
                    if val > 0 {
                        circuit_breaker_config.max_fails = val as u64;
                    }
                }
            }
            "window" => {
                if let Some(val) = entries
                    .first()
                    .and_then(|e| e.args.first())
                    .and_then(|v| v.as_duration())
                {
                    circuit_breaker_config.window = val;
                }
            }
            "open_duration" => {
                if let Some(val) = entries
                    .first()
                    .and_then(|e| e.args.first())
                    .and_then(|v| v.as_duration())
                {
                    circuit_breaker_config.open_duration = val;
                }
            }
            "consecutive_passes" => {
                if let Some(val) = entries
                    .first()
                    .and_then(|e| e.args.first())
                    .and_then(|v: &ServerConfigurationValue| v.as_number())
                {
                    if val > 0 {
                        circuit_breaker_config.consecutive_passes = val as u64;
                    }
                }
            }
            "record_5xx" => {
                if let Some(val) = entries.first().map(|e| e.get_flag()) {
                    circuit_breaker_config.record_5xx = val;
                }
            }
            "latency_threshold" => {
                if let Some(val) = entries
                    .first()
                    .and_then(|e| e.args.first())
                    .and_then(|v| v.as_duration())
                {
                    circuit_breaker_config.latency_threshold = Some(val);
                }
            }
            "flapping_transitions" => {
                if let Some(val) = entries
                    .first()
                    .and_then(|e| e.args.first())
                    .and_then(|v: &ServerConfigurationValue| v.as_number())
                {
                    if val > 0 {
                        circuit_breaker_config.flapping_transitions = val as u64;
                    }
                }
            }
            "flapping_window" => {
                if let Some(val) = entries
                    .first()
                    .and_then(|e| e.args.first())
                    .and_then(|v| v.as_duration())
                {
                    circuit_breaker_config.flapping_window = val;
                }
            }
            "slow_start" => {
                if let Some(val) = entries
                    .first()
                    .and_then(|e| e.args.first())
                    .and_then(|v| v.as_duration())
                {
                    circuit_breaker_config.slow_start_duration = val;
                }
            }
            _ => {}
        }
    }

    Ok(())
}
