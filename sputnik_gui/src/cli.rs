use std::path::PathBuf;

use clap::Parser;

#[derive(Debug, Parser)]
#[command(name = "sputnik", version, about = "A text editor")]
pub struct Cli {
    /// File to open in the editor
    pub file: Option<PathBuf>,
}
