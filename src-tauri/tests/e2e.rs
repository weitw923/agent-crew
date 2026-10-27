//! 端到端集成测试。
//!
//! 不依赖真实模型/真实 dsh：
//!   - 本地起一个 OpenAI 兼容 mock HTTP 服务，模拟 Planner / Reviewer / 全局评审；
//!   - 本地生成一个可执行的 mock `dsh`，模拟 Coder 写代码 + 写 summary.md；
//!   - 直接调用 orchestrator 的无 UI 入口，跑完整串行状态机。
//!
//! 覆盖：拆解、串行派发、代码驳回后同 Coder 重做、摘要生成/校验/落盘、
//! 全局评审驳回→模块轮次重置→返工→全局评审通过。

use std::fs;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::{SystemTime, UNIX_EPOCH};

use agent_crew_lib::db;
use agent_crew_lib::models::{AgentInput, AppSettings, NewTaskInput};
use agent_crew_lib::orchestrator;

fn temp_dir(tag: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!(
        "agent-crew-e2e-{}-{}-{}",
        tag,
        std::process::id(),
        nanos
    ));
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn http_body(req: &str) -> &str {
    req.split_once("\r\n\r\n").map(|(_, b)| b).unwrap_or("")
}

/// 极简 HTTP 读取：处理 Content-Length。
fn read_request(stream: &mut std::net::TcpStream) -> String {
    let mut buf = Vec::new();
    let mut tmp = [0u8; 4096];
    loop {
        let n = stream.read(&mut tmp).unwrap_or(0);
        if n == 0 {
            break;
        }
        buf.extend_from_slice(&tmp[..n]);
        if let Some(pos) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
            let head = String::from_utf8_lossy(&buf[..pos]).to_string();
            let content_len = head
                .lines()
                .find_map(|l| {
                    let lower = l.to_ascii_lowercase();
                    if lower.starts_with("content-length:") {
                        l.split(':').nth(1)?.trim().parse::<usize>().ok()
                    } else {
                        None
                    }
                })
                .unwrap_or(0);
            if buf.len() >= pos + 4 + content_len {
                break;
            }
        }
    }
    String::from_utf8_lossy(&buf).to_string()
}

fn start_mock_llm() -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let code_reviews = Arc::new(AtomicUsize::new(0));
    let global_reviews = Arc::new(AtomicUsize::new(0));

    thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { continue };
            let req = read_request(&mut stream);
            let body = http_body(&req);
            let content = build_mock_response(body, &code_reviews, &global_reviews);
            let payload = serde_json::json!({
                "id": "mock",
                "object": "chat.completion",
                "choices": [{
                    "index": 0,
                    "message": { "role": "assistant", "content": content },
                    "finish_reason": "stop"
                }]
            })
            .to_string();
            let resp = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                payload.len(),
                payload
            );
            let _ = stream.write_all(resp.as_bytes());
        }
    });
    port
}

