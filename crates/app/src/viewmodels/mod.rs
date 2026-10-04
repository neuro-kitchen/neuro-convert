//! One view model per screen: subscribes to the store events it shows, keeps display-ready
//! state, and offers the screen's commands. Pure functions build the display state, so it is
//! tested without a window.

pub mod contents;
pub mod convert;
pub mod metadata;
pub mod nav;
pub mod plan;
pub mod preview;
pub mod source;

pub use contents::ContentsVm;
pub use convert::ConvertVm;
pub use metadata::MetadataVm;
pub use nav::NavVm;
pub use plan::PlanVm;
pub use preview::PreviewVm;
pub use source::SourceVm;
