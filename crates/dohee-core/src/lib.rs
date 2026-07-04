use anyhow::{Context, Result};
use regex::Regex;
use std::io::Write;
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
}

pub fn system_prompt(tools: &[Arc<dyn dohee_tools::Tool>]) -> String {
    let mut prompt = String::new();
    prompt.push_str("You are Dohee (도회), an autonomous AI coding assistant. You run locally on the user's machine.\n");
    prompt.push_str("You have access to the following tools:\n\n");
    
    for tool in tools {
        prompt.push_str(&format!("- **{}**: {}\n", tool.name(), tool.description()));
        prompt.push_str(&format!("  Parameters schema: {}\n\n", serde_json::to_string(&tool.parameters_schema()).unwrap()));
    }
    
    prompt.push_str("To call a tool, you MUST use the following XML tags format:\n");
    prompt.push_str("- To list a directory: <list_dir>path</list_dir>\n");
    prompt.push_str("- To read a file: <read_file>path</read_file>\n");
    prompt.push_str("- To write a file: <write_file path=\"path\">content</write_file>\n");
    prompt.push_str("- To edit a file: <edit_file path=\"path\"><find>exact block to find</find><replace>replacement content</replace></edit_file>\n");
    prompt.push_str("- To run a shell command: <run_shell>command</run_shell>\n\n");
    
    prompt.push_str("Make sure to always close your tool tags. Only perform one tool call at a time. After receiving the tool output, proceed with your analysis or call another tool if needed. When you are finished and have resolved the user's request, print your final response without calling any tools.\n");
    
    prompt
}

pub fn format_messages(messages: &[Message]) -> String {
    let mut prompt = String::new();
    for msg in messages {
        match msg.role.as_str() {
            "system" => {
                prompt.push_str(&format!("<|im_start|>system\n{}<|im_end|>\n", msg.content));
            }
            "user" => {
                prompt.push_str(&format!("<|im_start|>user\n{}<|im_end|>\n", msg.content));
            }
            "assistant" => {
                prompt.push_str(&format!("<|im_start|>assistant\n{}<|im_end|>\n", msg.content));
            }
            "tool" => {
                prompt.push_str(&format!(
                    "<|im_start|>user\n[Tool Output ({}): {}]<|im_end|>\n",
                    msg.name.as_deref().unwrap_or("unknown"),
                    msg.content
                ));
            }
            _ => {
                prompt.push_str(&format!("<|im_start|>user\n{}<|im_end|>\n", msg.content));
            }
        }
    }
    prompt.push_str("<|im_start|>assistant\n");
    prompt
}

