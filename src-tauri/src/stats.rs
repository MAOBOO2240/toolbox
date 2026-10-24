//! 使用狀況回報：程式開啟 20 秒後、之後每 10 分鐘，向作者的 Cloudflare Worker 回報一次（Worker 那邊每小時才真的寫一次）。
//!
//! 只送：這台電腦的序號（跟授權用的同一個）、安裝編號（第一次執行時隨機產生）、程式版本、Windows 版本、有沒有授權、
//! 綁定的角色名稱、每個工具開了幾次／用了多久（每小時彙總一次）。不送帳號、金鑰、網址或任何遊戲資料。STATS_URL 空白時什麼都不送。
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};
use std::time::Duration;
use tauri::{AppHandle, Emitter};

/// 作者的 Worker 網址（例如 https://maboo-stats.xxx.workers.dev）。空白 = 不回報、也沒有公會自動授權
pub const STATS_URL: &str = "https://maboo-stats.1735aam.workers.dev";

static DIR: OnceLock<PathBuf> = OnceLock::new();

/// 一個工具的使用量：秒數、開啟次數、顯示名稱
#[derive(Default, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ToolUse {
    pub s: u64,
    pub n: u64,
    pub name: String,
}

#[derive(Default, Serialize, Deserialize)]
#[serde(default)]
struct Local {
    install: String,
    /// 還沒送出去的工具使用量（前端每分鐘累積過來，回報成功後清掉）
    usage: HashMap<String, ToolUse>,
    /// 上次把使用量送出去的時間（Unix 秒）；一小時送一次，省 Worker 的寫入次數
    usage_sent: u64,
}

static LOCAL: Mutex<()> = Mutex::new(());

fn local_path() -> PathBuf {
    DIR.get().cloned().unwrap_or_default().join("stats.json")
}

fn read_local() -> Local {
    std::fs::read_to_string(local_path()).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default()
}

fn write_local(l: &Local) {
    let _ = std::fs::create_dir_all(DIR.get().cloned().unwrap_or_default());
    let _ = std::fs::write(local_path(), serde_json::to_string(l).unwrap_or_default());
}

fn now_secs() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

fn install_id() -> String {
    let _g = LOCAL.lock().unwrap_or_else(|e| e.into_inner());
    let mut l = read_local();
    if l.install.is_empty() {
        let t = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
        l.install = crate::sha256::hex(format!("{t}|{}|{}", std::process::id(), crate::license::serial()).as_bytes())[..16].to_string();
        write_local(&l);
    }
    l.install
}

/// 前端每分鐘把這段時間的工具使用量交過來，先累積在本機
#[tauri::command]
pub fn stats_usage(tools: HashMap<String, ToolUse>) {
    let _g = LOCAL.lock().unwrap_or_else(|e| e.into_inner());
    let mut l = read_local();
    for (id, u) in tools {
        if id.len() > 32 || (u.s == 0 && u.n == 0) {
            continue;
        }
        let e = l.usage.entry(id).or_default();
        e.s += u.s;
        e.n += u.n;
        if !u.name.is_empty() {
            e.name = u.name.chars().take(20).collect();
        }
    }
    write_local(&l);
}

#[cfg(windows)]
fn os_version() -> String {
    crate::winsys::reg::get_lm("SOFTWARE\\Microsoft\\Windows NT\\CurrentVersion", "DisplayVersion")
        .map(|v| format!("Windows {v}"))
        .unwrap_or_else(|| "Windows".into())
}

#[cfg(not(windows))]
fn os_version() -> String {
    std::env::consts::OS.to_string()
}

#[derive(Serialize)]
struct Ping<'a> {
    serial: String,
    install: String,
    version: &'a str,
    run: &'a str,
    os: String,
    licensed: bool,
    /// 公會自動授權綁定的角色
    name: String,
    /// 這次一起送的工具使用量（一小時送一次；沒有就不帶）
    #[serde(skip_serializing_if = "Option::is_none")]
    usage: Option<HashMap<String, ToolUse>>,
}


fn send(app: &AppHandle) {
    if STATS_URL.trim().is_empty() {
        return;
    }
    // 累積滿一小時（或第一次）才把使用量帶上
    let usage = {
        let _g = LOCAL.lock().unwrap_or_else(|e| e.into_inner());
        let l = read_local();
        if !l.usage.is_empty() && now_secs().saturating_sub(l.usage_sent) >= 3600 { Some(l.usage) } else { None }
    };
    let p = Ping {
        serial: crate::license::serial(),
        install: install_id(),
        version: option_env!("GITHUB_SHA").unwrap_or(""),
        run: option_env!("GITHUB_RUN_NUMBER").unwrap_or(""),
        os: os_version(),
        licensed: crate::license::licensed(),
        name: crate::license::bound_name(),
        usage: usage.clone(),
    };
    let resp = ureq::AgentBuilder::new()
        .timeout(Duration::from_secs(15))
        .build()
        .post(&format!("{}/ping", STATS_URL.trim_end_matches('/')))
        .set("User-Agent", "MAOBOO-toolbox")
        .set("Content-Type", "application/json")
        .send_string(&serde_json::to_string(&p).unwrap_or_default());
    let ok = resp.is_ok();
    // 作者移除了授權或解除了綁定：本機跟著改，畫面重新整理
    let pong: Option<crate::license::Remote> = resp.ok().and_then(|r| r.into_string().ok()).and_then(|t| serde_json::from_str(&t).ok());
    // 送成功了：把送出去的那份從本機扣掉（期間新累積的留著）
    if ok {
        if let Some(sent) = usage {
            let _g = LOCAL.lock().unwrap_or_else(|e| e.into_inner());
            let mut l = read_local();
            for (id, u) in sent {
                if let Some(e) = l.usage.get_mut(&id) {
                    e.s = e.s.saturating_sub(u.s);
                    e.n = e.n.saturating_sub(u.n);
                }
            }
            l.usage.retain(|_, u| u.s > 0 || u.n > 0);
            l.usage_sent = now_secs();
            write_local(&l);
        }
    }
    if let Some(mut r) = pong {
        if crate::license::info().admin {
            r.revoked = false;
        }
        if crate::license::apply_remote(&r) {
            let _ = app.emit("license-changed", ());
        }
    }
}

/// 程式啟動時呼叫：背景回報，之後每 10 分鐘一次（順便收作者的「移除授權」）
pub fn start(app: AppHandle, dir: PathBuf) {
    let _ = DIR.set(dir);
    if STATS_URL.trim().is_empty() {
        return;
    }
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_secs(20));
        loop {
            send(&app);
            std::thread::sleep(Duration::from_secs(10 * 60));
        }
    });
}
