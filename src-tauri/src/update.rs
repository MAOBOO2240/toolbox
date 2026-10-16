//! 自動更新：用使用者自己的 GitHub 權杖讀取私人儲存庫的 v1.0.0 發行版本，
//! 版本以 commit 判斷（版本號固定 1.0.0），有新版就下載 MAOBOO.zip、替換執行檔並重新開啟。

use serde::{Deserialize, Serialize};
use std::io::Read;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};
use std::time::Duration;
use tauri::AppHandle;

const REPO: &str = "MAOBOO2240/toolbox";

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(default)]
pub struct UpdCfg {
    pub token: String,
    /// 發現新版本時自動更新（倒數後執行）
    pub auto: bool,
}

impl Default for UpdCfg {
    fn default() -> Self {
        Self { token: String::new(), auto: true }
    }
}

struct Inner {
    cfg: UpdCfg,
    path: PathBuf,
}
static INNER: OnceLock<Mutex<Inner>> = OnceLock::new();

fn inner() -> std::sync::MutexGuard<'static, Inner> {
    INNER.get().expect("update 尚未初始化").lock().unwrap_or_else(|e| e.into_inner())
}

/// 這個執行檔是用哪個 commit 編譯的（GitHub Actions 編譯時才有）
pub fn current() -> &'static str {
    option_env!("GITHUB_SHA").unwrap_or("")
}

pub fn init(path: PathBuf) {
    let cfg = std::fs::read_to_string(&path).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default();
    let _ = INNER.set(Mutex::new(Inner { cfg, path }));
    cleanup();
}

/// 更新後留下的舊執行檔，下次開啟時刪掉
fn cleanup() {
    if let Ok(exe) = std::env::current_exe() {
        let _ = std::fs::remove_file(old_path(&exe));
        if let Some(dir) = exe.parent() {
            let _ = std::fs::remove_file(dir.join("MAOBOO.update.exe"));
        }
    }
}

fn old_path(exe: &std::path::Path) -> PathBuf {
    let stem = exe.file_stem().and_then(|s| s.to_str()).unwrap_or("MAOBOO");
    exe.with_file_name(format!("{stem}.old.exe"))
}

#[derive(Serialize)]
pub struct UpdInfo {
    pub current: String,
    pub run: String,
    pub token_set: bool,
    pub auto: bool,
}

#[tauri::command]
pub fn update_info() -> UpdInfo {
    let g = inner();
    UpdInfo {
        current: current().chars().take(7).collect(),
        run: option_env!("GITHUB_RUN_NUMBER").unwrap_or("").to_string(),
        token_set: !g.cfg.token.trim().is_empty(),
        auto: g.cfg.auto,
    }
}

#[tauri::command]
pub fn update_set(token: Option<String>, auto: Option<bool>) {
    let mut g = inner();
    if let Some(t) = token {
        g.cfg.token = t.trim().to_string();
    }
    if let Some(a) = auto {
        g.cfg.auto = a;
    }
    if let Ok(s) = serde_json::to_string_pretty(&g.cfg) {
        let _ = std::fs::write(&g.path, s);
    }
}

fn token() -> Result<String, String> {
    let t = inner().cfg.token.clone();
    if t.trim().is_empty() {
        Err("尚未設定 GitHub 權杖".into())
    } else {
        Ok(t)
    }
}

fn github(url: &str, token: &str, accept: &str) -> Result<ureq::Response, String> {
    ureq::get(url)
        .set("Authorization", &format!("Bearer {token}"))
        .set("Accept", accept)
        .set("User-Agent", "MAOBOO-updater")
        .set("X-GitHub-Api-Version", "2022-11-28")
        .timeout(Duration::from_secs(60))
        .call()
        .map_err(|e| match e {
            ureq::Error::Status(401, _) => "權杖無效或已過期".to_string(),
            ureq::Error::Status(403, _) => "權杖沒有權限，或 GitHub 暫時限制查詢次數".to_string(),
            ureq::Error::Status(404, _) => "找不到發行版本（權杖需要能讀取 toolbox 儲存庫）".to_string(),
            ureq::Error::Status(c, _) => format!("GitHub 回應錯誤 {c}"),
            e => format!("連線失敗：{e}"),
        })
}

