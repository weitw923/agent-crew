//! Harness CLI（dsh）适配层 + git diff 快照。
//!
//! 应用自身不实现代码读写：Coder 的所有文件操作由 dsh 的 read_file/write/bash/git 工具完成。
//! 本模块只负责：
//!   1. 按 Coder Agent 配置生成临时 dsh provider patch；
//!   2. 用 stdin 传入 prompt、带超时执行 `dsh --profile headless --patch ... --json -`；
//!   3. 用临时 git index 对工作目录做 tree 快照，计算真实 diff（不污染用户真实 index）。

use std::env;
use std::fs;
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc};
use std::thread;
use std::time::{Duration, Instant};

use crate::models::{AgentConfig, AppSettings, Subtask};

#[derive(Debug, Clone)]
pub struct DshRunResult {
    pub exit_code: i32,
    pub stderr: String,
    pub final_text: String,
}

impl DshRunResult {
    pub fn success(&self) -> bool {
        self.exit_code == 0
    }
}

/// 解析 dsh 可执行文件；GUI 应用的 PATH 往往不含 Homebrew，因此主动兜底常见路径。
fn resolve_dsh(bin: &str) -> String {
    let bin = bin.trim();
    if bin.is_empty() {
        return "dsh".to_string();
    }
    if bin.contains('/') || bin.contains('\\') {
        return bin.to_string();
    }
    let mut dirs: Vec<PathBuf> = vec![
        PathBuf::from("/opt/homebrew/bin"),
        PathBuf::from("/usr/local/bin"),
        PathBuf::from("/usr/bin"),
    ];
    if let Some(home) = env::var_os("HOME") {
        dirs.push(PathBuf::from(&home).join(".local/bin"));
        dirs.push(PathBuf::from(&home).join(".cargo/bin"));
    }
    if let Some(path) = env::var_os("PATH") {
        dirs.extend(env::split_paths(&path));
    }
    for dir in dirs {
        let candidate = dir.join(bin);
        if candidate.is_file() {
            return candidate.to_string_lossy().to_string();
        }
    }
    bin.to_string()
}

fn yaml_quote(value: &str) -> String {
    // JSON 双引号字符串是合法 YAML 标量，可安全转义引号/换行/反斜杠。
    serde_json::to_string(value).unwrap_or_else(|_| "\"\"".to_string())
}

fn api_mode(provider: &str) -> &'static str {
    if provider.to_ascii_lowercase().contains("anthropic") {
        "anthropic-messages"
    } else {
        "openai-completions"
    }
}

/// 根据 Coder Agent 生成 dsh patch，让 dsh 使用该 Agent 的模型/密钥/baseUrl。
pub fn write_provider_patch(app_data_dir: &Path, agent: &AgentConfig) -> Result<PathBuf, String> {
    let runtime_dir = app_data_dir.join("runtime");
    fs::create_dir_all(&runtime_dir).map_err(|e| format!("创建 runtime 目录失败: {e}"))?;
    let base_url = agent.base_url.trim().trim_end_matches('/').to_string();
    if base_url.is_empty() {
        return Err("Coder Agent baseUrl 为空".into());
    }
    let patch = format!(
        r#"- id: llm-pi-ai
  name: "@deepseek-ai/dsh-llm-pi-ai"
  config:
    providers:
      orchestrator-coder:
        displayName: {display}
        apiKeyEnv: DSH_AGENT_API_KEY
        api: {api}
        baseURL: {base}
        models:
          - id: {model}
            name: {model}
            contextWindow: 200000
- id: agent-default-model
  name: "@deepseek-ai/dsh-agent-default-model"
  config:
    provider: orchestrator-coder
    model: {model}
"#,
        display = yaml_quote(&agent.name),
        api = api_mode(&agent.provider),
        base = yaml_quote(&base_url),
        model = yaml_quote(&agent.model_name),
    );
    let path = runtime_dir.join(format!("coder-{}-patch.yml", agent.id));
    fs::write(&path, patch).map_err(|e| format!("写入 dsh patch 失败: {e}"))?;
    Ok(path)
}

