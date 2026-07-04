use anyhow::{Context, Result};
use llama_cpp_2::context::params::LlamaContextParams;
use llama_cpp_2::llama_backend::LlamaBackend;
use llama_cpp_2::llama_batch::LlamaBatch;
use llama_cpp_2::model::params::LlamaModelParams;
use llama_cpp_2::model::AddBos;
use llama_cpp_2::model::LlamaModel;
use llama_cpp_2::sampling::LlamaSampler;
use std::num::NonZeroU32;
use std::path::Path;

pub struct DoheeModel {
    pub model: LlamaModel,
}

impl DoheeModel {
    pub fn new(backend: &LlamaBackend, path: impl AsRef<Path>, gpu_layers: u32) -> Result<Self> {
        #[allow(unused_mut)]
        let mut model_params = LlamaModelParams::default();
        #[cfg(any(feature = "cuda", feature = "vulkan"))]
        {
            if gpu_layers > 0 {
                model_params = model_params.with_n_gpu_layers(gpu_layers);
            }
        }
        // Silence warning if features are disabled
        let _ = gpu_layers;

        let model = LlamaModel::load_from_file(backend, path, &model_params)
            .context("Failed to load llama model from file")?;
        Ok(Self { model })
    }

    pub fn n_vocab(&self) -> i32 {
        self.model.n_vocab()
    }

    pub fn n_ctx_train(&self) -> u32 {
        self.model.n_ctx_train()
    }

    pub fn tokenize(&self, text: &str, add_bos: AddBos) -> Result<Vec<llama_cpp_2::token::LlamaToken>> {
        self.model.str_to_token(text, add_bos)
            .context("Failed to tokenize text")
    }

    pub fn token_to_piece(&self, token: llama_cpp_2::token::LlamaToken, decoder: &mut encoding_rs::Decoder) -> Result<String> {
        self.model.token_to_piece(token, decoder, true, None)
            .context("Failed to decode token to piece")
    }

    pub fn is_eog(&self, token: llama_cpp_2::token::LlamaToken) -> bool {
        self.model.is_eog_token(token)
    }
}

pub struct InferenceSession<'a> {
    pub ctx: llama_cpp_2::context::LlamaContext<'a>,
    pub batch: LlamaBatch<'a>,
    pub n_past: i32,
    pub decoder: encoding_rs::Decoder,
}

impl<'a> InferenceSession<'a> {
    pub fn new(backend: &'a LlamaBackend, model: &'a DoheeModel, ctx_size: u32, threads: Option<i32>) -> Result<Self> {
        let mut ctx_params = LlamaContextParams::default();
        if ctx_size > 0 {
            ctx_params = ctx_params.with_n_ctx(Some(NonZeroU32::new(ctx_size).unwrap()));
        }
        if let Some(t) = threads {
            ctx_params = ctx_params.with_n_threads(t);
        }

        let ctx = model.model.new_context(backend, ctx_params)
            .context("Failed to create context")?;
        
        let batch = LlamaBatch::new(512, 1);
        
        Ok(Self {
            ctx,
            batch,
            n_past: 0,
            decoder: encoding_rs::UTF_8.new_decoder(),
        })
    }

    pub fn advance(&mut self, model: &DoheeModel, text: &str) -> Result<()> {
        let add_bos = if self.n_past == 0 {
            AddBos::Always
        } else {
            AddBos::Never
        };

        let tokens = model.tokenize(text, add_bos)?;
        if tokens.is_empty() {
            return Ok(());
        }

        self.batch.clear();
        let last_index = (tokens.len() - 1) as i32;
        for (i, token) in tokens.into_iter().enumerate() {
            let is_last = i as i32 == last_index;
            self.batch.add(token, self.n_past + i as i32, &[0], is_last)?;
        }

        self.ctx.decode(&mut self.batch)
            .context("Failed to decode prompt batch")?;

        self.n_past += self.batch.n_tokens();
        Ok(())
    }

    pub fn sample_next(&mut self, model: &DoheeModel, sampler: &mut LlamaSampler) -> Result<Option<String>> {
        let token = sampler.sample(&self.ctx, self.batch.n_tokens() - 1);
        sampler.accept(token);

        if model.is_eog(token) {
            return Ok(None);
        }

        let piece = model.token_to_piece(token, &mut self.decoder)?;

        self.batch.clear();
        self.batch.add(token, self.n_past, &[0], true)?;

        self.ctx.decode(&mut self.batch)
            .context("Failed to decode sampled token")?;

        self.n_past += 1;
        Ok(Some(piece))
    }
}

pub fn default_sampler(seed: u32, temperature: f32) -> LlamaSampler {
    if temperature <= 0.0 {
        LlamaSampler::chain_simple([LlamaSampler::greedy()])
    } else {
        LlamaSampler::chain_simple([
            LlamaSampler::temp(temperature),
            LlamaSampler::dist(seed),
            LlamaSampler::greedy(),
        ])
    }
}

pub fn grammar_sampler(
    model: &DoheeModel,
    seed: u32,
    temperature: f32,
    grammar_str: &str,
) -> Result<LlamaSampler> {
    let sampler = LlamaSampler::grammar(&model.model, grammar_str, "root")
        .context("Failed to construct grammar sampler")?;
    
    if temperature <= 0.0 {
        Ok(LlamaSampler::chain_simple([
            sampler,
            LlamaSampler::greedy(),
        ]))
    } else {
        Ok(LlamaSampler::chain_simple([
            LlamaSampler::temp(temperature),
            LlamaSampler::dist(seed),
            sampler,
            LlamaSampler::greedy(),
        ]))
    }
}
