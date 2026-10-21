use anyhow::{Result, anyhow, bail};
use serde_json::{Map, Value, json};

use crate::provider::ProviderProtocol;

#[derive(Debug, Clone)]
struct NormalizedRequest {
    model: String,
    system: Vec<String>,
    messages: Vec<NormalizedMessage>,
    tools: Vec<NormalizedTool>,
    max_tokens: Option<u64>,
    temperature: Option<Value>,
    stream: bool,
}

#[derive(Debug, Clone)]
struct NormalizedMessage {
    role: String,
    blocks: Vec<NormalizedBlock>,
}

#[derive(Debug, Clone)]
enum NormalizedBlock {
    Text(String),
    ToolUse {
        id: String,
        name: String,
        input: Value,
    },
    ToolResult {
        id: String,
        content: String,
    },
}

#[derive(Debug, Clone)]
struct NormalizedTool {
    name: String,
    description: String,
    input_schema: Value,
}

#[derive(Debug, Clone)]
struct NormalizedResponse {
    id: String,
    model: String,
    blocks: Vec<NormalizedBlock>,
    finish_reason: String,
    input_tokens: u64,
    output_tokens: u64,
}

pub fn translate_request(
    source: ProviderProtocol,
    destination: ProviderProtocol,
    mut body: Value,
    configured_model: &str,
) -> Result<Value> {
    if let Some(object) = body.as_object_mut() {
        object.insert(
            "model".to_string(),
            Value::String(configured_model.to_string()),
        );
    } else {
        bail!("请求正文必须是JSON对象");
    }
    if source == destination {
        return Ok(body);
    }
    let mut normalized = normalize_request(source, &body)?;
    normalized.model = configured_model.to_string();
    encode_request(destination, normalized)
}

pub fn force_non_stream(body: &mut Value) {
    if let Some(object) = body.as_object_mut() {
        object.insert("stream".to_string(), Value::Bool(false));
        object.remove("stream_options");
    }
}

pub fn request_streaming(body: &Value) -> bool {
    body.get("stream").and_then(Value::as_bool).unwrap_or(false)
}

pub fn translate_response(
    source: ProviderProtocol,
    destination: ProviderProtocol,
    body: Value,
) -> Result<Value> {
    if source == destination {
        return Ok(body);
    }
    encode_response(destination, normalize_response(source, &body)?)
}