/// 以 stdin 传 prompt 执行 dsh。stdout 为 NDJSON 事件，逐行回调给页面实时输出。
pub fn run_coder(
    app_data_dir: &Path,
    settings: &AppSettings,
    agent: &AgentConfig,
    work_dir: &Path,
    prompt: &str,
    timeout_secs: u64,
    cancel: Option<Arc<AtomicBool>>,
    on_stdout: &mut dyn FnMut(&str),
) -> Result<DshRunResult, String> {
    if !work_dir.is_dir() {
        return Err(format!("工作目录不存在: {}", work_dir.display()));
    }
    let patch_path = write_provider_patch(app_data_dir, agent)?;
    let dsh = resolve_dsh(&settings.dsh_bin);
    let profile = if settings.dsh_profile.trim().is_empty() {
        "headless"
    } else {
        settings.dsh_profile.trim()
    };

    let mut cmd = Command::new(&dsh);
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        // 独立进程组：取消/超时时可以连同 dsh 派生的工具子进程一起杀掉。
        cmd.process_group(0);
    }
    cmd.arg("--profile")
        .arg(profile)
        .arg("--patch")
        .arg(&patch_path)
        .arg("--json")
        .arg("-")
        .current_dir(work_dir)
        .env("DSH_AGENT_API_KEY", &agent.api_key)
        // headless 无人值守：允许写工作目录与执行 bash；仍只在用户选择的项目目录内工作。
        .env("DSH_PERMISSION_MODE", "danger-full-access")
        .env("DSH_PROFILE", profile)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    let output = run_process(
        &mut cmd,
        Some(prompt),
        Duration::from_secs(timeout_secs.max(30)),
        cancel,
        on_stdout,
    )?;
    let final_text = parse_final_text(&output.0);
    Ok(DshRunResult {
        exit_code: output.1,
        stderr: output.2,
        final_text,
    })
}

fn parse_final_text(stdout: &str) -> String {
    let mut final_text = String::new();
    for line in stdout.lines() {
        let line = line.trim();
        if !line.starts_with('{') {
            continue;
        }
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(line) {
            if v.get("type").and_then(|t| t.as_str()) == Some("final") {
                if let Some(t) = v.get("text").and_then(|t| t.as_str()) {
                    final_text = t.to_string();
                }
            }
        }
    }
    final_text
}

fn drain_stdout(rx: &mpsc::Receiver<String>, stdout: &mut String, on_stdout: &mut dyn FnMut(&str)) {
    while let Ok(line) = rx.try_recv() {
        on_stdout(&line);
        stdout.push_str(&line);
        stdout.push('\n');
    }
}

#[cfg(unix)]
fn kill_process_tree(child: &mut std::process::Child) {
    let pid = child.id();
    // 负号表示整个进程组；dsh 已通过 process_group(0) 成为组长。
    let _ = Command::new("kill")
        .arg("-KILL")
        .arg(format!("-{pid}"))
        .status();
    let _ = child.kill();
    let _ = child.wait();
}

#[cfg(not(unix))]
fn kill_process_tree(child: &mut std::process::Child) {
    let _ = child.kill();
    let _ = child.wait();
}

