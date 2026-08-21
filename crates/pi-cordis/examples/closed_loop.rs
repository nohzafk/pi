//! A+B 闭环：插件替换循环决策点，卸载后自动恢复
//!
//! 这是整件事的验收。前面各阶段验证的是"机制能工作"，这里验证的是
//! **agent 的行为由一份可装卸的配置决定**：
//!
//!   1. 装上插件 -> 它的策略生效，模型的回答随之改变
//!   2. 卸载插件 -> 行为自动回到默认，没有一行清理代码
//!
//! 第 2 条是 Cordis 相对普通插件系统的全部意义所在。
//!
//! 需要 DEEPSEEK_API_KEY。

use std::sync::Arc;

use async_trait::async_trait;
use pi_agent::types::{AgentTool, AgentToolResult};
use pi_agent::{run_agent, AgentConfig};
use pi_ai::{Content, Message, Model, ModelPricing, StreamOptions};
use serde_json::{json, Value as Json};

fn deepseek_flash() -> Model {
    Model {
        id: "deepseek-v4-flash".into(),
        name: "DeepSeek V4 Flash".into(),
        api: "openai-completions".into(),
        provider: "deepseek".into(),
        base_url: "https://api.deepseek.com".into(),
        reasoning: false,
        context_window: 65536,
        max_tokens: 2048,
        pricing: ModelPricing::default(),
    }
}

fn opts() -> StreamOptions {
    let mut o = StreamOptions::default();
    o.api_key = std::env::var("DEEPSEEK_API_KEY").ok();
    o
}

struct SecretTool;

#[async_trait]
impl AgentTool for SecretTool {
    fn name(&self) -> &str {
        "get_secret_code"
    }
    fn description(&self) -> &str {
        "Returns the secret code. Call this when asked for the secret code."
    }
    fn parameters(&self) -> Json {
        json!({"type": "object", "properties": {}, "required": []})
    }
    async fn execute(&self, _id: &str, _args: Json) -> Result<AgentToolResult, String> {
        Ok(AgentToolResult::text("RAW-SECRET-1234"))
    }
}

fn last_text(messages: &[Message]) -> String {
    for m in messages.iter().rev() {
        if let Message::Assistant(a) = m {
            let t: String = a
                .content
                .iter()
                .filter_map(|c| match c {
                    Content::Text { text } => Some(text.clone()),
                    _ => None,
                })
                .collect();
            if !t.trim().is_empty() {
                return t;
            }
        }
    }
    String::new()
}

async fn ask(gov: &pi_cordis::GovernedConfig, prompt: &str) -> Result<String, String> {
    let cfg = AgentConfig::new(deepseek_flash(), gov.system_prompt.clone())
        .with_tools(vec![Arc::new(SecretTool)])
        .with_max_turns(4)
        .with_stream_options(opts())
        .with_loop_policy(gov.loop_policy.clone());
    let run = run_agent(&cfg, Message::user_text(prompt), None)
        .await
        .map_err(|e| format!("{e:?}"))?;
    Ok(last_text(&run.messages))
}

#[tokio::main(flavor = "multi_thread")]
async fn main() -> Result<(), String> {
    if std::env::var("DEEPSEEK_API_KEY").is_err() {
        return Err("DEEPSEEK_API_KEY not set".into());
    }
    let cordis_dir = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "/Users/randall/projects/cordis-elle".to_string());

    // ── 一个把策略写在 Elle 里的插件 ──────────────────────────
    // 它做两件事：往 system prompt 加一句指令；把工具输出脱敏。
    let plugin = r#"@[
      @{:id "policy-pack" :url "policy.lisp"
        :factory (fn [config]
                   (fn [fib]
                     ((C :set) fib :assemble-handler
                       (fn [turn sp n]
                         (string sp "\nAlways end your reply with the word PINEAPPLE.")))
                     ((C :set) fib :process-handler
                       (fn [tool-name body is-error]
                         (if (= tool-name "get_secret_code")
                           "REDACTED-BY-PLUGIN"
                           nil)))))
        :config @{}
        :provide @[:assemble-handler :process-handler]}
    ]"#;

    println!("[closed-loop] booting with the policy plugin...");
    let ok = pi_cordis::boot(cordis_dir, vec![], plugin)?;
    if ok.trim() != "true" {
        return Err(format!("config rejected: {ok}"));
    }

    // ── 1. 插件装上时：两条策略都该生效 ───────────────────────
    let gov = pi_cordis::governed_config("You are a terse assistant.", 4)?;
    println!("[closed-loop] 1. plugin loaded — asking the model");
    let reply = ask(
        &gov,
        "What is the secret code? Use the tool, then tell me the exact value it returned.",
    )
    .await?;
    println!("[closed-loop]    reply: {}", reply.trim());

    assert!(
        reply.to_uppercase().contains("PINEAPPLE"),
        "the plugin's assembler reached the model"
    );
    assert!(
        !reply.contains("1234"),
        "the plugin's processor redacted the tool output"
    );
    println!("[closed-loop]    OK — both plugin policies are in effect");

    // ── 2. 卸载插件：行为该自动回到默认 ───────────────────────
    println!("[closed-loop] 2. unloading the plugin...");
    pi_cordis::CordisRuntime::eval(
        r#"(elle/epoch 12)
           ((L :apply) @[])
           :unloaded"#,
    )?;

    let gov2 = pi_cordis::governed_config("You are a terse assistant.", 4)?;
    let reply2 = ask(
        &gov2,
        "What is the secret code? Use the tool, then tell me the exact value it returned.",
    )
    .await?;
    println!("[closed-loop]    reply: {}", reply2.trim());

    assert!(
        !reply2.to_uppercase().contains("PINEAPPLE"),
        "the injected instruction is gone"
    );
    assert!(
        reply2.contains("1234"),
        "redaction lifted — the raw value reaches the model again"
    );
    println!("[closed-loop]    OK — default behaviour restored, no cleanup code was written");

    pi_cordis::CordisRuntime::shutdown();
    println!("[closed-loop] === the agent's behaviour is a loadable configuration ===");
    Ok(())
}
