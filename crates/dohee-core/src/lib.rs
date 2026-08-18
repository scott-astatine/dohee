use anyhow::{Context, Result};
use regex::Regex;
use std::io::Write;
use std::path::Path;
use std::sync::Arc;
use dohee_infer as do_infer;
pub use dohee_context::{Message, Session};

#[derive(Debug, Clone)]
pub struct ParsedToolCall {
    pub name: String,
    pub args: serde_json::Value,
}

#[derive(Debug)]
pub enum AgentEvent {
    Token(String),
    Status(String),
    ToolRequest {
        name: String,
        args: serde_json::Value,
        approve_tx: tokio::sync::oneshot::Sender<bool>,
    },
    ToolResult {
        name: String,
        output: String,
    },
    Finished,
}

pub struct Agent<'a> {
    pub model: &'a dohee_infer::DoheeModel,
    pub backend: &'a llama_cpp_2::llama_backend::LlamaBackend,
    pub registry: dohee_tools::ToolRegistry,
    pub sandbox_policy: dohee_sandbox::SandboxPolicy,
    pub max_turns: u32,
    pub temperature: f32,
    pub seed: u32,
    pub use_grammar: bool,
    pub event_tx: Option<tokio::sync::mpsc::UnboundedSender<AgentEvent>>,
    pub silent: bool,
    pub renderer: std::sync::Arc<dyn dohee_prompt::PromptRenderer>,
}

pub fn system_prompt(tools: &[Arc<dyn dohee_tools::Tool>]) -> String {
    let mut prompt = String::new();
    prompt.push_str("You are Dohee (도회), an autonomous local AI coding assistant.\n");
    prompt.push_str("You help the user develop, debug, and understand code in their workspace.\n");
    prompt.push_str("You have access to the local filesystem and terminal. You execute actions by outputting special XML tags:\n\n");

    prompt.push_str("1. List files in a directory:\n");
    prompt.push_str("<list_dir>path</list_dir>\n\n");

    prompt.push_str("2. Read file contents:\n");
    prompt.push_str("<read_file>path</read_file>\n\n");

    prompt.push_str("3. Write or overwrite a file:\n");
    prompt.push_str("<write_file path=\"path\">\nfile content here\n</write_file>\n\n");

    prompt.push_str("4. Edit an existing file using find and replace blocks:\n");
    prompt.push_str("<edit_file path=\"path\"><find>exact block to find</find><replace>replacement content</replace></edit_file>\n\n");

    prompt.push_str("5. Run a shell command in the workspace:\n");
    prompt.push_str("<run_shell>command</run_shell>\n\n");
    
    prompt.push_str("6. Find AST definition of a symbol across files:\n");
    prompt.push_str("<find_definition>symbol</find_definition> or <find_definition path=\"path\">symbol</find_definition>\n\n");
    
    prompt.push_str("7. List top-level symbols in a file:\n");
    prompt.push_str("<list_symbols>path</list_symbols>\n\n");

    prompt.push_str("Available tools details:\n");
    for tool in tools {
        prompt.push_str(&format!("- **{}**: {}\n", tool.name(), tool.description()));
        prompt.push_str(&format!("  Parameters schema: {}\n", serde_json::to_string(&tool.parameters_schema()).unwrap()));
    }
    
    prompt.push_str("\nRules for Tool Use:\n");
    prompt.push_str("- Output only ONE tool invocation per turn when performing file edits or running commands, unless they are independent reads.\n");
    prompt.push_str("- Always use paths relative to the current working directory.\n");
    prompt.push_str("- Provide clear explanations before using tools, but let the tool block be the main actionable output.\n");
    prompt.push_str("- Once you invoke a tool, STOP generating text. Dohee will execute the tool, capture the output, and present it to you in the next turn.\n");
    prompt.push_str("- Once the tool output is presented to you, synthesize the retrieved information and output a final response summarizing the findings to answer the user's original request.\n");
    
    prompt
}

pub trait SystemContextProvider: Send + Sync {
    fn name(&self) -> &'static str;
    fn generate(&self, workspace: &Path) -> String;
}

pub struct GitStatusProvider;
impl SystemContextProvider for GitStatusProvider {
    fn name(&self) -> &'static str { "git_status" }
    fn generate(&self, workspace: &Path) -> String {
        let output = std::process::Command::new("git")
            .arg("status")
            .arg("--short")
            .current_dir(workspace)
            .output();
        match output {
            Ok(out) if out.status.success() => {
                let status = String::from_utf8_lossy(&out.stdout);
                if status.trim().is_empty() {
                    "Clean working tree".to_string()
                } else {
                    format!("Modified files:\n{}", status.trim())
                }
            }
            _ => "Not a git repository or git error".to_string(),
        }
    }
}