fn release(token: &str) -> Result<serde_json::Value, String> {
    github(&format!("https://api.github.com/repos/{REPO}/releases/tags/v1.0.0"), token, "application/vnd.github+json")?
        .into_json::<serde_json::Value>()
        .map_err(|e| e.to_string())
}

#[derive(Serialize)]
pub struct Check {
    pub has_update: bool,
    pub current: String,
    pub latest: String,
    pub published: String,
}

fn check_blocking() -> Result<Check, String> {
    let t = token()?;
    let rel = release(&t)?;
    let latest = rel["target_commitish"].as_str().unwrap_or("").to_string();
    let cur = current();
    // 自己在電腦上編譯的版本（沒有 commit）不更新
    let has_update = !cur.is_empty() && !latest.is_empty() && latest != cur;
    Ok(Check {
        has_update,
        current: cur.chars().take(7).collect(),
        latest: latest.chars().take(7).collect(),
        published: rel["published_at"].as_str().unwrap_or("").to_string(),
    })
}

#[tauri::command]
pub async fn update_check() -> Result<Check, String> {
    tauri::async_runtime::spawn_blocking(check_blocking).await.map_err(|e| e.to_string())?
}

/// 下載 MAOBOO.zip → 取出 exe → 正在執行的檔案改名成 .old.exe → 新檔放到原位置 → 開新版本 → 結束自己
fn install_blocking() -> Result<PathBuf, String> {
    let t = token()?;
    let rel = release(&t)?;
    let asset = rel["assets"]
        .as_array()
        .and_then(|a| a.iter().find(|x| x["name"].as_str().map(|n| n.ends_with(".zip")).unwrap_or(false)))
        .ok_or("發行版本裡找不到 MAOBOO.zip")?;
    let url = asset["url"].as_str().ok_or("下載網址錯誤")?;
    let resp = github(url, &t, "application/octet-stream")?;
    let mut zip_bytes = Vec::new();
    resp.into_reader().take(200 * 1024 * 1024).read_to_end(&mut zip_bytes).map_err(|e| format!("下載失敗：{e}"))?;

    let mut zip = zip::ZipArchive::new(std::io::Cursor::new(zip_bytes)).map_err(|e| format!("壓縮檔錯誤：{e}"))?;
    let mut exe_bytes = Vec::new();
    for i in 0..zip.len() {
        let mut f = zip.by_index(i).map_err(|e| e.to_string())?;
        if f.name().to_lowercase().ends_with(".exe") {
            f.read_to_end(&mut exe_bytes).map_err(|e| e.to_string())?;
            break;
        }
    }
    if exe_bytes.len() < 1024 {
        return Err("壓縮檔裡沒有執行檔".into());
    }

    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let dir = exe.parent().ok_or("找不到程式所在資料夾")?;
    let tmp = dir.join("MAOBOO.update.exe");
    std::fs::write(&tmp, &exe_bytes).map_err(|e| format!("無法寫入新版本：{e}"))?;
    let old = old_path(&exe);
    let _ = std::fs::remove_file(&old);
    // 執行中的 exe 不能覆蓋，但可以改名
    std::fs::rename(&exe, &old).map_err(|e| format!("無法替換執行檔：{e}"))?;
    if let Err(e) = std::fs::rename(&tmp, &exe) {
        let _ = std::fs::rename(&old, &exe);
        return Err(format!("無法替換執行檔：{e}"));
    }
    Ok(exe)
}

#[tauri::command]
pub async fn update_install(app: AppHandle) -> Result<(), String> {
    let exe = tauri::async_runtime::spawn_blocking(install_blocking).await.map_err(|e| e.to_string())??;
    std::process::Command::new(&exe).spawn().map_err(|e| format!("新版本已下載，但無法自動開啟：{e}"))?;
    app.exit(0);
    Ok(())
}
