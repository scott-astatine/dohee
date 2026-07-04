use anyhow::{Context, Result};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::fs;
use std::path::Path;
use std::process::Command;
use std::sync::Arc;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolOutput {
    pub content: String,
    pub truncated: bool,
    pub full_length: usize,
}

impl ToolOutput {
    pub fn new(content: String) -> Self {
        let len = content.len();
        Self {
            content,
            truncated: false,
            full_length: len,
        }
    }

    pub fn new_truncated(content: String, full_length: usize) -> Self {
        Self {
            content,
            truncated: true,
            full_length,
        }
    }
}

#[async_trait]
pub trait Tool: Send + Sync {
    fn name(&self) -> &'static str;
    fn description(&self) -> &'static str;
    fn parameters_schema(&self) -> Value;
    async fn execute(&self, args: Value) -> Result<ToolOutput>;
}

pub struct ToolRegistry {
    tools: HashMap<String, Arc<dyn Tool>>,
}

impl ToolRegistry {
    pub fn new() -> Self {
        Self {
            tools: HashMap::new(),
        }
    }

    pub fn register(&mut self, tool: Arc<dyn Tool>) {
        self.tools.insert(tool.name().to_string(), tool);
    }

    pub fn get(&self, name: &str) -> Option<&Arc<dyn Tool>> {
        self.tools.get(name)
    }

    pub fn list(&self) -> Vec<Arc<dyn Tool>> {
        self.tools.values().cloned().collect()
    }
}

impl Default for ToolRegistry {
    fn default() -> Self {
        Self::new()
    }
}

// 1. Read File Tool
pub struct ReadFileTool;

#[async_trait]
impl Tool for ReadFileTool {
    fn name(&self) -> &'static str {
        "read_file"
    }

    fn description(&self) -> &'static str {
        "Reads the contents of a file from disk."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "Absolute path or relative path to the file to read."
                }
            },
            "required": ["path"]
        })
    }

    async fn execute(&self, args: Value) -> Result<ToolOutput> {
        let path_str = args["path"].as_str().context("Missing or invalid 'path' argument")?;
        let path = Path::new(path_str);

        if !path.exists() {
            return Ok(ToolOutput::new(format!("Error: File '{}' does not exist.", path_str)));
        }
        if !path.is_file() {
            return Ok(ToolOutput::new(format!("Error: '{}' is not a file.", path_str)));
        }

        let content = fs::read_to_string(path)
            .with_context(|| format!("Failed to read file '{}'", path_str))?;

        let limit = 10000;
        if content.len() > limit {
            let truncated = content[..limit].to_string();
            Ok(ToolOutput::new_truncated(truncated, content.len()))
        } else {
            Ok(ToolOutput::new(content))
        }
    }
}

// 2. Write File Tool
pub struct WriteFileTool;

#[async_trait]
impl Tool for WriteFileTool {
    fn name(&self) -> &'static str {
        "write_file"
    }

    fn description(&self) -> &'static str {
        "Creates a new file or overwrites an existing one with new content."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "Absolute or relative path of the file to write."
                },
                "content": {
                    "type": "string",
                    "description": "The file content to write."
                }
            },
            "required": ["path", "content"]
        })
    }

    async fn execute(&self, args: Value) -> Result<ToolOutput> {
        let path_str = args["path"].as_str().context("Missing or invalid 'path' argument")?;
        let content = args["content"].as_str().context("Missing or invalid 'content' argument")?;
        let path = Path::new(path_str);

        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("Failed to create parent directories for '{}'", path_str))?;
        }

        fs::write(path, content)
            .with_context(|| format!("Failed to write file '{}'", path_str))?;

        Ok(ToolOutput::new(format!("Success: File '{}' written successfully.", path_str)))
    }
}

// 3. Edit File Tool (Find & Replace)
pub struct EditFileTool;

