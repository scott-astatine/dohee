use dohee_context::Message;
use crate::AgentMode;

#[derive(Debug, Clone)]
pub enum Action {
    SubmitPrompt(String),
    ApproveTool(bool),
    UpdateConfig {
        temp: Option<f32>,
        seed: Option<u32>,
        ctx_size: Option<u32>,
        sandbox_policy: Option<dohee_sandbox::SandboxPolicy>,
    },
    ScrollUp,
    ScrollDown,
    ScrollToTop,
    ScrollToBottom,
    YankSelection,
    CycleAutocomplete,
    ResetAutocomplete,
    ToggleCommandPalette,
    SetAgentMode(AgentMode),
    SetInputMode(crate::InputMode),
    Exit,
    AddToken(String),
    AddMessage(Message),
    SetStatus(String),
    ToolRequest {
        name: String,
        args: serde_json::Value,
    },
    ToolResult {
        name: String,
        output: String,
    },
    Finished,
    Noop,
}