fn build_mock_response(
    body: &str,
    code_reviews: &Arc<AtomicUsize>,
    global_reviews: &Arc<AtomicUsize>,
) -> String {
    // 临时问答：普通文本响应，不触发流水线。
    if body.contains("临时问答会话") || body.contains("用户最新问题") {
        return "临时问答回复：建议继续保持 JWT + 统一返回体，不要启动新的开发任务。".to_string();
    }
    // 已完成任务追加需求：增量梳理。
    if body.contains("增量需求评审者") {
        return serde_json::json!({
            "decision": "simple",
            "analysis": "本次只是增加一个导出入口，可在现有页面内完成，不需要拆分多个模块。",
            "subtasks": [
                { "name": "计划导出模块", "accept_criteria": "在现有日历页提供导出入口，按当前计划内容生成 Markdown 文件，并复用统一返回体。" }
            ]
        })
        .to_string();
    }
    // Planner：system prompt 中的关键词。
    if body.contains("软件架构评审者") || body.contains("请输出拆解 JSON") {
        return serde_json::json!({
            "tech_stack": "前端 Vue3 + TypeScript；后端 Rust；数据库 SQLite",
            "global_contract": "## 数据库前缀\napp_\n\n## 统一返回体\n{code,msg,data}\n\n## 异常规范\n统一错误码\n\n## 鉴权规则\nBearer Token\n\n## 接口前缀\n/api/v1\n\n## 公共枚举 / DTO 约定\n统一 DTO，分页参数 page/page_size",
            "subtasks": [
                { "name": "基础数据模块", "accept_criteria": "提供基础数据表与查询接口，返回统一返回体。" },
                { "name": "业务接口模块", "accept_criteria": "在基础数据模块之上提供业务接口，复用全局契约。" }
            ]
        })
        .to_string();
    }
    // 全局评审
    if body.contains("负责最终收口") || body.contains("全局评审者") {
        let n = global_reviews.fetch_add(1, Ordering::SeqCst);
        if n == 0 {
            return serde_json::json!({
                "passed": false,
                "feedback": "跨模块返回体存在不一致，需要返工。",
                "issues": [
                    { "subtask_id": 1, "subtask_name": "基础数据模块", "feedback": "统一返回体，补齐分页参数说明。" }
                ]
            })
            .to_string();
        }
        return serde_json::json!({
            "passed": true,
            "feedback": "跨模块接口、命名、安全规范一致，通过收口。",
            "issues": []
        })
        .to_string();
    }
    // 模块详细设计
    if body.contains("技术总监（Reviewer）") || body.contains("模块详细设计") {
        return serde_json::json!({
            "design_doc": "# 模块设计：Mock 模块\n1. 模块职责与边界：演示模块。\n2. 技术栈与关键选型：Vue3 + Rust + SQLite。\n3. 接口设计：GET /api/v1/demo。\n4. 数据模型：app_demo。\n5. 关键流程：查询返回。\n6. 依赖：无。\n7. 验收标准映射：满足验收。"
        })
        .to_string();
    }
    // 摘要校验
    if body.contains("模块摘要评审者") {
        return serde_json::json!({
            "passed": true,
            "feedback": "摘要模板完整，与 diff 一致。"
        })
        .to_string();
    }
    // 代码评审：第一次驳回，用于验证同一 Coder 的第 2 轮重做。
    if body.contains("代码 diff 评审") || body.contains("本轮 Coder 提交的 diff") {
        let n = code_reviews.fetch_add(1, Ordering::SeqCst);
        if n == 0 {
            return serde_json::json!({
                "passed": false,
                "feedback": "缺少边界处理，请补充异常分支后重新提交。"
            })
            .to_string();
        }
        return serde_json::json!({
            "passed": true,
            "feedback": "实现满足验收标准。"
        })
        .to_string();
    }
    serde_json::json!({ "passed": true, "feedback": "ok" }).to_string()
}

fn write_mock_dsh(dir: &Path) -> PathBuf {
    let path = dir.join("mock-dsh");
    let script = r##"#!/usr/bin/env python3
import sys, os, re, json

prompt = sys.stdin.read()
cwd = os.getcwd()

if "固定模板" in prompt:
    m = re.search(r"summaries/\d+-[^\n]+?\.md", prompt)
    rel = m.group(0) if m else "summary.md"
    parent = os.path.dirname(rel)
    if parent:
        os.makedirs(parent, exist_ok=True)
    content = """# 模块名称
1. 对外接口列表：GET /api/v1/demo，请求方式 GET，入参无，返回字段 data。
2. 关联数据库表：app_demo
3. 核心业务逻辑简述：读取演示数据并按统一返回体返回。
4. 对外依赖（依赖哪些其他模块能力）：无。
5. 本模块对外提供的能力（哪些其他模块会调用我）：提供演示数据查询能力。
6. 已知限制、边界、未实现能力：暂无鉴权与分页。
"""
    with open(rel, "w", encoding="utf-8") as f:
        f.write(content)
else:
    m = re.search(r"模块名称：(.+)", prompt)
    name = m.group(1).strip() if m else "module"
    safe = re.sub(r"[^0-9A-Za-z_\-\u4e00-\u9fa5]", "_", name)
    os.makedirs("src", exist_ok=True)
    path = os.path.join("src", safe + ".txt")
    old = ""
    if os.path.exists(path):
        with open(path, "r", encoding="utf-8") as f:
            old = f.read()
    with open(path, "a", encoding="utf-8") as f:
        f.write("\n// round update %d\n" % (len(old) + 1))

print(json.dumps({"type": "final", "text": "DONE"}))
"##;
    fs::write(&path, script).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = fs::metadata(&path).unwrap().permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&path, perms).unwrap();
    }
    path
}