pub fn synthetic_sse(protocol: ProviderProtocol, body: &Value) -> Result<Vec<u8>> {
    let normalized = normalize_response(protocol, body)?;
    let text = normalized
        .blocks
        .iter()
        .filter_map(|block| match block {
            NormalizedBlock::Text(text) => Some(text.as_str()),
            _ => None,
        })
        .collect::<String>();
    let output = match protocol {
        ProviderProtocol::Auto => bail!("自动协议必须在生成流式响应前完成解析"),
        ProviderProtocol::Responses => {
            let mut events = String::new();
            push_sse(
                &mut events,
                "response.created",
                &json!({"type":"response.created","response":{"id":normalized.id,"object":"response","status":"in_progress","model":normalized.model,"output":[]}}),
            )?;
            if !text.is_empty() {
                push_sse(
                    &mut events,
                    "response.output_text.delta",
                    &json!({"type":"response.output_text.delta","output_index":0,"content_index":0,"delta":text}),
                )?;
            }
            push_sse(
                &mut events,
                "response.completed",
                &json!({"type":"response.completed","response":body}),
            )?;
            events
        }
        ProviderProtocol::ChatCompletions => {
            let first = json!({
                "id": normalized.id,
                "object": "chat.completion.chunk",
                "model": normalized.model,
                "choices": [{"index":0,"delta":{"role":"assistant","content":text},"finish_reason":Value::Null}]
            });
            let last = json!({
                "id": normalized.id,
                "object": "chat.completion.chunk",
                "model": normalized.model,
                "choices": [{"index":0,"delta":{},"finish_reason":normalized.finish_reason}]
            });
            format!(
                "data: {}\n\ndata: {}\n\ndata: [DONE]\n\n",
                serde_json::to_string(&first)?,
                serde_json::to_string(&last)?
            )
        }
        ProviderProtocol::AnthropicMessages => {
            let mut events = String::new();
            push_sse(
                &mut events,
                "message_start",
                &json!({"type":"message_start","message":{"id":normalized.id,"type":"message","role":"assistant","model":normalized.model,"content":[],"stop_reason":Value::Null,"usage":{"input_tokens":normalized.input_tokens,"output_tokens":0}}}),
            )?;
            push_sse(
                &mut events,
                "content_block_start",
                &json!({"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}}),
            )?;
            if !text.is_empty() {
                push_sse(
                    &mut events,
                    "content_block_delta",
                    &json!({"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":text}}),
                )?;
            }
            push_sse(
                &mut events,
                "content_block_stop",
                &json!({"type":"content_block_stop","index":0}),
            )?;
            push_sse(
                &mut events,
                "message_delta",
                &json!({"type":"message_delta","delta":{"stop_reason":normalized.finish_reason},"usage":{"output_tokens":normalized.output_tokens}}),
            )?;
            push_sse(&mut events, "message_stop", &json!({"type":"message_stop"}))?;
            events
        }
    };
    Ok(output.into_bytes())
}

pub fn upstream_endpoint(base_url: &str, protocol: ProviderProtocol) -> String {
    let base = base_url.trim_end_matches('/');
    let suffix = match protocol {
        ProviderProtocol::Auto => "/chat/completions",
        ProviderProtocol::Responses => "/responses",
        ProviderProtocol::ChatCompletions => "/chat/completions",
        ProviderProtocol::AnthropicMessages => "/messages",
    };
    if base.ends_with(suffix) {
        return base.to_string();
    }
    if base.ends_with("/v1") {
        format!("{base}{suffix}")
    } else {
        format!("{base}/v1{suffix}")
    }
}

fn normalize_request(protocol: ProviderProtocol, body: &Value) -> Result<NormalizedRequest> {
    match protocol {
        ProviderProtocol::Auto => bail!("自动协议必须在规范化请求前完成解析"),
        ProviderProtocol::Responses => normalize_responses_request(body),
        ProviderProtocol::ChatCompletions => normalize_chat_request(body),
        ProviderProtocol::AnthropicMessages => normalize_anthropic_request(body),
    }
}

fn normalize_responses_request(body: &Value) -> Result<NormalizedRequest> {
    let mut request = base_request(body);
    if let Some(instructions) = body.get("instructions") {
        append_text_values(instructions, &mut request.system);
    }
    match body.get("input") {
        Some(Value::String(text)) => request.messages.push(NormalizedMessage {
            role: "user".to_string(),
            blocks: vec![NormalizedBlock::Text(text.clone())],
        }),
        Some(Value::Array(items)) => {
            for item in items {
                let item_type = item
                    .get("type")
                    .and_then(Value::as_str)
                    .unwrap_or("message");
                match item_type {
                    "message" => {
                        let role = item
                            .get("role")
                            .and_then(Value::as_str)
                            .unwrap_or("user")
                            .to_string();
                        request.messages.push(NormalizedMessage {
                            role,
                            blocks: normalize_content(item.get("content")),
                        });
                    }
                    "function_call" => request.messages.push(NormalizedMessage {
                        role: "assistant".to_string(),
                        blocks: vec![NormalizedBlock::ToolUse {
                            id: item
                                .get("call_id")
                                .or_else(|| item.get("id"))
                                .and_then(Value::as_str)
                                .unwrap_or("call_unknown")
                                .to_string(),
                            name: item
                                .get("name")
                                .and_then(Value::as_str)
                                .unwrap_or("unknown_tool")
                                .to_string(),
                            input: parse_json_or_string(item.get("arguments")),
                        }],
                    }),
                    "function_call_output" => request.messages.push(NormalizedMessage {
                        role: "user".to_string(),
                        blocks: vec![NormalizedBlock::ToolResult {
                            id: item
                                .get("call_id")
                                .and_then(Value::as_str)
                                .unwrap_or("call_unknown")
                                .to_string(),
                            content: text_value(item.get("output")),
                        }],
                    }),
                    unsupported => bail!(
                        "当前协议转换不支持Responses工具类型“{unsupported}”；Computer Use、远程MCP和本地Shell必须使用原生Responses供应商"
                    ),
                }
            }
        }
        _ => {}
    }
    request.tools = normalize_responses_tools(body.get("tools"))?;
    request.max_tokens = body
        .get("max_output_tokens")
        .or_else(|| body.get("max_tokens"))
        .and_then(Value::as_u64);
    Ok(request)
}

fn normalize_chat_request(body: &Value) -> Result<NormalizedRequest> {
    let mut request = base_request(body);
    for message in body
        .get("messages")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let role = message
            .get("role")
            .and_then(Value::as_str)
            .unwrap_or("user");
        if role == "system" || role == "developer" {
            append_text_values(
                message.get("content").unwrap_or(&Value::Null),
                &mut request.system,
            );
            continue;
        }
        if role == "tool" {
            request.messages.push(NormalizedMessage {
                role: "user".to_string(),
                blocks: vec![NormalizedBlock::ToolResult {
                    id: message
                        .get("tool_call_id")
                        .and_then(Value::as_str)
                        .unwrap_or("call_unknown")
                        .to_string(),
                    content: text_value(message.get("content")),
                }],
            });
            continue;
        }
        let mut blocks = normalize_content(message.get("content"));
        for tool_call in message
            .get("tool_calls")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            let function = tool_call.get("function").unwrap_or(&Value::Null);
            blocks.push(NormalizedBlock::ToolUse {
                id: tool_call
                    .get("id")
                    .and_then(Value::as_str)
                    .unwrap_or("call_unknown")
                    .to_string(),
                name: function
                    .get("name")
                    .and_then(Value::as_str)
                    .unwrap_or("unknown_tool")
                    .to_string(),
                input: parse_json_or_string(function.get("arguments")),
            });
        }
        request.messages.push(NormalizedMessage {
            role: role.to_string(),
            blocks,
        });
    }
    request.tools = body
        .get("tools")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|tool| tool.get("function"))
        .map(normalize_function_tool)
        .collect();
    request.max_tokens = body
        .get("max_completion_tokens")
        .or_else(|| body.get("max_tokens"))
        .and_then(Value::as_u64);
    Ok(request)
}

