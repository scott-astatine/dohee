use anyhow::{Context, Result};
use std::sync::Arc;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct Message {
    pub role: String, // "system", "user", "assistant", "tool"
    pub content: String,
    pub name: Option<String>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Session {
    pub messages: Vec<Message>,
    pub created_at: u64,
    pub compaction_generation: u32,
}

pub const DEFAULT_CHAT_TEMPLATE: &str = "{%- for message in messages -%}\n<|im_start|>{{ message.role }}\n{{ message.content }}<|im_end|>\n{%- endfor -%}\n{%- if add_generation_prompt -%}\n<|im_start|>assistant\n{%- endif -%}";

#[derive(Debug, Clone)]
pub enum PromptTemplate {
    Embedded(String),
    External(std::path::PathBuf),
    Builtin(String),
}

pub trait PromptRenderer: Send + Sync {
    fn render(&self, messages: &[Message], add_generation_prompt: bool) -> Result<String>;
}

pub struct JinjaRenderer {
    _template_str: String,
    env: Arc<minijinja::Environment<'static>>,
}

impl JinjaRenderer {
    pub fn new(source: PromptTemplate) -> Result<Self> {
        let template_str = match source {
            PromptTemplate::Embedded(tpl) => tpl,
            PromptTemplate::External(path) => {
                std::fs::read_to_string(&path)
                    .with_context(|| format!("Failed to read external template from {:?}", path))?
            }
            PromptTemplate::Builtin(name) => {
                if name == "chatml" {
                    DEFAULT_CHAT_TEMPLATE.to_string()
                } else {
                    anyhow::bail!("Unknown built-in template name: {}", name);
                }
            }
        };

        let mut env = minijinja::Environment::new();
        
        // Register standard custom filters with accurate semantics
        env.add_filter("strip_thinking", |val: String| -> String {
            let re = regex::Regex::new(r"(?s)<thinking>.*?</thinking>").unwrap();
            re.replace_all(&val, "").into_owned()
        });
        
        env.add_filter("raise_exception", |msg: String| -> Result<String, minijinja::Error> {
            Err(minijinja::Error::new(minijinja::ErrorKind::InvalidOperation, msg))
        });

        env.add_template_owned("chat".to_string(), template_str.clone())
            .context("Failed to compile Jinja template")?;

        Ok(Self {
            _template_str: template_str,
            env: Arc::new(env),
        })
    }
}

impl PromptRenderer for JinjaRenderer {
    fn render(&self, messages: &[Message], add_generation_prompt: bool) -> Result<String> {
        let tmpl = self.env.get_template("chat")
            .context("Jinja chat template not compiled in env")?;

        let mut processed: Vec<serde_json::Value> = Vec::new();
        for m in messages {
            let role = if m.role == "tool" { "user" } else { &m.role };
            let content = if m.role == "tool" {
                format!("[Tool Output ({}): {}]", m.name.as_deref().unwrap_or("unknown"), m.content)
            } else {
                m.content.clone()
            };

            if let Some(last) = processed.last_mut() {
                if last["role"] == role {
                    let old_content = last["content"].as_str().unwrap_or("");
                    last["content"] = serde_json::json!(format!("{}\n\n{}", old_content, content));
                    continue;
                }
            }

            processed.push(serde_json::json!({
                "role": role,
                "content": content,
            }));
        }

        let rendered = tmpl.render(minijinja::context! {
            messages => processed,
            add_generation_prompt => add_generation_prompt,
        }).context("Failed to render prompt template via minijinja")?;

        Ok(rendered)
    }
}