#[async_trait]
impl Tool for EditFileTool {
    fn name(&self) -> &'static str {
        "edit_file"
    }

    fn description(&self) -> &'static str {
        "Replaces a specific block of text in a file with new content."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "Path to the file to edit."
                },
                "find": {
                    "type": "string",
                    "description": "The exact block of text to locate. Must match exactly."
                },
                "replace": {
                    "type": "string",
                    "description": "The replacement content."
                }
            },
            "required": ["path", "find", "replace"]
        })
    }

    async fn execute(&self, args: Value) -> Result<ToolOutput> {
        let path_str = args["path"].as_str().context("Missing or invalid 'path' argument")?;
        let find = args["find"].as_str().context("Missing or invalid 'find' argument")?;
        let replace = args["replace"].as_str().context("Missing or invalid 'replace' argument")?;
        let path = Path::new(path_str);

        if !path.exists() {
            return Ok(ToolOutput::new(format!("Error: File '{}' does not exist.", path_str)));
        }

        let content = fs::read_to_string(path)
            .with_context(|| format!("Failed to read file '{}'", path_str))?;

        let count = content.matches(find).count();
        if count == 0 {
            return Ok(ToolOutput::new("Error: The 'find' text block was not found in the file.".to_string()));
        }
        if count > 1 {
            return Ok(ToolOutput::new(format!("Error: The 'find' block is ambiguous (found {} occurrences).", count)));
        }

        let updated = content.replace(find, replace);
        fs::write(path, updated)
            .with_context(|| format!("Failed to save edited file '{}'", path_str))?;

        Ok(ToolOutput::new(format!("Success: File '{}' edited successfully.", path_str)))
    }
}

// 4. List Directory Tool
pub struct ListDirTool;

#[async_trait]
impl Tool for ListDirTool {
    fn name(&self) -> &'static str {
        "list_dir"
    }

    fn description(&self) -> &'static str {
        "Lists files and directories inside a directory."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "The directory path to list. Defaults to '.' if empty."
                }
            }
        })
    }

    async fn execute(&self, args: Value) -> Result<ToolOutput> {
        let path_str = args["path"].as_str().unwrap_or(".");
        let path = Path::new(path_str);

        if !path.exists() {
            return Ok(ToolOutput::new(format!("Error: Directory '{}' does not exist.", path_str)));
        }
        if !path.is_dir() {
            return Ok(ToolOutput::new(format!("Error: '{}' is not a directory.", path_str)));
        }

        let entries = fs::read_dir(path)
            .with_context(|| format!("Failed to read directory '{}'", path_str))?;

        let mut output = String::new();
        output.push_str(&format!("Directory list for '{}':\n", path_str));
        for entry in entries {
            let entry = entry?;
            let name = entry.file_name().to_string_lossy().into_owned();
            let meta = entry.metadata()?;
            let type_str = if meta.is_dir() { "DIR " } else { "FILE" };
            output.push_str(&format!("  [{}] {} ({} bytes)\n", type_str, name, meta.len()));
        }

        Ok(ToolOutput::new(output))
    }
}

// 5. Grep Tool (Search Directory)
pub struct GrepTool;

#[async_trait]
impl Tool for GrepTool {
    fn name(&self) -> &'static str {
        "grep"
    }

    fn description(&self) -> &'static str {
        "Searches files recursively in a directory for a specific text pattern."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "pattern": {
                    "type": "string",
                    "description": "The text pattern or search string."
                },
                "path": {
                    "type": "string",
                    "description": "The directory to search. Defaults to '.' if empty."
                }
            },
            "required": ["pattern"]
        })
    }

    async fn execute(&self, args: Value) -> Result<ToolOutput> {
        let pattern = args["pattern"].as_str().context("Missing or invalid 'pattern' argument")?;
        let path_str = args["path"].as_str().unwrap_or(".");
        let path = Path::new(path_str);

        if !path.exists() {
            return Ok(ToolOutput::new(format!("Error: Path '{}' does not exist.", path_str)));
        }

        let mut results = String::new();
        let mut match_count = 0;

        fn walk(pattern: &str, dir: &Path, results: &mut String, match_count: &mut usize) -> Result<()> {
            if *match_count > 100 {
                return Ok(());
            }
            if dir.is_dir() {
                for entry in fs::read_dir(dir)? {
                    let entry = entry?;
                    let entry_path = entry.path();
                    let name = entry_path.file_name().unwrap_or_default().to_string_lossy();
                    // Skip binary/large directories
                    if name == "target" || name == ".git" || name == "node_modules" {
                        continue;
                    }
                    walk(pattern, &entry_path, results, match_count)?;
                }
            } else if dir.is_file() {
                if let Ok(content) = fs::read_to_string(dir) {
                    for (line_num, line) in content.lines().enumerate() {
                        if line.contains(pattern) {
                            results.push_str(&format!("{}:{}: {}\n", dir.display(), line_num + 1, line));
                            *match_count += 1;
                            if *match_count > 100 {
                                results.push_str("... [Too many matches, truncating]\n");
                                break;
                            }
                        }
                    }
                }
            }
            Ok(())
        }

        walk(pattern, path, &mut results, &mut match_count)?;

        if results.is_empty() {
            Ok(ToolOutput::new("No matches found.".to_string()))
        } else {
            Ok(ToolOutput::new(results))
        }
    }
}

