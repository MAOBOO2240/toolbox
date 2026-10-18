//! 版本更新：從新楓之谷官網的公告（「更新」與「重要」分類）抓出各版本的公告。
//!
//! 官網公告清單是 ASP.NET 頁面的 BulletinProxy，需要先開一次頁面拿到防偽權杖（隱藏欄位）
//! 與對應的 Cookie，再用 POST 查詢。抓到的公告記在 app 資料夾，之後只查最新幾筆補上新版本。

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const SITE: &str = "https://maplestory.beanfun.com";
/// 更新、重要
const KINDS: [&str; 2] = ["67", "68"];
/// 快取多久內不重查（秒）
const FRESH_SECS: u64 = 6 * 3600;

#[derive(Clone, Serialize, Deserialize)]
pub struct Bulletin {
    id: String,
    date: String,
    title: String,
    /// 公告網址（有外部連結時用外部連結）
    url: String,
}

#[derive(Default, Serialize, Deserialize)]
#[serde(default)]
struct Cache {
    fetched_at: u64,
    items: Vec<Bulletin>,
}

static STORE: OnceLock<Mutex<(Cache, PathBuf)>> = OnceLock::new();

pub fn init(path: PathBuf) {
    let cache = std::fs::read_to_string(&path).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default();
    let _ = STORE.set(Mutex::new((cache, path)));
}

fn now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

fn save(cache: &Cache, path: &PathBuf) {
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    if let Ok(json) = serde_json::to_string(cache) {
        let tmp = path.with_extension("json.tmp");
        if std::fs::write(&tmp, json).is_ok() {
            let _ = std::fs::rename(&tmp, path);
        }
    }
}

/// 標題裡有版本號（V282、v_025、V043…）才留下
fn has_version(title: &str) -> bool {
    let b = title.as_bytes();
    (0..b.len()).any(|i| {
        if b[i] != b'V' && b[i] != b'v' {
            return false;
        }
        let mut j = i + 1;
        if j < b.len() && matches!(b[j], b' ' | b'_' | b'.') {
            j += 1;
        }
        j < b.len() && b[j].is_ascii_digit()
    })
}

/// 官網的一個連線：Cookie 與防偽權杖
struct Session {
    cookie: String,
    token: String,
}

fn agent() -> ureq::Agent {
    ureq::AgentBuilder::new().timeout(Duration::from_secs(25)).user_agent("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/130.0.0.0 Safari/537.36").build()
}

fn open_session(agent: &ureq::Agent) -> Result<Session, String> {
    let resp = agent.get(&format!("{SITE}/main?section=mBulletin")).call().map_err(|e| format!("無法連線到新楓之谷官網：{e}"))?;
    let cookie = resp
        .all("set-cookie")
        .iter()
        .filter_map(|c| c.split(';').next())
        .map(str::trim)
        .filter(|c| c.contains('='))
        .collect::<Vec<_>>()
        .join("; ");
    let html = resp.into_string().map_err(|e| e.to_string())?;
    let at = html.find("__RequestVerificationToken").ok_or("官網頁面改版了，找不到公告的查詢權杖")?;
    let tail = &html[at..];
    let start = tail.find("value=\"").ok_or("找不到公告的查詢權杖")? + 7;
    let end = tail[start..].find('"').ok_or("找不到公告的查詢權杖")?;
    Ok(Session { cookie, token: tail[start..start + end].to_string() })
}

/// 查一頁公告；回傳 (公告, 總頁數)
fn fetch_page(agent: &ureq::Agent, s: &Session, kind: &str, page: u32, size: u32) -> Result<(Vec<Bulletin>, u32), String> {
    let page_s = page.to_string();
    let size_s = size.to_string();
    let resp = agent
        .post(&format!("{SITE}/main?handler=BulletinProxy"))
        .set("X-CSRF-TOKEN", &s.token)
        .set("X-Requested-With", "XMLHttpRequest")
        .set("Origin", SITE)
        .set("Referer", &format!("{SITE}/main?section=mBulletin"))
        .set("Cookie", &s.cookie)
        .send_form(&[
            ("Kind", kind),
            ("Page", &page_s),
            ("method", "3"),
            ("PageSize", &size_s),
            ("toAll", "0"),
            ("__RequestVerificationToken", &s.token),
        ])
        .map_err(|e| format!("官網公告查詢失敗：{e}"))?;
    let v: Value = resp.into_json().map_err(|e| format!("官網公告格式不對：{e}"))?;
    let ds = &v["data"]["myDataSet"];
    let pages = ds["systemTable"]["totalPage"].as_str().and_then(|s| s.parse().ok()).or_else(|| ds["systemTable"]["totalPage"].as_u64().map(|n| n as u32)).unwrap_or(1);
    let mut out = Vec::new();
    for it in ds["table"].as_array().map(Vec::as_slice).unwrap_or(&[]) {
        let title = it["title"].as_str().unwrap_or("").trim().to_string();
        let id = it["bullentinId"].as_str().map(str::to_string).or_else(|| it["bullentinId"].as_u64().map(|n| n.to_string())).unwrap_or_default();
        let date = it["startDate"].as_str().unwrap_or("").chars().take(10).collect::<String>().replace('-', "/");
        if id.is_empty() || !has_version(&title) || date.starts_with("19") {
            continue;
        }
        let link = it["urlLink"].as_str().map(str::trim).filter(|u| u.starts_with("https://") && host_ok(u)).map(str::to_string);
        out.push(Bulletin { url: link.unwrap_or_else(|| format!("{SITE}/bulletin?bid={id}")), id, date, title });
    }
    Ok((out, pages))
}

