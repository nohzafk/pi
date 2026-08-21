//! Cordis plugin governance for the pi agent harness.
//!
//! 让 pi 的工具集、权限、system prompt 由 Cordis 管理：插件可以在运行时
//! 装卸，卸载时它装的一切被干净撤销，越界的调用路由到待决队列由
//! orchestrator 决定。
//!
//! ── 阶段（见 docs/DECISIONS.md D6）────────────────────────────
//! A（当前）：Cordis 提供 AgentConfig 的内容 —— 工具、权限、prompt。
//!            控制循环仍是 pi 的 run_agent。
//! B（下一步）：循环本身拆成可替换的组件。
//!
//! ── 两个方向 ──────────────────────────────────────────────────
//!   plugin -> agent   bridge.rs：插件调用 pi 的工具
//!   agent  -> plugin  runtime.rs：agent 调用插件提供的工具

pub mod bridge;
pub mod config;
pub mod loop_policy;
pub mod runtime;

pub use config::{CordisPermission, GovernedConfig, PluginTool};
pub use loop_policy::plugin_loop_policy;
pub use runtime::CordisRuntime;

use std::sync::Arc;

/// 一步到位：启动 Cordis、装桥、应用一份插件配置。
///
/// `cordis_dir` 是 cordis-elle 仓库根；`plugin_config` 是一段 Elle 源码，
/// 求值后应当返回 entry 数组（也就是交给 `(L :apply)` 的东西）。
pub fn boot(
    cordis_dir: impl Into<String>,
    tools: Vec<Arc<dyn pi_agent::types::AgentTool>>,
    plugin_config: &str,
) -> Result<String, String> {
    let handle = tokio::runtime::Handle::try_current()
        .map_err(|e| format!("boot must run inside a tokio runtime: {e}"))?;
    bridge::install(handle, tools);
    CordisRuntime::start(cordis_dir.into())?;

    let src = format!(
        r#"(elle/epoch 12)
           (def cfg {plugin_config})
           (def r ((L :apply) cfg))
           (get r :ok)"#
    );
    CordisRuntime::eval(&src)
}

/// 读取当前由 Cordis 决定的 agent 配置。插件配置变化后重新调用。
///
/// `max_turns` 是停机的兜底 —— 插件不表态时仍受它约束，不能因为
/// 插件沉默就变成无限循环。
pub fn governed_config(
    system_prompt_fallback: &str,
    max_turns: u32,
) -> Result<GovernedConfig, String> {
    // 工具清单：插件在 :agent-tools 下登记的东西
    let listing = CordisRuntime::eval(
        r#"(elle/epoch 12)
           (let [reg (get (C :store) :plugin-tools)]
             (if reg (string/join (map (fn [t] (string (get t :name) "|" (get t :class) "|" (get t :handler) "|" (get t :description))) (get reg :value)) ";;") ""))"#,
    )?;

    let mut tools: Vec<Arc<dyn pi_agent::types::AgentTool>> = Vec::new();
    let cleaned = listing.trim().trim_matches('"');
    if !cleaned.is_empty() {
        for row in cleaned.split(";;") {
            let parts: Vec<&str> = row.split('|').collect();
            if parts.len() >= 4 {
                tools.push(Arc::new(PluginTool {
                    name: parts[0].to_string(),
                    class: parts[1].to_string(),
                    handler: parts[2].to_string(),
                    description: parts[3].to_string(),
                    parameters: serde_json::json!({
                        "type": "object",
                        "properties": {},
                    }),
                }));
            }
        }
    }

    // 被 harness 配置禁掉的能力类别
    let denied = CordisRuntime::eval(
        r#"(elle/epoch 12)
           (let [d (get (C :store) :denied-classes)]
             (if d (string/join (get d :value) ",") ""))"#,
    )?;
    let denied_classes: Vec<String> = denied
        .trim()
        .trim_matches('"')
        .split(',')
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
        .collect();

    // system prompt：插件可以贡献片段
    let prompt = CordisRuntime::eval(
        r#"(elle/epoch 12)
           (let [p (get (C :store) :system-prompt)]
             (if p (get p :value) ""))"#,
    )?;
    let prompt = prompt.trim().trim_matches('"').to_string();
    let system_prompt = if prompt.is_empty() {
        system_prompt_fallback.to_string()
    } else {
        format!("{system_prompt_fallback}\n\n{prompt}")
    };

    Ok(GovernedConfig {
        tools,
        permission: Arc::new(CordisPermission::new(denied_classes)),
        system_prompt,
        loop_policy: plugin_loop_policy(max_turns),
    })
}
