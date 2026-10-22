//! 版本更新：從新楓之谷官網找出各大版本的改版頁面。
//!
//! - 「更新」「重要」公告：標題有版本號的（V282 完整程式與手動更新下載開放通知…）→ 版本與日期
//! - 「活動」公告：連到活動頁（eventad）的 → 改版當天的活動頁；活動頁上方選單的第一項就是改版主頁
//! - 官網公告清單是 ASP.NET 頁面的 BulletinProxy，需要先開一次頁面拿到防偽權杖與 Cookie 再 POST 查詢
//! - 公告與改版主頁都記在 app 資料夾，之後只查最新幾筆補上新版本

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const SITE: &str = "https://maplestory.beanfun.com";
const EVENT_SITE: &str = "https://maplestory-event.beanfun.com";
/// 更新、重要、活動
const KINDS: [&str; 3] = ["67", "68", "72"];
/// 快取多久內不重查（秒）
const FRESH_SECS: u64 = 6 * 3600;

#[derive(Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct Bulletin {
    id: String,
    /// 公告分類：67 更新、68 重要、72 活動
    kind: String,
    date: String,
    title: String,
    /// 公告網址（有外部連結時用外部連結）
    url: String,
    /// 連到活動頁時的活動編號
    ev: Option<u32>,
}

/// 改版主頁：活動頁上方選單（第一項是主頁）
#[derive(Clone, Serialize, Deserialize)]
pub struct Hub {
    id: u32,
    title: String,
    url: String,
    sections: Vec<(u32, String)>,
}

#[derive(Default, Serialize, Deserialize)]
#[serde(default)]
struct Cache {
    fetched_at: u64,
    items: Vec<Bulletin>,
    /// 活動編號 → 改版主頁（None = 活動頁已經沒有內容，例如很舊的活動）
    hubs: HashMap<u32, Option<Hub>>,
    /// 查不到改版主頁的活動：（最後一次查的時間, 查過幾次）；官網暫時打不開或格式不對時過幾天再查，查 3 次都沒有才不再查
    none_at: HashMap<u32, (u64, u8)>,
}

/// 查不到改版主頁的活動，隔幾天再查一次
const NONE_RETRY_SECS: u64 = 3 * 86400;
const NONE_MAX_TRIES: u8 = 3;

/// 這個活動要不要（再）查改版主頁
fn hub_due(c: &Cache, id: u32, now: u64) -> bool {
    match c.hubs.get(&id) {
        None => true,
        Some(Some(_)) => false,
        // 舊版存下來的「沒有」沒有記錄時間：當作查過一次，馬上再查一次
        Some(None) => {
            let (at, n) = c.none_at.get(&id).copied().unwrap_or((0, 1));
            n < NONE_MAX_TRIES && now.saturating_sub(at) >= NONE_RETRY_SECS
        }
    }
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
        let link = it["urlLink"].as_str().map(str::trim).filter(|u| u.starts_with("https://") && host_ok(u)).map(str::to_string);
        let ev = link.as_deref().and_then(eventad_id);
        // 更新、重要：標題有版本號的；活動：連到活動頁的
        let keep = if kind == "72" { ev.is_some() } else { has_version(&title) };
        if id.is_empty() || !keep || date.starts_with("19") {
            continue;
        }
        out.push(Bulletin { url: link.unwrap_or_else(|| format!("{SITE}/bulletin?bid={id}")), id, kind: kind.to_string(), date, title, ev });
    }
    Ok((out, pages))
}

/// 活動頁網址裡的活動編號
fn eventad_id(url: &str) -> Option<u32> {
    let low = url.to_ascii_lowercase();
    if !low.contains("/eventad") {
        return None;
    }
    let at = low.find("eventadid=")? + "eventadid=".len();
    let digits: String = low[at..].chars().take_while(|c| c.is_ascii_digit()).collect();
    digits.parse().ok()
}