#[test]
fn full_serial_orchestration_flow() {
    let root = temp_dir("full");
    let app_data = root.join("app-data");
    let work_dir = root.join("project");
    fs::create_dir_all(&work_dir).unwrap();
    let db_path = db::init(&app_data).unwrap();
    let mock_dsh = write_mock_dsh(&root);
    let port = start_mock_llm();

    let conn = db::open(&db_path).unwrap();
    let reviewer = db::save_agent(
        &conn,
        &AgentInput {
            id: None,
            name: "Mock Reviewer".into(),
            role_type: "reviewer".into(),
            provider: "openai-compatible".into(),
            model_name: "mock-model".into(),
            base_url: format!("http://127.0.0.1:{port}/v1"),
            api_key: "test-key".into(),
            system_prompt: "你是严格评审者".into(),
            temperature: 0.0,
        },
    )
    .unwrap();
    let coder = db::save_agent(
        &conn,
        &AgentInput {
            id: None,
            name: "Mock Coder".into(),
            role_type: "coder".into(),
            provider: "openai-compatible".into(),
            model_name: "mock-model".into(),
            base_url: "http://127.0.0.1:1/v1".into(),
            api_key: "test-key".into(),
            system_prompt: "你是编码者".into(),
            temperature: 0.2,
        },
    )
    .unwrap();

    let mut settings = AppSettings::default();
    settings.dsh_bin = mock_dsh.to_string_lossy().to_string();
    settings.dsh_timeout_sec = "60".into();
    settings.llm_timeout_sec = "15".into();
    db::save_settings(&conn, &settings).unwrap();

    let task_id = db::create_task(
        &conn,
        &NewTaskInput {
            task_name: "E2E 演示任务".into(),
            top_requirement: "开发一个带基础数据和业务接口的演示系统".into(),
            global_accept_criteria: "接口统一、可运行、摘要齐全".into(),
            reviewer_agent_id: reviewer.id,
            coder_agent_ids: vec![coder.id],
            module_max_round: 3,
            global_max_round: 3,
            work_dir: work_dir.to_string_lossy().to_string(),
            manual_review_enabled: false,
            execution_mode: "immediate".into(),
            schedule_start: "01:00".into(),
            schedule_end: "06:00".into(),
            schedule_enabled: false,
        },
    )
    .unwrap();
    drop(conn);

    orchestrator::run_task_headless(&db_path, &app_data, task_id)
        .expect("headless orchestrator should complete");

    let conn = db::open(&db_path).unwrap();
    let detail = db::get_task_detail(&conn, task_id).unwrap();
    assert_eq!(detail.task.status, "completed", "task should be completed");
    assert_eq!(detail.subtasks.len(), 2);
    assert!(detail.subtasks.iter().all(|s| s.status == "completed"));
    assert!(
        detail.task.global_review_records.len() >= 2,
        "global review should have a reject then pass"
    );
    assert!(!detail.task.global_review_records[0].passed);
    assert!(detail.task.global_review_records.last().unwrap().passed);
    assert!(!detail.task.global_review_records[0].issues.is_empty());

    // 第一轮代码评审被 mock 驳回 -> 第 2 轮通过；之后全局返工又把轮次重置为 1。
    let first = detail.subtasks.iter().find(|s| s.sort_index == 1).unwrap();
    assert_eq!(first.current_round, 1, "global rework resets module round");
    assert!(first
        .iteration_records
        .iter()
        .any(|r| r.record_type == "global_rework"));
    assert!(first
        .iteration_records
        .iter()
        .any(|r| r.record_type == "code_review" && r.review_decision == "reject"));

    assert!(
        detail
            .execution_events
            .iter()
            .any(|e| e.kind == "module_design"),
        "module design should be recorded in execution events"
    );
    assert!(
        detail
            .execution_events
            .iter()
            .any(|e| e.kind == "agent_message:coder" || e.kind == "tool_call"),
        "structured coder dialogue/tool events should be streamed"
    );
    assert!(
        detail
            .execution_events
            .iter()
            .any(|e| e.kind == "agent_message:reviewer"),
        "reviewer dialogue should be recorded"
    );

    // 摘要落盘 & 设计文档 & DB 归档
    for sub in &detail.subtasks {
        assert!(
            !sub.design_content.is_empty(),
            "module design should be generated"
        );
        assert!(!sub.summary_content.is_empty());
        assert!(sub.summary_content.contains("# 模块名称"));
        assert!(!sub.summary_file_path.is_empty());
        assert!(Path::new(&sub.summary_file_path).exists());
        assert!(
            sub.summary_file_path.contains("tasks"),
            "summary snapshot should live under external task storage"
        );
        assert!(!sub.iteration_records.is_empty());
    }
    // 工作目录中确实生成了代码文件，说明 mock dsh 被真实调用过。
    assert!(work_dir.join("src").is_dir());
    // 摘要最终快照已移到项目外，项目目录不再残留 summaries。
    assert!(!work_dir.join("summaries").is_dir());

    drop(detail);
    drop(conn);

    // 任务完成后追加一个简单需求：评审应只创建一个子任务，不重新拆解已有模块。
    let conn = db::open(&db_path).unwrap();
    let mut task = db::get_task(&conn, task_id).unwrap().unwrap();
    task.followup_requirement = "给日历页增加导出当日计划为 Markdown 的按钮".into();
    task.followup_mode = "auto".into();
    task.task_phase = "requirement_intake".into();
    task.status = "planning".into();
    db::save_task(&conn, &task).unwrap();
    drop(conn);

    orchestrator::run_task_headless(&db_path, &app_data, task_id)
        .expect("follow-up requirement should complete");

    let conn = db::open(&db_path).unwrap();
    let detail = db::get_task_detail(&conn, task_id).unwrap();
    assert_eq!(detail.task.status, "completed");
    assert_eq!(
        detail.task.global_review_cycle, 2,
        "follow-up starts a new review cycle"
    );
    assert_eq!(
        detail.subtasks.len(),
        3,
        "simple follow-up must add exactly one module"
    );
    assert!(
        detail.task.top_requirement.contains("追加需求"),
        "top requirement should keep follow-up history"
    );
    assert!(detail.task.followup_requirement.is_empty());
    let followup = detail
        .subtasks
        .iter()
        .find(|s| s.sort_index == 3)
        .expect("new follow-up module");
    assert_eq!(followup.subtask_name, "计划导出模块");
    assert_eq!(followup.status, "completed");
    assert_eq!(followup.current_round, 1);
    assert!(!followup.summary_content.is_empty());
    assert!(
        detail
            .execution_events
            .iter()
            .any(|e| e.kind == "requirement_accepted"),
        "follow-up intake should be recorded"
    );
    assert_eq!(
        detail.task.global_review_records.last().unwrap().cycle,
        2,
        "new global review record should belong to cycle 2"
    );
}

