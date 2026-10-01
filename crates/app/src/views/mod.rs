//! Render-only screens, one per step. Each observes its view model(s), draws them with the
//! widgets, and sends user actions to the view model's commands. No data logic here.

pub mod contents;
pub mod metadata;
pub mod preview;
pub mod review;
pub mod source;

pub use contents::ContentsView;
pub use metadata::MetadataView;
pub use preview::PreviewView;
pub use review::ReviewView;
pub use source::SourceView;
