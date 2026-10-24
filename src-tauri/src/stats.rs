//! 使用狀況回報：程式開啟時與之後每 6 小時，向作者的 Cloudflare Worker 回報一次。
//!
//! 只送：這台電腦的序號（跟授權用的同一個）、安裝編號（第一次執行時隨機產生）、程式版本、Windows 版本、有沒有授權。
//! 不送帳號、角色名稱、金鑰、網址或任何遊戲資料。STATS_URL 空白時什麼都不送。
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::OnceLock;
use std::time::Duration;

/// 作者的 Worker 網址（例如 https://maboo-stats.xxx.workers.dev）。空白 = 不回報
const STATS_URL: &str = "";

static DIR: OnceLock<PathBuf> = OnceLock::new();

#[derive(Default, Serialize, Deserialize)]
#[serde(default)]
struct Local {
    install: String,
    /// 使用者在程式裡輸入過的楓之谷角色名稱（經驗追蹤、美容美髮、側欄）；設定頁有說明會回報
    names: Vec<String>,
}

fn read_local() -> Local {
    std::fs::read_to_string(local_path()).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default()
}

fn write_local(l: &Local) {
    let _ = std::fs::create_dir_all(DIR.get().cloned().unwrap_or_default());
    let _ = std::fs::write(local_path(), serde_json::to_string(&l).unwrap_or_default());
}

/// 前端把目前用到的角色名稱交過來（最多 20 個）
#[tauri::command]
pub fn stats_names(names: Vec<String>) {
    let mut l = read_local();
    let mut out: Vec<String> = Vec::new();
    for n in names.into_iter().map(|n| n.trim().to_string()).filter(|n| !n.is_empty()) {
        if !out.contains(&n) {
            out.push(n);
        }
    }
    out.truncate(20);
    if l.names != out {
        l.names = out;
        write_local(&l);
    }
}

fn local_path() -> PathBuf {
    DIR.get().cloned().unwrap_or_default().join("stats.json")
}

fn install_id() -> String {
    let mut l = read_local();
    if l.install.is_empty() {
        let t = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
        l.install = crate::sha256::hex(format!("{t}|{}|{}", std::process::id(), crate::license::serial()).as_bytes())[..16].to_string();
        write_local(&l);
    }
    l.install
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
    names: Vec<String>,
}

fn send() {
    if STATS_URL.trim().is_empty() {
        return;
    }
    let p = Ping {
        serial: crate::license::serial(),
        install: install_id(),
        version: option_env!("GITHUB_SHA").unwrap_or(""),
        run: option_env!("GITHUB_RUN_NUMBER").unwrap_or(""),
        os: os_version(),
        licensed: crate::license::licensed(),
        names: read_local().names,
    };
    let _ = ureq::AgentBuilder::new()
        .timeout(Duration::from_secs(15))
        .build()
        .post(&format!("{}/ping", STATS_URL.trim_end_matches('/')))
        .set("User-Agent", "MAOBOO-toolbox")
        .set("Content-Type", "application/json")
        .send_string(&serde_json::to_string(&p).unwrap_or_default());
}

/// 程式啟動時呼叫：背景回報，之後每 6 小時一次
pub fn start(dir: PathBuf) {
    let _ = DIR.set(dir);
    if STATS_URL.trim().is_empty() {
        return;
    }
    std::thread::spawn(|| {
        std::thread::sleep(Duration::from_secs(20));
        loop {
            send();
            std::thread::sleep(Duration::from_secs(6 * 3600));
        }
    });
}