/// HTML 裡的 &#xFF1A;、&amp; 之類轉回文字
fn decode_html(s: &str) -> String {
    let mut out = String::new();
    let mut rest = s;
    while let Some(i) = rest.find('&') {
        out.push_str(&rest[..i]);
        let tail = &rest[i..];
        let Some(end) = tail.find(';').filter(|&e| e <= 10) else {
            out.push('&');
            rest = &tail[1..];
            continue;
        };
        let ent = &tail[1..end];
        let ch = match ent {
            "amp" => Some('&'),
            "lt" => Some('<'),
            "gt" => Some('>'),
            "quot" => Some('"'),
            "apos" | "#39" => Some('\''),
            "nbsp" => Some(' '),
            _ if ent.starts_with("#x") || ent.starts_with("#X") => u32::from_str_radix(&ent[2..], 16).ok().and_then(char::from_u32),
            _ if ent.starts_with('#') => ent[1..].parse().ok().and_then(char::from_u32),
            _ => None,
        };
        match ch {
            Some(c) => {
                out.push(c);
                rest = &tail[end + 1..];
            }
            None => {
                out.push('&');
                rest = &tail[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

/// 活動頁上方選單（<div class="nav2"> 裡的連結）；第一項是改版主頁
fn parse_hub(html: &str) -> Option<Hub> {
    let at = html.find("class=\"nav2\"")?;
    let body = &html[at..];
    let end = body.find("</div>").unwrap_or(body.len());
    let nav = &body[..end];
    let mut sections = Vec::new();
    let mut rest = nav;
    while let Some(a) = rest.find("<a ") {
        let tag = &rest[a..];
        let Some(close) = tag.find("</a>") else { break };
        let el = &tag[..close];
        let href = el.find("href=\"").map(|h| &el[h + 6..]).and_then(|h| h.find('"').map(|e| &h[..e])).unwrap_or("");
        let text = el.find('>').map(|g| decode_html(el[g + 1..].trim())).unwrap_or_default();
        if let Some(id) = eventad_id(&decode_html(href)) {
            if !text.is_empty() && !sections.iter().any(|(x, _)| *x == id) {
                sections.push((id, text));
            }
        }
        rest = &tag[close + 4..];
    }
    let (id, title) = sections.first().cloned()?;
    Some(Hub { id, title, url: format!("{EVENT_SITE}/eventad/eventad?eventadid={id}"), sections })
}

fn fetch_hub(agent: &ureq::Agent, id: u32) -> Result<Option<Hub>, String> {
    let html = agent
        .get(&format!("{EVENT_SITE}/eventad/eventad?eventadid={id}"))
        .call()
        .map_err(|e| format!("活動頁讀取失敗：{e}"))?
        .into_string()
        .map_err(|e| e.to_string())?;
    Ok(parse_hub(&html))
}

/* ---------------- 抽獎模擬器的官網機率頁 ----------------
 * 黃金蘋果、閃亮彗星、星光錦囊、皇家美容院的機率頁是固定的活動頁編號，
 * 換新獎池時官網直接更新同一頁。這裡只負責把頁面 HTML 抓回來，
 * 轉成文字與解析在前端做（和使用者全選複製貼上的結果一樣）。 */

/// 閃亮彗星、黃金蘋果、星光錦囊、皇家美容院
const POOL_PAGES: [u32; 4] = [8368, 8369, 8370, 8372];

fn pool_page_blocking(id: u32) -> Result<String, String> {
    let resp = agent()
        .get(&format!("{EVENT_SITE}/eventad/eventad?eventadid={id}"))
        .call()
        .map_err(|e| format!("官網機率頁讀取失敗：{e}"))?;
    // 轉址後也必須還在官網
    if !host_ok(resp.get_url()) {
        return Err("官網機率頁轉到其他網站，已停止讀取".into());
    }
    resp.into_string().map_err(|e| format!("官網機率頁讀取失敗：{e}"))
}

/// 讀取抽獎模擬器的官網機率頁（只接受固定的幾個頁面）
#[tauri::command]
pub async fn pool_page(id: u32) -> Result<String, String> {
    if !POOL_PAGES.contains(&id) {
        return Err("不支援的頁面".into());
    }
    tauri::async_runtime::spawn_blocking(move || pool_page_blocking(id)).await.map_err(|e| e.to_string())?
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

/// 全部抓一次（每頁 1000 筆，大約 10 次查詢）
fn fetch_all() -> Result<Vec<Bulletin>, String> {
    let agent = agent();
    let s = open_session(&agent)?;
    let mut all = Vec::new();
    for kind in KINDS {
        let (first, pages) = fetch_page(&agent, &s, kind, 1, 1000)?;
        all.extend(first);
        for p in 2..=pages.min(30) {
            all.extend(fetch_page(&agent, &s, kind, p, 1000)?.0);
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
        // 舊版的快取沒有活動公告：重新全部抓一次
        let complete = g.0.items.iter().any(|b| b.kind == "72");
        (complete, now().saturating_sub(g.0.fetched_at) < FRESH_SECS)
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

/// 這些活動頁的改版主頁（查過的記住；一次最多查 40 個，介面會分批要）
#[tauri::command]
pub async fn version_hubs(ids: Vec<u32>) -> Result<HashMap<String, Option<Hub>>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let store = STORE.get().ok_or("尚未初始化")?;
        let need: Vec<u32> = {
            let g = store.lock().unwrap();
            let t = now();
            ids.iter().copied().filter(|&id| hub_due(&g.0, id, t)).take(40).collect()
        };
        if !need.is_empty() {
            let agent = agent();
            let mut got = Vec::new();
            for (i, id) in need.iter().enumerate() {
                if i > 0 {
                    std::thread::sleep(Duration::from_millis(120));
                }
                match fetch_hub(&agent, *id) {
                    Ok(h) => got.push((*id, h)),
                    Err(e) if got.is_empty() => return Err(e),
                    Err(_) => break,
                }
            }
            let mut g = store.lock().unwrap();
            let t = now();
            let c = &mut g.0;
            for (id, h) in got {
                // 選單裡的其他活動頁也是同一個改版主頁，一起記住
                if let Some(hub) = &h {
                    for (sid, _) in &hub.sections {
                        let e = c.hubs.entry(*sid).or_insert(None);
                        if e.is_none() {
                            *e = Some(hub.clone());
                            c.none_at.remove(sid);
                        }
                    }
                    c.none_at.remove(&id);
                } else {
                    let was_none = matches!(c.hubs.get(&id), Some(None));
                    let n = c.none_at.get(&id).map_or(if was_none { 1 } else { 0 }, |x| x.1);
                    c.none_at.insert(id, (t, n.saturating_add(1)));
                }
                c.hubs.insert(id, h);
            }
            let (cache, path) = &*g;
            save(cache, path);
        }
        let g = store.lock().unwrap();
        Ok(ids.iter().filter_map(|id| g.0.hubs.get(id).map(|h| (id.to_string(), h.clone()))).collect())
    })
    .await
    .map_err(|e| e.to_string())?
}

/// 用預設瀏覽器開官網的公告
#[tauri::command]
pub fn open_bulletin(url: String) -> Result<(), String> {
    if !host_ok(&url) {
        return Err("只能開啟新楓之谷官網的網址".into());
    }
    // 和一般程式開網頁的方式一樣（不開 rundll32）
    #[cfg(windows)]
    crate::winsys::open(&url)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_hub_is_rechecked_a_few_times() {
        let t = 1_800_000_000u64;
        let mut c = Cache::default();
        assert!(hub_due(&c, 1, t));
        c.hubs.insert(1, None);
        // 舊版存的「沒有」：馬上再查一次
        assert!(hub_due(&c, 1, t));
        c.none_at.insert(1, (t, 2));
        assert!(!hub_due(&c, 1, t + NONE_RETRY_SECS - 1));
        assert!(hub_due(&c, 1, t + NONE_RETRY_SECS));
        c.none_at.insert(1, (t, NONE_MAX_TRIES));
        assert!(!hub_due(&c, 1, t + 100 * NONE_RETRY_SECS));
        c.hubs.insert(2, Some(Hub { id: 2, title: String::new(), url: String::new(), sections: Vec::new() }));
        assert!(!hub_due(&c, 2, t));
    }

    #[test]
    fn version_titles() {
        assert!(has_version("新楓之谷《0908(二) V282完整程式與手動更新下載開放通知》"));
        assert!(has_version("楓之谷v_025 版本更新公告"));
        assert!(has_version("《0203(三)v186.4手動更新檔_更新步驟說明》"));
        assert!(!has_version("新楓之谷《1014(三)楓之谷學院例行維護開機公告》"));
        assert!(!has_version("VIP系統福利調整說明"));
    }

    #[test]
    fn hub_menu() {
        let html = r#"<div class="container"><div class="nav2">
            <a href="https://maplestory-event.beanfun.com/eventad/eventad?eventadid=19029" target="_self">楓之谷學院</a>
            <a href="https://maplestory-event.beanfun.com/eventad/eventad?eventadid=19030" target="_self">變身燃燒&#xFF1A;露希妲</a>
            <a href="https://maplestory-event.beanfun.com/eventad/eventad?eventadid=19035" target="_self">技能改善</a>
        </div></div>"#;
        let h = parse_hub(html).unwrap();
        assert_eq!(h.id, 19029);
        assert_eq!(h.title, "楓之谷學院");
        assert_eq!(h.sections.len(), 3);
        assert_eq!(h.sections[1].1, "變身燃燒：露希妲");
        assert!(parse_hub("<html><body>活動已結束</body></html>").is_none());
        assert_eq!(eventad_id("https://maplestory-event.beanfun.com/eventad/eventad?eventadid=19031"), Some(19031));
        assert_eq!(eventad_id("https://maplestory.beanfun.com/bulletin?bid=1"), None);
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