/// 人工审核模式：方案审批 + 逐模块设计审批 + 人工消息注入。
#[test]
fn manual_review_plan_and_design_gates() {
    use agent_crew_lib::models::PendingPlan;

    let root = temp_dir("manual");
    let app_data = root.join("app-data");
    let work_dir = root.join("project");
    fs::create_dir_all(&work_dir).unwrap();
    let db_path = db::init(&app_data).unwrap();
    let mock_dsh = write_mock_dsh(&root);
    let port = start_mock_llm();

    let conn = db::open(&db_path).unwrap();
    let reviewer = db::save_agent(
        &conn,
        &AgentInput {
            id: None,
            name: "Mock Reviewer".into(),
            role_type: "reviewer".into(),
            provider: "openai-compatible".into(),
            model_name: "mock-model".into(),
            base_url: format!("http://127.0.0.1:{port}/v1"),
            api_key: "test-key".into(),
            system_prompt: "严格评审".into(),
            temperature: 0.0,
        },
    )
    .unwrap();
    let coder = db::save_agent(
        &conn,
        &AgentInput {
            id: None,
            name: "Mock Coder".into(),
            role_type: "coder".into(),
            provider: "openai-compatible".into(),
            model_name: "mock-model".into(),
            base_url: "http://127.0.0.1:1/v1".into(),
            api_key: "test-key".into(),
            system_prompt: "编码".into(),
            temperature: 0.2,
        },
    )
    .unwrap();
    let mut settings = AppSettings::default();
    settings.dsh_bin = mock_dsh.to_string_lossy().to_string();
    settings.dsh_timeout_sec = "60".into();
    settings.llm_timeout_sec = "15".into();
    db::save_settings(&conn, &settings).unwrap();

    let task_id = db::create_task(
        &conn,
        &NewTaskInput {
            task_name: "人工审核 E2E".into(),
            top_requirement: "开发演示系统".into(),
            global_accept_criteria: "接口统一".into(),
            reviewer_agent_id: reviewer.id,
            coder_agent_ids: vec![coder.id],
            module_max_round: 3,
            global_max_round: 3,
            work_dir: work_dir.to_string_lossy().to_string(),
            manual_review_enabled: true,
            execution_mode: "immediate".into(),
            schedule_start: "01:00".into(),
            schedule_end: "06:00".into(),
            schedule_enabled: false,
        },
    )
    .unwrap();
    // 预置一条人工消息，验证会注入并消费。
    db::add_human_message(&conn, task_id, "请优先保证接口返回体统一").unwrap();
    drop(conn);

    // 第一次：拆解完成即停在方案审批，不创建 subtask。
    orchestrator::run_task_headless(&db_path, &app_data, task_id).unwrap();
    let conn = db::open(&db_path).unwrap();
    let detail = db::get_task_detail(&conn, task_id).unwrap();
    assert_eq!(detail.task.task_phase, "plan_approval");
    assert_eq!(detail.task.status, "needs_human");
    assert!(detail.subtasks.is_empty());
    assert!(detail.pending_plan.is_some());
    assert!(detail.human_messages.iter().all(|m| m.status == "consumed"));

    // 模拟人工“同意方案”。
    let mut task = db::get_task(&conn, task_id).unwrap().unwrap();
    let plan: PendingPlan =
        serde_json::from_str(&task.pending_plan_json).expect("pending plan json");
    let mut conn_mut = db::open(&db_path).unwrap();
    db::apply_pending_plan(&mut conn_mut, &mut task, &plan).unwrap();
    drop(conn_mut);
    drop(conn);

    // 反复运行，遇到“设计待审批”就模拟人工同意，直到任务完成。
    let mut design_approvals = 0;
    for _ in 0..12 {
        orchestrator::run_task_headless(&db_path, &app_data, task_id).unwrap();
        let conn = db::open(&db_path).unwrap();
        let detail = db::get_task_detail(&conn, task_id).unwrap();
        if detail.task.status == "completed" {
            assert!(
                detail.subtasks.iter().all(|s| !s.design_content.is_empty()),
                "every module needs a design doc"
            );
            assert!(
                design_approvals >= 2,
                "two modules should require design approval"
            );
            return;
        }
        assert_eq!(detail.task.status, "needs_human");
        let awaiting: Vec<_> = detail
            .subtasks
            .iter()
            .filter(|s| s.current_stage == "design_approval")
            .cloned()
            .collect();
        assert!(
            !awaiting.is_empty(),
            "needs_human should be a design gate, got phase={} subtasks={}",
            detail.task.task_phase,
            detail.subtasks.len()
        );
        for sub in awaiting {
            let mut sub = sub;
            sub.current_stage = "coding".into();
            sub.status = "in_progress".into();
            db::save_subtask(&conn, &sub).unwrap();
            design_approvals += 1;
        }
        let mut task = detail.task.clone();
        task.status = "running".into();
        db::save_task(&conn, &task).unwrap();
        drop(conn);
    }
    panic!("manual review flow did not complete in time");
}

