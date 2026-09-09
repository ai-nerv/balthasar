//! A real transformer, when somebody has asked for one and put the weights on disk.
//!
//! For retrieval only. Dense sentence embeddings barely move when one entity is swapped for
//! another, so deciding two claims are one claim stays [`balthasar_distil::same_claim`]'s job.
//!
//! Behind a feature because the weights are 127 MB and the tensor library is a large dependency.

use crate::{Embed, EmbedError};
use candle_core::{DType, Device, Tensor};
use candle_nn::VarBuilder;
use candle_transformers::models::bert::{BertModel, Config};
use std::path::Path;

/// The longest piece of text this will embed, in tokens.
///
/// BERT's positional embeddings stop at 512. Truncated rather than refused, so a memory too long
/// to embed is still findable lexically.
const MOST_TOKENS: usize = 512;

/// A sentence transformer, loaded from a directory of weights.
pub struct Dense {
    model: BertModel,
    tokenizer: tokenizers::Tokenizer,
    device: Device,
    name: String,
    dimensions: usize,
}

impl std::fmt::Debug for Dense {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Dense")
            .field("name", &self.name)
            .field("dimensions", &self.dimensions)
            .finish_non_exhaustive()
    }
}

impl Dense {
    /// Load a model from a directory holding `config.json`, `tokenizer.json` and
    /// `model.safetensors`.
    pub fn open(dir: &Path, name: &str) -> Result<Self, EmbedError> {
        let missing = |file: &str, why: String| {
            EmbedError::Unavailable(
                name.to_owned(),
                format!("{}: {why}", dir.join(file).display()),
            )
        };

        let config = std::fs::read_to_string(dir.join("config.json"))
            .map_err(|e| missing("config.json", e.to_string()))?;
        let config: Config = serde_json::from_str(&config)
            .map_err(|e| missing("config.json", format!("not a bert config: {e}")))?;
        let dimensions = config.hidden_size;

        let tokenizer = tokenizers::Tokenizer::from_file(dir.join("tokenizer.json"))
            .map_err(|e| missing("tokenizer.json", e.to_string()))?;

        let device = Device::Cpu;
        // Read, not mapped: `from_mmaped_safetensors` is `unsafe` and this workspace denies it.
        let weights = std::fs::read(dir.join("model.safetensors"))
            .map_err(|e| missing("model.safetensors", e.to_string()))?;
        let vars = VarBuilder::from_buffered_safetensors(weights, DType::F32, &device)
            .map_err(|e| missing("model.safetensors", e.to_string()))?;
        let model = BertModel::load(vars, &config)
            .map_err(|e| missing("model.safetensors", format!("not a bert: {e}")))?;

        Ok(Self {
            model,
            tokenizer,
            device,
            name: name.to_owned(),
            dimensions,
        })
    }

    /// One batch, forward.
    fn forward(&self, texts: &[String]) -> Result<Vec<Vec<f32>>, EmbedError> {
        let fail = |why: String| EmbedError::Failed(why);

        let mut ids = Vec::with_capacity(texts.len());
        let mut mask = Vec::with_capacity(texts.len());
        let mut longest = 1;

        for text in texts {
            let encoded = self
                .tokenizer
                .encode(text.as_str(), true)
                .map_err(|e| fail(e.to_string()))?;
            let mut token: Vec<u32> = encoded.get_ids().to_vec();
            token.truncate(MOST_TOKENS);
            longest = longest.max(token.len());
            ids.push(token);
        }

        // One rectangle, padded, with a mask saying which of it is real: attention over padding
        // would make embeddings depend on ingest order.
        for token in &mut ids {
            mask.push(
                std::iter::repeat_n(1u32, token.len())
                    .chain(std::iter::repeat_n(0u32, longest - token.len()))
                    .collect::<Vec<_>>(),
            );
            token.resize(longest, 0);
        }

        let rows = ids.len();
        let flat: Vec<u32> = ids.into_iter().flatten().collect();
        let flat_mask: Vec<u32> = mask.into_iter().flatten().collect();

        let ids = Tensor::from_vec(flat, (rows, longest), &self.device).map_err(as_failure)?;
        let mask =
            Tensor::from_vec(flat_mask, (rows, longest), &self.device).map_err(as_failure)?;
        let kinds = ids.zeros_like().map_err(as_failure)?;

        let hidden = self
            .model
            .forward(&ids, &kinds, Some(&mask))
            .map_err(as_failure)?;

        // The CLS token, which is what BGE models are trained to pool on; mean pooling over the
        // sequence is the wrong recipe for these weights.
        let cls = hidden.i((.., 0)).map_err(as_failure)?;
        let out: Vec<Vec<f32>> = cls.to_vec2().map_err(as_failure)?;

        Ok(out
            .into_iter()
            .map(|mut vector| {
                crate::normalise(&mut vector);
                vector
            })
            .collect())
    }
}

use candle_core::IndexOp;

/// A tensor error, as this crate reports failures.
fn as_failure(why: candle_core::Error) -> EmbedError {
    EmbedError::Failed(why.to_string())
}

impl Embed for Dense {
    fn model(&self) -> &str {
        &self.name
    }

    fn dimensions(&self) -> usize {
        self.dimensions
    }

    fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>, EmbedError> {
        if texts.is_empty() {
            return Ok(Vec::new());
        }
        self.forward(texts)
    }
}