fn normalize_anthropic_request(body: &Value) -> Result<NormalizedRequest> {
    let mut request = base_request(body);
    if let Some(system) = body.get("system") {
        append_text_values(system, &mut request.system);
    }
    for message in body
        .get("messages")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        request.messages.push(NormalizedMessage {
            role: message
                .get("role")
                .and_then(Value::as_str)
                .unwrap_or("user")
                .to_string(),
            blocks: normalize_content(message.get("content")),
        });
    }
    request.tools = body
        .get("tools")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .map(normalize_function_tool)
        .collect();
    request.max_tokens = body.get("max_tokens").and_then(Value::as_u64);
    Ok(request)
}

fn base_request(body: &Value) -> NormalizedRequest {
    NormalizedRequest {
        model: body
            .get("model")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string(),
        system: Vec::new(),
        messages: Vec::new(),
        tools: Vec::new(),
        max_tokens: None,
        temperature: body.get("temperature").cloned(),
        stream: request_streaming(body),
    }
}

fn encode_request(protocol: ProviderProtocol, request: NormalizedRequest) -> Result<Value> {
    match protocol {
        ProviderProtocol::Auto => bail!("自动协议必须在编码请求前完成解析"),
        ProviderProtocol::Responses => encode_responses_request(request),
        ProviderProtocol::ChatCompletions => encode_chat_request(request),
        ProviderProtocol::AnthropicMessages => encode_anthropic_request(request),
    }
}

