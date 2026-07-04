use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use dohee_config as do_config;
use dohee_core as do_core;
use dohee_infer as do_infer;
use dohee_sandbox as do_sandbox;
use dohee_store as do_store;
use dohee_tools as do_tools;
use llama_cpp_2::llama_backend::LlamaBackend;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

#[derive(Parser, Debug)]
#[command(name = "dohee", version = "0.1.0", about = "A local-first AI coding agent. No cloud required.")]
struct Cli {
    /// Path to GGUF model file
    #[arg(short, long)]
    model: Option<String>,

    /// Backend to use (cpu, vulkan)
    #[arg(short, long)]
    backend: Option<String>,

    /// GPU layers to offload
    #[arg(short, long)]
    gpu_layers: Option<u32>,

    /// Context size
    #[arg(short, long)]
    ctx_size: Option<u32>,

    /// Disable grammar-constrained tool calling
    #[arg(long)]
    no_grammar: bool,

    /// Optional raw prompt for single-shot execution
    prompt: Option<String>,

    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand, Debug, Clone)]
enum Commands {
    /// Start an TUI chat session (Stub for now)
    Chat,
    /// Run a prompt using the full agent turn loop
    Run {
        /// The prompt to execute
        prompt: String,
    },
    /// Show the merged configuration
    ConfigShow,
    /// List available models in the models directory
    ModelsList,
    /// List past agent sessions
    SessionsList,
    /// Resume a past agent session
    SessionsResume {
        /// The session ID to resume
        session_id: String,
    },
    /// Check system hardware, model path, Vulkan compatibility, and sandbox support
    Doctor,
}

fn get_config_paths() -> (Option<PathBuf>, Option<PathBuf>) {
    let home = std::env::var("HOME").ok();
    let global_config = home.map(|h| PathBuf::from(h).join(".config/dohee/config.toml"));
    let local_config = Some(PathBuf::from(".dohee.toml"));
    (global_config, local_config)
}

fn get_db_path() -> Result<PathBuf> {
    let home = std::env::var("HOME").ok();
    let config_dir = home.map(|h| PathBuf::from(h).join(".config/dohee"));
    if let Some(ref dir) = config_dir {
        fs::create_dir_all(dir)?;
    }
    Ok(config_dir.unwrap_or_else(|| PathBuf::from(".")).join("sessions.db"))
}

