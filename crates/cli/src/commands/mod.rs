pub mod convert;
pub mod formats;
pub mod inspect;

/// Input options shared by commands that open a recording.
#[derive(clap::Args, Debug, Default)]
pub struct OpenArgs {
    /// Container formats (TDT tank): the block to open
    #[arg(long)]
    pub block: Option<String>,
    /// Offline spike sort to apply to snippets (TDT: folder name under sort/)
    #[arg(long)]
    pub sort: Option<String>,
    /// Only load these streams / stores (by source name; repeat or comma-separate)
    #[arg(long, value_delimiter = ',')]
    pub only: Vec<String>,
}

impl OpenArgs {
    pub fn options(&self) -> nc_convert::OpenOptions {
        nc_convert::OpenOptions {
            block: self.block.clone(),
            sort: self.sort.clone(),
            only: (!self.only.is_empty()).then(|| self.only.clone()),
        }
    }
}
