use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Serialize, Deserialize, Clone, Default)]
pub struct PartialConfig {
    pub model_path: Option<String>,
    pub backend: Option<String>,
    pub ctx_size: Option<u32>,
    pub temperature: Option<f32>,
    pub seed: Option<u32>,
    pub gpu_layers: Option<u32>,
    pub threads: Option<i32>,
    pub sandbox_policy: Option<String>,
    pub allowed_tools: Option<Vec<String>>,
    pub denied_tools: Option<Vec<String>>,
    pub chat_template: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct DoheeConfig {
    pub model_path: Option<PathBuf>,
    pub backend: String,
    pub ctx_size: u32,
    pub temperature: f32,
    pub seed: u32,
    pub gpu_layers: u32,
    pub threads: Option<i32>,
    pub sandbox_policy: String,
    pub allowed_tools: Option<Vec<String>>,
    pub denied_tools: Option<Vec<String>>,
    pub chat_template: Option<String>,
}

impl Default for DoheeConfig {
    fn default() -> Self {
        Self {
            model_path: None,
            backend: "vulkan".to_string(),
            ctx_size: 8192,
            temperature: 0.2,
            seed: 1234,
            gpu_layers: 99,
            threads: None,
            sandbox_policy: "WorkspaceWrite".to_string(),
            allowed_tools: None,
            denied_tools: None,
            chat_template: None,
        }
    }
}

impl DoheeConfig {
    pub fn merge(&mut self, other: PartialConfig) {
        if let Some(mp) = other.model_path {
            self.model_path = Some(PathBuf::from(mp));
        }
        if let Some(b) = other.backend {
            self.backend = b;
        }
        if let Some(c) = other.ctx_size {
            self.ctx_size = c;
        }
        if let Some(t) = other.temperature {
            self.temperature = t;
        }
        if let Some(s) = other.seed {
            self.seed = s;
        }
        if let Some(g) = other.gpu_layers {
            self.gpu_layers = g;
        }
        if let Some(th) = other.threads {
            self.threads = Some(th);
        }
        if let Some(sp) = other.sandbox_policy {
            self.sandbox_policy = sp;
        }
        if other.allowed_tools.is_some() {
            self.allowed_tools = other.allowed_tools;
        }
        if other.denied_tools.is_some() {
            self.denied_tools = other.denied_tools;
        }
        if let Some(ct) = other.chat_template {
            self.chat_template = Some(ct);
        }
    }

    pub fn load_layered(
        global_path: Option<&Path>,
        local_path: Option<&Path>,
        cli_overrides: PartialConfig,
    ) -> Result<Self> {
        let mut config = DoheeConfig::default();

        // 1. Load global config
        if let Some(g_path) = global_path {
            if g_path.exists() {
                let content = fs::read_to_string(g_path)
                    .with_context(|| format!("Failed to read global config: {}", g_path.display()))?;
                let partial: PartialConfig = toml::from_str(&content)
                    .with_context(|| format!("Failed to parse global config: {}", g_path.display()))?;
                config.merge(partial);
            }
        }

        // 2. Load local config
        if let Some(l_path) = local_path {
            if l_path.exists() {
                let content = fs::read_to_string(l_path)
                    .with_context(|| format!("Failed to read local config: {}", l_path.display()))?;
                let partial: PartialConfig = toml::from_str(&content)
                    .with_context(|| format!("Failed to parse local config: {}", l_path.display()))?;
                config.merge(partial);
            }
        }

        // 3. Merge CLI overrides
        config.merge(cli_overrides);

        Ok(config)
    }

    pub fn validate(&self) -> Result<()> {
        if let Some(ref path) = self.model_path {
            if !path.exists() {
                anyhow::bail!(
                    "Config validation failed: Model path '{}' does not exist.",
                    path.display()
                );
            }
            if !path.is_file() {
                anyhow::bail!(
                    "Config validation failed: Model path '{}' is not a file.",
                    path.display()
                );
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::NamedTempFile;

    #[test]
    fn test_merge_config() {
        let mut config = DoheeConfig::default();
        let partial = PartialConfig {
            model_path: Some("custom/path.gguf".to_string()),
            backend: Some("cpu".to_string()),
            ctx_size: Some(4096),
            ..Default::default()
        };

        config.merge(partial);

        assert_eq!(config.model_path, Some(PathBuf::from("custom/path.gguf")));
        assert_eq!(config.backend, "cpu");
        assert_eq!(config.ctx_size, 4096);
        assert_eq!(config.temperature, 0.2); // untouched
    }

    #[test]
    fn test_load_layered() -> Result<()> {
        let mut global_file = NamedTempFile::new()?;
        let mut local_file = NamedTempFile::new()?;

        write!(global_file, "backend = \"cuda\"\ntemperature = 0.5\n")?;
        write!(local_file, "temperature = 0.7\nctx_size = 1024\n")?;

        let cli = PartialConfig {
            ctx_size: Some(8192),
            ..Default::default()
        };

        let config = DoheeConfig::load_layered(
            Some(global_file.path()),
            Some(local_file.path()),
            cli,
        )?;

        assert_eq!(config.backend, "cuda");
        assert_eq!(config.temperature, 0.7); // local overrides global
        assert_eq!(config.ctx_size, 8192);   // cli overrides local

        Ok(())
    }

    #[test]
    fn test_validate_nonexistent_model() {
        let config = DoheeConfig {
            model_path: Some(PathBuf::from("nonexistent_model_file.gguf")),
            ..DoheeConfig::default()
        };

        assert!(config.validate().is_err());
    }
}
