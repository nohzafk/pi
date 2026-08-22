use std::sync::Arc;

use async_trait::async_trait;
use pi_ai::{Content, Message, Model, StreamOptions, ThinkingLevel, Tool};
use serde_json::Value;

/// A live result returned from a tool execution.
#[derive(Debug, Clone, Default)]
pub struct AgentToolResult {
    pub content: Vec<Content>,
    pub details: Value,
    pub terminate: bool,
}

impl AgentToolResult {
    pub fn text(s: impl Into<String>) -> Self {
        Self {
            content: vec![Content::text(s)],
            details: Value::Null,
            terminate: false,
        }
    }
}

/// Permission outcome for a tool call. Returned by a [`PermissionPolicy`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PermissionDecision {
    Allow,
    /// Allow this call and remember the choice for the rest of the run.
    AllowSession,
    /// Deny this call; emit an error tool result with `reason`.
    Deny {
        reason: String,
    },
}

/// User-supplied permission policy. Implementations may prompt interactively or
/// consult a static allow-list.
#[async_trait]
pub trait PermissionPolicy: Send + Sync {
    async fn check(&self, tool_name: &str, args: &Value) -> PermissionDecision;
}

/// Always-allow policy — useful for tests and non-interactive runs.
pub struct AllowAllPolicy;

#[async_trait]
impl PermissionPolicy for AllowAllPolicy {
    async fn check(&self, _tool_name: &str, _args: &Value) -> PermissionDecision {
        PermissionDecision::Allow
    }
}

/// Tool execution trait — analog of `AgentTool.execute` in TS.
#[async_trait]
pub trait AgentTool: Send + Sync {
    fn name(&self) -> &str;
    fn label(&self) -> &str {
        self.name()
    }
    fn description(&self) -> &str;
    fn parameters(&self) -> Value;
    /// Whether the tool requires user permission by default. Read-only tools
    /// (`read`, `ls`, `grep`, `glob`) return `false`; mutating or side-effecting
    /// tools (`bash`, `write`, `edit`) return `true`.
    fn requires_permission(&self) -> bool {
        false
    }
    async fn execute(&self, tool_call_id: &str, args: Value) -> Result<AgentToolResult, String>;
}

/// The parameter every tool carries for the UI, injected by [`tool_def`].
///
/// The model states its intent in the same call that does the work. A
/// summariser downstream cannot recover that: it sees `sed -n '60,130p' x.rs`
/// and can say "read part of a file", but not *why* those lines. Intent only
/// exists in the context of the model that made the call.
pub const TITLE_PARAM: &str = "title";

const TITLE_DESCRIPTION: &str = "A short phrase, 5 to 10 words, that says what \
this call is for. It is shown to the user in place of the raw arguments while \
the tool runs. Write it in the language the user writes in. Describe the intent, \
not the syntax: \"check why the build fails\", not \"run cargo build\".";

/// Add the `title` parameter to a tool's own schema.
///
/// Injected here rather than in each tool so every tool gets it, including
/// ones added later, and so no tool has to know the UI exists.
fn with_title_param(mut params: Value) -> Value {
    let Some(obj) = params.as_object_mut() else {
        return params;
    };
    let props = obj
        .entry("properties")
        .or_insert_with(|| Value::Object(Default::default()));
    if let Some(props) = props.as_object_mut() {
        // A tool that already defines `title` keeps its own meaning.
        if !props.contains_key(TITLE_PARAM) {
            props.insert(
                TITLE_PARAM.to_string(),
                serde_json::json!({
                    "type": "string",
                    "description": TITLE_DESCRIPTION,
                }),
            );
        }
    }
    // Required, not optional. An optional field gets dropped when the model is
    // in a hurry, and then the UI has a blank line to render.
    match obj.get_mut("required") {
        Some(Value::Array(req)) => {
            if !req.iter().any(|v| v.as_str() == Some(TITLE_PARAM)) {
                req.push(Value::String(TITLE_PARAM.to_string()));
            }
        }
        _ => {
            obj.insert(
                "required".to_string(),
                Value::Array(vec![Value::String(TITLE_PARAM.to_string())]),
            );
        }
    }
    params
}

/// Take `title` out of the arguments before the tool runs.
///
/// Tools never see it: it is for the UI, and a tool that validates its input
/// strictly would reject an argument it does not declare.
pub fn split_title(args: &mut Value) -> Option<String> {
    let obj = args.as_object_mut()?;
    match obj.remove(TITLE_PARAM) {
        Some(Value::String(s)) if !s.trim().is_empty() => Some(s),
        _ => None,
    }
}

pub fn tool_def(t: &dyn AgentTool) -> Tool {
    Tool {
        name: t.name().to_string(),
        description: t.description().to_string(),
        parameters: with_title_param(t.parameters()),
    }
}

/// Agent configuration controlling the loop.
#[derive(Clone)]
pub struct AgentConfig {
    pub model: Model,
    pub thinking_level: ThinkingLevel,
    pub stream_options: StreamOptions,
    pub max_turns: u32,
    pub tools: Vec<Arc<dyn AgentTool>>,
    pub system_prompt: String,
    pub permission: Arc<dyn PermissionPolicy>,
    /// 循环里三个可替换的决策点。默认与拆分前行为等价。
    pub loop_policy: crate::policy::LoopPolicy,
}

