use anyhow::Result;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Message {
    pub role: String, // "system", "user", "assistant", "tool"
    pub content: String,
    pub name: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Session {
    pub messages: Vec<Message>,
    pub created_at: u64,
    pub compaction_generation: u32,
}

pub fn count_tokens(model: &dohee_infer::DoheeModel, text: &str) -> usize {
    model.tokenize(text, llama_cpp_2::model::AddBos::Never)
        .map(|t| t.len())
        .unwrap_or(0)
}

pub fn prune_old_tool_outputs(messages: &mut [Message]) {
    let mut last_tool_idx = None;
    for (i, msg) in messages.iter().enumerate() {
        if msg.role == "tool" {
            last_tool_idx = Some(i);
        }
    }
    
    if let Some(last_idx) = last_tool_idx {
        for (i, msg) in messages.iter_mut().enumerate() {
            if msg.role == "tool" && i < last_idx {
                if msg.content.len() > 1000 {
                    let truncated_len = 500;
                    let original_len = msg.content.len();
                    let summary = if msg.content.len() > truncated_len {
                        format!("{}\n... [Pruned old tool output, original size: {} chars]", &msg.content[..truncated_len], original_len)
                    } else {
                        msg.content.clone()
                    };
                    msg.content = summary;
                }
            }
        }
    }
}

pub fn compact_history(
    backend: &llama_cpp_2::llama_backend::LlamaBackend,
    model: &dohee_infer::DoheeModel,
    messages: &mut Vec<Message>,
    compaction_gen: &mut u32,
    ctx_size: u32,
) -> Result<()> {
    *compaction_gen += 1;
    
    if messages.len() <= 4 {
        return Ok(()); // Nothing to compact
    }
    
    let system_msg = messages.first().cloned();
    let last_few = messages.iter().skip(messages.len() - 3).cloned().collect::<Vec<_>>();
    
    let mid_messages = messages.iter().skip(1).take(messages.len() - 4).cloned().collect::<Vec<_>>();
    
    let mut prompt = String::new();
    prompt.push_str("<|im_start|>system\nYou are a concise summarizer. Summarize the following conversation history between the User and the Assistant into a structured summary containing: Goal, Constraints, Progress, Decisions, Next Steps.\n<|im_end|>\n");
    
    prompt.push_str("<|im_start|>user\n");
    for msg in mid_messages {
        prompt.push_str(&format!("{}: {}\n", msg.role, msg.content));
    }
    prompt.push_str("\nProvide the summary now:\n<|im_end|>\n<|im_start|>assistant\n");
    
    let mut session = dohee_infer::InferenceSession::new(backend, model, ctx_size, None)?;
    session.advance(model, &prompt)?;
    
    let mut sampler = dohee_infer::default_sampler(1234, 0.2);
    let mut summary = String::new();
    while let Some(piece) = session.sample_next(model, &mut sampler)? {
        summary.push_str(&piece);
    }
    
    let mut new_messages = Vec::new();
    if let Some(sys) = system_msg {
        new_messages.push(sys);
    }
    new_messages.push(Message {
        role: "system".to_string(),
        content: format!(
            "--- [Conversation History Summary (Generation {})] ---\n{}\n-------------------------------------------------",
            *compaction_gen, summary.trim()
        ),
        name: None,
    });
    new_messages.extend(last_few);
    
    *messages = new_messages;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_prune_old_tool_outputs() {
        let mut messages = vec![
            Message {
                role: "system".to_string(),
                content: "sys".to_string(),
                name: None,
            },
            Message {
                role: "tool".to_string(),
                content: "A".repeat(2000),
                name: Some("tool_a".to_string()),
            },
            Message {
                role: "user".to_string(),
                content: "ok".to_string(),
                name: None,
            },
            Message {
                role: "tool".to_string(),
                content: "B".repeat(2000),
                name: Some("tool_b".to_string()),
            },
        ];

        prune_old_tool_outputs(&mut messages);

        // Tool A should be pruned because it is older than Tool B
        assert!(messages[1].content.contains("Pruned old tool output"));
        // Tool B is the most recent tool and should NOT be pruned
        assert_eq!(messages[3].content, "B".repeat(2000));
    }
}