// 6. Run Shell Tool
pub struct RunShellTool {
    pub policy: dohee_sandbox::SandboxPolicy,
}

impl RunShellTool {
    pub fn new(policy: dohee_sandbox::SandboxPolicy) -> Self {
        Self { policy }
    }
}

#[async_trait]
impl Tool for RunShellTool {
    fn name(&self) -> &'static str {
        "run_shell"
    }

    fn description(&self) -> &'static str {
        "Executes a command in a system shell. Target is Linux (sh)."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "command": {
                    "type": "string",
                    "description": "The command string to execute."
                },
                "cwd": {
                    "type": "string",
                    "description": "Optional working directory for execution. Defaults to current directory."
                }
            },
            "required": ["command"]
        })
    }

    async fn execute(&self, args: Value) -> Result<ToolOutput> {
        let command_str = args["command"].as_str().context("Missing or invalid 'command' argument")?;
        let cwd_str = args["cwd"].as_str();

        let mut cmd = Command::new("sh");
        cmd.arg("-c").arg(command_str);

        if let Some(cwd) = cwd_str {
            cmd.current_dir(Path::new(cwd));
        }

        #[cfg(target_os = "linux")]
        {
            use std::os::unix::process::CommandExt;
            let policy = self.policy.clone();
            unsafe {
                cmd.pre_exec(move || {
                    if let Err(e) = dohee_sandbox::Sandbox::apply(&policy) {
                        eprintln!("Landlock sandbox application failed: {:?}", e);
                        return Err(std::io::Error::new(std::io::ErrorKind::PermissionDenied, e));
                    }
                    Ok(())
                });
            }
        }

        let output = cmd.output().with_context(|| format!("Failed to spawn shell for command '{}'", command_str))?;

        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        let exit_code = output.status.code().unwrap_or(-1);

        let mut content = format!("Exit code: {}\n", exit_code);
        if !stdout.is_empty() {
            content.push_str(&format!("Stdout:\n{}", stdout));
        }
        if !stderr.is_empty() {
            content.push_str(&format!("Stderr:\n{}", stderr));
        }

        let limit = 6000;
        if content.len() > limit {
            let truncated = content[..limit].to_string();
            Ok(ToolOutput::new_truncated(truncated, content.len()))
        } else {
            Ok(ToolOutput::new(content))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[tokio::test]
    async fn test_read_write_delete_flow() -> Result<()> {
        let dir = tempdir()?;
        let file_path = dir.path().join("test.txt");

        let write_tool = WriteFileTool;
        let write_args = json!({
            "path": file_path.to_string_lossy(),
            "content": "Hello World!"
        });
        let write_res = write_tool.execute(write_args).await?;
        assert!(write_res.content.contains("Success"));

        let read_tool = ReadFileTool;
        let read_args = json!({
            "path": file_path.to_string_lossy()
        });
        let read_res = read_tool.execute(read_args).await?;
        assert_eq!(read_res.content, "Hello World!");

        Ok(())
    }

    #[tokio::test]
    async fn test_edit_file() -> Result<()> {
        let dir = tempdir()?;
        let file_path = dir.path().join("edit_test.txt");
        fs::write(&file_path, "apple banana cherry")?;

        let edit_tool = EditFileTool;
        let edit_args = json!({
            "path": file_path.to_string_lossy(),
            "find": "banana",
            "replace": "orange"
        });
        let edit_res = edit_tool.execute(edit_args).await?;
        assert!(edit_res.content.contains("Success"));

        let updated = fs::read_to_string(file_path)?;
        assert_eq!(updated, "apple orange cherry");

        Ok(())
    }

    #[tokio::test]
    async fn test_run_shell() -> Result<()> {
        let tool = RunShellTool::new(dohee_sandbox::SandboxPolicy::DangerFullAccess);
        let args = json!({
            "command": "echo 'Hello Shell'"
        });
        let res = tool.execute(args).await?;
        assert!(res.content.contains("Hello Shell"));
        assert!(res.content.contains("Exit code: 0"));

        Ok(())
    }
}
