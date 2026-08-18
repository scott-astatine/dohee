use anyhow::{Context, Result};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::fs;
use std::path::Path;
use std::process::Command;
use std::sync::Arc;
use std::time::Duration;

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

#[derive(Clone)]
pub struct ToolRegistry {
    tools: std::collections::HashMap<String, Arc<dyn Tool>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentMode {
    Build,
    Plan,
    Explore,
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

    pub fn for_mode(&self, mode: AgentMode) -> Self {
        let mut filtered = Self::new();
        for (name, tool) in &self.tools {
            match mode {
                AgentMode::Build => {
                    filtered.register(Arc::clone(tool));
                }
                AgentMode::Plan => {
                    if name != "write_file" && name != "edit_file" && name != "run_shell" {
                        filtered.register(Arc::clone(tool));
                    }
                }
                AgentMode::Explore => {
                    if name == "read_file" || name == "list_dir" || name == "grep" || name == "find_definition" || name == "list_symbols" {
                        filtered.register(Arc::clone(tool));
                    }
                }
            }
        }
        filtered
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
        "Searches files recursively in a directory for a specific text pattern with glob filtering."
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
                },
                "include_glob": {
                    "type": "string",
                    "description": "Optional glob pattern to include matching files (e.g. '*.rs')."
                },
                "exclude_glob": {
                    "type": "string",
                    "description": "Optional glob pattern to exclude matching files (e.g. '*test*')."
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

        let include_matcher = if let Some(g) = args["include_glob"].as_str() {
            globset::Glob::new(g).ok().map(|glob| glob.compile_matcher())
        } else {
            None
        };

        let exclude_matcher = if let Some(g) = args["exclude_glob"].as_str() {
            globset::Glob::new(g).ok().map(|glob| glob.compile_matcher())
        } else {
            None
        };

        let mut results = String::new();
        let mut match_count = 0;
        let max_matches = 100;

        for entry in ignore::WalkBuilder::new(path).hidden(false).parents(true).build() {
            let entry = match entry {
                Ok(e) => e,
                Err(_) => continue,
            };
            let entry_path = entry.path();
            if entry_path.is_file() {
                if let Some(ref inc) = include_matcher {
                    if !inc.is_match(entry_path) {
                        continue;
                    }
                }
                if let Some(ref exc) = exclude_matcher {
                    if exc.is_match(entry_path) {
                        continue;
                    }
                }

                if let Ok(content) = fs::read_to_string(entry_path) {
                    for (line_num, line) in content.lines().enumerate() {
                        if line.contains(pattern) {
                            results.push_str(&format!("{}:{}: {}\n", entry_path.display(), line_num + 1, line));
                            match_count += 1;
                            if match_count >= max_matches {
                                results.push_str(&format!("\n... [{} matches reached, remaining matches not shown. Narrow your search with a glob filter]\n", max_matches));
                                break;
                            }
                        }
                    }
                }
            }
            if match_count >= max_matches {
                break;
            }
        }

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

// 7. Find Definition Tool (AST via tree-sitter)
pub struct FindDefinitionTool;

#[async_trait]
impl Tool for FindDefinitionTool {
    fn name(&self) -> &'static str {
        "find_definition"
    }

    fn description(&self) -> &'static str {
        "Locates the AST definition of a symbol (struct, function, trait, enum) across source files."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "symbol": {
                    "type": "string",
                    "description": "The exact identifier or symbol name to find."
                },
                "path": {
                    "type": "string",
                    "description": "Directory or file to search. Defaults to '.' if empty."
                }
            },
            "required": ["symbol"]
        })
    }

    async fn execute(&self, args: Value) -> Result<ToolOutput> {
        let symbol = args["symbol"].as_str().context("Missing or invalid 'symbol' argument")?;
        let path_str = args["path"].as_str().unwrap_or(".");
        let path = Path::new(path_str);

        let mut parser = tree_sitter::Parser::new();
        let language = tree_sitter_rust::language();
        parser.set_language(&language).context("Error loading Rust grammar")?;

        let mut results = String::new();
        let mut count = 0;

        for entry in ignore::WalkBuilder::new(path).hidden(false).parents(false).build() {
            let entry = match entry {
                Ok(e) => e,
                Err(_) => continue,
            };
            let entry_path = entry.path();
            if entry_path.is_file() && entry_path.extension().map_or(false, |ext| ext == "rs") {
                if let Ok(source) = fs::read_to_string(entry_path) {
                    if let Some(tree) = parser.parse(&source, None) {
                        let root = tree.root_node();
                        fn walk_def(node: tree_sitter::Node, source: &str, symbol: &str, entry_path: &Path, results: &mut String, count: &mut usize) {
                            let kind = node.kind();
                            if kind == "struct_item" || kind == "function_item" || kind == "fn_item" || kind == "enum_item" || kind == "trait_item" {
                                if let Some(name_node) = node.child_by_field_name("name") {
                                    let name = &source[name_node.start_byte()..name_node.end_byte()];
                                    if name == symbol {
                                        let range = node.range();
                                        results.push_str(&format!(
                                            "{}:{}:{}: Definition of '{}'\n",
                                            entry_path.display(),
                                            range.start_point.row + 1,
                                            range.start_point.column + 1,
                                            symbol
                                        ));
                                        *count += 1;
                                    }
                                }
                            }
                            let mut cursor = node.walk();
                            for child in node.children(&mut cursor) {
                                walk_def(child, source, symbol, entry_path, results, count);
                            }
                        }
                        walk_def(root, &source, symbol, entry_path, &mut results, &mut count);
                    }
                }
            }
        }

        if count == 0 {
            Ok(ToolOutput::new(format!("No AST definition found for symbol '{}'.", symbol)))
        } else {
            Ok(ToolOutput::new(results))
        }
    }
}

