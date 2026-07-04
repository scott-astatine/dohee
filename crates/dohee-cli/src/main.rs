use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use dohee_config as do_config;
use dohee_infer as do_infer;
use llama_cpp_2::llama_backend::LlamaBackend;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

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

    /// Optional raw prompt for single-shot execution
    prompt: Option<String>,

    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand, Debug, Clone)]
enum Commands {
    /// Start an interactive chat session (Stub for Phase 4)
    Chat,
    /// Run a single-shot prompt
    Run {
        /// The prompt to execute
        prompt: String,
    },
    /// Show the merged configuration
    ConfigShow,
    /// List available models in the models directory
    ModelsList,
}

fn get_config_paths() -> (Option<PathBuf>, Option<PathBuf>) {
    let home = std::env::var("HOME").ok();
    let global_config = home.map(|h| PathBuf::from(h).join(".config/dohee/config.toml"));
    let local_config = Some(PathBuf::from(".dohee.toml"));
    (global_config, local_config)
}

fn execute_prompt(config: &do_config::DoheeConfig, prompt: &str) -> Result<()> {
    // Validate model path exists first
    config.validate().context("Configuration validation failed")?;

    let model_path = config.model_path.as_ref().context("Model path not specified")?;

    println!("Initializing llama.cpp backend...");
    let backend = LlamaBackend::init().context("Failed to initialize llama.cpp backend")?;

    println!("Loading model from {}...", model_path.display());
    // Map backend config
    #[cfg(any(feature = "cuda", feature = "vulkan"))]
    let gpu_layers = if config.backend == "cpu" { 0 } else { config.gpu_layers };
    #[cfg(not(any(feature = "cuda", feature = "vulkan")))]
    let gpu_layers = 0;

    let model = do_infer::DoheeModel::new(&backend, model_path, gpu_layers)
        .context("Failed to load model in-process")?;

    println!("Vocab size: {}", model.n_vocab());
    println!("Train context size: {}", model.n_ctx_train());

    println!("Creating inference session...");
    let mut session = do_infer::InferenceSession::new(&backend, &model, config.ctx_size, config.threads)
        .context("Failed to initialize inference session")?;

    println!("\nGenerating stream for: \"{}\"", prompt);
    print!("Response: ");
    std::io::stdout().flush()?;

    session.advance(&model, prompt).context("Failed to process prompt")?;

    let mut sampler = do_infer::default_sampler(config.seed, config.temperature);

    let mut token_count = 0;
    while let Some(piece) = session.sample_next(&model, &mut sampler).context("Failed to sample next token")? {
        print!("{}", piece);
        std::io::stdout().flush()?;
        token_count += 1;
        if token_count > 512 {
            break;
        }
    }
    println!();
    println!("\nDone (generated {} tokens).", token_count);

    Ok(())
}

fn main() -> Result<()> {
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
    } else if let Some(raw_prompt) = cli.prompt {
        Some(Commands::Run { prompt: raw_prompt })
    } else {
        None
    };

    match command_to_run {
        Some(Commands::Chat) => {
            println!("Interactive chat mode is a stub for now. Will be fully wired in Phase 7.");
        }
        Some(Commands::Run { prompt }) => {
            execute_prompt(&config, &prompt)?;
        }
        Some(Commands::ConfigShow) => {
            println!("=== Merged Configuration ===");
            println!("Model Path:     {:?}", config.model_path);
            println!("Backend:        {}", config.backend);
            println!("Context Size:   {}", config.ctx_size);
            println!("Temperature:    {}", config.temperature);
            assert!(config.temperature >= 0.0);
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
        None => {
            println!("No subcommand or prompt provided. Run 'dohee --help' for details.");
        }
    }

    Ok(())
}
