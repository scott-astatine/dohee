use anyhow::{Context, Result};
use clap::Parser;
use dohee_infer::{DoheeModel, InferenceSession, default_sampler};
use llama_cpp_2::llama_backend::LlamaBackend;
use std::io::Write;
use std::path::PathBuf;

#[derive(Parser, Debug)]
struct Args {
    /// Path to GGUF model
    #[arg(short, long, default_value = "models/qwen2.5-1.5b-instruct-q4_k_m.gguf")]
    model: PathBuf,

    /// Text prompt to generate from
    #[arg(short, long, default_value = "안녕하세요! 자기소개 부탁드립니다.")]
    prompt: String,

    /// Number of GPU layers to offload
    #[arg(short, long, default_value_t = 99)]
    gpu_layers: u32,

    /// Context size
    #[arg(short, long, default_value_t = 2048)]
    ctx_size: u32,
}

fn main() -> Result<()> {
    let args = Args::parse();

    println!("Initializing backend...");
    let backend = LlamaBackend::init().context("Failed to init llama backend")?;

    println!("Loading model from {}...", args.model.display());
    let model = DoheeModel::new(&backend, &args.model, args.gpu_layers)
        .context("Failed to load model")?;

    println!("Vocab size: {}", model.n_vocab());
    println!("Train context size: {}", model.n_ctx_train());

    println!("Creating inference session...");
    let mut session = InferenceSession::new(&backend, &model, args.ctx_size, None)
        .context("Failed to create session")?;

    println!("\nPrompt: \"{}\"", args.prompt);
    print!("Response: ");
    std::io::stdout().flush()?;

    session.advance(&model, &args.prompt).context("Failed to advance prompt")?;

    let mut sampler = default_sampler(1234, 0.7);

    let mut token_count = 0;
    while let Some(piece) = session.sample_next(&model, &mut sampler).context("Failed to sample next token")? {
        print!("{}", piece);
        std::io::stdout().flush()?;
        token_count += 1;
        if token_count > 256 {
            break;
        }
    }
    println!();
    println!("\nSuccessfully generated {} tokens.", token_count);

    Ok(())
}