// 8. List Symbols Tool (AST via tree-sitter)
pub struct ListSymbolsTool;

#[async_trait]
impl Tool for ListSymbolsTool {
    fn name(&self) -> &'static str {
        "list_symbols"
    }

    fn description(&self) -> &'static str {
        "Lists top-level functions, structs, enums, and traits in a source file using AST parsing."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "Path to the source file to list symbols from."
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

        let source = fs::read_to_string(path)?;
        let mut parser = tree_sitter::Parser::new();
        let language = tree_sitter_rust::language();
        parser.set_language(&language).context("Error loading Rust grammar")?;

        let tree = parser.parse(&source, None).context("Failed to parse file AST")?;
        let root = tree.root_node();

        let mut output = String::new();
        output.push_str(&format!("AST Symbols in '{}':\n", path_str));

        let mut cursor = root.walk();
        for child in root.children(&mut cursor) {
            let kind = child.kind();
            if kind == "struct_item" || kind == "function_item" || kind == "fn_item" || kind == "enum_item" || kind == "trait_item" || kind == "impl_item" {
                let range = child.range();
                let snippet = source[child.start_byte()..child.end_byte()].lines().next().unwrap_or("");
                output.push_str(&format!("  Line {}: {}\n", range.start_point.row + 1, snippet));
            }
        }

        Ok(ToolOutput::new(output))
    }
}

// 9. Web Search Tool
pub struct WebSearchTool;

#[async_trait]
impl Tool for WebSearchTool {
    fn name(&self) -> &'static str {
        "web_search"
    }

    fn description(&self) -> &'static str {
        "Performs a web search using a metasearch engine (SearXNG / DuckDuckGo API)."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "query": {
                    "type": "string",
                    "description": "Search query string."
                }
            },
            "required": ["query"]
        })
    }

    async fn execute(&self, args: Value) -> Result<ToolOutput> {
        let query = args["query"].as_str().context("Missing or invalid 'query' argument")?;
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(10))
            .build()?;

        let url = format!("https://html.duckduckgo.com/html/?q={}", urlencoding::encode(query));
        let res = client.get(&url).header("User-Agent", "Mozilla/5.0 (X11; Linux x86_64)").send().await;

        match res {
            Ok(resp) => {
                let body = resp.text().await.unwrap_or_default();
                let markdown = html2md::parse_html(&body);
                let limit = 4000;
                if markdown.len() > limit {
                    Ok(ToolOutput::new_truncated(markdown[..limit].to_string(), markdown.len()))
                } else {
                    Ok(ToolOutput::new(markdown))
                }
            }
            Err(e) => Ok(ToolOutput::new(format!("Web search error: {}", e))),
        }
    }
}

// 10. Web Fetch Tool
pub struct WebFetchTool;

#[async_trait]
impl Tool for WebFetchTool {
    fn name(&self) -> &'static str {
        "web_fetch"
    }

    fn description(&self) -> &'static str {
        "Fetches a web page URL and converts HTML content into Markdown."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "url": {
                    "type": "string",
                    "description": "Target web page URL to fetch."
                }
            },
            "required": ["url"]
        })
    }

    async fn execute(&self, args: Value) -> Result<ToolOutput> {
        let url_str = args["url"].as_str().context("Missing or invalid 'url' argument")?;
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(15))
            .build()?;

        let res = client.get(url_str).header("User-Agent", "Mozilla/5.0 (X11; Linux x86_64)").send().await;

        match res {
            Ok(resp) => {
                let body = resp.text().await.unwrap_or_default();
                let markdown = html2md::parse_html(&body);
                let limit = 8000;
                if markdown.len() > limit {
                    Ok(ToolOutput::new_truncated(markdown[..limit].to_string(), markdown.len()))
                } else {
                    Ok(ToolOutput::new(markdown))
                }
            }
            Err(e) => Ok(ToolOutput::new(format!("Web fetch error: {}", e))),
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

    #[tokio::test]
    async fn test_ast_tools() -> Result<()> {
        let dir = tempdir()?;
        let file_path = dir.path().join("sample.rs");
        fs::write(&file_path, "pub struct SampleStruct;\npub fn sample_function() {}\n")?;

        let def_tool = FindDefinitionTool;
        let def_res = def_tool.execute(json!({
            "symbol": "SampleStruct",
            "path": dir.path().to_string_lossy()
        })).await?;
        assert!(def_res.content.contains("Definition of 'SampleStruct'"));

        let sym_tool = ListSymbolsTool;
        let sym_res = sym_tool.execute(json!({
            "path": file_path.to_string_lossy()
        })).await?;
        assert!(sym_res.content.contains("struct SampleStruct"));
        assert!(sym_res.content.contains("fn sample_function"));

        Ok(())
    }

    #[test]
    fn test_tool_registry_modes() {
        let mut reg = ToolRegistry::new();
        reg.register(Arc::new(ReadFileTool));
        reg.register(Arc::new(WriteFileTool));
        reg.register(Arc::new(EditFileTool));
        reg.register(Arc::new(ListDirTool));
        reg.register(Arc::new(GrepTool));

        let plan_reg = reg.for_mode(AgentMode::Plan);
        assert!(plan_reg.get("read_file").is_some());
        assert!(plan_reg.get("write_file").is_none());
        assert!(plan_reg.get("edit_file").is_none());

        let explore_reg = reg.for_mode(AgentMode::Explore);
        assert!(explore_reg.get("read_file").is_some());
        assert!(explore_reg.get("grep").is_some());
        assert!(explore_reg.get("write_file").is_none());
    }
}