async fn run_agent_session(
    config: &do_config::DoheeConfig,
    session_id: &str,
    mut session: do_core::Session,
    no_grammar: bool,
) -> Result<()> {
    // Validate model path exists first
    config.validate().context("Configuration validation failed")?;
    let model_path = config.model_path.as_ref().context("Model path not specified")?;

    println!("[Agent] Initializing backend...");
    let backend = LlamaBackend::init().context("Failed to initialize llama.cpp backend")?;

    println!("[Agent] Loading model from {}...", model_path.display());
    #[cfg(any(feature = "cuda", feature = "vulkan"))]
    let gpu_layers = if config.backend == "cpu" { 0 } else { config.gpu_layers };
    #[cfg(not(any(feature = "cuda", feature = "vulkan")))]
    let gpu_layers = 0;

    let model = do_infer::DoheeModel::new(&backend, model_path, gpu_layers)
        .context("Failed to load model")?;

    // 1. Resolve sandbox policy
    let sandbox_policy = match config.sandbox_policy.as_str() {
        "ReadOnly" => do_sandbox::SandboxPolicy::ReadOnly,
        "DangerFullAccess" => do_sandbox::SandboxPolicy::DangerFullAccess,
        _ => do_sandbox::SandboxPolicy::WorkspaceWrite {
            root: std::env::current_dir().unwrap_or_default(),
        },
    };
    println!("[Agent] Sandboxing policy: {:?}", sandbox_policy);

    // 2. Set up tool registry
    let mut registry = do_tools::ToolRegistry::new();
    registry.register(Arc::new(do_tools::ReadFileTool));
    registry.register(Arc::new(do_tools::WriteFileTool));
    registry.register(Arc::new(do_tools::EditFileTool));
    registry.register(Arc::new(do_tools::ListDirTool));
    registry.register(Arc::new(do_tools::GrepTool));
    registry.register(Arc::new(do_tools::RunShellTool::new(sandbox_policy.clone())));

    // 3. Inject system prompt if this is a fresh session
    if session.messages.is_empty() {
        let sys_prompt = do_core::system_prompt(&registry.list());
        session.messages.push(do_core::Message {
            role: "system".to_string(),
            content: sys_prompt,
            name: None,
        });
    }

    // 4. Instantiate Agent
    let mut agent = do_core::Agent::new(&model, &backend, registry, sandbox_policy);
    agent.temperature = config.temperature;
    agent.seed = config.seed;
    agent.use_grammar = !no_grammar;

    println!("[Agent] Starting turn loop for session '{}'...", session_id);
    agent.run_turn_loop(&mut session.messages, config.ctx_size, config.threads).await?;

    // 5. Save session
    let db_path = get_db_path()?;
    let mut store = do_store::Store::open(db_path)?;
    store.save_session(session_id, &session)?;
    println!("[Agent] Session saved to database.");

    Ok(())
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    // Map CLI overrides to partial config
    let cli_overrides = do_config::PartialConfig {
        model_path: cli.model,
        backend: cli.backend,
        ctx_size: cli.ctx_size,
        gpu_layers: cli.gpu_layers,
        ..Default::default()
    };

    let (global_path, local_path) = get_config_paths();
    let config = do_config::DoheeConfig::load_layered(
        global_path.as_deref(),
        local_path.as_deref(),
        cli_overrides,
    ).context("Failed to load layered configuration")?;

    // Determine the command
    let command_to_run = if let Some(cmd) = cli.command {
        Some(cmd)
    } else if let Some(raw_prompt) = cli.prompt.clone() {
        Some(Commands::Run { prompt: raw_prompt })
    } else {
        None
    };

    match command_to_run {
        Some(Commands::Chat) => {
            println!("Launching interactive TUI chat...");
            dohee_tui::run_tui(config, cli.prompt).await?;
        }
        Some(Commands::Run { prompt }) => {
            let session_id = format!(
                "session-{}",
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_secs()
            );
            
            let mut session = do_core::Session {
                messages: Vec::new(),
                created_at: std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_secs(),
                compaction_generation: 0,
            };

            session.messages.push(do_core::Message {
                role: "user".to_string(),
                content: prompt,
                name: None,
            });

            run_agent_session(&config, &session_id, session, cli.no_grammar).await?;
        }
        Some(Commands::ConfigShow) => {
            println!("=== Merged Configuration ===");
            println!("Model Path:     {:?}", config.model_path);
            println!("Backend:        {}", config.backend);
            println!("Context Size:   {}", config.ctx_size);
            println!("Temperature:    {}", config.temperature);
            println!("Seed:           {}", config.seed);
            println!("GPU Layers:     {}", config.gpu_layers);
            println!("Threads:        {:?}", config.threads);
            println!("Sandbox Policy: {}", config.sandbox_policy);
            println!("Allowed Tools:  {:?}", config.allowed_tools);
            println!("Denied Tools:   {:?}", config.denied_tools);
        }
        Some(Commands::ModelsList) => {
            let models_dir = Path::new("models");
            if models_dir.exists() && models_dir.is_dir() {
                println!("=== Available GGUF Models in 'models/' ===");
                let mut found = false;
                for entry in fs::read_dir(models_dir)? {
                    let entry = entry?;
                    let path = entry.path();
                    if path.is_file() && path.extension().map_or(false, |ext| ext == "gguf") {
                        println!("  - {}", path.file_name().unwrap().to_string_lossy());
                        found = true;
                    }
                }
                if !found {
                    println!("No .gguf files found in models/.");
                }
            } else {
                println!("Local 'models/' directory not found in current directory.");
            }
        }
        Some(Commands::SessionsList) => {
            let db_path = get_db_path()?;
            if db_path.exists() {
                let store = do_store::Store::open(db_path)?;
                let list = store.list_sessions()?;
                println!("=== Past Agent Sessions ===");
                for (id, created_at) in list {
                    println!("  - ID: {} (Created: {})", id, created_at);
                }
            } else {
                println!("No past sessions database found.");
            }
        }
        Some(Commands::SessionsResume { session_id }) => {
            let db_path = get_db_path()?;
            let store = do_store::Store::open(db_path)?;
            let loaded = store.load_session(&session_id)?;
            
            if let Some(session) = loaded {
                run_agent_session(&config, &session_id, session, cli.no_grammar).await?;
            } else {
                println!("Error: Session '{}' not found in store.", session_id);
            }
        }
        Some(Commands::Doctor) => {
            println!("=== Dohee Doctor Diagnostic Report ===");
            println!("OS:             {}", std::env::consts::OS);
            println!("Arch:           {}", std::env::consts::ARCH);
            println!("--------------------------------------");

            // 1. Check Config & Model path
            print!("Checking model path... ");
            if let Some(ref model_path) = config.model_path {
                if model_path.exists() {
                    println!("OK (Exists at: {})", model_path.display());
                } else {
                    println!("ERROR (Path specified but file not found: {})", model_path.display());
                }
            } else {
                println!("WARNING (No model path specified in configuration)");
            }

            // 2. Check Backend
            print!("Initializing llama.cpp backend... ");
            match LlamaBackend::init() {
                Ok(_) => {
                    println!("OK (LlamaBackend initialized successfully)");
                }
                Err(e) => {
                    println!("ERROR (Failed to initialize llama.cpp backend: {:?})", e);
                }
            }

            // 3. Check Sandboxing Support (Landlock)
            print!("Checking Landlock sandboxing support... ");
            #[cfg(target_os = "linux")]
            {
                match do_sandbox::check_support() {
                    Ok(_) => {
                        println!("OK (Landlock is supported and active on this kernel)");
                    }
                    Err(e) => {
                        println!("WARNING (Landlock not supported or failed to initialize: {:?})", e);
                    }
                }
            }
            #[cfg(not(target_os = "linux"))]
            {
                println!("UNSUPPORTED (Landlock LSM sandboxing is only available on Linux)");
            }
            println!("--------------------------------------");
        }
        None => {
            println!("No subcommand or prompt provided. Run 'dohee --help' for details.");
        }
    }

    Ok(())
}