/// 只開官網（beanfun.com）的網址
pub fn host_ok(url: &str) -> bool {
    let Some(rest) = url.strip_prefix("https://") else { return false };
    let host = rest.split(['/', '?', '#']).next().unwrap_or("");
    if host.contains('@') || host.contains(':') {
        return false;
    }
    host == "beanfun.com" || host.ends_with(".beanfun.com")
}

/// 全部抓一次（每頁 500 筆，大約 3～4 次查詢）
fn fetch_all() -> Result<Vec<Bulletin>, String> {
    let agent = agent();
    let s = open_session(&agent)?;
    let mut all = Vec::new();
    for kind in KINDS {
        let (first, pages) = fetch_page(&agent, &s, kind, 1, 500)?;
        all.extend(first);
        for p in 2..=pages.min(20) {
            all.extend(fetch_page(&agent, &s, kind, p, 500)?.0);
        }
    }
    Ok(all)
}

/// 只抓最新的幾筆（補上新版本）
fn fetch_recent() -> Result<Vec<Bulletin>, String> {
    let agent = agent();
    let s = open_session(&agent)?;
    let mut all = Vec::new();
    for kind in KINDS {
        all.extend(fetch_page(&agent, &s, kind, 1, 100)?.0);
    }
    Ok(all)
}

#[derive(Serialize)]
pub struct VersionOut {
    items: Vec<Bulletin>,
    fetched_at: u64,
    /// 這次沒連上官網，用的是之前存的資料
    stale: bool,
    error: Option<String>,
}

fn list_blocking(force: bool) -> Result<VersionOut, String> {
    let store = STORE.get().ok_or("尚未初始化")?;
    let (has, fresh) = {
        let g = store.lock().unwrap();
        (!g.0.items.is_empty(), now().saturating_sub(g.0.fetched_at) < FRESH_SECS)
    };
    if has && fresh && !force {
        let g = store.lock().unwrap();
        return Ok(VersionOut { items: g.0.items.clone(), fetched_at: g.0.fetched_at, stale: false, error: None });
    }
    let got = if has { fetch_recent() } else { fetch_all() };
    let mut g = store.lock().unwrap();
    match got {
        Ok(list) => {
            for b in list {
                if let Some(old) = g.0.items.iter_mut().find(|x| x.id == b.id) {
                    *old = b;
                } else {
                    g.0.items.push(b);
                }
            }
            g.0.items.sort_by(|a, b| b.date.cmp(&a.date).then(b.id.cmp(&a.id)));
            g.0.fetched_at = now();
            let (cache, path) = &*g;
            save(cache, path);
            Ok(VersionOut { items: g.0.items.clone(), fetched_at: g.0.fetched_at, stale: false, error: None })
        }
        Err(e) if has => Ok(VersionOut { items: g.0.items.clone(), fetched_at: g.0.fetched_at, stale: true, error: Some(e) }),
        Err(e) => Err(e),
    }
}

/// 版本更新公告清單（官網公告裡標題有版本號的；分組、挑選在介面做）
#[tauri::command]
pub async fn version_list(force: bool) -> Result<VersionOut, String> {
    tauri::async_runtime::spawn_blocking(move || list_blocking(force)).await.map_err(|e| e.to_string())?
}

/// 用預設瀏覽器開官網的公告
#[tauri::command]
pub fn open_bulletin(url: String) -> Result<(), String> {
    if !host_ok(&url) {
        return Err("只能開啟新楓之谷官網的網址".into());
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        std::process::Command::new("rundll32")
            .args(["url.dll,FileProtocolHandler", &url])
            .creation_flags(0x0800_0000)
            .spawn()
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_titles() {
        assert!(has_version("新楓之谷《0908(二) V282完整程式與手動更新下載開放通知》"));
        assert!(has_version("楓之谷v_025 版本更新公告"));
        assert!(has_version("《0203(三)v186.4手動更新檔_更新步驟說明》"));
        assert!(!has_version("新楓之谷《1014(三)楓之谷學院例行維護開機公告》"));
        assert!(!has_version("VIP系統福利調整說明"));
    }

    #[test]
    fn hosts() {
        assert!(host_ok("https://maplestory.beanfun.com/bulletin?bid=1"));
        assert!(host_ok("https://maplestory-event.beanfun.com/eventad/eventad?eventadid=1"));
        assert!(!host_ok("http://maplestory.beanfun.com/"));
        assert!(!host_ok("https://maplestory.beanfun.com.evil.example/"));
        assert!(!host_ok("https://evil.example/?x=beanfun.com"));
        assert!(!host_ok("https://maplestory.beanfun.com@evil.example/"));
    }
}
