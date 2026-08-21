//! 端到端：用真实 provider 验证循环改造后的行为
//!
//! stage_b 只验证了三个决策点被调用，没有跑完整的循环 —— 因为那需要
//! 一个会回话的模型。这个用 DeepSeek v4 flash 补上。
//!
//! 验证：
//!   1. 默认策略下，改造后的循环仍能完成一次带工具调用的完整对话
//!   2. ContextAssembler 改写 system prompt 会真的影响模型的回答
//!      —— 这是"装配器确实作用于送出去的请求"的硬证据
//!   3. ResultProcessor 改写工具结果会真的影响模型看到的东西
//!   4. StopPolicy 能在多轮对话中途截停
//!
//! 需要 DEEPSEEK_API_KEY。

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;

use async_trait::async_trait;
use pi_agent::policy::{
    NoObserver,
    Assembled, ContextAssembler, LoopPolicy, PassThroughAssembler, PassThroughProcessor,
    ResultProcessor, StopPolicy, TurnLimitPolicy,
};
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

/// 一个确定性的工具 —— 模型调用它，我们检查结果如何进入对话。
struct SecretTool {
    calls: Arc<AtomicU32>,
}

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
        self.calls.fetch_add(1, Ordering::SeqCst);
        Ok(AgentToolResult::text("RAW-SECRET-1234"))
    }
}

/// 把工具输出改写掉 —— 如果模型复述的是改写后的值，就证明
/// ResultProcessor 确实作用在模型看到的东西上。
struct RedactingProcessor {
    calls: Arc<AtomicU32>,
}

#[async_trait]
impl ResultProcessor for RedactingProcessor {
    async fn process(
        &self,
        tool_name: &str,
        content: Vec<Content>,
        is_error: bool,
    ) -> (Vec<Content>, bool) {
        self.calls.fetch_add(1, Ordering::SeqCst);
        if tool_name == "get_secret_code" {
            return (vec![Content::text("REDACTED-BY-POLICY")], is_error);
        }
        (content, is_error)
    }
}

/// 往 system prompt 里塞一个指令 —— 如果模型照做了，就证明
/// ContextAssembler 确实作用在送出去的请求上。
struct InjectingAssembler;

#[async_trait]
impl ContextAssembler for InjectingAssembler {
    async fn assemble(&self, _turn: u32, sp: &str, m: &[Message]) -> Assembled {
        Assembled {
            system_prompt: format!("{sp}\nAlways end your reply with the word PINEAPPLE."),
            messages: m.to_vec(),
        }
    }
}

/// 第 N 轮截停
struct StopAfter {
    n: u32,
}

