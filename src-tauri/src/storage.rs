//! 外部存储根目录：任务数据库、运行日志、模块快照统一放在项目目录之外。
//!
//! - 默认根目录：Tauri app_data_dir（保持与旧版本一致）
//! - 自定义根目录：写在 app_data_dir/storage.json 指针文件里
//! - 切换根目录时用 `VACUUM INTO` 生成一致性数据库快照，并复制任务快照目录，
//!   旧目录保留不删除，确保任何情况下都不丢数据。

use std::fs;
use std::path::{Path, PathBuf};

use rusqlite::params;
use serde::{Deserialize, Serialize};

use crate::models::Subtask;

#[derive(Debug, Clone)]
pub struct ResolvedStorage {
    pub root: PathBuf,
    pub db_path: PathBuf,
}

#[derive(Debug, Serialize, Deserialize)]
struct StoragePointer {
    root: String,
}

pub fn pointer_path(app_data_dir: &Path) -> PathBuf {
    app_data_dir.join("storage.json")
}

fn pointer_root(app_data_dir: &Path) -> Option<PathBuf> {
    let text = fs::read_to_string(pointer_path(app_data_dir)).ok()?;
    let pointer: StoragePointer = serde_json::from_str(&text).ok()?;
    let root = PathBuf::from(pointer.root.trim());
    if root.is_absolute() {
        Some(root)
    } else {
        None
    }
}

/// 解析当前存储根目录。指针损坏/目录不可用时回退到旧默认目录，避免应用无法启动。
pub fn resolve(app_data_dir: &Path) -> ResolvedStorage {
    let root = pointer_root(app_data_dir)
        .filter(|p| p.is_dir() || fs::create_dir_all(p).is_ok())
        .unwrap_or_else(|| app_data_dir.to_path_buf());
    let _ = fs::create_dir_all(&root);
    let db_path = root.join("orchestrator.db");
    ResolvedStorage { root, db_path }
}

pub fn write_pointer(app_data_dir: &Path, root: &Path) -> Result<(), String> {
    fs::create_dir_all(app_data_dir)
        .map_err(|e| format!("创建应用配置目录失败（{}）: {e}", app_data_dir.display()))?;
    let text = serde_json::to_string_pretty(&StoragePointer {
        root: root.to_string_lossy().to_string(),
    })
    .map_err(|e| format!("存储路径序列化失败: {e}"))?;
    let tmp = pointer_path(app_data_dir).with_extension("json.tmp");
    fs::write(&tmp, text).map_err(|e| format!("写入存储路径指针失败（{}）: {e}", tmp.display()))?;
    fs::rename(&tmp, pointer_path(app_data_dir))
        .map_err(|e| format!("保存存储路径指针失败: {e}"))?;
    Ok(())
}

pub fn task_dir(root: &Path, task_id: i64) -> PathBuf {
    root.join("tasks").join(task_id.to_string())
}

pub fn module_dir(root: &Path, task_id: i64, sub: &Subtask) -> PathBuf {
    let slug = sanitize_slug(&sub.subtask_name);
    task_dir(root, task_id)
        .join("modules")
        .join(format!("{:02}-{}", sub.sort_index, slug))
}

pub fn ensure_task_dir(root: &Path, task_id: i64) -> Result<PathBuf, String> {
    let dir = task_dir(root, task_id);
    fs::create_dir_all(&dir)
        .map_err(|e| format!("创建任务存储目录失败（{}）: {e}", dir.display()))?;
    Ok(dir)
}

pub fn save_design(
    root: &Path,
    task_id: i64,
    sub: &Subtask,
    content: &str,
) -> Result<PathBuf, String> {
    let dir = module_dir(root, task_id, sub);
    fs::create_dir_all(&dir)
        .map_err(|e| format!("创建模块快照目录失败（{}）: {e}", dir.display()))?;
    let path = dir.join("design.md");
    fs::write(&path, content)
        .map_err(|e| format!("写入设计快照失败（{}）: {e}", path.display()))?;
    Ok(path)
}

pub fn save_round_diff(
    root: &Path,
    task_id: i64,
    sub: &Subtask,
    round: i64,
    diff: &str,
) -> Result<PathBuf, String> {
    let dir = module_dir(root, task_id, sub).join("rounds");
    fs::create_dir_all(&dir)
        .map_err(|e| format!("创建轮次快照目录失败（{}）: {e}", dir.display()))?;
    let path = dir.join(format!("round-{round:02}.diff"));
    fs::write(&path, diff).map_err(|e| format!("写入 diff 快照失败（{}）: {e}", path.display()))?;
    Ok(path)
}

