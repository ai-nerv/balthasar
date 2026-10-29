#![cfg(feature = "dense")]

mod support;

use balthasar_embed::{Dense, Embed};
use candle_core::{DType, Device};
use candle_nn::{VarBuilder, VarMap};
use candle_transformers::models::bert::{BertModel, Config};
use support::{Fixture, TestResult};

fn fixture() -> TestResult<Fixture> {
    let world = Fixture::new()?;
    let config = serde_json::json!({
        "vocab_size": 5, "hidden_size": 8, "num_hidden_layers": 1,
        "num_attention_heads": 2, "intermediate_size": 16, "hidden_act": "gelu",
        "hidden_dropout_prob": 0.0, "max_position_embeddings": 512,
        "type_vocab_size": 2, "initializer_range": 0.02, "layer_norm_eps": 1e-12,
        "pad_token_id": 0, "classifier_dropout": null, "model_type": "bert"
    });
    std::fs::write(world.0.join("config.json"), serde_json::to_vec(&config)?)?;
    let config: Config = serde_json::from_value(config)?;
    let vars = VarMap::new();
    let _model = BertModel::load(
        VarBuilder::from_varmap(&vars, DType::F32, &Device::Cpu),
        &config,
    )?;
    vars.save(world.0.join("model.safetensors"))?;
    let vocab = [
        ("[PAD]", 0),
        ("[UNK]", 1),
        ("alpha", 2),
        ("beta", 3),
        ("gamma", 4),
    ]
    .into_iter()
    .map(|(word, id)| (word.to_owned(), id))
    .collect();
    let words = tokenizers::models::wordlevel::WordLevel::builder()
        .vocab(vocab)
        .unk_token("[UNK]".to_owned())
        .build()?;
    let mut tokenizer = tokenizers::Tokenizer::new(words);
    tokenizer.with_pre_tokenizer(Some(tokenizers::pre_tokenizers::whitespace::Whitespace));
    tokenizer.save(world.0.join("tokenizer.json"), false)?;
    Ok(world)
}

#[test]
fn dense_loads_local_weights_and_embeds_masked_batches() -> TestResult {
    let world = fixture()?;
    let model = Dense::open(&world.0, "synthetic-tiny-bert")?;
    assert_eq!(model.model(), "synthetic-tiny-bert");
    assert_eq!(model.dimensions(), 8);
    assert!(model.embed(&[])?.is_empty());
    let alone = model.embed(&["alpha".to_owned()])?;
    let batch = model.embed(&["alpha".to_owned(), "alpha beta gamma".to_owned()])?;
    assert_eq!(batch.len(), 2);
    for vector in &batch {
        assert_eq!(vector.len(), 8);
        assert!(vector.iter().all(|value| value.is_finite()));
        assert!((vector.iter().map(|value| value * value).sum::<f32>() - 1.).abs() < 1e-4);
    }
    let a = alone.first().ok_or("no single embedding")?;
    let b = batch.first().ok_or("no batch embedding")?;
    assert!(a.iter().zip(b).all(|(a, b)| (a - b).abs() < 1e-4));
    let long = model.embed(&["alpha ".repeat(600)])?;
    assert_eq!(long.first().ok_or("no truncated embedding")?.len(), 8);
    std::fs::write(world.0.join("model.safetensors"), b"invalid weights")?;
    assert!(Dense::open(&world.0, "broken").is_err());
    Ok(())
}
