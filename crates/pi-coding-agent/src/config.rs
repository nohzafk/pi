use std::path::PathBuf;

use pi_ai::Model;

pub const APP_NAME: &str = "pi";

#[derive(Debug, Clone)]
pub struct AppConfig {
    pub model: Model,
    pub max_turns: u32,
    pub config_dir: PathBuf,
}

impl Default for AppConfig {
    fn default() -> Self {
        let config_dir = dirs::config_dir()
            .map(|p| p.join(APP_NAME))
            .unwrap_or_else(|| PathBuf::from(".pi"));
        Self {
            model: default_model_from_env(),
            max_turns: 32,
            config_dir,
        }
    }
}

pub fn default_model_from_env() -> Model {
    if let Ok(id) = std::env::var("PI_MODEL") {
        match id.as_str() {
            "claude-sonnet-4-6" | "claude-sonnet" | "sonnet" => {
                return Model::anthropic_claude_sonnet_4_6();
            }
            "claude-opus-4-7" | "claude-opus" | "opus" => {
                return Model::anthropic_claude_opus_4_7();
            }
            "gpt-4o" => return Model::openai_gpt_4o(),
            "gpt-4o-mini" => return Model::openai_gpt_4o_mini(),
            _ => {}
        }
    }
    if std::env::var("ANTHROPIC_API_KEY").is_ok() {
        Model::anthropic_claude_sonnet_4_6()
    } else if std::env::var("OPENAI_API_KEY").is_ok() {
        Model::openai_gpt_4o_mini()
    } else {
        Model::anthropic_claude_sonnet_4_6()
    }
}
