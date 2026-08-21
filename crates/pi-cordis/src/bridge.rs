//! 工具桥：把 pi 的 tokio async 工具暴露给 Cordis 组件。
//!
//! ── D5：为什么分成多个 primitive ────────────────────────────────
//! Elle 的 signal 是**静态声明**的（编译期推断的一部分），而工具名是
//! 运行时参数。一个通用的 `pi/tool` 无法既表示 :io 又表示 :exec ——
//! 那样 `:deny |:exec|` 要么全拦要么全不拦，capability 失去分辨率。
//!
//! 所以按能力分成三个入口，各自声明真实 signal：
//!   pi/read-tool   Signal::io_yields_errors()   read/ls/grep/glob/...
//!   pi/write-tool  Signal::io_yields_errors()   write/edit
//!   pi/exec-tool   Signal::subprocess()          bash（含 SIG_EXEC）
//!
//! 这不是优化，是地基：第一版桥声明成 Signal::silent()，实测发现被
//! `:deny |:io|` 限制的插件照样能通过它调 bash —— 整套 capability
//! 变成装饰品。

use std::sync::{Arc, OnceLock};

use elle::primitives::ctx::NativeCtx;
use elle::primitives::def::{PrimitiveDef, RegionEffect};
use elle::signals::Signal;
use elle::value::fiber::SignalBits;
use elle::value::types::Arity;
use elle::Value;

use pi_agent::types::AgentTool;

/// 桥需要的宿主状态。用 OnceLock 而非参数传递，因为 Elle primitive 的
/// 签名是固定的 `fn(&mut NativeCtx, &[Value]) -> (SignalBits, Value)`，
/// 没有地方挂上下文。
pub static RT: OnceLock<tokio::runtime::Handle> = OnceLock::new();
pub static TOOLS: OnceLock<Vec<Arc<dyn AgentTool>>> = OnceLock::new();

/// 安装桥所需的宿主状态。必须在跑任何 Elle 代码之前调用。
pub fn install(handle: tokio::runtime::Handle, tools: Vec<Arc<dyn AgentTool>>) {
    let _ = RT.set(handle);
    let _ = TOOLS.set(tools);
}

/// Value 的 Display 给字符串加引号，剥掉。
fn unquote(s: &str) -> String {
    let t = s.trim();
    if t.len() >= 2 && t.starts_with('"') && t.ends_with('"') {
        t[1..t.len() - 1].replace("\\\"", "\"")
    } else {
        t.to_string()
    }
}

/// 工具的能力类别。依据是 pi 自己的 requires_permission() 加上
/// "是否 spawn 子进程"。
pub fn tool_class(name: &str) -> &'static str {
    match name {
        "bash" => "exec",
        "write" | "edit" => "write",
        _ => "read",
    }
}

fn call_tool(ctx: &mut NativeCtx<'_>, args: &[Value], allowed: &[&str]) -> (SignalBits, Value) {
    let name = unquote(&format!("{}", args[0]));
    let args_json = args
        .get(1)
        .map(|v| unquote(&format!("{}", v)))
        .unwrap_or_else(|| "{}".to_string());

    // 入口与工具类别必须匹配 —— 否则组件能用 pi/read-tool 调 bash
    // 绕过 :exec 边界。
    let class = tool_class(&name);
    if !allowed.contains(&class) {
        return (
            SignalBits::EMPTY,
            ctx.error(
                "wrong-tool-class",
                format!("tool '{name}' is class '{class}'; not callable through this entry"),
            ),
        );
    }

    let tools = match TOOLS.get() {
        Some(t) => t,
        None => return (SignalBits::EMPTY, ctx.error("not-installed", "tools")),
    };
    let tool = match tools.iter().find(|t| t.name() == name) {
        Some(t) => t.clone(),
        None => {
            return (
                SignalBits::EMPTY,
                ctx.error("no-such-tool", format!("unknown tool: {name}")),
            )
        }
    };

    let parsed: serde_json::Value =
        serde_json::from_str(&args_json).unwrap_or(serde_json::Value::Object(Default::default()));

    // ── 桥本体：同步 primitive 里把 tokio future 跑完 ─────────────
    // block_on 在 tokio worker 线程上会 panic，所以用 block_in_place
    // 把当前线程让出去。Elle 的 fiber 调度不受影响，因为整个 Elle
    // 运行时是在一个线程里跑的。
    let handle = match RT.get() {
        Some(h) => h.clone(),
        None => return (SignalBits::EMPTY, ctx.error("not-installed", "runtime")),
    };
    let result = tokio::task::block_in_place(|| {
        handle.block_on(async move { tool.execute("cordis-bridge", parsed).await })
    });

    match result {
        Ok(r) => {
            let text = r
                .content
                .iter()
                .filter_map(|c| match c {
                    pi_ai::Content::Text { text } => Some(text.clone()),
                    _ => None,
                })
                .collect::<Vec<_>>()
                .join("\n");
            (SignalBits::EMPTY, ctx.string(text))
        }
        Err(e) => (SignalBits::EMPTY, ctx.error("tool-failed", e)),
    }
}

