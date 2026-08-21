//! Cordis 运行时：持有嵌入的 Elle 实例，把治理决定交给 Rust 侧。
//!
//! ── 线程模型（重要）────────────────────────────────────────────
//! Elle 的 `Runtime` 不是 Send/Sync —— VM、符号表、堆都是单线程的。
//! 所以整个 Cordis 运行时被关在一个专用线程里，Rust 侧通过 channel
//! 与它通信。这不是权宜之计：Elle 的 per-fiber arena 内存模型本来就
//! 假定单线程所有权。
//!
//! ── 两个方向都要打通 ──────────────────────────────────────────
//!   plugin -> agent   bridge.rs 的 pi/*-tool primitive
//!   agent  -> plugin  本文件的 dispatch_tool
//! 只有前者的话插件能用工具但不能被 agent 调用，那不叫插件系统。

use std::sync::mpsc;
use std::sync::OnceLock;
use std::thread;

use elle::runtime::Runtime;
use elle::{compile_file, Value};
use serde_json::Value as Json;

use pi_agent::types::AgentToolResult;

/// 送进 Elle 线程的请求
enum Req {
    /// 跑一段 Elle 源码，返回它的 Display 形式
    Eval { src: String, reply: mpsc::Sender<Result<String, String>> },
    /// agent 调一个插件提供的工具
    DispatchTool {
        handler: String,
        id: String,
        args: Json,
        reply: mpsc::Sender<Result<String, String>>,
    },
    Shutdown,
}

static TX: OnceLock<mpsc::Sender<Req>> = OnceLock::new();

pub struct CordisRuntime;

impl CordisRuntime {
    /// 启动 Elle 线程并加载 Cordis。`cordis_dir` 是 cordis-elle 仓库根 ——
    /// Elle 的 include 相对 cwd 解析，所以线程启动后立刻 chdir 过去。
    pub fn start(cordis_dir: String) -> Result<(), String> {
        let (tx, rx) = mpsc::channel::<Req>();
        let (ready_tx, ready_rx) = mpsc::channel::<Result<(), String>>();

        thread::Builder::new()
            .name("cordis".into())
            .spawn(move || {
                // include 相对 cwd —— 见 D3 里记的那个缺陷
                if let Err(e) = std::env::set_current_dir(&cordis_dir) {
                    let _ = ready_tx.send(Err(format!("chdir {cordis_dir}: {e}")));
                    return;
                }

                let mut rt = Runtime::new();

                // 注册工具桥
                for def in crate::bridge::ALL {
                    let sym = rt.symbols().intern(def.name);
                    let native = Value::native_fn(def);
                    let (cctx, heap) = rt.compile_and_heap();
                    cctx.register_repl_binding(heap, sym, native, def.signal, Some(def.arity));
                }

                // 加载 loader，并把 L / C 注册成 **REPL 绑定**。
                //
                // 为什么不能只靠 (def L ...)：每次 eval 是一个独立的编译
                // 单元（compile_file 把源码包成一个 synthetic letrec），
                // 顶层 def 出来的绑定不跨调用可见。实测报
                //   undefined variable: C
                // REPL 语义走的是 CompileCtx 的 meta 表，要用
                // register_repl_binding 显式注册。
                let boot = r#"(elle/epoch 12)
                    (import-file "src/loader.lisp")"#;
                let loader_val = {
                    let (vm, symbols, cctx) = rt.parts();
                    match compile_file(boot, symbols, cctx, "<boot>") {
                        Ok(c) => match vm.execute_scheduled(&c.bytecode, symbols, cctx) {
                            Ok(v) => v,
                            Err(e) => {
                                let _ = ready_tx.send(Err(format!("boot exec: {e:?}")));
                                return;
                            }
                        },
                        Err(e) => {
                            let _ = ready_tx.send(Err(format!("boot compile: {e:?}")));
                            return;
                        }
                    }
                };

                // L = loader 的导出 struct
                let l_sym = rt.symbols().intern("L");
                {
                    let (cctx, heap) = rt.compile_and_heap();
                    cctx.register_repl_binding(
                        heap,
                        l_sym,
                        loader_val,
                        elle::signals::Signal::unknown(),
                        None,
                    );
                }

                // C = (get L :cordis)
                let cordis_val = {
                    match run_value(&mut rt, r#"(elle/epoch 12)
                        (get L :cordis)"#, "<boot-c>") {
                        Ok(v) => v,
                        Err(e) => {
                            let _ = ready_tx.send(Err(format!("boot C: {e}")));
                            return;
                        }
                    }
                };
                let c_sym = rt.symbols().intern("C");
                {
                    let (cctx, heap) = rt.compile_and_heap();
                    cctx.register_repl_binding(
                        heap,
                        c_sym,
                        cordis_val,
                        elle::signals::Signal::unknown(),
                        None,
                    );
                }

                let _ = ready_tx.send(Ok(()));

                // 服务循环
                while let Ok(req) = rx.recv() {
                    match req {
                        Req::Shutdown => break,
                        Req::Eval { src, reply } => {
                            let out = run_source(&mut rt, &src, "<eval>");
                            let _ = reply.send(out);
                        }
                        Req::DispatchTool { handler, id, args, reply } => {
                            // 把 agent 的调用转成一次 Elle 求值。参数走 JSON
                            // 字符串 —— 避免在两套值表示之间做深转换。
                            let src = format!(
                                r#"(elle/epoch 12)
                                   (def h (get (get (C :store) :{handler}) :value))
                                   (h "{id}" {args_json})"#,
                                handler = handler,
                                id = id,
                                args_json = elle_string_literal(&args.to_string()),
                            );
                            let out = run_source(&mut rt, &src, "<dispatch>");
                            let _ = reply.send(out);
                        }
                    }
                }
            })
            .map_err(|e| format!("spawn cordis thread: {e}"))?;

        ready_rx
            .recv()
            .map_err(|e| format!("cordis thread died: {e}"))??;
        TX.set(tx).map_err(|_| "already started".to_string())?;
        Ok(())
    }