pub fn parse_tool_calls(text: &str) -> Vec<ParsedToolCall> {
    let mut calls = Vec::new();
    
    let list_re = Regex::new(r"(?s)<list_dir>(.*?)</list_dir>").unwrap();
    let read_re = Regex::new(r"(?s)<read_file>(.*?)</read_file>").unwrap();
    let write_re = Regex::new(r#"(?s)<write_file\s+path=["'](.*?)["']>(.*?)</write_file>"#).unwrap();
    let edit_re = Regex::new(r#"(?s)<edit_file\s+path=["'](.*?)["']>(.*?)</edit_file>"#).unwrap();
    let run_re = Regex::new(r"(?s)<run_shell>(.*?)</run_shell>").unwrap();
    let run_cmd_re = Regex::new(r"(?s)<run_command>(.*?)</run_command>").unwrap();

    let find_re = Regex::new(r"(?s)<find>(.*?)</find>").unwrap();
    let replace_re = Regex::new(r"(?s)<replace>(.*?)</replace>").unwrap();

    // List dir
    for cap in list_re.captures_iter(text) {
        calls.push(ParsedToolCall {
            name: "list_dir".to_string(),
            args: serde_json::json!({ "path": cap[1].trim() }),
        });
    }
    
    // Read file
    for cap in read_re.captures_iter(text) {
        calls.push(ParsedToolCall {
            name: "read_file".to_string(),
            args: serde_json::json!({ "path": cap[1].trim() }),
        });
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
    
    calls
}

pub fn validate_formatting(text: &str) -> Option<String> {
    let tags = [
        ("<list_dir>", "</list_dir>"),
        ("<read_file>", "</read_file>"),
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
        }
    }

    pub async fn run_turn_loop(&self, messages: &mut Vec<Message>, ctx_size: u32, threads: Option<i32>) -> Result<()> {
        let mut turn = 0;
        
        loop {
            if turn >= self.max_turns {
                println!("[Agent] Maximum turn count reached ({}). Stopping.", self.max_turns);
                if let Some(ref tx) = self.event_tx {
                    let _ = tx.send(AgentEvent::Finished);
                }
                break;
            }
            
            println!("\n[Agent] Running turn {}/{}...", turn + 1, self.max_turns);
            if let Some(ref tx) = self.event_tx {
                let _ = tx.send(AgentEvent::Status(format!("Turn {}/{}...", turn + 1, self.max_turns)));
            }
            let prompt = format_messages(messages);
                 let mut completion = String::new();
            {
                let mut session = do_infer::InferenceSession::new(self.backend, self.model, ctx_size, threads)
                    .context("Failed to construct inference session")?;
                session.advance(self.model, &prompt).context("Failed to process prompt")?;
                
                let mut sampler = if self.use_grammar {
                    let grammar_str = r#"
root ::= (text | tool-call)*
text ::= [^<]+
tool-call ::= list-dir | read-file | write-file | edit-file | run-shell
list-dir ::= "<list_dir>" [^<]* "</list_dir>"
read-file ::= "<read_file>" [^<]* "</read_file>"
write-file ::= "<write_file path=\"" [^\"]* "\">" [^<]* "</write_file>"
edit-file ::= "<edit_file path=\"" [^\"]* "\">" "<find>" [^<]* "</find>" "<replace>" [^<]* "</replace>" "</edit_file>"
run-shell ::= "<run_shell>" [^<]* "</run_shell>"
"#;
                    do_infer::grammar_sampler(self.model, self.seed, self.temperature, grammar_str)
                        .context("Failed to construct grammar sampler")?
                } else {
                    do_infer::default_sampler(self.seed, self.temperature)
                };
                
                print!("Response: ");
                std::io::stdout().flush()?;
                
                if let Some(ref tx) = self.event_tx {
                    let _ = tx.send(AgentEvent::Status("Model generating response...".to_string()));
                }

                while let Some(piece) = session.sample_next(self.model, &mut sampler).context("Failed to sample next token")? {
                    print!("{}", piece);
                    std::io::stdout().flush()?;
                    completion.push_str(&piece);
                    if let Some(ref tx) = self.event_tx {
                        let _ = tx.send(AgentEvent::Token(piece.clone()));
                    }
                }
            }

            // 1. Validate tag formatting
            if let Some(format_error) = validate_formatting(&completion) {
                println!("[Agent] Malformed formatting detected. Requesting correction.");
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
                println!("[Agent] No tool calls requested. Task complete.");
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
                    println!("[Agent] Tool call '{}' denied by user.", call.name);
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

                println!("[Agent] Executing '{}'...", call.name);
                if let Some(ref tx) = self.event_tx {
                    let _ = tx.send(AgentEvent::Status(format!("Executing tool '{}'...", call.name)));
                }
                if let Some(tool) = self.registry.get(&call.name) {
                    match tool.execute(call.args.clone()).await {
                        Ok(output) => {
                            messages.push(Message {
                                role: "tool".to_string(),
                                content: output.content,
                                name: Some(call.name.clone()),
                            });
                        }
                        Err(e) => {
                            messages.push(Message {
                                role: "tool".to_string(),
                                content: format!("Error: Tool execution failed: {:?}", e),
                                name: Some(call.name.clone()),
                            });
                        }
                    }
                } else {
                    messages.push(Message {
                        role: "tool".to_string(),
                        content: format!("Error: Tool '{}' not found in registry.", call.name),
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
        
        let prompt = format_messages(&messages);
        assert!(prompt.contains("<|im_start|>system\nsys prompt<|im_end|>"));
        assert!(prompt.contains("<|im_start|>user\nhello<|im_end|>"));
        assert!(prompt.ends_with("<|im_start|>assistant\n"));
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
        let bad_text = "Let's list: <list_dir>.";
        assert!(validate_formatting(bad_text).is_some());

        let good_text = "Let's list: <list_dir>.</list_dir>";
        assert!(validate_formatting(good_text).is_none());
    }
}
