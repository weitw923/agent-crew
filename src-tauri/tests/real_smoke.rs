//! 真实环境冒烟测试（默认忽略，手动执行）。
//!
//! 与 `e2e.rs` 不同：这里使用**本机真实 Agent 配置 + 真实 LLM + 真实 dsh**，
//! 因此会消耗少量真实 token，并且依赖网络。
//!
//! 覆盖：
//!   1. 复制真实数据库到临时目录，走一次真实旧库迁移（历史任务 → dev-1.0，幂等）；
//!   2. 用真实 Reviewer / Coder 新建并跑完一个小任务（dev-1.0）；
//!   3. 追加一个简单需求，验证 dev-1.1 全链路与 token 统计；
//!   4. 校验两次迭代的 token_total 均被真实用量填充。
//!
//! 运行方式：
//! ```bash
//! cd src-tauri
//! AGENTCREW_REAL_SMOKE=1 cargo test --test real_smoke -- --ignored --nocapture
//! ```
//! 可用 `AGENTCREW_REAL_DB` 指定数据库路径（默认 macOS 应用数据目录）。
//!
//! 注意：脚本只操作数据库副本与临时工作目录，不会改动真实数据。

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use agent_crew_lib::db;
use agent_crew_lib::models::{AgentConfig, NewTaskInput};
use agent_crew_lib::orchestrator;

fn temp_dir(tag: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!(
        "agentcrew-real-smoke-{}-{}-{}",
        tag,
        std::process::id(),
        nanos
    ));
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn real_db_path() -> Option<PathBuf> {
    if let Ok(raw) = std::env::var("AGENTCREW_REAL_DB") {
        let path = PathBuf::from(raw);
        return path.is_file().then_some(path);
    }
    let home = std::env::var("HOME").ok()?;
    let path = PathBuf::from(home)
        .join("Library/Application Support/com.local.agent-orchestrator/orchestrator.db");
    path.is_file().then_some(path)
}

/// 复制数据库（连同 WAL/SHM，若存在），返回副本路径。
fn copy_database(src: &Path, dest_root: &Path) -> PathBuf {
    let dest = dest_root.join("orchestrator.db");
    fs::copy(src, &dest).expect("复制真实数据库失败");
    for suffix in ["-wal", "-shm"] {
        let side = PathBuf::from(format!("{}{suffix}", src.display()));
        if side.is_file() {
            let target = PathBuf::from(format!("{}{suffix}", dest.display()));
            let _ = fs::copy(side, target);
        }
    }
    dest
}

fn list_files(root: &Path) -> Vec<String> {
    let mut out = Vec::new();
    if let Ok(entries) = fs::read_dir(root) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                out.push(format!("{}/", path.file_name().unwrap().to_string_lossy()));
                for sub in list_files(&path) {
                    out.push(format!("{}/{sub}", path.file_name().unwrap().to_string_lossy()));
                }
            } else {
                out.push(path.file_name().unwrap().to_string_lossy().to_string());
            }
        }
    }
    out
}

fn has_readme(root: &Path) -> bool {
    list_files(root)
        .iter()
        .any(|f| f.to_lowercase().contains("readme"))
}

fn pick_agents(conn: &rusqlite::Connection) -> (AgentConfig, AgentConfig) {
    let agents = db::list_agents(conn).expect("读取真实 Agent 配置失败");
    let reviewer = agents
        .iter()
        .find(|a| a.role_type == "reviewer" && !a.api_key.trim().is_empty())
        .cloned()
        .expect("真实数据库里没有可用的 reviewer（需要 apiKey）");
    let coder = agents
        .iter()
        .find(|a| a.role_type == "coder" && !a.api_key.trim().is_empty())
        .cloned()
        .expect("真实数据库里没有可用的 coder（需要 apiKey）");
    (reviewer, coder)
}

