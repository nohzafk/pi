//! 把 Cordis 插件接到循环的三个决策点上（A+B 闭环）。
//!
//! B 阶段在 pi-agent 里定义了三个 trait，但那时没有实现者。这里让
//! **插件**成为实现者：策略写在 Elle 里，Rust 侧薄薄包一层。
//!
//! ── 为什么策略要能写在 Elle 里 ────────────────────────────────
//! 如果策略只能用 Rust 写，那它就不是插件 —— 换策略要重新编译，也就
//! 谈不上"运行时修改 harness"。写在 Elle 里才能装卸，而装卸走的是
//! Cordis 的 revertible effect，所以**卸载插件后自动恢复默认行为**。
//!
//! ── 契约 ──────────────────────────────────────────────────────
//! 插件通过 coeffect 登记三个可选的处理函数：
//!   :assemble-handler   (fn [turn system-prompt messages-json] -> prompt)
//!   :process-handler    (fn [tool-name content-json is-error] -> content)
//!   :stop-handler       (fn [turn message-count] -> nil | reason-string)
//! 没登记的那个就用默认（pass-through / 轮次上限）。

use std::sync::Arc;

use async_trait::async_trait;
use pi_agent::policy::{
    Assembled, ContextAssembler, LoopPolicy, PassThroughAssembler, PassThroughProcessor,
    ResultProcessor, StopPolicy, TurnLimitPolicy,
};
use pi_ai::{Content, Message};

use crate::runtime::CordisRuntime;

/// 把 Elle 侧的返回值转成 Rust 字符串。nil / 空串都当"没有意见"。
fn as_opt_string(raw: &str) -> Option<String> {
    let t = raw.trim().trim_matches('"').trim();
    if t.is_empty() || t == "nil" {
        None
    } else {
        Some(t.to_string())
    }
}

/// 把文本包成 Elle 字符串字面量。
fn lit(s: &str) -> String {
    format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""))
}

/// 装配器：让插件改写 system prompt。
///
/// 只传 prompt 和消息条数，不传完整消息体 —— 见下面的说明。
pub struct PluginAssembler;

#[async_trait]
impl ContextAssembler for PluginAssembler {
    async fn assemble(&self, turn: u32, system_prompt: &str, messages: &[Message]) -> Assembled {
        let src = format!(
            r#"(elle/epoch 12)
               (let [h (get (C :store) :assemble-handler)]
                 (if h ((get h :value) {turn} {sp} {n}) nil))"#,
            turn = turn,
            sp = lit(system_prompt),
            n = messages.len(),
        );
        let out = CordisRuntime::eval(&src).ok().and_then(|r| as_opt_string(&r));
        Assembled {
            // 插件没意见就原样传递
            system_prompt: out.unwrap_or_else(|| system_prompt.to_string()),
            // ── 有意的限制 ─────────────────────────────────────
            // 消息体不交给插件改写。跨边界传完整对话既贵又危险：
            // 插件返回一个畸形的消息列表，错误会出现在 provider 的
            // 请求里，而那时已经离出错点很远了。改 prompt 够用，
            // 改历史需要更强的契约（结构化而非字符串）。
            messages: messages.to_vec(),
        }
    }
}

/// 结果加工：让插件改写工具输出。
pub struct PluginProcessor;

#[async_trait]
impl ResultProcessor for PluginProcessor {
    async fn process(
        &self,
        tool_name: &str,
        content: Vec<Content>,
        is_error: bool,
    ) -> (Vec<Content>, bool) {
        let text: String = content
            .iter()
            .filter_map(|c| match c {
                Content::Text { text } => Some(text.clone()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("\n");

        let src = format!(
            r#"(elle/epoch 12)
               (let [h (get (C :store) :process-handler)]
                 (if h ((get h :value) {name} {body} {err}) nil))"#,
            name = lit(tool_name),
            body = lit(&text),
            err = if is_error { "true" } else { "false" },
        );

        match CordisRuntime::eval(&src).ok().and_then(|r| as_opt_string(&r)) {
            Some(replaced) => (vec![Content::text(replaced)], is_error),
            None => (content, is_error),
        }
    }
}

/// 停机：让插件决定何时停。
///
/// 插件没意见时**回落到轮次上限** —— 不能因为插件沉默就变成无限循环。
pub struct PluginStop {
    pub fallback: TurnLimitPolicy,
}

#[async_trait]
impl StopPolicy for PluginStop {
    async fn should_stop(&self, turn: u32, messages: &[Message]) -> Option<String> {
        let src = format!(
            r#"(elle/epoch 12)
               (let [h (get (C :store) :stop-handler)]
                 (if h ((get h :value) {turn} {n}) nil))"#,
            turn = turn,
            n = messages.len(),
        );
        if let Some(reason) = CordisRuntime::eval(&src).ok().and_then(|r| as_opt_string(&r)) {
            return Some(reason);
        }
        // 插件不表态 -> 仍然受轮次上限约束
        self.fallback.should_stop(turn, messages).await
    }
}

/// 读取当前插件配置决定的循环策略。
///
/// 每个点独立判断有没有插件登记：没登记就用默认实现，避免为一个
/// 空 handler 付一次跨边界调用的代价。
pub fn plugin_loop_policy(max_turns: u32) -> LoopPolicy {
    let has = |key: &str| -> bool {
        let src = format!(
            r#"(elle/epoch 12)
               (if (get (C :store) :{key}) :yes :no)"#
        );
        CordisRuntime::eval(&src)
            .map(|r| r.contains("yes"))
            .unwrap_or(false)
    };

    LoopPolicy {
        assembler: if has("assemble-handler") {
            Arc::new(PluginAssembler)
        } else {
            Arc::new(PassThroughAssembler)
        },
        processor: if has("process-handler") {
            Arc::new(PluginProcessor)
        } else {
            Arc::new(PassThroughProcessor)
        },
        stop: if has("stop-handler") {
            Arc::new(PluginStop {
                fallback: TurnLimitPolicy { max_turns },
            })
        } else {
            Arc::new(TurnLimitPolicy { max_turns })
        },
    }
}