    /// 跑一段 Elle 源码。给 Rust 侧驱动治理用。
    pub fn eval(src: &str) -> Result<String, String> {
        let tx = TX.get().ok_or("cordis not started")?;
        let (reply, rx) = mpsc::channel();
        tx.send(Req::Eval { src: src.to_string(), reply })
            .map_err(|e| format!("send: {e}"))?;
        rx.recv().map_err(|e| format!("recv: {e}"))?
    }

    /// agent -> plugin 方向的派发。由 PluginTool::execute 调用。
    pub fn dispatch_tool(handler: &str, id: &str, args: Json) -> Result<AgentToolResult, String> {
        let tx = TX.get().ok_or("cordis not started")?;
        let (reply, rx) = mpsc::channel();
        tx.send(Req::DispatchTool {
            handler: handler.to_string(),
            id: id.to_string(),
            args,
            reply,
        })
        .map_err(|e| format!("send: {e}"))?;
        let text = rx.recv().map_err(|e| format!("recv: {e}"))??;
        Ok(AgentToolResult::text(text))
    }

    pub fn shutdown() {
        if let Some(tx) = TX.get() {
            let _ = tx.send(Req::Shutdown);
        }
    }
}

fn run_source(rt: &mut Runtime, src: &str, name: &str) -> Result<String, String> {
    run_value(rt, src, name).map(|v| format!("{v}"))
}

fn run_value(rt: &mut Runtime, src: &str, name: &str) -> Result<Value, String> {
    let (vm, symbols, cctx) = rt.parts();
    let compiled =
        compile_file(src, symbols, cctx, name).map_err(|e| format!("compile: {e:?}"))?;
    vm.execute_scheduled(&compiled.bytecode, symbols, cctx)
        .map_err(|e| format!("exec: {e:?}"))
}

/// 把一段文本包成 Elle 字符串字面量。
fn elle_string_literal(s: &str) -> String {
    let escaped = s.replace('\\', "\\\\").replace('"', "\\\"");
    format!("\"{escaped}\"")
}
