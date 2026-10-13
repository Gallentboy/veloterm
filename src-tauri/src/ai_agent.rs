//! AI 智能体工具循环：AI 自主调用终端工具（读屏幕/执行命令/读输出）。
//!
//! 架构：rig 在 Rust 侧发起带 ToolDefinition 的补全请求，模型返回 ToolCall 时
//! 经 Channel 推给前端执行（终端在 webview 里），前端回传 `ai_tool_result`
//! 唤醒挂起的 oneshot，Rust 组装 ToolResult 消息继续循环，直到模型给出纯文本。

use std::collections::HashMap;
use std::sync::Mutex as StdMutex;

use serde::{Deserialize, Serialize};
use serde_json::json;
use tauri::ipc::Channel;
use tauri::State;
use tokio::sync::oneshot;

use crate::ai::{default_provider, get_provider, pick_key, AiChatMessage, AiProviderRecord};
use crate::db::DbPool;

/// 前端执行完工具后回传结果：唤醒对应挂起的 oneshot
static PENDING_TOOL_CALLS: StdMutex<Option<HashMap<String, oneshot::Sender<String>>>> =
  StdMutex::new(None);

fn register_pending(id: String) -> oneshot::Receiver<String> {
  let (tx, rx) = oneshot::channel();
  let mut guard = PENDING_TOOL_CALLS.lock().unwrap();
  guard
    .get_or_insert_with(HashMap::new)
    .insert(id, tx);
  rx
}

fn resolve_pending(id: &str, result: String) -> bool {
  let mut guard = PENDING_TOOL_CALLS.lock().unwrap();
  if let Some(map) = guard.as_mut() {
    if let Some(tx) = map.remove(id) {
      return tx.send(result).is_ok();
    }
  }
  false
}

#[tauri::command]
pub fn ai_tool_result(call_id: String, result: String) -> Result<bool, String> {
  Ok(resolve_pending(&call_id, result))
}

/// 推送给前端的工具调用事件
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AiToolCallEvent {
  pub call_id: String,
  pub name: String,
  /// JSON 参数（前端按工具名解析）
  pub arguments: serde_json::Value,
}

/// AiChunk 扩展：工具调用事件（delta 为空、toolCall 非空）
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentChunk {
  pub delta: Option<String>,
  pub key_index: Option<usize>,
  pub error: Option<String>,
  pub done: bool,
  /// 非空 = 模型请求执行工具（前端执行后回传 ai_tool_result）
  pub tool_call: Option<AiToolCallEvent>,
}

/// 终端工具定义（发给模型的 schema）
pub fn terminal_tool_definitions() -> Vec<rig::completion::ToolDefinition> {
  vec![
    rig::completion::ToolDefinition {
      name: "terminal_get_screen".to_string(),
      description: "获取当前激活终端的屏幕内容和最近滚动缓冲（最多 lines 行，默认 120）。用于观察命令输出、系统状态、报错现场。".to_string(),
      parameters: json!({
        "type": "object",
        "properties": {
          "lines": { "type": "integer", "description": "读取的最大行数（1-2000，默认 120）" },
          "visible_only": { "type": "boolean", "description": "仅读可见屏幕区，不含滚动缓冲（默认 false）" }
        }
      }),
    },
    rig::completion::ToolDefinition {
      name: "terminal_send".to_string(),
      description: "向当前激活终端发送输入并按回车执行（等价用户键盘输入）。控制字符：\\r=回车 \\u0003=Ctrl-C。命令执行后应调用 terminal_get_screen 或 terminal_read_new 观察输出。".to_string(),
      parameters: json!({
        "type": "object",
        "properties": {
          "input": { "type": "string", "description": "要发送的内容（可含多行）" },
          "press_enter": { "type": "boolean", "description": "发送后是否追加回车执行（默认 true）" }
        },
        "required": ["input"]
      }),
    },
    rig::completion::ToolDefinition {
      name: "terminal_read_new".to_string(),
      description: "等待 wait_ms 毫秒后读取终端的新增输出（用于捕获 terminal_send 之后的命令回显）。".to_string(),
      parameters: json!({
        "type": "object",
        "properties": {
          "wait_ms": { "type": "integer", "description": "等待毫秒数（200-10000，默认 2000）" }
        }
      }),
    },
  ]
}

