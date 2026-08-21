//! A 阶段端到端验证
//!
//! 验证三件事，任一不成立则 A 没做成：
//!   1. 插件能向 agent 注册工具（agent 的工具集由 Cordis 决定）
//!   2. agent 调用插件工具时，能派发回 Elle 并拿到结果
//!   3. 卸载插件后，它注册的工具从 agent 的工具集里消失
//!      —— 且不需要任何清理代码（inverse 自动做）

use std::sync::Arc;

use pi_agent::tools::default_tools;
use pi_agent::types::{AgentTool, PermissionDecision, PermissionPolicy};

#[tokio::main(flavor = "multi_thread")]
async fn main() -> Result<(), String> {
    let cordis_dir = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "/Users/randall/projects/cordis-elle".to_string());

    // ── 1. 启动 Cordis + 应用一份插件配置 ─────────────────────
    // 插件做两件事：注册一个工具给 agent，声明禁用 exec 能力。
    let plugin_config = r#"@[
      @{:id "toolkit" :url "toolkit.lisp"
        :factory (fn [config]
                   (fn [fib]
                     ## 提供一个 agent 可调用的工具
                     ((C :set) fib :greet-handler
                       (fn [id args] (string "hello from plugin, args=" args)))
                     ## 登记到工具清单
                     ((C :set) fib :plugin-tools
                       @[@{:name "greet" :class "read" :handler "greet-handler"
                           :description "Greet from the Cordis plugin"}])
                     ## 声明 harness 层面禁用的能力
                     ((C :set) fib :denied-classes @["exec"])
                     ## 贡献一段 system prompt
                     ((C :set) fib :system-prompt
                       "You have a greet tool provided by a Cordis plugin.")))
        :config @{}
        :provide @[:greet-handler :plugin-tools :denied-classes :system-prompt]}
    ]"#;

    println!("[test] booting cordis...");
    let ok = pi_cordis::boot(cordis_dir, default_tools(), plugin_config)?;
    println!("[test] apply ok: {ok}");
    if ok.trim() != "true" {
        return Err(format!("config was rejected: {ok}"));
    }

    // ── 2. 读取由 Cordis 决定的 agent 配置 ────────────────────
    let cfg = pi_cordis::governed_config("You are a coding agent.", 32)?;
    println!("[test] tools from cordis: {}", cfg.tools.len());
    for t in &cfg.tools {
        println!("[test]   - {} : {}", t.name(), t.description());
    }
    assert_eq!(cfg.tools.len(), 1, "plugin registered exactly one tool");
    assert_eq!(cfg.tools[0].name(), "greet");
    println!("[test] system prompt:\n{}", cfg.system_prompt);
    assert!(
        cfg.system_prompt.contains("greet tool"),
        "plugin contributed to the prompt"
    );

    // ── 3. agent 调用插件工具（agent -> plugin 方向）───────────
    println!("[test] agent invoking the plugin tool...");
    let result = cfg.tools[0]
        .execute("call-1", serde_json::json!({"who": "world"}))
        .await?;
    let text = result
        .content
        .iter()
        .filter_map(|c| match c {
            pi_ai::Content::Text { text } => Some(text.clone()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("");
    println!("[test] plugin returned: {text}");
    assert!(text.contains("hello from plugin"), "dispatch reached Elle");

    // ── 4. 权限：exec 类被 harness 配置禁掉 ───────────────────
    println!("[test] checking permission policy...");
    let decision = cfg.permission.check("bash", &serde_json::json!({})).await;
    match &decision {
        PermissionDecision::Deny { reason } => {
            println!("[test] bash denied: {reason}");
        }
        other => return Err(format!("expected bash to be denied, got {other:?}")),
    }
    let allowed = cfg.permission.check("read", &serde_json::json!({})).await;
    assert!(
        matches!(allowed, PermissionDecision::Allow),
        "read should be allowed"
    );
    println!("[test] read allowed");

    // ── 5. 卸载插件 -> 工具消失，无需清理代码 ─────────────────
    println!("[test] unloading the plugin...");
    let after = pi_cordis::CordisRuntime::eval(
        r#"(elle/epoch 12)
           ((L :apply) @[])
           (let [r (get (C :store) :plugin-tools)] (if r :still-there :gone))"#,
    )?;
    println!("[test] after unload: {after}");
    assert!(after.contains("gone"), "the inverse removed the registration");

    let cfg2 = pi_cordis::governed_config("You are a coding agent.", 32)?;
    println!("[test] tools after unload: {}", cfg2.tools.len());
    assert_eq!(cfg2.tools.len(), 0, "agent's toolset shrank");

    pi_cordis::CordisRuntime::shutdown();
    println!("[test] === A stage verified ===");
    Ok(())
}
