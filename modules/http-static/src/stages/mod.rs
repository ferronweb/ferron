//! HTTP static file serving stages

mod error_page;
mod listing;
mod r#static;

pub use error_page::ErrorPageStage;
pub use listing::DirectoryListingStage;
pub use r#static::StaticFileStage;
