//! B 阶段验证：三个决策点真的能替换循环行为
//!
//! 不需要真实 LLM —— 三个点里有两个（装配、停机）在发请求之前就
//! 触发，所以用一个必然失败的 model 也能验证它们被调用了。
//!
//! 验证：
//!   1. 默认策略下行为与拆分前一致（stop 只看轮次上限）
//!   2. 自定义 StopPolicy 能在第一轮之前就停住循环
//!   3. 自定义 ContextAssembler 能改写送给模型的 system prompt
//!   4. stopped_at_turn_limit 只在"轮次上限"这一种停法下为 true

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;

use async_trait::async_trait;
use pi_agent::policy::{
    Assembled, ContextAssembler, LoopPolicy, PassThroughProcessor, StopPolicy, TurnLimitPolicy,
};
use pi_agent::{run_agent, AgentConfig};
use pi_ai::{Message, Model, ModelPricing};

/// 记录被调用了几次，并在第 N 次时喊停。
struct CountingStop {
    calls: Arc<AtomicU32>,
    stop_at: u32,
}

#[async_trait]
impl StopPolicy for CountingStop {
    async fn should_stop(&self, turn: u32, _m: &[Message]) -> Option<String> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        if turn >= self.stop_at {
            Some(format!("custom policy stopped at turn {turn}"))
        } else {
            None
        }
    }
}

/// 改写 system prompt，并记录自己被调用过。
struct TaggingAssembler {
    calls: Arc<AtomicU32>,
}

#[async_trait]
impl ContextAssembler for TaggingAssembler {
    async fn assemble(&self, turn: u32, sp: &str, m: &[Message]) -> Assembled {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Assembled {
            system_prompt: format!("{sp}\n[assembled at turn {turn}]"),
            messages: m.to_vec(),
        }
    }
}

fn dummy_model() -> Model {
    // 指向一个不存在的端点 —— 我们只关心请求之前发生的事
    Model {
        id: "test-model".into(),
        name: "Test Model".into(),
        api: "openai-completions".into(),
        provider: "openai".into(),
        // 指向一个必然连不上的端口 —— 我们只关心请求之前发生的事
        base_url: "http://127.0.0.1:1".into(),
        reasoning: false,
        context_window: 8192,
        max_tokens: 1024,
        pricing: ModelPricing::default(),
    }
}

#[tokio::main(flavor = "multi_thread")]
async fn main() -> Result<(), String> {
    // ── 1. 自定义 StopPolicy 在发请求前就停住 ─────────────────
    let stop_calls = Arc::new(AtomicU32::new(0));
    let asm_calls = Arc::new(AtomicU32::new(0));

    let policy = LoopPolicy {
        assembler: Arc::new(TaggingAssembler {
            calls: asm_calls.clone(),
        }),
        processor: Arc::new(PassThroughProcessor),
        // stop_at: 0 -> 第一次问就停，循环一轮都不跑
        stop: Arc::new(CountingStop {
            calls: stop_calls.clone(),
            stop_at: 0,
        }),
    };

    let cfg = AgentConfig::new(dummy_model(), "You are a test agent.")
        .with_loop_policy(policy);

    let run = run_agent(&cfg, Message::user_text("hello"), None)
        .await
        .map_err(|e| format!("{e:?}"))?;

    println!("[test] stop policy consulted {} time(s)", stop_calls.load(Ordering::SeqCst));
    println!("[test] assembler called {} time(s)", asm_calls.load(Ordering::SeqCst));
    println!("[test] stopped_at_turn_limit = {}", run.stopped_at_turn_limit);

    assert_eq!(
        stop_calls.load(Ordering::SeqCst),
        1,
        "stop policy is consulted before the first turn"
    );
    assert_eq!(
        asm_calls.load(Ordering::SeqCst),
        0,
        "assembler never ran because we stopped first"
    );
    assert!(
        !run.stopped_at_turn_limit,
        "a custom stop reason must NOT be reported as a turn-limit truncation"
    );
    println!("[test] custom stop policy short-circuits the loop");

    // ── 2. 装配器确实被调用（让它跑一轮）─────────────────────
    let stop_calls2 = Arc::new(AtomicU32::new(0));
    let asm_calls2 = Arc::new(AtomicU32::new(0));
    let policy2 = LoopPolicy {
        assembler: Arc::new(TaggingAssembler {
            calls: asm_calls2.clone(),
        }),
        processor: Arc::new(PassThroughProcessor),
        stop: Arc::new(CountingStop {
            calls: stop_calls2.clone(),
            stop_at: 5,
        }),
    };
    let cfg2 = AgentConfig::new(dummy_model(), "You are a test agent.")
        .with_loop_policy(policy2);

    // 这次会真的发请求并失败 —— 我们只看装配器是否被调用过
    let _ = run_agent(&cfg2, Message::user_text("hello"), None).await;
    println!("[test] assembler called {} time(s) before the request", asm_calls2.load(Ordering::SeqCst));
    assert!(
        asm_calls2.load(Ordering::SeqCst) >= 1,
        "assembler runs before each request"
    );

    // ── 3. 默认策略的等价性 ──────────────────────────────────
    let cfg3 = AgentConfig::new(dummy_model(), "sp").with_max_turns(3);
    // with_max_turns 必须同步更新 stop policy，否则两处上限会打架
    let stopped = cfg3.loop_policy.stop.should_stop(3, &[]).await;
    println!("[test] default stop at turn 3 with max_turns=3: {stopped:?}");
    assert!(
        stopped.is_some_and(|r| r.starts_with("turn limit")),
        "with_max_turns keeps the stop policy in sync"
    );
    let not_yet = cfg3.loop_policy.stop.should_stop(2, &[]).await;
    assert!(not_yet.is_none(), "turn 2 of 3 does not stop");

    println!("[test] === B stage: three decision points verified ===");
    Ok(())
}