fn encode_responses_request(request: NormalizedRequest) -> Result<Value> {
    let mut input = Vec::new();
    for message in request.messages {
        let mut message_blocks = Vec::new();
        for block in message.blocks {
            match block {
                NormalizedBlock::Text(text) => message_blocks.push(json!({
                    "type": if message.role == "assistant" {"output_text"} else {"input_text"},
                    "text": text
                })),
                NormalizedBlock::ToolUse {
                    id,
                    name,
                    input: arguments,
                } => {
                    input.push(json!({"type":"function_call","call_id":id,"name":name,"arguments":serde_json::to_string(&arguments)?}));
                }
                NormalizedBlock::ToolResult { id, content } => {
                    input
                        .push(json!({"type":"function_call_output","call_id":id,"output":content}));
                }
            }
        }
        if !message_blocks.is_empty() {
            input.push(json!({"type":"message","role":message.role,"content":message_blocks}));
        }
    }
    let mut body = json!({
        "model": request.model,
        "input": input,
        "stream": request.stream
    });
    if !request.system.is_empty() {
        body["instructions"] = Value::String(request.system.join("\n\n"));
    }
    if let Some(max_tokens) = request.max_tokens {
        body["max_output_tokens"] = json!(max_tokens);
    }
    if let Some(temperature) = request.temperature {
        body["temperature"] = temperature;
    }
    if !request.tools.is_empty() {
        body["tools"] = Value::Array(
            request
                .tools
                .into_iter()
                .map(|tool| {
                    json!({"type":"function","name":tool.name,"description":tool.description,"parameters":tool.input_schema})
                })
                .collect(),
        );
    }
    Ok(body)
}

fn encode_chat_request(request: NormalizedRequest) -> Result<Value> {
    let mut messages = Vec::new();
    if !request.system.is_empty() {
        messages.push(json!({"role":"system","content":request.system.join("\n\n")}));
    }
    for message in request.messages {
        let mut text = String::new();
        let mut tool_calls = Vec::new();
        for block in message.blocks {
            match block {
                NormalizedBlock::Text(value) => text.push_str(&value),
                NormalizedBlock::ToolUse { id, name, input } => {
                    tool_calls.push(json!({"id":id,"type":"function","function":{"name":name,"arguments":serde_json::to_string(&input)?}}));
                }
                NormalizedBlock::ToolResult { id, content } => {
                    messages.push(json!({"role":"tool","tool_call_id":id,"content":content}));
                }
            }
        }
        if !text.is_empty() || !tool_calls.is_empty() {
            let mut value = json!({"role":message.role,"content":text});
            if !tool_calls.is_empty() {
                value["tool_calls"] = Value::Array(tool_calls);
            }
            messages.push(value);
        }
    }
    let mut body = json!({"model":request.model,"messages":messages,"stream":request.stream});
    if request.stream {
        body["stream_options"] = json!({"include_usage":true});
    }
    if let Some(max_tokens) = request.max_tokens {
        body["max_tokens"] = json!(max_tokens);
    }
    if let Some(temperature) = request.temperature {
        body["temperature"] = temperature;
    }
    if !request.tools.is_empty() {
        body["tools"] = Value::Array(
            request
                .tools
                .into_iter()
                .map(|tool| {
                    json!({"type":"function","function":{"name":tool.name,"description":tool.description,"parameters":tool.input_schema}})
                })
                .collect(),
        );
    }
    Ok(body)
}