#[async_trait]
impl StopPolicy for StopAfter {
    async fn should_stop(&self, turn: u32, _m: &[Message]) -> Option<String> {
        if turn >= self.n {
            Some(format!("custom: stopped after {} turn(s)", self.n))
        } else {
            None
        }
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

#[tokio::main(flavor = "multi_thread")]
async fn main() -> Result<(), String> {
    if std::env::var("DEEPSEEK_API_KEY").is_err() {
        return Err("DEEPSEEK_API_KEY not set".into());
    }

    // ── 1. 默认策略：完整走一轮带工具调用的对话 ───────────────
    println!("[e2e] 1. default policy, full tool-calling round");
    let calls = Arc::new(AtomicU32::new(0));
    let cfg = AgentConfig::new(deepseek_flash(), "You are a terse assistant.")
        .with_tools(vec![Arc::new(SecretTool {
            calls: calls.clone(),
        })])
        .with_max_turns(4)
        .with_stream_options(opts());

    let run = run_agent(
        &cfg,
        Message::user_text("What is the secret code? Use the tool, then tell me the value."),
        None,
    )
    .await
    .map_err(|e| format!("{e:?}"))?;

    let reply = last_text(&run.messages);
    println!("[e2e]    tool called {} time(s)", calls.load(Ordering::SeqCst));
    println!("[e2e]    reply: {}", reply.trim());
    assert!(
        calls.load(Ordering::SeqCst) >= 1,
        "the model actually called the tool"
    );
    assert!(
        reply.contains("1234"),
        "the raw tool output reached the model (default processor passes through)"
    );
    println!("[e2e]    OK — the rebuilt loop completes a real conversation");

    // ── 2. ResultProcessor 改写模型看到的工具输出 ─────────────
    println!("[e2e] 2. ResultProcessor rewrites what the model sees");
    let calls2 = Arc::new(AtomicU32::new(0));
    let proc_calls = Arc::new(AtomicU32::new(0));
    let cfg2 = AgentConfig::new(deepseek_flash(), "You are a terse assistant.")
        .with_tools(vec![Arc::new(SecretTool {
            calls: calls2.clone(),
        })])
        .with_max_turns(4)
        .with_stream_options(opts())
        .with_loop_policy(LoopPolicy {
            assembler: Arc::new(PassThroughAssembler),
            processor: Arc::new(RedactingProcessor {
                calls: proc_calls.clone(),
            }),
            stop: Arc::new(TurnLimitPolicy { max_turns: 4 }),
            observer: Arc::new(NoObserver),
        });

    let run2 = run_agent(
        &cfg2,
        Message::user_text("What is the secret code? Use the tool, then tell me the exact value it returned."),
        None,
    )
    .await
    .map_err(|e| format!("{e:?}"))?;

    let reply2 = last_text(&run2.messages);
    println!("[e2e]    processor called {} time(s)", proc_calls.load(Ordering::SeqCst));
    println!("[e2e]    reply: {}", reply2.trim());
    assert!(
        proc_calls.load(Ordering::SeqCst) >= 1,
        "processor ran"
    );
    assert!(
        !reply2.contains("1234"),
        "the raw value never reached the model"
    );
    println!("[e2e]    OK — redaction is effective, not cosmetic");

    // ── 3. ContextAssembler 改写送出去的 system prompt ────────
    println!("[e2e] 3. ContextAssembler affects the outgoing request");
    let cfg3 = AgentConfig::new(deepseek_flash(), "You are a terse assistant.")
        .with_max_turns(2)
        .with_stream_options(opts())
        .with_loop_policy(LoopPolicy {
            assembler: Arc::new(InjectingAssembler),
            processor: Arc::new(PassThroughProcessor),
            stop: Arc::new(TurnLimitPolicy { max_turns: 2 }),
            observer: Arc::new(NoObserver),
        });

    let run3 = run_agent(&cfg3, Message::user_text("Say hello."), None)
        .await
        .map_err(|e| format!("{e:?}"))?;
    let reply3 = last_text(&run3.messages);
    println!("[e2e]    reply: {}", reply3.trim());
    assert!(
        reply3.to_uppercase().contains("PINEAPPLE"),
        "the injected instruction reached the model"
    );
    println!("[e2e]    OK — the assembler is on the real request path");

    // ── 4. StopPolicy 截停 ───────────────────────────────────
    println!("[e2e] 4. StopPolicy halts the loop");
    let calls4 = Arc::new(AtomicU32::new(0));
    let cfg4 = AgentConfig::new(deepseek_flash(), "You are a terse assistant.")
        .with_tools(vec![Arc::new(SecretTool {
            calls: calls4.clone(),
        })])
        .with_stream_options(opts())
        .with_loop_policy(LoopPolicy {
            assembler: Arc::new(PassThroughAssembler),
            processor: Arc::new(PassThroughProcessor),
            // 0 -> 一轮都不跑
            stop: Arc::new(StopAfter { n: 0 }),
            observer: Arc::new(NoObserver),
        });

    let run4 = run_agent(&cfg4, Message::user_text("What is the secret code?"), None)
        .await
        .map_err(|e| format!("{e:?}"))?;
    println!("[e2e]    tool called {} time(s)", calls4.load(Ordering::SeqCst));
    println!("[e2e]    stopped_at_turn_limit = {}", run4.stopped_at_turn_limit);
    assert_eq!(
        calls4.load(Ordering::SeqCst),
        0,
        "nothing ran — stopped before the first turn"
    );
    assert!(
        !run4.stopped_at_turn_limit,
        "a custom stop is not a turn-limit truncation"
    );
    println!("[e2e]    OK");

    println!("[e2e] === all four verified against a real provider ===");
    Ok(())
}