pub fn save_review_snapshot(
    root: &Path,
    task_id: i64,
    sub: &Subtask,
    round: i64,
    kind: &str,
    content: &str,
) -> Result<PathBuf, String> {
    let dir = module_dir(root, task_id, sub).join("rounds");
    fs::create_dir_all(&dir)
        .map_err(|e| format!("创建轮次快照目录失败（{}）: {e}", dir.display()))?;
    let path = dir.join(format!("round-{round:02}-{kind}.json"));
    fs::write(&path, content)
        .map_err(|e| format!("写入评审快照失败（{}）: {e}", path.display()))?;
    Ok(path)
}

/// 摘要最终快照只能写入外部存储根目录，项目中生成的中转文件会被移除。
pub fn persist_summary(
    root: &Path,
    task_id: i64,
    sub: &Subtask,
    content: &str,
) -> Result<PathBuf, String> {
    let dir = module_dir(root, task_id, sub);
    fs::create_dir_all(&dir)
        .map_err(|e| format!("创建模块快照目录失败（{}）: {e}", dir.display()))?;
    let path = dir.join("summary.md");
    fs::write(&path, content)
        .map_err(|e| format!("写入摘要快照失败（{}）: {e}", path.display()))?;
    Ok(path)
}

pub fn remove_project_summary(work_dir: &Path, rel_path: &str) {
    let path = work_dir.join(rel_path);
    let _ = fs::remove_file(&path);
    // 只在 summaries 目录为空时移除它，避免误删用户自己的文件。
    if let Some(parent) = path.parent() {
        if parent.file_name().and_then(|s| s.to_str()) == Some("summaries") {
            let _ = fs::remove_dir(parent);
        }
    }
}

