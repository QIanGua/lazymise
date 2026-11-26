pub mod schema;

use std::path::Path;
use std::process::Command;

use anyhow::{Context, Result, bail};

use self::schema::{MiseEnvMap, MiseTask, MiseToolsMap};

#[derive(Debug, Clone)]
pub struct MiseClient {
    workdir: std::path::PathBuf,
}

impl MiseClient {
    pub fn new(workdir: impl AsRef<Path>) -> Self {
        Self {
            workdir: workdir.as_ref().to_path_buf(),
        }
    }

    pub fn workdir(&self) -> &Path {
        &self.workdir
    }

    pub fn list_tasks(&self) -> Result<Vec<MiseTask>> {
        let out = self.run_json(&["tasks", "--json"])?;
        if out.trim().is_empty() {
            return Ok(Vec::new());
        }
        let mut tasks: Vec<MiseTask> =
            serde_json::from_str(&out).context("failed to parse `mise tasks --json`")?;
        tasks.retain(|t| !t.hide);
        Ok(tasks)
    }

    pub fn list_tools(&self) -> Result<MiseToolsMap> {
        let out = self.run_json(&["ls", "--json"])?;
        if out.trim().is_empty() {
            return Ok(MiseToolsMap::new());
        }
        serde_json::from_str(&out).context("failed to parse `mise ls --json`")
    }

    pub fn list_env(&self) -> Result<MiseEnvMap> {
        let out = self.run_json(&["env", "--json"])?;
        if out.trim().is_empty() {
            return Ok(MiseEnvMap::new());
        }
        serde_json::from_str(&out).context("failed to parse `mise env --json`")
    }

    fn run_json(&self, args: &[&str]) -> Result<String> {
        let output = Command::new("mise")
            .args(args)
            .current_dir(&self.workdir)
            .output()
            .with_context(|| format!("failed to execute `mise {}`", args.join(" ")))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            bail!(
                "`mise {}` exited with {}: {}",
                args.join(" "),
                output.status,
                stderr.trim()
            );
        }

        String::from_utf8(output.stdout).context("mise output was not valid UTF-8")
    }
}

#[cfg(test)]
mod tests {
    use super::schema::{MiseEnvMap, MiseTask, MiseToolsMap};

    #[test]
    fn parses_real_mise_tasks_json() {
        let raw = r#"[
          {
            "name": "exp:01-baseline",
            "description": "Run baseline",
            "source": "/tmp/demo/.mise/tasks/exp/01-baseline",
            "depends": ["data:prep", ["lint", "--fast"]],
            "dir": "/tmp/demo/experiments/01-baseline",
            "hide": false,
            "global": false,
            "sources": ["README.md"],
            "outputs": ["../../outputs/01-baseline/summary.json"],
            "run": []
          },
          {
            "name": "info",
            "description": "Show env",
            "source": "/Users/test/.config/mise/config.toml",
            "global": true,
            "run": ["echo hi"]
          }
        ]"#;

        let tasks: Vec<MiseTask> = serde_json::from_str(raw).unwrap();
        assert_eq!(tasks.len(), 2);
        assert_eq!(tasks[0].group_key(), "exp");
        assert_eq!(tasks[0].short_name(), "01-baseline");
        assert_eq!(tasks[0].depends_display(), vec!["data:prep", "lint --fast"]);
        assert_eq!(
            tasks[0].outputs_display(),
            vec!["../../outputs/01-baseline/summary.json"]
        );
        assert_eq!(tasks[1].group_key(), "[global]");
    }

    #[test]
    fn parses_tools_and_env_json() {
        let tools_raw = r#"{
          "rust": [{
            "version": "1.90.0",
            "requested_version": "stable",
            "installed": true,
            "active": true,
            "source": {"type": "mise.toml", "path": "/tmp/mise.toml"}
          }]
        }"#;
        let tools: MiseToolsMap = serde_json::from_str(tools_raw).unwrap();
        assert_eq!(tools["rust"][0].version, "1.90.0");

        let env_raw = r#"{"PATH": "/usr/bin:/bin", "API_TOKEN": "secret"}"#;
        let env: MiseEnvMap = serde_json::from_str(env_raw).unwrap();
        assert_eq!(env["API_TOKEN"], "secret");
    }
}
