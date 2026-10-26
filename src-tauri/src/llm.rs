//! Reviewer / Planner 的 LLM 调用层。
//!
//! - 统一 OpenAI 兼容 `/chat/completions` 或 Anthropic `/v1/messages`
//! - 强制 JSON 输出；若首次不是合法 JSON，自动发起一次 JSON 修复重试
//! - 所有错误都转为可读字符串返回给状态机写日志

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{mpsc, Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use reqwest::blocking::{Client, Response};
use reqwest::StatusCode;
use serde_json::{json, Value};

use crate::models::AgentConfig;
use crate::prompts;

static RATE_LIMIT_PER_MIN: AtomicU64 = AtomicU64::new(0);

fn rate_times() -> &'static Mutex<VecDeque<Instant>> {
    static TIMES: OnceLock<Mutex<VecDeque<Instant>>> = OnceLock::new();
    TIMES.get_or_init(|| Mutex::new(VecDeque::new()))
}

/// 设置每分钟 LLM HTTP 请求上限；0 表示不限。
pub fn set_rate_limit_per_min(limit: u64) {
    RATE_LIMIT_PER_MIN.store(limit, Ordering::SeqCst);
}

/// 简单的进程级滑动窗口限速。等待发生在调度线程内，不会阻塞 UI。
fn wait_rate_limit() {
    let limit = RATE_LIMIT_PER_MIN.load(Ordering::SeqCst);
    if limit == 0 {
        return;
    }
    loop {
        let wait = {
            let mut times = match rate_times().lock() {
                Ok(t) => t,
                Err(_) => return,
            };
            let now = Instant::now();
            while let Some(front) = times.front() {
                if now.duration_since(*front) >= Duration::from_secs(60) {
                    times.pop_front();
                } else {
                    break;
                }
            }
            if (times.len() as u64) < limit {
                times.push_back(now);
                return;
            }
            let front = *times.front().unwrap();
            Duration::from_secs(60).saturating_sub(now.duration_since(front))
        };
        std::thread::sleep(wait.max(Duration::from_millis(50)));
    }
}

fn is_anthropic(provider: &str) -> bool {
    provider.to_ascii_lowercase().contains("anthropic")
}

fn endpoint(agent: &AgentConfig) -> Result<String, String> {
    let base = agent.base_url.trim().trim_end_matches('/');
    if base.is_empty() {
        return Err("Agent baseUrl 为空".into());
    }
    if is_anthropic(&agent.provider) {
        if base.ends_with("/messages") {
            Ok(base.to_string())
        } else if base.ends_with("/v1") {
            Ok(format!("{base}/messages"))
        } else {
            Ok(format!("{base}/v1/messages"))
        }
    } else if base.ends_with("/chat/completions") {
        Ok(base.to_string())
    } else {
        Ok(format!("{base}/chat/completions"))
    }
}

fn timeout_secs(raw: &str) -> u64 {
    raw.trim().parse::<u64>().unwrap_or(180).clamp(5, 3600)
}

fn truncate_for_error(raw: &str) -> String {
    prompts::truncate_chars(raw, 1500)
}

/// 发起一次结构化 JSON 对话，返回解析后的 JSON。
pub fn chat_json(
    agent: &AgentConfig,
    system: &str,
    user: &str,
    llm_timeout_sec: &str,
) -> Result<Value, String> {
    if agent.api_key.trim().is_empty() {
        return Err(format!("Agent「{}」未配置 apiKey", agent.name));
    }
    let timeout = Duration::from_secs(timeout_secs(llm_timeout_sec));
    let client = Client::builder()
        .timeout(timeout)
        .build()
        .map_err(|e| format!("创建 HTTP client 失败: {e}"))?;

    wait_rate_limit();
    let first = request_once(&client, agent, system, user, true)?;
    match extract_json(&first) {
        Ok(v) => Ok(v),
        Err(first_err) => {
            // 按需求强制结构化输出：解析失败不进入业务状态，先做一次 JSON 修复重试。
            let fix_prompt = prompts::json_fix_user(user, &first);
            wait_rate_limit();
            let second = request_once(&client, agent, system, &fix_prompt, true)?;
            extract_json(&second).map_err(|second_err| {
                format!(
                    "LLM 返回内容无法解析为 JSON（已自动重试一次）。首次错误: {first_err}；重试错误: {second_err}；原始片段: {}",
                    truncate_for_error(&second)
                )
            })
        }
    }
}

