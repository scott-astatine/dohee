use dohee_core::AgentEvent;
use crate::TuiCommand;

pub struct AgentWorker {
    pub ui_cmd_tx: tokio::sync::mpsc::UnboundedSender<TuiCommand>,
    pub agent_rx: tokio::sync::mpsc::UnboundedReceiver<AgentEvent>,
}

impl AgentWorker {
    pub fn spawn(
        config: dohee_config::DoheeConfig,
        registry: dohee_tools::ToolRegistry,
        model_ref: &'static dohee_infer::DoheeModel,
        backend_ref: &'static llama_cpp_2::llama_backend::LlamaBackend,
    ) -> anyhow::Result<Self> {
        let (ui_cmd_tx, mut ui_cmd_rx) = tokio::sync::mpsc::unbounded_channel::<TuiCommand>();
        let (agent_event_tx, agent_rx) = tokio::sync::mpsc::unbounded_channel::<AgentEvent>();

        // 1. Resolve template source
        let template_source = if let Some(ref custom_tpl) = config.chat_template {
            let path = std::path::Path::new(custom_tpl);
            if path.exists() {
                dohee_prompt::PromptTemplate::External(path.to_path_buf())
            } else {
                dohee_prompt::PromptTemplate::Builtin(custom_tpl.clone())
            }
        } else if let Ok(embedded_tpl) = model_ref.model.meta_val_str("tokenizer.chat_template") {
            dohee_prompt::PromptTemplate::Embedded(embedded_tpl)
        } else {
            anyhow::bail!(
                "No chat template was found in the GGUF model metadata, and no custom template was configured via configuration. Please specify a chat template to proceed."
            );
        };

        // 2. Compile renderer once
        let renderer = std::sync::Arc::new(dohee_prompt::JinjaRenderer::new(template_source)?);

        let config_clone = config.clone();
        let registry_clone = registry.clone();
        let agent_event_tx_clone = agent_event_tx.clone();
        let renderer_clone = renderer.clone();

        tokio::spawn(async move {
            let local_config = config_clone;
            let mut temp = local_config.temperature;
            let mut seed = local_config.seed;
            let mut ctx_size = local_config.ctx_size;
            let mut sandbox_policy = match local_config.sandbox_policy.as_str() {
                "ReadOnly" => dohee_sandbox::SandboxPolicy::ReadOnly,
                "DangerFullAccess" => dohee_sandbox::SandboxPolicy::DangerFullAccess,
                _ => dohee_sandbox::SandboxPolicy::WorkspaceWrite {
                    root: std::env::current_dir().unwrap_or_default(),
                },
            };

            while let Some(cmd) = ui_cmd_rx.recv().await {
                match cmd {
                    TuiCommand::SubmitPrompt { prompt: _, messages } => {
                        let mut agent = dohee_core::Agent::new(
                            model_ref,
                            backend_ref,
                            registry_clone.clone(),
                            sandbox_policy.clone(),
                            renderer_clone.clone(),
                        );
                        agent.temperature = temp;
                        agent.seed = seed;
                        agent.use_grammar = true;
                        agent.silent = true;

                        let (event_inner_tx, mut event_inner_rx) = tokio::sync::mpsc::unbounded_channel::<AgentEvent>();
                        agent.event_tx = Some(event_inner_tx);

                        let agent_event_tx_inner = agent_event_tx_clone.clone();

                        let mut join_handle = tokio::task::spawn_blocking(move || {
                            let mut messages = messages;
                            let rt = tokio::runtime::Handle::current();
                            let _ = rt.block_on(async {
                                agent.run_turn_loop(&mut messages, ctx_size, None).await
                            });
                        });

                        loop {
                            tokio::select! {
                                Some(event) = event_inner_rx.recv() => {
                                    match event {
                                        AgentEvent::ToolRequest { name, args, approve_tx } => {
                                            let (fwd_tx, fwd_rx) = tokio::sync::oneshot::channel::<bool>();
                                            let _ = agent_event_tx_inner.send(AgentEvent::ToolRequest {
                                                name,
                                                args,
                                                approve_tx: fwd_tx,
                                            });
                                            let approved = fwd_rx.await.unwrap_or(false);
                                            let _ = approve_tx.send(approved);
                                        }
                                        other => {
                                            let _ = agent_event_tx_inner.send(other);
                                        }
                                    }
                                }
                                _ = &mut join_handle => {
                                    break;
                                }
                            }
                        }
                        let _ = agent_event_tx_inner.send(AgentEvent::Finished);
                    }
                    TuiCommand::UpdateConfig { temp: t, seed: s, ctx_size: c, sandbox_policy: sp } => {
                        if let Some(val) = t { temp = val; }
                        if let Some(val) = s { seed = val; }
                        if let Some(val) = c { ctx_size = val; }
                        if let Some(val) = sp { sandbox_policy = val; }
                    }
                }
            }
        });

        Ok(Self {
            ui_cmd_tx,
            agent_rx,
        })
    }
}
