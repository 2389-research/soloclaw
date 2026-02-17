// ABOUTME: LLM provider factory — creates the right client based on config.
// ABOUTME: Supports anthropic, openai, gemini, openrouter, and ollama.

use std::sync::Arc;

use mux::llm::{
    AnthropicClient, GeminiClient, LlmClient, OllamaClient, OpenAIClient, OpenRouterClient,
};

use crate::config::LlmConfig;

/// Create an LLM client based on the provider name in config.
pub fn create_client(config: &LlmConfig) -> anyhow::Result<Arc<dyn LlmClient>> {
    match config.provider.as_str() {
        "anthropic" => {
            let mut client = AnthropicClient::from_env()?;
            if let Some(url) = config
                .anthropic
                .base_url
                .as_deref()
                .filter(|s| !s.is_empty())
            {
                client = client.with_base_url(url);
            }
            Ok(Arc::new(client))
        }
        "openai" => {
            let mut client = OpenAIClient::from_env()?;
            if let Some(url) = config.openai.base_url.as_deref().filter(|s| !s.is_empty()) {
                client = client.with_base_url(url);
            }
            Ok(Arc::new(client))
        }
        "gemini" => {
            let mut client = GeminiClient::from_env()?;
            if let Some(url) = config.gemini.base_url.as_deref().filter(|s| !s.is_empty()) {
                client = client.with_base_url(url);
            }
            Ok(Arc::new(client))
        }
        "openrouter" => {
            if let Some(url) = config
                .openrouter
                .base_url
                .as_deref()
                .filter(|s| !s.is_empty())
            {
                let client = OpenAIClient::openrouter_from_env()?.with_base_url(url);
                Ok(Arc::new(client))
            } else {
                let client = OpenRouterClient::from_env()?;
                Ok(Arc::new(client))
            }
        }
        "ollama" => {
            let base_url = format!("{}/v1", config.ollama.base_url.trim_end_matches('/'));
            let client = OllamaClient::with_base_url(&base_url, &config.model);
            Ok(Arc::new(client))
        }
        other => anyhow::bail!(
            "Unknown LLM provider: '{}'. Expected: anthropic, openai, gemini, openrouter, ollama",
            other
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unknown_provider_errors() {
        let config = LlmConfig {
            provider: "fakeprovider".to_string(),
            ..Default::default()
        };
        let result = create_client(&config);
        assert!(result.is_err());
        let err = result.err().unwrap();
        assert!(err.to_string().contains("fakeprovider"));
    }
}