/// 智能体循环：带工具的流式对话，模型可多轮调用工具。
/// 每轮：发请求 → 流式收集文本与 ToolCall → 有 ToolCall 则推前端执行并等回传 →
/// 追加 assistant(tool_calls) + user(tool_results) 消息 → 下一轮；纯文本即结束。
async fn agent_loop<M>(
  model: M,
  mut history: Vec<rig::message::Message>,
  preamble: Option<String>,
  key_index: usize,
  on_chunk: &Channel<AgentChunk>,
  max_turns: usize,
) -> Result<(), String>
where
  M: rig::completion::CompletionModel + Clone,
{
  use futures::StreamExt;
  use rig::completion::CompletionModel;
  use rig::message::{AssistantContent, Message};

  let tools = terminal_tool_definitions();

  for _turn in 0..max_turns {
    // rig 无公开的 request 构造器带 tools 的简写：用 builder 链
    // （completion_request 的 prompt 会追加到末尾；这里 prompt 就是 history 最后一条）
    let mut builder = model
      .completion_request(history.clone().pop().unwrap_or_else(|| rig::message::Message::user("")))
      .tools(tools.clone())
      .max_tokens(4096);
    if let Some(p) = &preamble {
      builder = builder.preamble(p.clone());
    }
    let request = builder.build();

    let mut stream = model.stream(request).await.map_err(|e| e.to_string())?;
    let mut text_parts: Vec<String> = Vec::new();
    let mut tool_calls: Vec<rig::message::ToolCall> = Vec::new();

    while let Some(part) = stream.next().await {
      match part {
        Ok(rig::streaming::StreamedAssistantContent::Text(text)) => {
          let _ = on_chunk.send(AgentChunk {
            delta: Some(text.text.clone()),
            key_index: Some(key_index),
            error: None,
            done: false,
            tool_call: None,
          });
          text_parts.push(text.text);
        }
        Ok(rig::streaming::StreamedAssistantContent::ToolCall { tool_call, .. }) => {
          tool_calls.push(tool_call);
        }
        Ok(rig::streaming::StreamedAssistantContent::ToolCallDelta { .. }) => {}
        Ok(_) => {}
        Err(e) => {
          let _ = on_chunk.send(AgentChunk {
            delta: None,
            key_index: None,
            error: Some(e.to_string()),
            done: true,
            tool_call: None,
          });
          return Err(e.to_string());
        }
      }
    }

    if tool_calls.is_empty() {
      // 纯文本回复，循环结束
      let _ = on_chunk.send(AgentChunk {
        delta: None,
        key_index: None,
        error: None,
        done: true,
        tool_call: None,
      });
      return Ok(());
    }

    // 本轮既有文本又有工具调用：把 assistant 消息（文本+工具调用）入史
    let mut assistant_content: Vec<AssistantContent> = text_parts
      .iter()
      .map(|t| AssistantContent::text(t.clone()))
      .collect();
    for call in &tool_calls {
      assistant_content.push(AssistantContent::ToolCall(call.clone()));
    }
    history.push(Message::Assistant {
      id: None,
      content: assistant_content,
    });

    // 逐个推前端执行并等待回传
    let mut receivers = Vec::new();
    for call in &tool_calls {
      let call_id = call.id.as_str().to_string();
      let rx = register_pending(call_id.clone());
      receivers.push((call_id.clone(), call.function.name.clone(), rx));

      let _ = on_chunk.send(AgentChunk {
        delta: None,
        key_index: None,
        error: None,
        done: false,
        tool_call: Some(AiToolCallEvent {
          call_id,
          name: call.function.name.clone(),
          arguments: call.function.arguments.clone(),
        }),
      });
    }

    for (call_id, tool_name, rx) in receivers {
      let result = match tokio::time::timeout(std::time::Duration::from_secs(30), rx).await {
        Ok(Ok(result)) => result,
        Ok(Err(_)) => "[前端会话已关闭，工具未执行]".to_string(),
        Err(_) => "[工具执行超时（30s）]".to_string(),
      };
      history.push(Message::tool_result(call_id, tool_name, result));
    }
    // 继续下一轮：模型看到工具结果后决定继续调用还是给出最终文本
  }

  let _ = on_chunk.send(AgentChunk {
    delta: None,
    key_index: None,
    error: Some(format!("达到最大工具调用轮数（{max_turns}），已停止")),
    done: true,
    tool_call: None,
  });
  Err(format!("达到最大工具调用轮数（{max_turns}）"))
}