fn encode_anthropic_request(request: NormalizedRequest) -> Result<Value> {
    let mut messages = Vec::new();
    for message in request.messages {
        let content = message
            .blocks
            .into_iter()
            .map(|block| match block {
                NormalizedBlock::Text(text) => json!({"type":"text","text":text}),
                NormalizedBlock::ToolUse { id, name, input } => {
                    json!({"type":"tool_use","id":id,"name":name,"input":input})
                }
                NormalizedBlock::ToolResult { id, content } => {
                    json!({"type":"tool_result","tool_use_id":id,"content":content})
                }
            })
            .collect::<Vec<_>>();
        messages.push(json!({"role":message.role,"content":content}));
    }
    let mut body = json!({
        "model": request.model,
        "messages": messages,
        "max_tokens": request.max_tokens.unwrap_or(4096),
        "stream": request.stream
    });
    if !request.system.is_empty() {
        body["system"] = Value::String(request.system.join("\n\n"));
    }
    if let Some(temperature) = request.temperature {
        body["temperature"] = temperature;
    }
    if !request.tools.is_empty() {
        body["tools"] = Value::Array(
            request
                .tools
                .into_iter()
                .map(|tool| {
                    json!({"name":tool.name,"description":tool.description,"input_schema":tool.input_schema})
                })
                .collect(),
        );
    }
    Ok(body)
}

fn normalize_response(protocol: ProviderProtocol, body: &Value) -> Result<NormalizedResponse> {
    match protocol {
        ProviderProtocol::Auto => bail!("自动协议必须在规范化响应前完成解析"),
        ProviderProtocol::Responses => normalize_responses_response(body),
        ProviderProtocol::ChatCompletions => normalize_chat_response(body),
        ProviderProtocol::AnthropicMessages => normalize_anthropic_response(body),
    }
}

fn normalize_responses_response(body: &Value) -> Result<NormalizedResponse> {
    let mut blocks = Vec::new();
    for item in body
        .get("output")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        match item.get("type").and_then(Value::as_str).unwrap_or("") {
            "message" => blocks.extend(normalize_content(item.get("content"))),
            "function_call" => blocks.push(NormalizedBlock::ToolUse {
                id: item
                    .get("call_id")
                    .or_else(|| item.get("id"))
                    .and_then(Value::as_str)
                    .unwrap_or("call_unknown")
                    .to_string(),
                name: item
                    .get("name")
                    .and_then(Value::as_str)
                    .unwrap_or("unknown_tool")
                    .to_string(),
                input: parse_json_or_string(item.get("arguments")),
            }),
            _ => {}
        }
    }
    let usage = body.get("usage").unwrap_or(&Value::Null);
    Ok(NormalizedResponse {
        id: string_or(body.get("id"), "resp_yunshu"),
        model: string_or(body.get("model"), ""),
        blocks,
        finish_reason: if body.get("status").and_then(Value::as_str) == Some("incomplete") {
            "length".to_string()
        } else {
            "stop".to_string()
        },
        input_tokens: usage
            .get("input_tokens")
            .and_then(Value::as_u64)
            .unwrap_or(0),
        output_tokens: usage
            .get("output_tokens")
            .and_then(Value::as_u64)
            .unwrap_or(0),
    })
}

fn normalize_chat_response(body: &Value) -> Result<NormalizedResponse> {
    let choice = body
        .get("choices")
        .and_then(Value::as_array)
        .and_then(|choices| choices.first())
        .ok_or_else(|| anyhow!("Chat Completions响应缺少choices"))?;
    let message = choice
        .get("message")
        .ok_or_else(|| anyhow!("Chat Completions响应缺少message"))?;
    let mut blocks = normalize_content(message.get("content"));
    for tool_call in message
        .get("tool_calls")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let function = tool_call.get("function").unwrap_or(&Value::Null);
        blocks.push(NormalizedBlock::ToolUse {
            id: string_or(tool_call.get("id"), "call_unknown"),
            name: string_or(function.get("name"), "unknown_tool"),
            input: parse_json_or_string(function.get("arguments")),
        });
    }
    let usage = body.get("usage").unwrap_or(&Value::Null);
    Ok(NormalizedResponse {
        id: string_or(body.get("id"), "chatcmpl_yunshu"),
        model: string_or(body.get("model"), ""),
        blocks,
        finish_reason: string_or(choice.get("finish_reason"), "stop"),
        input_tokens: usage
            .get("prompt_tokens")
            .and_then(Value::as_u64)
            .unwrap_or(0),
        output_tokens: usage
            .get("completion_tokens")
            .and_then(Value::as_u64)
            .unwrap_or(0),
    })
}