/// 驱动任务跑到终态：若真实评审把某个模块的轮次跑完，就用与 UI 相同的
/// 「人工追加轮次」逻辑继续，最多介入 5 次，避免真实模型偶发波动导致冒烟失败。
fn drive_to_completion(db_path: &Path, app_data_dir: &Path, task_id: i64) -> usize {
    let mut interventions = 0;
    for _ in 0..6 {
        let _ = orchestrator::run_task_headless(db_path, app_data_dir, task_id);
        let conn = db::open(db_path).expect("打开数据库失败");
        let Some(task) = db::get_task(&conn, task_id).unwrap() else {
            panic!("任务 {task_id} 不存在");
        };
        match task.status.as_str() {
            "completed" => return interventions,
            "needs_human" => {
                let subtasks = db::list_subtasks(&conn, task_id).unwrap();
                let Some(sub) = subtasks.iter().find(|s| s.status == "needs_human") else {
                    panic!("任务处于 needs_human 但找不到人工介入模块");
                };
                let reason = sub
                    .iteration_records
                    .last()
                    .map(|r| r.review_feedback.clone())
                    .unwrap_or_default();
                println!(
                    "模块 #{}「{}」轮次耗尽（第 {}/{} 轮，阶段 {}），最后评审意见：{}",
                    sub.sort_index,
                    sub.subtask_name,
                    sub.current_round,
                    sub.max_round,
                    sub.current_stage,
                    prompts_truncate(&reason, 260)
                );
                orchestrator::extend_subtask_rounds_state(None, &conn, task_id, sub.id, 2)
                    .expect("人工追加轮次失败");
                interventions += 1;
                println!("已按「人工追加轮次」继续 +2 轮");
            }
            "terminated" | "failed" | "paused" => {
                let conn2 = db::open(db_path).unwrap();
                let logs = db::list_logs(&conn2, task_id).unwrap();
                let tail = logs
                    .iter()
                    .rev()
                    .take(8)
                    .map(|l| format!("[{}] {}", l.level, l.message))
                    .collect::<Vec<_>>()
                    .join("\n");
                panic!("真实任务中断（{}），最近日志：\n{tail}", task.status);
            }
            _ => {}
        }
    }
    panic!("真实任务在 6 次驱动后仍未达到终态（人工介入 {interventions} 次）");
}

fn prompts_truncate(raw: &str, limit: usize) -> String {
    let chars: Vec<char> = raw.chars().collect();
    if chars.len() <= limit {
        return raw.to_string();
    }
    format!("{}…", chars[..limit].iter().collect::<String>())
}