/// 返回 (stdout, exit_code, stderr)。超时/人工终止会杀掉子进程并返回 Err。
fn run_process(
    cmd: &mut Command,
    stdin_data: Option<&str>,
    timeout: Duration,
    cancel: Option<Arc<AtomicBool>>,
    on_stdout: &mut dyn FnMut(&str),
) -> Result<(String, i32, String), String> {
    let mut child = cmd
        .spawn()
        .map_err(|e| format!("启动 dsh 失败（可在设置中配置 dsh 绝对路径）: {e}"))?;

    if let Some(data) = stdin_data {
        if let Some(mut stdin) = child.stdin.take() {
            stdin
                .write_all(data.as_bytes())
                .map_err(|e| format!("向 dsh 写入 prompt 失败: {e}"))?;
        }
    }

    let stdout_pipe = child.stdout.take().ok_or("无法捕获 dsh stdout")?;
    let mut stderr_pipe = child.stderr.take().ok_or("无法捕获 dsh stderr")?;
    let (line_tx, line_rx) = mpsc::channel::<String>();
    let out_handle = thread::spawn(move || {
        let reader = BufReader::new(stdout_pipe);
        for line in reader.lines() {
            match line {
                Ok(line) => {
                    if line_tx.send(line).is_err() {
                        break;
                    }
                }
                Err(_) => break,
            }
        }
    });
    let err_handle = thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = stderr_pipe.read_to_end(&mut buf);
        String::from_utf8_lossy(&buf).to_string()
    });

    let start = Instant::now();
    let mut stdout = String::new();
    let exit_code = loop {
        drain_stdout(&line_rx, &mut stdout, on_stdout);
        let canceled = cancel
            .as_ref()
            .map(|c| c.load(Ordering::SeqCst))
            .unwrap_or(false);
        if canceled {
            kill_process_tree(&mut child);
            let _ = out_handle.join();
            drain_stdout(&line_rx, &mut stdout, on_stdout);
            let _ = err_handle.join();
            return Err("人工终止：已停止 dsh 执行".into());
        }
        match child.try_wait() {
            Ok(Some(status)) => break status.code().unwrap_or(-1),
            Ok(None) => {
                if start.elapsed() > timeout {
                    kill_process_tree(&mut child);
                    let _ = out_handle.join();
                    drain_stdout(&line_rx, &mut stdout, on_stdout);
                    let _ = err_handle.join();
                    return Err(format!(
                        "dsh 执行超时（{} 秒），已强制终止。请检查模型服务或调大设置中的 dsh 超时。",
                        timeout.as_secs()
                    ));
                }
                thread::sleep(Duration::from_millis(150));
            }
            Err(e) => {
                kill_process_tree(&mut child);
                return Err(format!("等待 dsh 进程失败: {e}"));
            }
        }
    };

    // 子进程结束后，等待读取线程把剩余行发完，再统一 drain。
    let _ = out_handle.join();
    drain_stdout(&line_rx, &mut stdout, on_stdout);
    let stderr = err_handle
        .join()
        .map_err(|_| "读取 dsh stderr 线程异常".to_string())?;
    Ok((stdout, exit_code, stderr))
}

// ------------------------------------------------------------------
// git 快照 / diff
// ------------------------------------------------------------------