impl AgentConfig {
    pub fn new(model: Model, system_prompt: impl Into<String>) -> Self {
        Self {
            model,
            thinking_level: ThinkingLevel::Off,
            stream_options: StreamOptions::default(),
            max_turns: 32,
            tools: Vec::new(),
            system_prompt: system_prompt.into(),
            permission: Arc::new(AllowAllPolicy),
            loop_policy: crate::policy::LoopPolicy::default_with_turns(32),
        }
    }

    pub fn with_tools(mut self, tools: Vec<Arc<dyn AgentTool>>) -> Self {
        self.tools = tools;
        self
    }

    /// 设一个轮数上限。**默认没有上限** —— 默认策略是连续失败计数
    /// （见 policy::ConsecutiveFailures），因为上游 pi 和 codex 都不数轮，
    /// 而数轮会在长任务做到一半时把它截断。
    ///
    /// 调这个方法才装上 TurnLimitPolicy，也就是明确要一个硬顶。
    pub fn with_max_turns(mut self, n: u32) -> Self {
        self.max_turns = n;
        self.loop_policy.stop = Arc::new(crate::policy::TurnLimitPolicy { max_turns: n });
        self
    }

    pub fn with_loop_policy(mut self, p: crate::policy::LoopPolicy) -> Self {
        self.loop_policy = p;
        self
    }

    pub fn with_permission(mut self, p: Arc<dyn PermissionPolicy>) -> Self {
        self.permission = p;
        self
    }

    pub fn with_thinking(mut self, level: ThinkingLevel) -> Self {
        self.thinking_level = level;
        self
    }

    pub fn with_stream_options(mut self, o: StreamOptions) -> Self {
        self.stream_options = o;
        self
    }
}

/// Events emitted by the agent loop, mirroring `AgentEvent` in TS.
#[derive(Debug, Clone)]
pub enum AgentEvent {
    AgentStart,
    AgentEnd {
        messages: Vec<Message>,
    },
    TurnStart,
    TurnEnd,
    AssistantMessage {
        message: Message,
    },
    UserMessage {
        message: Message,
    },
    /// Streaming text chunk while the assistant types.
    TextDelta {
        delta: String,
    },
    /// Streaming thinking chunk.
    ThinkingDelta {
        delta: String,
    },
    ToolExecutionStart {
        tool_call_id: String,
        tool_name: String,
        /// Arguments as the tool receives them: `title` is already removed.
        args: Value,
        /// What the model said this call is for. `None` if it omitted the
        /// field despite the schema requiring it.
        title: Option<String>,
    },
    ToolExecutionEnd {
        tool_call_id: String,
        tool_name: String,
        is_error: bool,
        content: Vec<Content>,
        /// Tool-specific metadata. `bash` reports truncation here so a
        /// renderer can size its preview without re-parsing the text.
        details: Value,
    },
    /// Permission denied for a tool call (the loop appended an error tool result).
    PermissionDenied {
        tool_name: String,
        reason: String,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Dummy {
        params: Value,
    }

    #[async_trait]
    impl AgentTool for Dummy {
        fn name(&self) -> &str {
            "dummy"
        }
        fn description(&self) -> &str {
            "test tool"
        }
        fn parameters(&self) -> Value {
            self.params.clone()
        }
        async fn execute(&self, _id: &str, _args: Value) -> Result<AgentToolResult, String> {
            Ok(AgentToolResult::text("ok"))
        }
    }

    fn def_of(params: Value) -> Tool {
        tool_def(&Dummy { params })
    }

    #[test]
    fn title_is_injected_and_required() {
        let d = def_of(serde_json::json!({
            "type": "object",
            "properties": {"path": {"type": "string"}},
            "required": ["path"],
        }));
        let props = &d.parameters["properties"];
        assert_eq!(props[TITLE_PARAM]["type"], "string");
        // The tool's own parameters survive.
        assert_eq!(props["path"]["type"], "string");

        let req = d.parameters["required"].as_array().unwrap();
        assert!(req.iter().any(|v| v == "path"));
        assert!(req.iter().any(|v| v == TITLE_PARAM), "title must be required");
    }

    #[test]
    fn title_is_added_when_there_is_no_required_list() {
        let d = def_of(serde_json::json!({
            "type": "object",
            "properties": {},
        }));
        assert_eq!(d.parameters["required"], serde_json::json!([TITLE_PARAM]));
    }

    #[test]
    fn a_tool_keeps_its_own_title_parameter() {
        let d = def_of(serde_json::json!({
            "type": "object",
            "properties": {"title": {"type": "integer"}},
        }));
        // Not overwritten: the tool meant something else by that name.
        assert_eq!(d.parameters["properties"]["title"]["type"], "integer");
    }

    #[test]
    fn split_title_takes_the_field_out() {
        let mut args = serde_json::json!({"command": "ls", "title": "list the files"});
        assert_eq!(split_title(&mut args).as_deref(), Some("list the files"));
        // The tool must not see it.
        assert_eq!(args, serde_json::json!({"command": "ls"}));
    }

    #[test]
    fn split_title_ignores_junk() {
        // Missing, blank, and wrong-typed all mean "no title" rather than an
        // error: a bad title must never fail the tool call.
        let mut a = serde_json::json!({"command": "ls"});
        assert!(split_title(&mut a).is_none());

        let mut b = serde_json::json!({"title": "   "});
        assert!(split_title(&mut b).is_none());

        let mut c = serde_json::json!({"title": 42});
        assert!(split_title(&mut c).is_none());
        assert!(c.get("title").is_none(), "junk is still removed");
    }
}