async fn run_agent_provider(
  record: &AiProviderRecord,
  api_key: &str,
  history: Vec<AiChatMessage>,
  preamble: Option<String>,
  key_index: usize,
  on_chunk: &Channel<AgentChunk>,
) -> Result<(), String> {
  use rig::client::CompletionClient;

  let mut messages = Vec::with_capacity(history.len());
  for item in &history {
    match item.role.as_str() {
      "assistant" => messages.push(rig::message::Message::assistant(item.content.clone())),
      _ => messages.push(rig::message::Message::user(item.content.clone())),
    }
  }
  let prompt = messages.pop().unwrap_or_else(|| rig::message::Message::user(""));

  match record.provider_type.as_str() {
    "openai" => {
      use rig::providers::openai::CompletionsClient;
      let mut builder = CompletionsClient::builder().api_key(api_key);
      if let Some(base_url) = record.base_url.as_deref().filter(|u| !u.trim().is_empty()) {
        builder = builder.base_url(base_url);
      }
      let client = builder.build().map_err(|e| e.to_string())?;
      let model = client.completion_model(record.model.clone());
      agent_loop_with_prompt(model, prompt, messages, preamble, key_index, on_chunk).await
    }
    "anthropic" => {
      use rig::providers::anthropic;
      let mut builder = anthropic::Client::builder().api_key(api_key);
      if let Some(base_url) = record.base_url.as_deref().filter(|u| !u.trim().is_empty()) {
        builder = builder.base_url(base_url);
      }
      let client = builder.build().map_err(|e| e.to_string())?;
      let model = client.completion_model(record.model.clone());
      agent_loop_with_prompt(model, prompt, messages, preamble, key_index, on_chunk).await
    }
    "gemini" => {
      use rig::providers::gemini;
      let mut builder = gemini::Client::builder().api_key(api_key);
      if let Some(base_url) = record.base_url.as_deref().filter(|u| !u.trim().is_empty()) {
        builder = builder.base_url(base_url);
      }
      let client = builder.build().map_err(|e| e.to_string())?;
      let model = client.completion_model(record.model.clone());
      agent_loop_with_prompt(model, prompt, messages, preamble, key_index, on_chunk).await
    }
    other => Err(format!("不支持的供应商类型: {other}")),
  }
}

/// 把 prompt（最后一条 user 消息）展开进 chat_history 后进入智能体循环
async fn agent_loop_with_prompt<M>(
  model: M,
  prompt: rig::message::Message,
  mut history: Vec<rig::message::Message>,
  preamble: Option<String>,
  key_index: usize,
  on_chunk: &Channel<AgentChunk>,
) -> Result<(), String>
where
  M: rig::completion::CompletionModel + Clone,
{
  history.push(prompt);
  agent_loop(model, history, preamble, key_index, on_chunk, 8).await
}

/// 带工具的 AI 智能体对话（ai_chat 的智能体版）：
/// 模型可自主调用 terminal_get_screen / terminal_send / terminal_read_new。
#[tauri::command]
pub async fn ai_agent_chat(
  provider_id: Option<usize>,
  history: Vec<AiChatMessage>,
  preamble: Option<String>,
  on_chunk: Channel<AgentChunk>,
  state: State<'_, DbPool>,
) -> Result<(), String> {
  let record = match provider_id {
    Some(id) => get_provider(&state, id)?,
    None => default_provider(&state)?.ok_or("尚未配置任何 AI 供应商")?,
  };

  let (key_index, api_key) = pick_key(&record)?;

  let _ = on_chunk.send(AgentChunk {
    delta: None,
    key_index: Some(key_index),
    error: None,
    done: false,
    tool_call: None,
  });

  run_agent_provider(&record, &api_key, history, preamble, key_index, &on_chunk).await
}