fn normalize_anthropic_response(body: &Value) -> Result<NormalizedResponse> {
    let usage = body.get("usage").unwrap_or(&Value::Null);
    Ok(NormalizedResponse {
        id: string_or(body.get("id"), "msg_yunshu"),
        model: string_or(body.get("model"), ""),
        blocks: normalize_content(body.get("content")),
        finish_reason: match body
            .get("stop_reason")
            .and_then(Value::as_str)
            .unwrap_or("end_turn")
        {
            "max_tokens" => "length".to_string(),
            "tool_use" => "tool_calls".to_string(),
            _ => "stop".to_string(),
        },
        input_tokens: usage
            .get("input_tokens")
            .and_then(Value::as_u64)
            .unwrap_or(0),
        output_tokens: usage
            .get("output_tokens")
            .and_then(Value::as_u64)
            .unwrap_or(0),
    })
}

fn encode_response(protocol: ProviderProtocol, response: NormalizedResponse) -> Result<Value> {
    match protocol {
        ProviderProtocol::Auto => bail!("自动协议必须在编码响应前完成解析"),
        ProviderProtocol::Responses => {
            let mut output = Vec::new();
            let mut content = Vec::new();
            for block in response.blocks {
                match block {
                    NormalizedBlock::Text(text) => {
                        content.push(json!({"type":"output_text","text":text,"annotations":[]}))
                    }
                    NormalizedBlock::ToolUse { id, name, input } => output.push(json!({
                        "type":"function_call","id":format!("fc_{id}"),"call_id":id,"name":name,"arguments":serde_json::to_string(&input)?,"status":"completed"
                    })),
                    NormalizedBlock::ToolResult { .. } => {}
                }
            }
            if !content.is_empty() {
                output.insert(0, json!({"type":"message","id":format!("msg_{}",response.id),"role":"assistant","status":"completed","content":content}));
            }
            Ok(json!({
                "id":response.id,"object":"response","status":"completed","model":response.model,"output":output,
                "usage":{"input_tokens":response.input_tokens,"output_tokens":response.output_tokens,"total_tokens":response.input_tokens+response.output_tokens}
            }))
        }
        ProviderProtocol::ChatCompletions => {
            let mut content = String::new();
            let mut tool_calls = Vec::new();
            for block in response.blocks {
                match block {
                    NormalizedBlock::Text(text) => content.push_str(&text),
                    NormalizedBlock::ToolUse { id, name, input } => tool_calls.push(json!({
                        "id":id,"type":"function","function":{"name":name,"arguments":serde_json::to_string(&input)?}
                    })),
                    NormalizedBlock::ToolResult { .. } => {}
                }
            }
            let mut message = json!({"role":"assistant","content":content});
            if !tool_calls.is_empty() {
                message["tool_calls"] = Value::Array(tool_calls);
            }
            Ok(json!({
                "id":response.id,"object":"chat.completion","model":response.model,
                "choices":[{"index":0,"message":message,"finish_reason":response.finish_reason}],
                "usage":{"prompt_tokens":response.input_tokens,"completion_tokens":response.output_tokens,"total_tokens":response.input_tokens+response.output_tokens}
            }))
        }
        ProviderProtocol::AnthropicMessages => {
            let content = response
                .blocks
                .into_iter()
                .filter_map(|block| match block {
                    NormalizedBlock::Text(text) => Some(json!({"type":"text","text":text})),
                    NormalizedBlock::ToolUse { id, name, input } => {
                        Some(json!({"type":"tool_use","id":id,"name":name,"input":input}))
                    }
                    NormalizedBlock::ToolResult { .. } => None,
                })
                .collect::<Vec<_>>();
            let stop_reason = match response.finish_reason.as_str() {
                "length" => "max_tokens",
                "tool_calls" => "tool_use",
                _ => "end_turn",
            };
            Ok(json!({
                "id":response.id,"type":"message","role":"assistant","model":response.model,"content":content,
                "stop_reason":stop_reason,"stop_sequence":Value::Null,
                "usage":{"input_tokens":response.input_tokens,"output_tokens":response.output_tokens}
            }))
        }
    }
}