/// 临时问答通道：普通文本回复，不强制 JSON，也不会触发任何任务调度。
pub fn chat_text(
    agent: &AgentConfig,
    system: &str,
    user: &str,
    llm_timeout_sec: &str,
) -> Result<String, String> {
    if agent.api_key.trim().is_empty() {
        return Err(format!("Agent「{}」未配置 apiKey", agent.name));
    }
    let timeout = Duration::from_secs(timeout_secs(llm_timeout_sec));
    let client = Client::builder()
        .timeout(timeout)
        .build()
        .map_err(|e| format!("创建 HTTP client 失败: {e}"))?;
    wait_rate_limit();
    request_once(&client, agent, system, user, false)
}

fn request_once(
    client: &Client,
    agent: &AgentConfig,
    system: &str,
    user: &str,
    json_mode: bool,
) -> Result<String, String> {
    match endpoint(agent) {
        Ok(url) => {
            if is_anthropic(&agent.provider) {
                anthropic_request(client, agent, &url, system, user)
            } else {
                openai_request(client, agent, &url, system, user, json_mode)
            }
        }
        Err(e) => Err(e),
    }
}

fn openai_request(
    client: &Client,
    agent: &AgentConfig,
    url: &str,
    system: &str,
    user: &str,
    json_mode: bool,
) -> Result<String, String> {
    let mut body = json!({
        "model": agent.model_name,
        "temperature": agent.temperature,
        "messages": [
            { "role": "system", "content": system },
            { "role": "user", "content": user }
        ]
    });
    if json_mode {
        if let Some(obj) = body.as_object_mut() {
            obj.insert("response_format".into(), json!({ "type": "json_object" }));
        }
    }

    let mut resp = post_json(client, url, &agent.api_key, &body, false)?;
    // 少数 OpenAI 兼容服务不识别 response_format，降级重试一次。
    if json_mode
        && matches!(
            resp.status(),
            StatusCode::BAD_REQUEST | StatusCode::UNPROCESSABLE_ENTITY
        )
    {
        let bad_status = resp.status();
        let text = resp.text().unwrap_or_default();
        if text.contains("response_format") || text.contains("json_object") {
            if let Some(obj) = body.as_object_mut() {
                obj.remove("response_format");
            }
            resp = post_json(client, url, &agent.api_key, &body, false)?;
        } else {
            return Err(format!(
                "LLM HTTP 错误 {bad_status}：{}",
                truncate_for_error(&text)
            ));
        }
    }

    let status = resp.status();
    let text = resp.text().map_err(|e| format!("读取 LLM 响应失败: {e}"))?;
    if !status.is_success() {
        return Err(format!(
            "LLM HTTP 错误 {status}（URL: {url}，Agent: {} / {}）：{}",
            agent.name,
            agent.model_name,
            truncate_for_error(&text)
        ));
    }
    let value: Value =
        serde_json::from_str(&text).map_err(|e| format!("LLM 响应不是 JSON: {e}"))?;
    content_from_openai(&value)
}

fn anthropic_request(
    client: &Client,
    agent: &AgentConfig,
    url: &str,
    system: &str,
    user: &str,
) -> Result<String, String> {
    let body = json!({
        "model": agent.model_name,
        "max_tokens": 8192,
        "temperature": agent.temperature,
        "system": system,
        "messages": [{ "role": "user", "content": user }]
    });
    let resp = post_json(client, url, &agent.api_key, &body, true)?;
    let status = resp.status();
    let text = resp.text().map_err(|e| format!("读取 LLM 响应失败: {e}"))?;
    if !status.is_success() {
        return Err(format!(
            "LLM HTTP 错误 {status}（URL: {url}，Agent: {} / {}）：{}",
            agent.name,
            agent.model_name,
            truncate_for_error(&text)
        ));
    }
    let value: Value =
        serde_json::from_str(&text).map_err(|e| format!("LLM 响应不是 JSON: {e}"))?;
    content_from_anthropic(&value)
}

fn post_json(
    client: &Client,
    url: &str,
    api_key: &str,
    body: &Value,
    anthropic: bool,
) -> Result<Response, String> {
    let mut req = client.post(url).json(body);
    if anthropic {
        req = req
            .header("x-api-key", api_key)
            .header("anthropic-version", "2023-06-01");
    } else {
        req = req.bearer_auth(api_key);
    }
    req.send().map_err(|e| {
        if e.is_timeout() {
            format!("LLM 调用超时（{url}）: {e}")
        } else {
            format!("LLM 请求失败（{url}）: {e}")
        }
    })
}