pub fn sanitize_slug(name: &str) -> String {
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

fn copy_file_if_absent(src: &Path, dst: &Path) -> Result<(), String> {
    if dst.exists() {
        return Ok(());
    }
    if let Some(parent) = dst.parent() {
        fs::create_dir_all(parent)
            .map_err(|e| format!("创建目录失败（{}）: {e}", parent.display()))?;
    }
    fs::copy(src, dst).map(|_| ()).map_err(|e| {
        format!(
            "复制文件失败（{} -> {}）: {e}",
            src.display(),
            dst.display()
        )
    })
}

fn copy_dir_merge(src: &Path, dst: &Path) -> Result<(), String> {
    if !src.is_dir() {
        return Ok(());
    }
    fs::create_dir_all(dst).map_err(|e| format!("创建目录失败（{}）: {e}", dst.display()))?;
    for entry in fs::read_dir(src).map_err(|e| format!("读取目录失败（{}）: {e}", src.display()))?
    {
        let entry = entry.map_err(|e| e.to_string())?;
        let path = entry.path();
        let target = dst.join(entry.file_name());
        if path.is_dir() {
            copy_dir_merge(&path, &target)?;
        } else if path.is_file() {
            copy_file_if_absent(&path, &target)?;
        }
    }
    Ok(())
}

/// 迁移存储根目录：生成数据库一致性快照 + 复制任务快照/备份；旧目录保留。
pub fn migrate_root(current: &Path, target: &Path, current_db: &Path) -> Result<(), String> {
    if !target.is_absolute() {
        return Err("存储路径必须是绝对路径".into());
    }
    let current_raw = current.to_path_buf();
    let current = current
        .canonicalize()
        .unwrap_or_else(|_| current.to_path_buf());
    let target_abs = if target.exists() {
        target
            .canonicalize()
            .unwrap_or_else(|_| target.to_path_buf())
    } else {
        target.to_path_buf()
    };
    if current == target_abs {
        return Ok(());
    }
    if target_abs.starts_with(&current) || current.starts_with(&target_abs) {
        return Err("新存储目录不能位于当前存储目录内部（或反之）".into());
    }
    fs::create_dir_all(&target_abs)
        .map_err(|e| format!("创建目标存储目录失败（{}）: {e}", target_abs.display()))?;
    let target_db = target_abs.join("orchestrator.db");
    if target_db.exists() {
        return Err(format!(
            "目标目录已存在数据库文件，请换一个空目录：{}",
            target_db.display()
        ));
    }

    let tmp_db = target_abs.join("orchestrator.db.migrating");
    let _ = fs::remove_file(&tmp_db);
    {
        let conn = crate::db::open(current_db)?;
        conn.execute(
            "VACUUM INTO ?1",
            params![tmp_db.to_string_lossy().to_string()],
        )
        .map_err(|e| format!("生成数据库迁移快照失败: {e}"))?;
    }
    fs::rename(&tmp_db, &target_db).map_err(|e| format!("落盘迁移数据库失败: {e}"))?;

    // 摘要快照路径存的是绝对路径，迁移后同步改写，避免 UI 指向旧目录。
    {
        let conn = crate::db::open(&target_db)?;
        let new_prefix = target_abs.to_string_lossy().to_string();
        let mut prefixes = vec![
            current_raw.to_string_lossy().to_string(),
            current.to_string_lossy().to_string(),
        ];
        prefixes.dedup();
        for old_prefix in prefixes {
            // 用 REPLACE 保留 tasks/<id>/... 的相对部分；同时兼容 /tmp -> /private/tmp 等软链接。
            conn.execute(
                "UPDATE subtask SET summary_file_path=REPLACE(summary_file_path, ?1, ?2)
                 WHERE summary_file_path LIKE ?3",
                params![old_prefix, new_prefix, format!("{old_prefix}%")],
            )
            .map_err(|e| format!("迁移摘要快照路径失败: {e}"))?;
        }
    }

    copy_dir_merge(&current.join("tasks"), &target_abs.join("tasks"))?;
    copy_dir_merge(&current.join("runtime"), &target_abs.join("runtime"))?;

    // 把自动备份也复制过去，方便在设置页看到同一份历史。
    if let Ok(entries) = fs::read_dir(&current) {
        for entry in entries.flatten() {
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if name.starts_with("orchestrator.db.backup-") && entry.path().is_file() {
                copy_file_if_absent(&entry.path(), &target_abs.join(name.as_ref()))?;
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn slug_and_module_path_are_stable() {
        let root = Path::new("/tmp/agentcrew-storage");
        let sub = Subtask {
            id: 1,
            task_id: 2,
            subtask_name: "登录 / 鉴权".into(),
            sort_index: 3,
            accept_criteria: String::new(),
            assigned_coder_id: None,
            current_round: 0,
            max_round: 3,
            status: "waiting".into(),
            current_stage: "queued".into(),
            summary_content: String::new(),
            summary_file_path: String::new(),
            iteration_records: Vec::new(),
            design_content: String::new(),
        };
        let dir = module_dir(root, 2, &sub);
        assert!(dir.ends_with("tasks/2/modules/03-登录---鉴权"));
    }

    #[test]
    fn migrates_database_snapshot_between_roots() {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let base = std::env::temp_dir().join(format!(
            "agentcrew-migrate-{}-{}",
            std::process::id(),
            nanos
        ));
        let old = base.join("old");
        let new = base.join("new");
        fs::create_dir_all(&old).unwrap();
        let db = old.join("orchestrator.db");
        let old_summary = old.join("tasks/1/modules/01-demo/summary.md");
        {
            let conn = crate::db::open(&db).unwrap();
            conn.execute_batch(crate::db::SCHEMA).unwrap();
            conn.execute("INSERT INTO app_setting(key,value) VALUES ('x','1')", [])
                .unwrap();
            conn.execute(
                "INSERT INTO task_main(task_name,top_requirement,global_accept_criteria,
                    reviewer_agent_id,coder_agent_ids,module_max_round,global_max_round,work_dir,create_time)
                 VALUES ('t','r','a',1,'[1]',1,1,'/tmp/project','2026-01-01 00:00:00')",
                [],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO subtask(task_id,subtask_name,sort_index,max_round,status,current_stage,
                    summary_content,summary_file_path,iteration_records)
                 VALUES (1,'demo',1,1,'completed','done','x',?1,'[]')",
                params![old_summary.to_string_lossy().to_string()],
            )
            .unwrap();
        }
        fs::create_dir_all(old_summary.parent().unwrap()).unwrap();
        fs::write(&old_summary, "summary").unwrap();
        migrate_root(&old, &new, &db).unwrap();
        let conn = crate::db::open(&new.join("orchestrator.db")).unwrap();
        let v: String = conn
            .query_row("SELECT value FROM app_setting WHERE key='x'", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(v, "1");
        let path: String = conn
            .query_row(
                "SELECT summary_file_path FROM subtask WHERE id=1",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert!(path.starts_with(new.join("tasks").to_string_lossy().as_ref()));
        assert!(Path::new(&path).exists());
        let _ = fs::remove_dir_all(&base);
    }
}