fn normalize_content(value: Option<&Value>) -> Vec<NormalizedBlock> {
    match value {
        Some(Value::String(text)) => vec![NormalizedBlock::Text(text.clone())],
        Some(Value::Array(items)) => items.iter().filter_map(normalize_content_block).collect(),
        Some(value) if !value.is_null() => vec![NormalizedBlock::Text(text_value(Some(value)))],
        _ => Vec::new(),
    }
}

fn normalize_content_block(block: &Value) -> Option<NormalizedBlock> {
    match block.get("type").and_then(Value::as_str).unwrap_or("text") {
        "text" | "input_text" | "output_text" => Some(NormalizedBlock::Text(
            block
                .get("text")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string(),
        )),
        "tool_use" => Some(NormalizedBlock::ToolUse {
            id: string_or(block.get("id"), "call_unknown"),
            name: string_or(block.get("name"), "unknown_tool"),
            input: block.get("input").cloned().unwrap_or_else(|| json!({})),
        }),
        "tool_result" => Some(NormalizedBlock::ToolResult {
            id: string_or(block.get("tool_use_id"), "call_unknown"),
            content: text_value(block.get("content")),
        }),
        _ => None,
    }
}

fn normalize_responses_tools(value: Option<&Value>) -> Result<Vec<NormalizedTool>> {
    let mut tools = Vec::new();
    for tool in value.and_then(Value::as_array).into_iter().flatten() {
        if tool.get("type").and_then(Value::as_str) != Some("function") {
            let unsupported = tool
                .get("type")
                .and_then(Value::as_str)
                .unwrap_or("unknown");
            bail!(
                "当前协议转换不支持Responses工具类型“{unsupported}”；Computer Use、远程MCP和本地Shell必须使用原生Responses供应商"
            );
        }
        tools.push(normalize_function_tool(tool));
    }
    Ok(tools)
}

fn normalize_function_tool(tool: &Value) -> NormalizedTool {
    NormalizedTool {
        name: string_or(
            tool.get("name")
                .or_else(|| tool.get("function").and_then(|f| f.get("name"))),
            "unknown_tool",
        ),
        description: string_or(
            tool.get("description")
                .or_else(|| tool.get("function").and_then(|f| f.get("description"))),
            "",
        ),
        input_schema: tool
            .get("input_schema")
            .or_else(|| tool.get("parameters"))
            .or_else(|| tool.get("function").and_then(|f| f.get("parameters")))
            .cloned()
            .unwrap_or_else(|| json!({"type":"object","properties":{}})),
    }
}

fn append_text_values(value: &Value, output: &mut Vec<String>) {
    match value {
        Value::String(text) if !text.is_empty() => output.push(text.clone()),
        Value::Array(items) => {
            for item in items {
                if let Some(text) = item.get("text").and_then(Value::as_str)
                    && !text.is_empty()
                {
                    output.push(text.to_string());
                }
            }
        }
        _ => {}
    }
}

