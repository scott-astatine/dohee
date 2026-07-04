use anyhow::{Context, Result};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::process::Stdio;
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, ChildStdout};
use tokio::sync::Mutex;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpToolInfo {
    pub name: String,
    pub description: String,
    #[serde(rename = "inputSchema")]
    pub input_schema: Value,
}

pub struct McpConnection {
    stdin: ChildStdin,
    stdout_reader: BufReader<ChildStdout>,
    request_id: i64,
}

impl McpConnection {
    pub async fn send_request(&mut self, method: &str, params: Value) -> Result<Value> {
        self.request_id += 1;
        let id = self.request_id;
        
        let request = json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
            "params": params
        });
        
        let mut req_str = request.to_string();
        req_str.push('\n');
        
        self.stdin.write_all(req_str.as_bytes()).await?;
        self.stdin.flush().await?;
        
        let mut line = String::new();
        loop {
            line.clear();
            let bytes_read = self.stdout_reader.read_line(&mut line).await?;
            if bytes_read == 0 {
                anyhow::bail!("MCP server disconnected");
            }
            
            if let Ok(resp) = serde_json::from_str::<Value>(&line) {
                if resp.get("id").is_none() {
                    // Skip logs/notifications
                    continue;
                }
                
                if resp["id"].as_i64() == Some(id) {
                    if let Some(error) = resp.get("error") {
                        anyhow::bail!("MCP server error: {}", error);
                    }
                    return Ok(resp["result"].clone());
                }
            }
        }
    }
}

pub struct McpClient {
    connection: Arc<Mutex<McpConnection>>,
    _child: Child,
}

impl McpClient {
    pub async fn connect(command: &str, args: &[String]) -> Result<Self> {
        let mut child = tokio::process::Command::new(command)
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .with_context(|| format!("Failed to spawn MCP server '{}'", command))?;
            
        let stdin = child.stdin.take().context("Failed to open stdin")?;
        let stdout = child.stdout.take().context("Failed to open stdout")?;
        
        let mut connection = McpConnection {
            stdin,
            stdout_reader: BufReader::new(stdout),
            request_id: 0,
        };
        
        // Protocol handshake
        connection.send_request("initialize", json!({
            "protocolVersion": "2024-11-05",
            "capabilities": {},
            "clientInfo": {
                "name": "dohee",
                "version": "0.1.0"
            }
        })).await?;
        
        let initialized_notification = json!({
            "jsonrpc": "2.0",
            "method": "notifications/initialized"
        });
        let mut req_str = initialized_notification.to_string();
        req_str.push('\n');
        connection.stdin.write_all(req_str.as_bytes()).await?;
        connection.stdin.flush().await?;
        
        Ok(Self {
            connection: Arc::new(Mutex::new(connection)),
            _child: child,
        })
    }
    
    pub async fn list_tools(&self) -> Result<Vec<McpToolInfo>> {
        let result = self.connection.lock().await.send_request("tools/list", json!({})).await?;
        let tools_val = result.get("tools").context("Missing 'tools' field in list response")?;
        let tools: Vec<McpToolInfo> = serde_json::from_value(tools_val.clone())?;
        Ok(tools)
    }

    pub fn into_tools(self) -> Result<Vec<McpTool>> {
        // Fetch tools synchronously for caller conversions
        // Wait, we need an async call. Let's make it a builder method.
        Ok(Vec::new()) // Stub, we will construct in main CLI
    }
}

pub struct McpTool {
    pub tool_name: &'static str,
    pub tool_desc: &'static str,
    pub schema: Value,
    pub raw_name: String,
    pub connection: Arc<Mutex<McpConnection>>,
}

impl McpTool {
    pub fn new(info: McpToolInfo, connection: Arc<Mutex<McpConnection>>) -> Self {
        let tool_name = Box::leak(info.name.clone().into_boxed_str());
        let tool_desc = Box::leak(info.description.into_boxed_str());
        Self {
            tool_name,
            tool_desc,
            schema: info.input_schema,
            raw_name: info.name,
            connection,
        }
    }
}

#[async_trait]
impl dohee_tools::Tool for McpTool {
    fn name(&self) -> &'static str {
        self.tool_name
    }
    
    fn description(&self) -> &'static str {
        self.tool_desc
    }
    
    fn parameters_schema(&self) -> Value {
        self.schema.clone()
    }
    
    async fn execute(&self, args: Value) -> Result<dohee_tools::ToolOutput> {
        let result = self.connection.lock().await.send_request("tools/call", json!({
            "name": self.raw_name,
            "arguments": args
        })).await?;
        
        let content_arr = result.get("content").context("Missing content in tool call response")?;
        let mut output = String::new();
        
        if let Some(arr) = content_arr.as_array() {
            for item in arr {
                if let Some(text) = item.get("text").and_then(|t| t.as_str()) {
                    output.push_str(text);
                }
            }
        }
        
        Ok(dohee_tools::ToolOutput::new(output))
    }
}