pub struct CwdProvider;
impl SystemContextProvider for CwdProvider {
    fn name(&self) -> &'static str { "cwd" }
    fn generate(&self, workspace: &Path) -> String {
        workspace.display().to_string()
    }
}

pub struct DateProvider;
impl SystemContextProvider for DateProvider {
    fn name(&self) -> &'static str { "date" }
    fn generate(&self, _workspace: &Path) -> String {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();
        format!("Unix timestamp: {}", now)
    }
}

pub struct OsProvider;
impl SystemContextProvider for OsProvider {
    fn name(&self) -> &'static str { "os" }
    fn generate(&self, _workspace: &Path) -> String {
        format!("{} ({})", std::env::consts::OS, std::env::consts::ARCH)
    }
}



pub fn parse_tool_calls(text: &str) -> Vec<ParsedToolCall> {
    let mut calls = Vec::new();
     let list_re = Regex::new(r"(?s)<list_dir>(.*?)</list_dir>").unwrap();
    let read_re = Regex::new(r"(?s)<read_file>(.*?)</read_file>").unwrap();
    let write_re = Regex::new(r#"(?s)<write_file\s+path=["'](.*?)["']>(.*?)</write_file>"#).unwrap();
    let edit_re = Regex::new(r#"(?s)<edit_file\s+path=["'](.*?)["']>(.*?)</edit_file>"#).unwrap();
    let run_re = Regex::new(r"(?s)<run_shell>(.*?)</run_shell>").unwrap();
    let run_cmd_re = Regex::new(r"(?s)<run_command>(.*?)</run_command>").unwrap();
    let find_def_re = Regex::new(r#"(?s)<find_definition\s+path=["'](.*?)["']>(.*?)</find_definition>"#).unwrap();
    let find_def_simple_re = Regex::new(r"(?s)<find_definition>(.*?)</find_definition>").unwrap();
    let list_sym_re = Regex::new(r"(?s)<list_symbols>(.*?)</list_symbols>").unwrap();

    let find_re = Regex::new(r"(?s)<find>(.*?)</find>").unwrap();
    let replace_re = Regex::new(r"(?s)<replace>(.*?)</replace>").unwrap();

    // List dir
    for cap in list_re.captures_iter(text) {
        calls.push(ParsedToolCall {
            name: "list_dir".to_string(),
            args: serde_json::json!({ "path": cap[1].trim() }),
        });
    }
    if calls.is_empty() {
        let list_fallback = Regex::new(r"<list_dir>\s*([a-zA-Z0-9_./\-]+)").unwrap();
        for cap in list_fallback.captures_iter(text) {
            calls.push(ParsedToolCall {
                name: "list_dir".to_string(),
                args: serde_json::json!({ "path": cap[1].trim() }),
            });
        }
    }
    
    // Read file
    for cap in read_re.captures_iter(text) {
        calls.push(ParsedToolCall {
            name: "read_file".to_string(),
            args: serde_json::json!({ "path": cap[1].trim() }),
        });
    }
    if calls.iter().all(|c| c.name != "read_file") {
        let read_fallback = Regex::new(r"<read_file>\s*([a-zA-Z0-9_./\-]+)").unwrap();
        for cap in read_fallback.captures_iter(text) {
            calls.push(ParsedToolCall {
                name: "read_file".to_string(),
                args: serde_json::json!({ "path": cap[1].trim() }),
            });
        }
    }
    
    // Write file
    for cap in write_re.captures_iter(text) {
        calls.push(ParsedToolCall {
            name: "write_file".to_string(),
            args: serde_json::json!({ "path": cap[1].trim(), "content": cap[2] }),
        });
    }

    // Edit file
    for cap in edit_re.captures_iter(text) {
        let path = cap[1].trim().to_string();
        let inner = &cap[2];
        let find = find_re.captures(inner).map(|c| c[1].to_string()).unwrap_or_default();
        let replace = replace_re.captures(inner).map(|c| c[1].to_string()).unwrap_or_default();
        calls.push(ParsedToolCall {
            name: "edit_file".to_string(),
            args: serde_json::json!({ "path": path, "find": find, "replace": replace }),
        });
    }
    
    // Run shell
    for cap in run_re.captures_iter(text) {
        calls.push(ParsedToolCall {
            name: "run_shell".to_string(),
            args: serde_json::json!({ "command": cap[1].trim() }),
        });
    }
    for cap in run_cmd_re.captures_iter(text) {
        calls.push(ParsedToolCall {
            name: "run_shell".to_string(),
            args: serde_json::json!({ "command": cap[1].trim() }),
        });
    }

    // Find Definition
    for cap in find_def_re.captures_iter(text) {
        calls.push(ParsedToolCall {
            name: "find_definition".to_string(),
            args: serde_json::json!({ "path": cap[1].trim(), "symbol": cap[2].trim() }),
        });
    }
    for cap in find_def_simple_re.captures_iter(text) {
        let symbol = cap[1].trim().to_string();
        if !calls.iter().any(|c| c.name == "find_definition" && c.args["symbol"] == symbol) {
            calls.push(ParsedToolCall {
                name: "find_definition".to_string(),
                args: serde_json::json!({ "symbol": symbol, "path": "." }),
            });
        }
    }

    // List symbols
    for cap in list_sym_re.captures_iter(text) {
        calls.push(ParsedToolCall {
            name: "list_symbols".to_string(),
            args: serde_json::json!({ "path": cap[1].trim() }),
        });
    }
    
    calls
}

pub fn validate_formatting(text: &str) -> Option<String> {
    let tags = [
        ("<write_file", "</write_file>"),
        ("<edit_file", "</edit_file>"),
        ("<run_shell>", "</run_shell>"),
        ("<run_command>", "</run_command>"),
    ];
    
    for (open, close) in tags {
        if text.contains(open) && !text.contains(close) {
            return Some(format!(
                "Error: You opened a tool tag '{}' but did not close it with '{}'. Please ensure all tool tags are closed correctly.",
                open, close
            ));
        }
    }
    None
}

fn ask_user_approval(tool_name: &str, args: &serde_json::Value) -> bool {
    println!("\n⚠️  [DOHEE] TOOL APPROVAL REQUIRED:");
    println!("Tool:  {}", tool_name);
    println!("Args:  {}", serde_json::to_string_pretty(args).unwrap_or_default());
    print!("Approve execution? [y/N]: ");
    let _ = std::io::stdout().flush();
    
    let mut input = String::new();
    if std::io::stdin().read_line(&mut input).is_ok() {
        let trimmed = input.trim().to_lowercase();
        trimmed == "y" || trimmed == "yes"
    } else {
        false
    }
}

impl<'a> Agent<'a> {
    pub fn new(
        model: &'a do_infer::DoheeModel,
        backend: &'a llama_cpp_2::llama_backend::LlamaBackend,
        registry: dohee_tools::ToolRegistry,
        sandbox_policy: dohee_sandbox::SandboxPolicy,
        renderer: std::sync::Arc<dyn dohee_prompt::PromptRenderer>,
    ) -> Self {
        Self {
            model,
            backend,
            registry,
            sandbox_policy,
            max_turns: 10,
            temperature: 0.2,
            seed: 1234,
            use_grammar: true,
            event_tx: None,
            silent: false,
            renderer,
        }
    }

    pub async fn run_turn_loop(&self, messages: &mut Vec<Message>, ctx_size: u32, threads: Option<i32>) -> Result<()> {
        let mut turn = 0;
        
        loop {
            if turn >= self.max_turns {
                if !self.silent {
                    println!("[Agent] Maximum turn count reached ({}). Stopping.", self.max_turns);
                }
                if let Some(ref tx) = self.event_tx {
                    let _ = tx.send(AgentEvent::Finished);
                }
                break;
            }
            
            if !self.silent {
                println!("\n[Agent] Running turn {}/{}...", turn + 1, self.max_turns);
            }
            if let Some(ref tx) = self.event_tx {
                let _ = tx.send(AgentEvent::Status(format!("Turn {}/{}...", turn + 1, self.max_turns)));
            }

            // 1. Prune old tool outputs to free context space
            dohee_context::prune_old_tool_outputs(messages);

            // 2. Check if we need to compact the history
            let mut prompt = self.renderer.render(messages, true)?;
            if !self.silent {
                println!("[DEBUG PROMPT]\n{}\n[/DEBUG PROMPT]", prompt);
            }
            let token_count = dohee_context::count_tokens(self.model, &prompt);
            let limit = ctx_size.saturating_sub(512) as usize;
            
            if token_count > limit {
                if !self.silent {
                    println!("[Agent] Context token count ({} tokens) approaching limit ({} tokens). Compacting history...", token_count, limit);
                }
                if let Some(ref tx) = self.event_tx {
                    let _ = tx.send(AgentEvent::Status("Compacting history...".to_string()));
                }
                
                let mut compaction_gen = 0;
                if let Err(e) = dohee_context::compact_history(
                    self.backend,
                    self.model,
                    messages,
                    &mut compaction_gen,
                    ctx_size,
                    self.renderer.as_ref(),
                ) {
                    if !self.silent {
                        println!("[Agent] Warning: History compaction failed: {:?}", e);
                    }
                } else {
                    if !self.silent {
                        println!("[Agent] History compacted successfully.");
                    }
                    prompt = self.renderer.render(messages, true)?;
                }
            }

            let mut completion = String::new();
            {
                let mut session = do_infer::InferenceSession::new(self.backend, self.model, ctx_size, threads)
                    .context("Failed to construct inference session")?;
                session.advance(self.model, &prompt).context("Failed to process prompt")?;
                         let mut sampler = if self.use_grammar {
                    let grammar_str = r#"
root ::= (text | tool-call)*
text ::= ([^<] | "<" [^lrewf/] | "<e" [^d] | "<r" [^eu] | "<l" [^i] | "<w" [^r] | "<f" [^i])+
tool-call ::= list-dir | read-file | write-file | edit-file | run-shell | find-definition | list-symbols
path ::= "." | "./" | [a-zA-Z0-9_.-]+ ("/" [a-zA-Z0-9_.-]+)*
list-dir ::= "<list_dir>" path "</list_dir>"
read-file ::= "<read_file>" path "</read_file>"
write-file ::= "<write_file path=\"" path "\">" [^<]* "</write_file>"
edit-file ::= "<edit_file path=\"" path "\">" "<find>" [^<]* "</find>" "<replace>" [^<]* "</replace>" "</edit_file>"
run-shell ::= "<run_shell>" [^\r\n<]* "</run_shell>"
find-definition ::= "<find_definition>" [a-zA-Z0-9_]+ "</find_definition>" | "<find_definition path=\"" path "\">" [a-zA-Z0-9_]+ "</find_definition>"
list-symbols ::= "<list_symbols>" path "</list_symbols>"
"#;
                    do_infer::grammar_sampler(self.model, self.seed, self.temperature, grammar_str)
                        .context("Failed to construct grammar sampler")?
                } else {
                    do_infer::default_sampler(self.seed, self.temperature)
                };
                
                if !self.silent {
                    print!("Response: ");
                    std::io::stdout().flush()?;
                }
                
                if let Some(ref tx) = self.event_tx {
                    let _ = tx.send(AgentEvent::Status("Model generating response...".to_string()));
                }
 
                while let Some(piece) = session.sample_next(self.model, &mut sampler).context("Failed to sample next token")? {
                    if !self.silent {
                        print!("{}", piece);
                        std::io::stdout().flush()?;
                    }
                    completion.push_str(&piece);
                    if let Some(ref tx) = self.event_tx {
                        let _ = tx.send(AgentEvent::Token(piece.clone()));
                    }
 
                    // Stop sampling if turn-end or closed tool call is produced
                    if completion.contains("<|im_end|>")
                        || completion.contains("<end_of_turn>")
                        || completion.contains("</list_dir>")
                        || completion.contains("</read_file>")
                        || completion.contains("</write_file>")
                        || completion.contains("</edit_file>")
                        || completion.contains("</run_shell>")
                        || completion.contains("</find_definition>")
                        || completion.contains("</list_symbols>")
                    {
                        break;
                    }
                }
            }

            // 1. Validate tag formatting
            if let Some(format_error) = validate_formatting(&completion) {
                if !self.silent {
                    println!("[Agent] Malformed formatting detected. Requesting correction.");
                }
                messages.push(Message {
                    role: "assistant".to_string(),
                    content: completion,
                    name: None,
                });
                messages.push(Message {
                    role: "user".to_string(),
                    content: format_error,
                    name: None,
                });
                turn += 1;
                continue;
            }

            // Save response
            messages.push(Message {
                role: "assistant".to_string(),
                content: completion.clone(),
                name: None,
            });

            // 2. Parse tool calls
            let tool_calls = parse_tool_calls(&completion);
            if tool_calls.is_empty() {
                if !self.silent {
                    println!("[Agent] No tool calls requested. Task complete.");
                }
                break;
            }

            // 3. Process each tool call
            for call in tool_calls {
                // Determine if confirmation is needed
                let requires_approval = match &self.sandbox_policy {
                    dohee_sandbox::SandboxPolicy::DangerFullAccess => false,
                    _ => {
                        // Under sandboxed modes, write or shell operations require confirmation
                        call.name == "write_file" || call.name == "edit_file" || call.name == "run_shell"
                    }
                };

                let approved = if requires_approval {
                    if let Some(ref tx) = self.event_tx {
                        let (approve_tx, approve_rx) = tokio::sync::oneshot::channel();
                        let _ = tx.send(AgentEvent::ToolRequest {
                            name: call.name.clone(),
                            args: call.args.clone(),
                            approve_tx,
                        });
                        approve_rx.await.unwrap_or(false)
                    } else {
                        ask_user_approval(&call.name, &call.args)
                    }
                } else {
                    true
                };

                if !approved {
                    if !self.silent {
                        println!("[Agent] Tool call '{}' denied by user.", call.name);
                    }
                    if let Some(ref tx) = self.event_tx {
                        let _ = tx.send(AgentEvent::Status(format!("Tool '{}' execution denied.", call.name)));
                    }
                    messages.push(Message {
                        role: "tool".to_string(),
                        content: format!("Error: Tool '{}' execution denied by user.", call.name),
                        name: Some(call.name.clone()),
                    });
                    continue;
                }

                if !self.silent {
                    println!("[Agent] Executing '{}'...", call.name);
                }
                if let Some(ref tx) = self.event_tx {
                    let _ = tx.send(AgentEvent::Status(format!("Executing tool '{}'...", call.name)));
                }
                if let Some(tool) = self.registry.get(&call.name) {
                    match tool.execute(call.args.clone()).await {
                        Ok(output) => {
                            if !self.silent {
                                println!("\n[Agent] Tool Output:\n{}", output.content);
                            }
                            if let Some(ref tx) = self.event_tx {
                                let _ = tx.send(AgentEvent::ToolResult {
                                    name: call.name.clone(),
                                    output: output.content.clone(),
                                });
                            }
                            messages.push(Message {
                                role: "tool".to_string(),
                                content: output.content,
                                name: Some(call.name.clone()),
                            });
                        }
                        Err(e) => {
                            if !self.silent {
                                println!("\n[Agent] Tool Execution Error: {:?}", e);
                            }
                            let err_msg = format!("Error: Tool execution failed: {:?}", e);
                            if let Some(ref tx) = self.event_tx {
                                let _ = tx.send(AgentEvent::ToolResult {
                                    name: call.name.clone(),
                                    output: err_msg.clone(),
                                });
                            }
                            messages.push(Message {
                                role: "tool".to_string(),
                                content: err_msg,
                                name: Some(call.name.clone()),
                            });
                        }
                    }
                } else {
                    if !self.silent {
                        println!("\n[Agent] Tool '{}' not found in registry.", call.name);
                    }
                    let err_msg = format!("Error: Tool '{}' not found in registry.", call.name);
                    if let Some(ref tx) = self.event_tx {
                        let _ = tx.send(AgentEvent::ToolResult {
                            name: call.name.clone(),
                            output: err_msg.clone(),
                        });
                    }
                    messages.push(Message {
                        role: "tool".to_string(),
                        content: err_msg,
                        name: Some(call.name.clone()),
                    });
                }
            }

            turn += 1;
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dohee_prompt::PromptRenderer;

    #[test]
    fn test_format_messages() {
        let messages = vec![
            Message {
                role: "system".to_string(),
                content: "sys prompt".to_string(),
                name: None,
            },
            Message {
                role: "user".to_string(),
                content: "hello".to_string(),
                name: None,
            },
        ];
        
        let renderer = dohee_prompt::JinjaRenderer::new(dohee_prompt::PromptTemplate::Builtin("chatml".to_string())).unwrap();
        let prompt = renderer.render(&messages, true).unwrap();
        println!("DEBUG_PROMPT_RENDERED: {:?}", prompt);
        assert!(prompt.contains("<|im_start|>system\nsys prompt<|im_end|>"));
        assert!(prompt.contains("<|im_start|>user\nhello<|im_end|>"));
        assert!(prompt.ends_with("<|im_start|>assistant"));
    }

    #[test]
    fn test_parse_tool_calls() {
        let text = "Thinking...\n<run_shell>cargo test</run_shell>\nAnd <write_file path=\"test.rs\">fn main() {}</write_file>";
        let calls = parse_tool_calls(text);
        
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[0].name, "write_file");
        assert_eq!(calls[0].args["path"], "test.rs");
        assert_eq!(calls[0].args["content"], "fn main() {}");
        assert_eq!(calls[1].name, "run_shell");
        assert_eq!(calls[1].args["command"], "cargo test");
    }

    #[test]
    fn test_validate_formatting() {
        let bad_text = "Let's run: <run_shell>.";
        assert!(validate_formatting(bad_text).is_some());

        let good_text = "Let's run: <run_shell>.</run_shell>";
        assert!(validate_formatting(good_text).is_none());
    }
}
