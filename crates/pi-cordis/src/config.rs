//! Cordis 治理的 pi agent 配置。
//!
//! ── D6：A 阶段的范围 ────────────────────────────────────────────
//! Cordis 提供 `AgentConfig` 的三样东西：
//!   tools        哪些工具可用（插件装卸即改变工具集）
//!   permission   工具调用是否放行（接到 Cordis 的 capability 上）
//!   system_prompt 上下文装配的一部分
//! 控制循环本身还是 pi 的 `run_agent`，不可替换 —— 那是 B 阶段。

use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use pi_agent::types::{AgentTool, AgentToolResult, PermissionDecision, PermissionPolicy};
use serde_json::Value as Json;

use crate::runtime::CordisRuntime;

/// Cordis 侧登记的一个工具。插件通过 coeffect 注册它，
/// 卸载时 inverse 自动摘除 —— 无需清理约定。
#[derive(Clone)]
pub struct PluginTool {
    pub name: String,
    pub description: String,
    pub parameters: Json,
    pub class: String,
    /// 插件里那个 Elle 函数的名字。调用时由 Cordis 运行时派发。
    pub handler: String,
}

#[async_trait]
impl AgentTool for PluginTool {
    fn name(&self) -> &str {
        &self.name
    }
    fn description(&self) -> &str {
        &self.description
    }
    fn parameters(&self) -> Json {
        self.parameters.clone()
    }
    fn requires_permission(&self) -> bool {
        // 与桥的分级一致：写和执行要许可，读不要
        self.class != "read"
    }
    async fn execute(&self, id: &str, args: Json) -> Result<AgentToolResult, String> {
        // 派发回 Elle 侧的插件处理函数。
        // 注意：这里是 agent -> plugin 方向，与 bridge.rs 的
        // plugin -> agent 方向相反，两者都要有才算打通。
        CordisRuntime::dispatch_tool(&self.handler, id, args)
    }
}

/// 把工具调用交给 Cordis 判断。
///
/// 这是 A 阶段最有价值的一件事：越界不再是简单 deny，而是**路由到
/// Cordis 的待决队列**，orchestrator 可以 grant / provide / abandon。
/// 论文的 L-Raise 没有这个状态。
pub struct CordisPermission {
    /// 已被 orchestrator 放行的工具（会话级）
    granted: Arc<Mutex<Vec<String>>>,
    /// 配置里声明为禁止的能力类别
    denied_classes: Arc<Mutex<Vec<String>>>,
}

impl CordisPermission {
    pub fn new(denied_classes: Vec<String>) -> Self {
        Self {
            granted: Arc::new(Mutex::new(Vec::new())),
            denied_classes: Arc::new(Mutex::new(denied_classes)),
        }
    }

    pub fn grant(&self, tool: &str) {
        if let Ok(mut g) = self.granted.lock() {
            g.push(tool.to_string());
        }
    }
}

#[async_trait]
impl PermissionPolicy for CordisPermission {
    async fn check(&self, tool_name: &str, _args: &Json) -> PermissionDecision {
        if let Ok(g) = self.granted.lock() {
            if g.iter().any(|t| t == tool_name) {
                return PermissionDecision::Allow;
            }
        }
        let class = crate::bridge::tool_class(tool_name);
        if let Ok(d) = self.denied_classes.lock() {
            if d.iter().any(|c| c == class) {
                return PermissionDecision::Deny {
                    reason: format!(
                        "capability '{class}' is withheld by the harness configuration"
                    ),
                };
            }
        }
        PermissionDecision::Allow
    }
}

/// Cordis 决定的 agent 配置。由 `CordisRuntime::agent_config()` 产出，
/// 每次插件配置变化后重新读取。
pub struct GovernedConfig {
    pub tools: Vec<Arc<dyn AgentTool>>,
    pub permission: Arc<dyn PermissionPolicy>,
    pub system_prompt: String,
}