fn read_tool(ctx: &mut NativeCtx<'_>, args: &[Value]) -> (SignalBits, Value) {
    call_tool(ctx, args, &["read"])
}
fn write_tool(ctx: &mut NativeCtx<'_>, args: &[Value]) -> (SignalBits, Value) {
    call_tool(ctx, args, &["read", "write"])
}
fn exec_tool(ctx: &mut NativeCtx<'_>, args: &[Value]) -> (SignalBits, Value) {
    call_tool(ctx, args, &["read", "write", "exec"])
}

fn tool_names(ctx: &mut NativeCtx<'_>, _args: &[Value]) -> (SignalBits, Value) {
    let tools = match TOOLS.get() {
        Some(t) => t,
        None => return (SignalBits::EMPTY, ctx.error("not-installed", "tools")),
    };
    let names: Vec<String> = tools
        .iter()
        .map(|t| format!("{}:{}", t.name(), tool_class(t.name())))
        .collect();
    (SignalBits::EMPTY, ctx.string(names.join(",")))
}

pub static PI_READ: PrimitiveDef = PrimitiveDef {
    name: "pi/read-tool",
    func: read_tool,
    signal: Signal::io_yields_errors(),
    arity: Arity::Exact(2),
    doc: "Call a read-only pi tool (read, ls, grep, glob, web_fetch, todo)",
    params: &["name", "args-json"],
    category: "pi",
    example: "(pi/read-tool \"ls\" \"{\\\"path\\\":\\\".\\\"}\")",
    effect: RegionEffect::Immediate,
    ..PrimitiveDef::DEFAULT
};

pub static PI_WRITE: PrimitiveDef = PrimitiveDef {
    name: "pi/write-tool",
    func: write_tool,
    signal: Signal::io_yields_errors(),
    arity: Arity::Exact(2),
    doc: "Call a mutating pi tool (write, edit), or any read tool",
    params: &["name", "args-json"],
    category: "pi",
    example: "(pi/write-tool \"write\" \"{...}\")",
    effect: RegionEffect::Immediate,
    ..PrimitiveDef::DEFAULT
};

pub static PI_EXEC: PrimitiveDef = PrimitiveDef {
    name: "pi/exec-tool",
    func: exec_tool,
    signal: Signal::subprocess(),
    arity: Arity::Exact(2),
    doc: "Call bash, or any lesser tool",
    params: &["name", "args-json"],
    category: "pi",
    example: "(pi/exec-tool \"bash\" \"{\\\"command\\\":\\\"ls\\\"}\")",
    effect: RegionEffect::Immediate,
    ..PrimitiveDef::DEFAULT
};

pub static PI_TOOLS: PrimitiveDef = PrimitiveDef {
    name: "pi/tools",
    func: tool_names,
    signal: Signal::silent(),
    arity: Arity::Exact(0),
    doc: "List available pi tools as name:class pairs",
    params: &[],
    category: "pi",
    example: "(pi/tools)",
    effect: RegionEffect::Immediate,
    ..PrimitiveDef::DEFAULT
};

pub static ALL: &[&PrimitiveDef] = &[&PI_READ, &PI_WRITE, &PI_EXEC, &PI_TOOLS];