fn run_git(work_dir: &Path, args: &[&str], index_path: Option<&Path>) -> Result<String, String> {
    let mut cmd = Command::new("git");
    cmd.args(args)
        .current_dir(work_dir)
        .env("GIT_TERMINAL_PROMPT", "0");
    if let Some(index) = index_path {
        cmd.env("GIT_INDEX_FILE", index);
    }
    let output = cmd
        .output()
        .map_err(|e| format!("执行 git 失败（请确认本机已安装 git）: {e}"))?;
    if !output.status.success() {
        return Err(format!(
            "git {} 失败: {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

pub fn ensure_git_repo(work_dir: &Path) -> Result<(), String> {
    if !work_dir.is_dir() {
        return Err(format!("工作目录不存在: {}", work_dir.display()));
    }
    if work_dir.join(".git").is_dir() {
        return Ok(());
    }
    run_git(work_dir, &["init"], None)?;
    Ok(())
}

fn temp_index_path(work_dir: &Path) -> PathBuf {
    work_dir.join(".git").join("agent-crew-temp-index")
}

/// 用临时 index 对整个工作目录做 tree 快照，不修改用户真实 index。
pub fn capture_tree(work_dir: &Path) -> Result<String, String> {
    ensure_git_repo(work_dir)?;
    let index = temp_index_path(work_dir);
    let _ = fs::remove_file(&index);
    run_git(work_dir, &["read-tree", "--empty"], Some(&index))?;
    run_git(work_dir, &["add", "-A"], Some(&index))?;
    let tree = run_git(work_dir, &["write-tree"], Some(&index))?;
    let _ = fs::remove_file(&index);
    Ok(tree.trim().to_string())
}

/// 计算两个 tree 之间的完整 diff（含统计与文本 diff）。
pub fn diff_trees(work_dir: &Path, baseline: &str, current: &str) -> Result<String, String> {
    if baseline.trim().is_empty() {
        return Err("缺少基线 git tree".into());
    }
    let stat = run_git(
        work_dir,
        &["diff", "--stat", "--no-color", baseline, current],
        None,
    )
    .unwrap_or_else(|_| "（无法生成统计）".to_string());
    let detail = run_git(
        work_dir,
        &["diff", "--no-color", "--no-ext-diff", baseline, current],
        None,
    )
    .unwrap_or_else(|_| "（无法生成 diff）".to_string());
    if detail.trim().is_empty() {
        return Ok(String::new());
    }
    Ok(format!(
        "### 变更统计\n{}\n### 完整 diff\n{}",
        stat.trim(),
        detail
    ))
}

/// 捕获当前 tree 并与基线比较；用于每轮 Coder diff。
pub fn diff_since(work_dir: &Path, baseline: &str) -> Result<String, String> {
    let current = capture_tree(work_dir)?;
    diff_trees(work_dir, baseline, &current)
}

/// 任务级累计 diff（全局评审用）。
pub fn task_cumulative_diff(work_dir: &Path, baseline: &str) -> Result<String, String> {
    diff_since(work_dir, baseline)
}

// ------------------------------------------------------------------
// summary.md
// ------------------------------------------------------------------

fn sanitize_slug(name: &str) -> String {
    let mut out = String::new();
    for c in name.chars() {
        if c.is_alphanumeric() || c == '-' || c == '_' {
            out.push(c);
        } else if c == ' ' || c == '/' || c == '\\' {
            out.push('-');
        }
        if out.chars().count() >= 48 {
            break;
        }
    }
    let trimmed = out.trim_matches('-').to_string();
    if trimmed.is_empty() {
        "module".to_string()
    } else {
        trimmed
    }
}

pub fn summary_rel_path(subtask: &Subtask) -> String {
    format!(
        "summaries/{:02}-{}.md",
        subtask.sort_index,
        sanitize_slug(&subtask.subtask_name)
    )
}

pub fn read_summary(work_dir: &Path, rel_path: &str) -> Option<String> {
    let path = work_dir.join(rel_path);
    fs::read_to_string(path).ok().map(|s| s.trim().to_string())
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use crate::models::AgentConfig;
    use std::os::unix::fs::PermissionsExt;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn cancellation_kills_running_dsh() {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root =
            std::env::temp_dir().join(format!("orch-dsh-cancel-{}-{}", std::process::id(), nanos));
        let app_data = root.join("app");
        let work = root.join("work");
        fs::create_dir_all(&app_data).unwrap();
        fs::create_dir_all(&work).unwrap();
        let script = root.join("slow-dsh");
        fs::write(&script, "#!/bin/sh\nsleep 30\n").unwrap();
        let mut perms = fs::metadata(&script).unwrap().permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&script, perms).unwrap();

        let agent = AgentConfig {
            id: 1,
            name: "coder".into(),
            role_type: "coder".into(),
            provider: "openai-compatible".into(),
            model_name: "mock".into(),
            base_url: "https://example.com/v1".into(),
            api_key: "test".into(),
            system_prompt: String::new(),
            temperature: 0.0,
            create_time: String::new(),
            role_name: String::new(),
            role_color: String::new(),
            role_capabilities: Vec::new(),
        };
        let mut settings = AppSettings::default();
        settings.dsh_bin = script.to_string_lossy().to_string();
        let cancel = Arc::new(AtomicBool::new(false));
        let cancel2 = cancel.clone();
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(400));
            cancel2.store(true, Ordering::SeqCst);
        });
        let started = Instant::now();
        let result = run_coder(
            &app_data,
            &settings,
            &agent,
            &work,
            "hello",
            60,
            Some(cancel),
            &mut |_| {},
        );
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("人工终止"));
        assert!(started.elapsed() < Duration::from_secs(10));
        let _ = fs::remove_dir_all(&root);
    }
}
