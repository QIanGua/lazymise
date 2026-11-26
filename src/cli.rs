use std::path::PathBuf;

use clap::{Parser, ValueEnum};

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum StartTab {
    Tasks,
    Tools,
    Env,
    Templates,
}

#[derive(Debug, Parser)]
#[command(name = "lazymise", version, about = "A Vim-first terminal workbench for mise")]
pub struct Cli {
    /// Working directory to inspect (defaults to current directory)
    #[arg(short = 'C', long = "dir")]
    pub dir: Option<PathBuf>,

    /// Initial tab to open
    #[arg(short, long, value_enum, default_value_t = StartTab::Tasks)]
    pub tab: StartTab,
}
