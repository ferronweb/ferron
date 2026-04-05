//! HTTP static file serving stages

mod listing;
mod r#static;

pub use listing::DirectoryListingStage;
pub use r#static::StaticFileStage;
