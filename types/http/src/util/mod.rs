pub const CSS_COMMON: &str =
    include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/assets/common.css"));

pub mod anti_xss;
pub mod default_html_page;
pub mod parse_q_value_header;