/// 定时窗口外启动：在派发新模块前自动暂停为 waiting_window。
#[test]
fn scheduled_task_pauses_outside_window() {
    use chrono::{Duration, Local};

    let root = temp_dir("schedule");
    let app_data = root.join("app-data");
    let work_dir = root.join("project");
    fs::create_dir_all(&work_dir).unwrap();
    let db_path = db::init(&app_data).unwrap();
    let conn = db::open(&db_path).unwrap();

    let start = (Local::now() + Duration::hours(2))
        .format("%H:%M")
        .to_string();
    let end = (Local::now() + Duration::hours(3))
        .format("%H:%M")
        .to_string();
    let task_id = db::create_task(
        &conn,
        &NewTaskInput {
            task_name: "定时窗口外".into(),
            top_requirement: "需求".into(),
            global_accept_criteria: "验收".into(),
            reviewer_agent_id: 1,
            coder_agent_ids: vec![1],
            module_max_round: 3,
            global_max_round: 2,
            work_dir: work_dir.to_string_lossy().to_string(),
            manual_review_enabled: false,
            execution_mode: "scheduled".into(),
            schedule_start: start,
            schedule_end: end,
            schedule_enabled: true,
        },
    )
    .unwrap();
    drop(conn);

    orchestrator::run_task_headless(&db_path, &app_data, task_id).unwrap();
    let conn = db::open(&db_path).unwrap();
    let task = db::get_task(&conn, task_id).unwrap().unwrap();
    assert_eq!(task.status, "paused");
    assert_eq!(task.pause_reason, "waiting_window");
    let n: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM subtask WHERE task_id=?1",
            rusqlite::params![task_id],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(n, 0, "outside window must not dispatch a new module");
}

