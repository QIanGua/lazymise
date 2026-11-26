use std::path::PathBuf;

use clap::{Parser, ValueEnum};

use crate::action::Panel;

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum StartTab {
    Tasks,
    Tools,
    Env,
    #[value(alias = "templates")]
    Config,
    Jobs,
}

impl From<StartTab> for Panel {
    fn from(v: StartTab) -> Self {
        match v {
            StartTab::Tasks => Panel::Tasks,
            StartTab::Tools => Panel::Tools,
            StartTab::Env => Panel::Env,
            StartTab::Config => Panel::Config,
            StartTab::Jobs => Panel::Jobs,
        }
    }
}

#[derive(Debug, Parser)]
#[command(
    name = "lazymise",
    version,
    about = "A Vim-first terminal workbench for mise"
)]
pub struct Cli {
    /// Working directory to inspect (defaults to current directory)
    #[arg(short = 'C', long = "dir")]
    pub dir: Option<PathBuf>,

    /// Initial MISE_ENV profile (e.g. dev, prod, or comma-separated)
    #[arg(short = 'E', long = "env")]
    pub env: Option<String>,

    /// Initial panel to focus
    #[arg(short, long, value_enum, default_value_t = StartTab::Tasks)]
    pub tab: StartTab,
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;

    use super::*;

    #[test]
    fn parses_env_dir_and_legacy_templates_tab_alias() {
        let cli = Cli::try_parse_from(["lazymise", "-C", "/tmp/demo", "-E", "dev", "--tab", "templates"])
            .unwrap();
        assert_eq!(cli.dir, Some(PathBuf::from("/tmp/demo")));
        assert_eq!(cli.env.as_deref(), Some("dev"));
        assert_eq!(Panel::from(cli.tab), Panel::Config);

        let jobs_cli = Cli::try_parse_from(["lazymise", "--tab", "jobs"]).unwrap();
        assert_eq!(Panel::from(jobs_cli.tab), Panel::Jobs);
    }
}
