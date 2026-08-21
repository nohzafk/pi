//! 循环里可被替换的决策点。
//!
//! ── 为什么只有三个（见 cordis-elle/docs/DECISIONS.md D9）────────
//! 243 行的循环里真正"可以有别的做法"的只有三处，其余是不变的骨架
//! （收流、发事件、拼消息、记账）。把骨架也做成可替换的，等于让插件
//! 去重写 SSE 解析和消息拼装 —— 那不是治理，是把 bug 面放大。
//!
//! 三个点的共同性质：**输入输出都是纯数据**，所以能跨 Rust/Elle 边界
//! 传，也能被审计。流式接收持有 provider 的连接状态，跨不了边界。

use std::sync::Arc;

use async_trait::async_trait;
use pi_ai::{Content, Message};

/// 装配每一轮送给模型的上下文。
///
/// 默认实现原样传递。插件可以压缩历史、注入检索结果、改 prompt 结构。
#[async_trait]
pub trait ContextAssembler: Send + Sync {
    async fn assemble(&self, turn: u32, system_prompt: &str, messages: &[Message]) -> Assembled;
}

pub struct Assembled {
    pub system_prompt: String,
    pub messages: Vec<Message>,
}

/// 原样传递。
pub struct PassThroughAssembler;

#[async_trait]
impl ContextAssembler for PassThroughAssembler {
    async fn assemble(&self, _turn: u32, system_prompt: &str, messages: &[Message]) -> Assembled {
        Assembled {
            system_prompt: system_prompt.to_string(),
            messages: messages.to_vec(),
        }
    }
}

/// 加工工具执行的结果，然后再进消息历史。
///
/// 默认实现原样传递。插件可以截断大输出、脱敏、把失败转成给模型的提示。
#[async_trait]
pub trait ResultProcessor: Send + Sync {
    async fn process(
        &self,
        tool_name: &str,
        content: Vec<Content>,
        is_error: bool,
    ) -> (Vec<Content>, bool);
}

pub struct PassThroughProcessor;

#[async_trait]
impl ResultProcessor for PassThroughProcessor {
    async fn process(
        &self,
        _tool_name: &str,
        content: Vec<Content>,
        is_error: bool,
    ) -> (Vec<Content>, bool) {
        (content, is_error)
    }
}

/// 决定循环是否该停。
///
/// 默认实现只看轮次上限。插件可以按 token 预算、检测重复、外部中断来停。
#[async_trait]
pub trait StopPolicy: Send + Sync {
    /// 每轮开始前问一次。返回 Some(reason) 表示停。
    async fn should_stop(&self, turn: u32, messages: &[Message]) -> Option<String>;
}

pub struct TurnLimitPolicy {
    pub max_turns: u32,
}

#[async_trait]
impl StopPolicy for TurnLimitPolicy {
    async fn should_stop(&self, turn: u32, _messages: &[Message]) -> Option<String> {
        if turn >= self.max_turns {
            Some(format!("turn limit {} reached", self.max_turns))
        } else {
            None
        }
    }
}

/// 三个决策点打包。放进 AgentConfig。
#[derive(Clone)]
pub struct LoopPolicy {
    pub assembler: Arc<dyn ContextAssembler>,
    pub processor: Arc<dyn ResultProcessor>,
    pub stop: Arc<dyn StopPolicy>,
}

impl LoopPolicy {
    /// 默认策略 —— 与拆分之前的行为逐字节相同。
    pub fn default_with_turns(max_turns: u32) -> Self {
        Self {
            assembler: Arc::new(PassThroughAssembler),
            processor: Arc::new(PassThroughProcessor),
            stop: Arc::new(TurnLimitPolicy { max_turns }),
        }
    }
}