/// 临时问答必须直接回复，不创建子任务，也不改变任务状态。
#[test]
fn chat_channel_does_not_trigger_pipeline() {
    let root = temp_dir("chat");
    let app_data = root.join("app-data");
    let work_dir = root.join("project");
    fs::create_dir_all(&work_dir).unwrap();
    let db_path = db::init(&app_data).unwrap();
    let port = start_mock_llm();
    let conn = db::open(&db_path).unwrap();

    let reviewer = db::save_agent(
        &conn,
        &AgentInput {
            id: None,
            name: "Mock Reviewer".into(),
            role_type: "reviewer".into(),
            provider: "openai-compatible".into(),
            model_name: "mock-model".into(),
            base_url: format!("http://127.0.0.1:{port}/v1"),
            api_key: "test-key".into(),
            system_prompt: "你是技术总监".into(),
            temperature: 0.0,
        },
    )
    .unwrap();
    let mut settings = AppSettings::default();
    settings.llm_timeout_sec = "15".into();
    db::save_settings(&conn, &settings).unwrap();

    let task_id = db::create_task(
        &conn,
        &NewTaskInput {
            task_name: "问答隔离测试".into(),
            top_requirement: "只做问答，不开发".into(),
            global_accept_criteria: "不创建子任务".into(),
            reviewer_agent_id: reviewer.id,
            coder_agent_ids: vec![reviewer.id],
            module_max_round: 3,
            global_max_round: 2,
            work_dir: work_dir.to_string_lossy().to_string(),
            manual_review_enabled: false,
            execution_mode: "later".into(),
            schedule_start: "01:00".into(),
            schedule_end: "06:00".into(),
            schedule_enabled: false,
        },
    )
    .unwrap();

    let reply =
        orchestrator::chat_with_connection(&conn, task_id, "这个模块为什么选 JWT？", &settings)
            .unwrap();
    assert_eq!(reply.role, "assistant");
    assert!(reply.content.contains("临时问答回复"));

    let detail = db::get_task_detail(&conn, task_id).unwrap();
    assert_eq!(detail.task.status, "pending");
    assert!(
        detail.subtasks.is_empty(),
        "chat must not dispatch subtasks"
    );
    assert_eq!(detail.chat_messages.len(), 2);
    assert_eq!(detail.chat_messages[0].role, "user");
    assert_eq!(detail.chat_messages[1].role, "assistant");
}
