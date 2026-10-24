//! 自動更新：讀取 GitHub 的 v1.0.0 發行版本（版本號固定 1.0.0，以編譯編號／commit 分新舊），
//! 有新版就下載 MAOBOO.zip、替換執行檔並重新開啟。

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
    /// 舊版手動填的權杖（內建權杖不存在時才用）
    pub token: String,
}

impl Default for UpdCfg {
    fn default() -> Self {
        Self { token: String::new() }
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

// 編譯時由 build.rs 產生：K_SEED、K_LEN、K_A、K_B
include!(concat!(env!("OUT_DIR"), "/update_token.rs"));

/// 還原內建的權杖（沒有內建時是空字串；倉庫公開後已不再使用，留著給測試）
#[allow(dead_code)]
#[inline(never)]
fn builtin() -> String {
    let mut enc = vec![0u8; K_LEN];
    for (j, b) in K_A.iter().rev().enumerate() {
        enc[j * 2] = *b;
    }
    for (j, b) in K_B.iter().rev().enumerate() {
        enc[j * 2 + 1] = *b;
    }
    let mut x = K_SEED;
    let mut out = Vec::with_capacity(K_LEN);
    for (i, b) in enc.iter().enumerate() {
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        let k = (x >> ((i % 7) * 8)) as u8;
        out.push(b ^ k ^ (i as u8).wrapping_mul(0x5B));
    }
    String::from_utf8(out).unwrap_or_default()
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

/// GitHub 倉庫網址（關於頁的連結）
pub const REPO_URL: &str = "https://github.com/MAOBOO2240/toolbox";

#[derive(Serialize)]
pub struct UpdInfo {
    pub current: String,
    pub run: String,
    pub token_set: bool,
    /// 版本號（tauri.conf.json / Cargo.toml 的 version）
    pub version: String,
    pub repo_url: String,
}

#[tauri::command]
pub fn update_info() -> UpdInfo {
    UpdInfo {
        current: current().chars().take(7).collect(),
        run: option_env!("GITHUB_RUN_NUMBER").unwrap_or("").to_string(),
        token_set: true,
        version: env!("CARGO_PKG_VERSION").to_string(),
        repo_url: REPO_URL.to_string(),
    }
}

/// 用瀏覽器開 GitHub 倉庫頁
#[tauri::command]
pub fn open_repo() -> Result<(), String> {
    crate::winsys::open(REPO_URL)
}

fn token() -> Result<String, String> {
    // 倉庫是公開的，不用權杖（私人倉庫時代留下的舊權杖已經失效，帶上反而會被拒）
    Ok(String::new())
}

fn github(url: &str, token: &str, accept: &str) -> Result<ureq::Response, String> {
    let mut req = ureq::get(url);
    if !token.trim().is_empty() {
        req = req.set("Authorization", &format!("Bearer {token}"));
    }
    req.set("Accept", accept)
        .set("User-Agent", "MAOBOO-updater")
        .set("X-GitHub-Api-Version", "2022-11-28")
        .timeout(Duration::from_secs(60))
        .call()
        .map_err(|e| match e {
            ureq::Error::Status(401, _) => "更新權限已失效（權杖過期）".to_string(),
            ureq::Error::Status(403, r) | ureq::Error::Status(429, r) => {
                // 沒帶權杖時 GitHub 每小時每個 IP 只給 60 次；把剩餘次數與重設時間一起告訴使用者
                let reset = r
                    .header("x-ratelimit-reset")
                    .and_then(|v| v.parse::<u64>().ok())
                    .and_then(|t| {
                        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).ok()?.as_secs();
                        Some(t.saturating_sub(now) / 60 + 1)
                    });
                let remaining = r.header("x-ratelimit-remaining").map(str::to_string);
                let msg: String = r.into_json::<serde_json::Value>().ok().and_then(|v| v["message"].as_str().map(str::to_string)).unwrap_or_default();
                if remaining.as_deref() == Some("0") || msg.to_lowercase().contains("rate limit") {
                    format!("GitHub 暫時限制查詢次數，請 {} 分鐘後再試", reset.unwrap_or(60))
                } else {
                    format!("GitHub 拒絕了這次查詢：{}", if msg.is_empty() { "403".into() } else { msg })
                }
            }
            ureq::Error::Status(404, _) => "找不到發行版本".to_string(),
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

/// 發行版本裡的 MAOBOO.sha256（GitHub 編譯時算好的新版本雜湊值）；舊的發行版本沒有這個檔案時不驗證
fn expected_hash(rel: &serde_json::Value, token: &str) -> Result<Option<String>, String> {
    let Some(asset) = rel["assets"].as_array().and_then(|a| a.iter().find(|x| x["name"].as_str() == Some("MAOBOO.sha256"))) else {
        return Ok(None);
    };
    let url = asset["url"].as_str().ok_or("下載網址錯誤")?;
    let mut text = String::new();
    github(url, token, "application/octet-stream")?
        .into_reader()
        .take(4096)
        .read_to_string(&mut text)
        .map_err(|e| format!("下載失敗：{e}"))?;
    parse_hash(&text).map(Some).ok_or_else(|| "新版本的驗證資料格式不對，已取消更新".into())
}

/// 「雜湊值  檔名」格式的第一個欄位（64 個十六進位字元）
fn parse_hash(text: &str) -> Option<String> {
    let h = text.trim_start_matches('\u{feff}').split_whitespace().next()?.to_ascii_lowercase();
    (h.len() == 64 && h.chars().all(|c| c.is_ascii_hexdigit())).then_some(h)
}

/// 下載 MAOBOO.zip → 取出 exe → 和 MAOBOO.sha256 比對 → 正在執行的檔案改名成 .old.exe → 新檔放到原位置 → 開新版本 → 結束自己
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
    // 確認和 GitHub 編譯出來的檔案一模一樣（下載不完整或被改過就不更新）
    if let Some(want) = expected_hash(&rel, &t)? {
        if crate::sha256::hex(&exe_bytes) != want {
            return Err("下載的新版本驗證失敗（可能下載不完整），已取消更新，請稍後再試".into());
        }
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
    // 告訴新版本要等這個舊版本結束（只能開一個的檢查）
    std::process::Command::new(&exe)
        .arg(format!("--after-update={}", std::process::id()))
        .spawn()
        .map_err(|e| format!("新版本已下載，但無法自動開啟：{e}"))?;
    app.exit(0);
    Ok(())
}

#[cfg(test)]
mod tests {
    #[test]
    fn hash_file_format() {
        let h = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";
        assert_eq!(super::parse_hash(&format!("{h}  MAOBOO.exe")).as_deref(), Some(h));
        assert_eq!(super::parse_hash(&format!("\u{feff}{}\r\n", h.to_uppercase())).as_deref(), Some(h));
        assert_eq!(super::parse_hash("abc  MAOBOO.exe"), None);
        assert_eq!(super::parse_hash(""), None);
    }

    #[test]
    fn builtin_roundtrip() {
        // 測試時通常沒有 UPDATE_TOKEN，至少要能正常還原（長度一致、不會 panic）
        let t = super::builtin();
        assert_eq!(t.len(), super::K_LEN);
    }
}