fn parse_json_or_string(value: Option<&Value>) -> Value {
    match value {
        Some(Value::String(text)) => {
            serde_json::from_str(text).unwrap_or_else(|_| Value::String(text.clone()))
        }
        Some(value) => value.clone(),
        None => json!({}),
    }
}

fn text_value(value: Option<&Value>) -> String {
    match value {
        Some(Value::String(text)) => text.clone(),
        Some(Value::Array(items)) => items
            .iter()
            .filter_map(|item| {
                item.as_str()
                    .map(str::to_string)
                    .or_else(|| item.get("text").and_then(Value::as_str).map(str::to_string))
            })
            .collect::<Vec<_>>()
            .join(""),
        Some(value) if !value.is_null() => value.to_string(),
        _ => String::new(),
    }
}

fn string_or(value: Option<&Value>, fallback: &str) -> String {
    value
        .and_then(Value::as_str)
        .unwrap_or(fallback)
        .to_string()
}

fn push_sse(output: &mut String, event: &str, data: &Value) -> Result<()> {
    output.push_str("event: ");
    output.push_str(event);
    output.push('\n');
    output.push_str("data: ");
    output.push_str(&serde_json::to_string(data)?);
    output.push_str("\n\n");
    Ok(())
}

pub fn safe_error_body(code: &str, message: &str) -> Value {
    let mut error = Map::new();
    error.insert("type".to_string(), Value::String(code.to_string()));
    error.insert("message".to_string(), Value::String(message.to_string()));
    json!({"error": Value::Object(error)})
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn responses_and_anthropic_round_trip_text_and_tool_call() {
        let request = json!({
            "model":"ignored",
            "instructions":"请简洁回答",
            "input":[{"type":"message","role":"user","content":[{"type":"input_text","text":"你好"}]}],
            "tools":[{"type":"function","name":"weather","description":"天气","parameters":{"type":"object"}}],
            "stream":true
        });
        let anthropic = translate_request(
            ProviderProtocol::Responses,
            ProviderProtocol::AnthropicMessages,
            request,
            "claude-test",
        )
        .unwrap();
        assert_eq!(anthropic["model"], "claude-test");
        assert_eq!(anthropic["system"], "请简洁回答");
        assert_eq!(anthropic["messages"][0]["content"][0]["text"], "你好");
        assert_eq!(anthropic["tools"][0]["name"], "weather");

        let response = json!({
            "id":"msg_1","type":"message","model":"claude-test",
            "content":[{"type":"text","text":"你好！"},{"type":"tool_use","id":"tool_1","name":"weather","input":{"city":"上海"}}],
            "stop_reason":"tool_use","usage":{"input_tokens":10,"output_tokens":5}
        });
        let responses = translate_response(
            ProviderProtocol::AnthropicMessages,
            ProviderProtocol::Responses,
            response,
        )
        .unwrap();
        assert_eq!(responses["output"][0]["content"][0]["text"], "你好！");
        assert_eq!(responses["output"][1]["name"], "weather");
    }

    #[test]
    fn unsupported_computer_use_is_explicit() {
        let error = translate_request(
            ProviderProtocol::Responses,
            ProviderProtocol::ChatCompletions,
            json!({"model":"x","input":"test","tools":[{"type":"computer_use_preview"}]}),
            "model",
        )
        .unwrap_err()
        .to_string();
        assert!(error.contains("Computer Use"));
        assert!(error.contains("原生Responses"));
    }

    #[test]
    fn endpoints_do_not_duplicate_v1() {
        assert_eq!(
            upstream_endpoint("https://example.com/v1", ProviderProtocol::ChatCompletions),
            "https://example.com/v1/chat/completions"
        );
        assert_eq!(
            upstream_endpoint(
                "https://example.com/v1/messages",
                ProviderProtocol::AnthropicMessages
            ),
            "https://example.com/v1/messages"
        );
    }
}
