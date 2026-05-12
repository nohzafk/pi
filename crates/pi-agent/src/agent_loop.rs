//! Agent loop — Rust port of `packages/agent/src/agent-loop.ts`.
//!
//! The flow is the same as the TS version:
//!   1. Append user prompt to context.
//!   2. Call `pi_ai::stream_simple` to get the assistant message.
//!   3. Execute any tool calls in the assistant message sequentially.
//!   4. Append tool results to context and continue until `stop_reason ≠ ToolUse`
//!      or `max_turns` is reached.

use std::collections::HashMap;
use std::sync::Arc;

use futures::StreamExt;
use pi_ai::{
    stream_simple, AssistantMessageEvent, Content, Context, Message, StopReason,
    ToolResultMessage,
};
use serde_json::Value;
use tokio::sync::mpsc;

use crate::types::{AgentConfig, AgentEvent, AgentTool, AgentToolResult};

pub struct AgentRun {
    pub messages: Vec<Message>,
    pub stopped_at_turn_limit: bool,
}

pub async fn run_agent(
    config: &AgentConfig,
    initial_prompt: Message,
    events: Option<mpsc::UnboundedSender<AgentEvent>>,
) -> Result<AgentRun, String> {
    let mut messages: Vec<Message> = Vec::new();
    messages.push(initial_prompt.clone());
    emit(&events, AgentEvent::UserMessage { message: initial_prompt });
    emit(&events, AgentEvent::AgentStart);

    let tool_index: HashMap<String, Arc<dyn AgentTool>> = config
        .tools
        .iter()
        .map(|t| (t.name().to_string(), t.clone()))
        .collect();
    let tool_defs: Vec<pi_ai::Tool> = config
        .tools
        .iter()
        .map(|t| crate::types::tool_def(t.as_ref()))
        .collect();

    let mut turn: u32 = 0;
    let mut stopped_at_turn_limit = false;

    while turn < config.max_turns {
        turn += 1;
        emit(&events, AgentEvent::TurnStart);

        let ctx = Context {
            system_prompt: Some(config.system_prompt.clone()),
            messages: messages.clone(),
            tools: tool_defs.clone(),
        };

        let mut options = config.stream_options.clone();
        if options.reasoning.is_none() && config.thinking_level != pi_ai::ThinkingLevel::Off {
            options.reasoning = Some(config.thinking_level);
        }

        let mut stream = stream_simple(&config.model, &ctx, &options)
            .await
            .map_err(|e| format!("provider error: {e}"))?;

        let mut final_message: Option<pi_ai::AssistantMessage> = None;
        let mut stop = StopReason::Stop;

        while let Some(ev) = stream.next().await {
            match ev {
                Ok(AssistantMessageEvent::Done { reason, message }) => {
                    stop = reason;
                    final_message = Some(message);
                    break;
                }
                Ok(AssistantMessageEvent::Error { reason, error }) => {
                    stop = reason;
                    final_message = Some(error);
                    break;
                }
                Ok(_) => {} // partial deltas are ignored in this minimal port
                Err(e) => return Err(format!("stream error: {e}")),
            }
        }

        let Some(msg) = final_message else {
            return Err("provider stream produced no terminal event".into());
        };

        let assistant_message = Message::Assistant(msg.clone());
        messages.push(assistant_message.clone());
        emit(
            &events,
            AgentEvent::AssistantMessage {
                message: assistant_message,
            },
        );

        // Collect tool calls
        let tool_calls: Vec<(String, String, Value)> = msg
            .content
            .iter()
            .filter_map(|c| match c {
                Content::ToolCall { id, name, arguments } => {
                    Some((id.clone(), name.clone(), arguments.clone()))
                }
                _ => None,
            })
            .collect();

        if tool_calls.is_empty() || stop != StopReason::ToolUse {
            emit(&events, AgentEvent::TurnEnd);
            break;
        }

        let mut any_terminate = !tool_calls.is_empty();
        for (id, name, args) in tool_calls {
            emit(
                &events,
                AgentEvent::ToolExecutionStart {
                    tool_call_id: id.clone(),
                    tool_name: name.clone(),
                    args: args.clone(),
                },
            );
            let (content, is_error, terminate) = match tool_index.get(&name) {
                Some(tool) => match tool.execute(&id, args).await {
                    Ok(AgentToolResult {
                        content,
                        details: _,
                        terminate,
                    }) => (content, false, terminate),
                    Err(e) => (vec![Content::text(format!("tool error: {e}"))], true, false),
                },
                None => (
                    vec![Content::text(format!("unknown tool: {name}"))],
                    true,
                    false,
                ),
            };
            if !terminate {
                any_terminate = false;
            }
            emit(
                &events,
                AgentEvent::ToolExecutionEnd {
                    tool_call_id: id.clone(),
                    tool_name: name.clone(),
                    is_error,
                    content: content.clone(),
                },
            );
            let tr = ToolResultMessage {
                tool_call_id: id,
                tool_name: name,
                content,
                is_error,
                timestamp: pi_ai::now_ms(),
            };
            messages.push(Message::ToolResult(tr));
        }
        emit(&events, AgentEvent::TurnEnd);
        if any_terminate {
            break;
        }
    }

    if turn >= config.max_turns {
        stopped_at_turn_limit = true;
    }

    emit(
        &events,
        AgentEvent::AgentEnd {
            messages: messages.clone(),
        },
    );
    Ok(AgentRun {
        messages,
        stopped_at_turn_limit,
    })
}

fn emit(sink: &Option<mpsc::UnboundedSender<AgentEvent>>, ev: AgentEvent) {
    if let Some(s) = sink {
        let _ = s.send(ev);
    }
}