#[test]
#[ignore = "真实 LLM + dsh 冒烟测试：需要网络、真实 apiKey 与 dsh，手动执行"]
fn real_agents_end_to_end_smoke() {
    if std::env::var("AGENTCREW_REAL_SMOKE").ok().as_deref() != Some("1") {
        eprintln!("跳过：设置 AGENTCREW_REAL_SMOKE=1 才会执行真实冒烟测试");
        return;
    }
    let Some(real_db) = real_db_path() else {
        eprintln!("跳过：找不到真实数据库，可用 AGENTCREW_REAL_DB 指定");
        return;
    };
    println!("真实数据库：{}", real_db.display());

    let root = temp_dir("root");
    let work_dir = temp_dir("work");
    let db_path = copy_database(&real_db, &root);

    // 1) 走真实 db::init（SCHEMA + migrate），验证旧库迁移
    let initialized = db::init(&root).expect("初始化/迁移数据库副本失败");
    assert_eq!(initialized, db_path);
    {
        let conn = db::open(&db_path).unwrap();
        let tasks = db::list_tasks(&conn).unwrap();
        println!("迁移后：{} 个历史任务", tasks.len());
        assert!(!tasks.is_empty(), "真实数据库里应当有历史任务");
        for summary in &tasks {
            let task = &summary.task;
            let iterations = db::list_iterations(&conn, task.id).unwrap();
            assert!(
                !iterations.is_empty(),
                "任务「{}」(#{}）迁移后应有 dev-1.0 迭代",
                task.task_name,
                task.id
            );
            println!(
                "  - #{} {}：{} 个迭代，最新 {}，状态 {}",
                task.id,
                task.task_name,
                iterations.len(),
                summary.version,
                task.status
            );
        }
        // 幂等：再跑一次迁移不应新增迭代
        let before = tasks.len();
        db::init(&root).expect("重复 init 失败");
        let conn2 = db::open(&db_path).unwrap();
        assert_eq!(db::list_tasks(&conn2).unwrap().len(), before);
    }

    // 2) 用真实 Agent 跑一个小任务
    let conn = db::open(&db_path).unwrap();
    let (reviewer, coder) = pick_agents(&conn);
    println!(
        "使用真实 Agent：reviewer={}（{}），coder={}（{}）",
        reviewer.name, reviewer.model_name, coder.name, coder.model_name
    );
    let task_id = db::create_task(
        &conn,
        &NewTaskInput {
            task_name: "真实冒烟：加法函数".into(),
            top_requirement:
                "在项目里实现一个纯函数 add(a, b) 用于整数相加，并写一个极简的 README 说明如何使用。不要引入任何第三方依赖。".into(),
            global_accept_criteria:
                "存在可运行的源码文件实现 add(a, b)；README 说明用法；无第三方依赖。".into(),
            reviewer_agent_id: reviewer.id,
            coder_agent_ids: vec![coder.id],
            module_max_round: 4,
            global_max_round: 2,
            work_dir: work_dir.to_string_lossy().to_string(),
            manual_review_enabled: false,
            execution_mode: "later".into(),
            schedule_start: "01:00".into(),
            schedule_end: "06:00".into(),
            schedule_enabled: true,
        },
    )
    .expect("创建任务失败");
    drop(conn);

    println!("开始执行真实任务 #{} …", task_id);
    let interventions = drive_to_completion(&db_path, &root, task_id);
    println!("人工介入（追加轮次）次数：{interventions}");

    let conn = db::open(&db_path).unwrap();
    let detail = db::get_task_detail(&conn, task_id).unwrap();
    println!(
        "任务结束：status={}，模块={}，迭代={}",
        detail.task.status,
        detail.subtasks.len(),
        detail.iterations.len()
    );
    assert_eq!(detail.task.status, "completed", "真实任务应执行完成");
    assert!(!detail.subtasks.is_empty(), "应拆出至少一个模块");

    let first_iteration = detail
        .iterations
        .iter()
        .find(|i| i.version == "dev-1.0")
        .expect("首次任务应有 dev-1.0 迭代");
    println!(
        "dev-1.0：状态={}，模块 {}/{}，耗时 {}s，token {}",
        first_iteration.status,
        first_iteration.subtask_completed,
        first_iteration.subtask_total,
        first_iteration.duration_sec,
        first_iteration.token_total
    );
    assert_eq!(first_iteration.status, "completed");
    assert!(
        first_iteration.token_total > 0,
        "真实 LLM 调用应累计 token（当前 {}）",
        first_iteration.token_total
    );
    assert!(
        has_readme(&work_dir),
        "Coder 应在工作目录生成 README（目录内容：{:?}）",
        list_files(&work_dir)
    );

    // 3) 追加需求 → dev-1.1 全链路
    let mut task = db::get_task(&conn, task_id).unwrap().unwrap();
    task.followup_requirement = "把 add 函数改名为 sumNumbers，保持行为完全一致，并同步更新 README。".into();
    task.followup_mode = "single".into();
    task.task_phase = "requirement_intake".into();
    task.status = "planning".into();
    db::save_task(&conn, &task).unwrap();
    drop(conn);

    println!("追加需求，执行 dev-1.1 …");
    let followup_interventions = drive_to_completion(&db_path, &root, task_id);
    println!("追加需求阶段人工介入次数：{followup_interventions}");

    let conn = db::open(&db_path).unwrap();
    let detail = db::get_task_detail(&conn, task_id).unwrap();
    assert_eq!(detail.task.status, "completed");
    let versions: Vec<String> = detail.iterations.iter().map(|i| i.version.clone()).collect();
    println!("迭代版本：{versions:?}");
    assert!(versions.contains(&"dev-1.0".to_string()));
    assert!(
        versions.iter().any(|v| v.starts_with("dev-1.") && v != "dev-1.0"),
        "追加需求应升小版本（当前 {versions:?}）"
    );
    let followup = detail
        .iterations
        .iter()
        .find(|i| i.version != "dev-1.0")
        .expect("应有追加需求迭代");
    assert_eq!(followup.status, "completed");
    assert!(
        followup.token_total > 0,
        "追加需求迭代也应统计 token（当前 {}）",
        followup.token_total
    );
    let new_subtask = detail
        .subtasks
        .iter()
        .filter(|s| s.iteration_id == followup.id)
        .count();
    assert!(new_subtask >= 1, "追加需求应新增至少一个模块");
    println!(
        "dev-1.1：模块 {}，token {}，耗时 {}s",
        new_subtask, followup.token_total, followup.duration_sec
    );

    println!(
        "✅ 真实冒烟通过：任务 #{} 完成，{} 个迭代，工作目录 {}",
        task_id,
        detail.iterations.len(),
        work_dir.display()
    );
    // 保留临时目录便于人工检查：打印路径即可
    println!("（临时数据保留在 {}）", root.display());
}