fn content_from_openai(value: &Value) -> Result<String, String> {
    let content = value.pointer("/choices/0/message/content").ok_or_else(|| {
        format!(
            "LLM 响应缺少 choices[0].message.content: {}",
            truncate_for_error(&value.to_string())
        )
    })?;
    let text = match content {
        Value::String(s) => s.clone(),
        Value::Array(items) => items
            .iter()
            .filter_map(|item| item.get("text").and_then(Value::as_str))
            .collect::<Vec<_>>()
            .join("\n"),
        Value::Null => String::new(),
        other => other.to_string(),
    };
    if text.trim().is_empty() {
        return Err("LLM 返回内容为空".into());
    }
    Ok(text)
}

fn content_from_anthropic(value: &Value) -> Result<String, String> {
    let items = value
        .get("content")
        .and_then(Value::as_array)
        .ok_or_else(|| {
            format!(
                "Anthropic 响应缺少 content: {}",
                truncate_for_error(&value.to_string())
            )
        })?;
    let text = items
        .iter()
        .filter(|item| item.get("type").and_then(Value::as_str) == Some("text"))
        .filter_map(|item| item.get("text").and_then(Value::as_str))
        .collect::<Vec<_>>()
        .join("\n");
    if text.trim().is_empty() {
        return Err("LLM 返回内容为空".into());
    }
    Ok(text)
}

/// 从可能带 Markdown 围栏/前后解释的文本中提取第一个平衡的 JSON 对象。
pub fn extract_json(raw: &str) -> Result<Value, String> {
    let trimmed = raw.trim();
    if let Ok(v) = serde_json::from_str::<Value>(trimmed) {
        return Ok(v);
    }

    let chars: Vec<char> = trimmed.chars().collect();
    let start = chars
        .iter()
        .position(|c| *c == '{')
        .ok_or_else(|| "未找到 JSON 对象起始 '{'".to_string())?;
    let mut depth = 0i32;
    let mut in_string = false;
    let mut escaped = false;
    for i in start..chars.len() {
        let c = chars[i];
        if in_string {
            if escaped {
                escaped = false;
            } else if c == '\\' {
                escaped = true;
            } else if c == '"' {
                in_string = false;
            }
            continue;
        }
        match c {
            '"' => in_string = true,
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    let candidate: String = chars[start..=i].iter().collect();
                    return serde_json::from_str::<Value>(&candidate)
                        .map_err(|e| format!("提取到的 JSON 解析失败: {e}"));
                }
            }
            _ => {}
        }
    }
    Err("JSON 对象未闭合".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_json_from_fence_and_prose() {
        let raw = "好的，结果如下：\n```json\n{\"passed\": true, \"feedback\": \"ok {not a brace}\"}\n```";
        let v = extract_json(raw).unwrap();
        assert_eq!(v["passed"], true);
    }

    #[test]
    fn handles_escaped_quotes() {
        let raw = r#"{"feedback":"say \"hello\" {x}","passed":false}"#;
        let v = extract_json(raw).unwrap();
        assert_eq!(v["passed"], false);
    }
}

/// 可取消版本：HTTP 请求放后台线程，主线程轮询取消标记。
/// 取消后丢弃后台返回结果；请求线程无法强制中断，但不会再影响调度。
pub fn chat_json_cancellable(
    agent: &AgentConfig,
    system: &str,
    user: &str,
    llm_timeout_sec: &str,
    cancel: Arc<AtomicBool>,
) -> Result<Value, String> {
    let agent = agent.clone();
    let system = system.to_string();
    let user = user.to_string();
    let timeout = llm_timeout_sec.to_string();
    let (tx, rx) = mpsc::channel::<Result<Value, String>>();
    std::thread::spawn(move || {
        let result = chat_json(&agent, &system, &user, &timeout);
        let _ = tx.send(result);
    });
    loop {
        if cancel.load(Ordering::SeqCst) {
            return Err("人工终止：已放弃当前 LLM 调用".into());
        }
        match rx.recv_timeout(Duration::from_millis(150)) {
            Ok(result) => return result,
            Err(mpsc::RecvTimeoutError::Timeout) => continue,
            Err(mpsc::RecvTimeoutError::Disconnected) => return Err("LLM 调用线程异常退出".into()),
        }
    }
}
