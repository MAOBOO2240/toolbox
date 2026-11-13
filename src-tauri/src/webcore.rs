//! 瀏覽器的純邏輯（不碰視窗、不碰 Tauri，可以單獨測試）：
//! 設定、歷史記錄、下載記錄、網址列的建議、鍵盤快捷鍵、縮放、內部頁面（chrome://…）的網址、右鍵選單的內容。

pub use crate::webev::{CtxTarget, NativeItem};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use url::Url;

pub fn now_ms() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

/* ---------------- 內部頁面 ---------------- */

/// 內部頁面：chrome://<名稱>/
pub const UI_PAGES: [&str; 6] = ["newtab", "history", "downloads", "bookmarks", "extensions", "settings"];
pub const NEWTAB: &str = "chrome://newtab/";
const UI_FILE: &str = "web-ui.html";

/// 程式本身頁面的網址開頭（Windows 上是 http://tauri.localhost/）
pub fn ui_base() -> &'static str {
    if cfg!(windows) {
        "http://tauri.localhost/"
    } else {
        "tauri://localhost/"
    }
}

pub fn ui_title(page: &str) -> &'static str {
    match page {
        "newtab" => "新分頁",
        "history" => "歷史記錄",
        "downloads" => "下載",
        "bookmarks" => "書籤",
        "extensions" => "擴充功能",
        "settings" => "設定",
        _ => "",
    }
}

/// chrome://history/?q=x 是哪一頁（不是內部頁面就 None）
pub fn virtual_page(v: &str) -> Option<&'static str> {
    let u = Url::parse(v.trim()).ok()?;
    if u.scheme() != "chrome" {
        return None;
    }
    let host = u.host_str()?.to_lowercase();
    let host = match host.as_str() {
        "new-tab-page" | "newtab" => "newtab",
        h => h,
    };
    UI_PAGES.iter().copied().find(|p| *p == host)
}

pub fn is_newtab(v: &str) -> bool {
    virtual_page(v) == Some("newtab")
}

/// 內部頁面網址的標準寫法：chrome://history/、chrome://settings/clearBrowserData、chrome://history/?q=x
pub fn canon_virtual(v: &str) -> Option<String> {
    local_to_virtual(&virtual_to_local(v)?)
}

/// chrome://settings/clearBrowserData?x=1 → 程式裡真正的網址（web-ui.html?p=settings&s=clearBrowserData&x=1）
pub fn virtual_to_local(v: &str) -> Option<Url> {
    let page = virtual_page(v)?;
    let u = Url::parse(v.trim()).ok()?;
    let mut out = Url::parse(ui_base()).ok()?.join(UI_FILE).ok()?;
    {
        let mut q = out.query_pairs_mut();
        q.append_pair("p", page);
        let sub = u.path().trim_matches('/');
        if !sub.is_empty() {
            q.append_pair("s", sub);
        }
        for (k, val) in u.query_pairs() {
            if k != "p" && k != "s" {
                q.append_pair(&k, &val);
            }
        }
    }
    Some(out)
}

/// 程式裡的網址 → chrome://…（不是內部頁面就 None）
pub fn local_to_virtual(u: &Url) -> Option<String> {
    let base = Url::parse(ui_base()).ok()?;
    if u.scheme() != base.scheme() || u.host_str() != base.host_str() || u.path().trim_start_matches('/') != UI_FILE {
        return None;
    }
    let mut page = "";
    let mut sub = String::new();
    let mut rest: Vec<(String, String)> = Vec::new();
    for (k, v) in u.query_pairs() {
        match &*k {
            "p" => page = UI_PAGES.iter().copied().find(|p| *p == v).unwrap_or(""),
            "s" => sub = v.to_string(),
            _ => rest.push((k.to_string(), v.to_string())),
        }
    }
    if page.is_empty() {
        return None;
    }
    let mut out = Url::parse(&format!("chrome://{page}/")).ok()?;
    if !sub.is_empty() && sub.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '/') {
        out.set_path(&format!("/{sub}"));
    }
    if !rest.is_empty() {
        let mut q = out.query_pairs_mut();
        for (k, v) in &rest {
            q.append_pair(k, v);
        }
    }
    Some(out.to_string())
}

/// 程式本身的頁面（不管是不是瀏覽器的內部頁面）
pub fn is_app_url(u: &Url) -> bool {
    Url::parse(ui_base()).is_ok_and(|b| u.scheme() == b.scheme() && u.host_str() == b.host_str())
}

/* ---------------- 搜尋引擎、網址列輸入 ---------------- */

pub struct Engine {
    pub id: &'static str,
    pub name: &'static str,
    /// 搜尋網址（{} 換成關鍵字）
    pub search: &'static str,
    /// 搜尋建議（回傳 OpenSearch 格式的 JSON）
    pub suggest: &'static str,
}

pub const ENGINES: [Engine; 4] = [
    Engine {
        id: "google",
        name: "Google",
        search: "https://www.google.com/search?q={}",
        suggest: "https://www.google.com/complete/search?client=chrome&ie=utf-8&oe=utf-8&q={}",
    },
    Engine { id: "bing", name: "Bing", search: "https://www.bing.com/search?q={}", suggest: "https://api.bing.com/osjson.aspx?query={}" },
    Engine {
        id: "yahoo",
        name: "Yahoo奇摩",
        search: "https://tw.search.yahoo.com/search?p={}",
        suggest: "https://sugg.search.yahoo.net/sg/?output=fxjson&nresults=8&command={}",
    },
    Engine { id: "duckduckgo", name: "DuckDuckGo", search: "https://duckduckgo.com/?q={}", suggest: "https://duckduckgo.com/ac/?type=list&q={}" },
];

pub fn engine(id: &str) -> &'static Engine {
    ENGINES.iter().find(|e| e.id == id).unwrap_or(&ENGINES[0])
}

fn encode_query(s: &str) -> String {
    url::form_urlencoded::byte_serialize(s.as_bytes()).collect()
}

pub fn search_url(e: &Engine, text: &str) -> String {
    e.search.replace("{}", &encode_query(text.trim()))
}

pub fn suggest_url(e: &Engine, text: &str) -> String {
    e.suggest.replace("{}", &encode_query(text.trim()))
}

/// 搜尋建議的回應（["關鍵字", ["建議1", "建議2", …], …]）→ 建議清單
pub fn parse_suggest(body: &str) -> Vec<String> {
    let body = body.trim_start_matches(")]}'").trim();
    let Ok(v) = serde_json::from_str::<serde_json::Value>(body) else { return Vec::new() };
    v.get(1)
        .and_then(|a| a.as_array())
        .map(|a| a.iter().filter_map(|x| x.as_str()).map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).take(8).collect())
        .unwrap_or_default()
}

/// 看起來像網址（不是要搜尋的字）
fn looks_like_host(s: &str) -> bool {
    if s.is_empty() || s.contains(char::is_whitespace) || s.starts_with('.') {
        return false;
    }
    let host = s.split(['/', '?', '#']).next().unwrap_or("");
    let host = host.rsplit('@').next().unwrap_or("");
    let name = host.split(':').next().unwrap_or("");
    if name.is_empty() || name.ends_with('.') {
        return false;
    }
    name == "localhost" || (name.contains('.') && name.chars().all(|c| c.is_alphanumeric() || c == '.' || c == '-' || c == '_'))
}

/// 網址列輸入的文字要去哪裡：網址、內部頁面（chrome://…）或搜尋。回傳 (網址, 是不是搜尋)
pub fn resolve_input(input: &str, e: &Engine) -> Option<(String, bool)> {
    let s = input.trim();
    if s.is_empty() {
        return None;
    }
    if let Some(v) = canon_virtual(s) {
        return Some((v, false));
    }
    if let Some(inner) = s.strip_prefix("view-source:") {
        if let Some((u, false)) = resolve_input(inner, e) {
            if u.starts_with("http") {
                return Some((format!("view-source:{u}"), false));
            }
        }
    }
    if let Ok(u) = Url::parse(s) {
        if matches!(u.scheme(), "http" | "https") && u.host_str().is_some() {
            return Some((u.to_string(), false));
        }
    }
    if looks_like_host(s) {
        // localhost、IP、有指定連接埠的本機位址用 http，其他用 https
        let host = s.split(['/', '?', '#']).next().unwrap_or("");
        let name = host.split(':').next().unwrap_or("");
        let local = name == "localhost" || name.parse::<std::net::Ipv4Addr>().is_ok();
        let scheme = if local { "http" } else { "https" };
        if let Ok(u) = Url::parse(&format!("{scheme}://{s}")) {
            if u.host_str().is_some() {
                return Some((u.to_string(), false));
            }
        }
    }
    Some((search_url(e, s), true))
}

/* ---------------- 網址的顯示 ---------------- */

/// 把網址裡編碼過的中文等文字還原（%E6%A5%93 → 楓）；英數符號的編碼（%20、%2F…）、
/// 壞掉的編碼、看不見的字元都維持原樣
pub fn decode_unicode(s: &str) -> String {
    let b = s.as_bytes();
    let hex = |c: u8| (c as char).to_digit(16).map(|d| d as u8);
    let mut out: Vec<u8> = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] != b'%' {
            out.push(b[i]);
            i += 1;
            continue;
        }
        // 連續的 %XX 一起看
        let mut bytes: Vec<u8> = Vec::new();
        let mut j = i;
        while j + 3 <= b.len() && b[j] == b'%' {
            match (hex(b[j + 1]), hex(b[j + 2])) {
                (Some(h), Some(l)) => {
                    bytes.push(h * 16 + l);
                    j += 3;
                }
                _ => break,
            }
        }
        if bytes.is_empty() {
            out.push(b'%');
            i += 1;
            continue;
        }
        let mut k = 0;
        while k < bytes.len() {
            let lead = bytes[k];
            let len = if lead >= 0xF0 { 4 } else if lead >= 0xE0 { 3 } else if lead >= 0xC0 { 2 } else { 1 };
            let end = k + len;
            let decoded = if len > 1 && end <= bytes.len() {
                std::str::from_utf8(&bytes[k..end]).ok().filter(|t| t.chars().all(char::is_alphanumeric))
            } else {
                None
            };
            match decoded {
                Some(t) => {
                    out.extend_from_slice(t.as_bytes());
                    k = end;
                }
                None => {
                    // 這個位元組維持原本的 %XX
                    out.extend_from_slice(&b[i + 3 * k..i + 3 * k + 3]);
                    k += 1;
                }
            }
        }
        i = j;
    }
    String::from_utf8(out).unwrap_or_else(|_| s.to_string())
}

/// 網址列、建議清單顯示的樣子：不顯示 https:// 與 www.，只有網域時不顯示最後的 /
pub fn display_url(u: &str) -> String {
    let s = u.trim();
    if is_newtab(s) {
        return String::new();
    }
    let rest = s.strip_prefix("https://").or_else(|| s.strip_prefix("http://")).unwrap_or(s);
    if rest.len() == s.len() {
        return s.to_string();
    }
    let rest = rest.strip_prefix("www.").unwrap_or(rest);
    let rest = if rest.ends_with('/') && rest.matches('/').count() == 1 { &rest[..rest.len() - 1] } else { rest };
    decode_unicode(rest)
}

pub fn host_of(u: &str) -> String {
    Url::parse(u).ok().and_then(|x| x.host_str().map(|h| h.to_lowercase())).unwrap_or_default()
}

/* ---------------- 設定 ---------------- */

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Settings {
    /// 搜尋引擎（google / bing / yahoo / duckduckgo）
    pub engine: String,
    /// 在網址列打字時顯示搜尋引擎的建議（打的字會送給搜尋引擎，跟 Chrome 一樣）
    pub suggest: bool,
    /// 工具列顯示「首頁」按鈕
    pub home_button: bool,
    /// 首頁按鈕要開的網址（空白 = 新分頁）
    pub home_url: String,
    /// 顯示書籤列
    pub show_bm: bool,
    /// 打開視窗時：restore = 繼續上次的分頁；newtab = 開新分頁
    pub startup: String,
    /// 下載前先問要存在哪裡
    pub ask_download: bool,
    /// 下載的位置（空白 = Windows 的「下載」資料夾）
    pub download_dir: String,
    /// 詢問是否儲存密碼、自動登入
    pub save_passwords: bool,
    /// 預設的網頁縮放（1.0 = 100%）
    pub default_zoom: f64,
    /// 個別網站的縮放
    pub zoom: HashMap<String, f64>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            engine: "google".into(),
            suggest: true,
            home_button: true,
            home_url: String::new(),
            show_bm: true,
            startup: "restore".into(),
            ask_download: false,
            download_dir: String::new(),
            save_passwords: true,
            default_zoom: 1.0,
            zoom: HashMap::new(),
        }
    }
}

impl Settings {
    /// 這個網址要用的縮放
    pub fn zoom_for(&self, url: &str) -> f64 {
        let d = if (0.25..=5.0).contains(&self.default_zoom) { self.default_zoom } else { 1.0 };
        let key = zoom_key(url);
        if key.is_empty() {
            return d;
        }
        self.zoom.get(&key).copied().filter(|z| (0.25..=5.0).contains(z)).unwrap_or(d)
    }

    /// 記住這個網站的縮放（跟預設一樣就不用記）。有變動回傳 true
    pub fn set_zoom(&mut self, url: &str, z: f64) -> bool {
        let key = zoom_key(url);
        if key.is_empty() || !(0.25..=5.0).contains(&z) {
            return false;
        }
        let d = if (0.25..=5.0).contains(&self.default_zoom) { self.default_zoom } else { 1.0 };
        if (z - d).abs() < 0.005 {
            self.zoom.remove(&key).is_some()
        } else {
            let old = self.zoom.insert(key, z);
            old.is_none_or(|o| (o - z).abs() >= 0.005)
        }
    }
}

/// 縮放是照「網站」記的；內部頁面全部算同一個
pub fn zoom_key(url: &str) -> String {
    if virtual_page(url).is_some() {
        return "chrome".into();
    }
    host_of(url)
}

/* ---------------- 縮放 ---------------- */

/// Chrome 的縮放級距
pub const ZOOMS: [f64; 17] = [0.25, 1.0 / 3.0, 0.5, 2.0 / 3.0, 0.75, 0.8, 0.9, 1.0, 1.1, 1.25, 1.5, 1.75, 2.0, 2.5, 3.0, 4.0, 5.0];

pub fn zoom_step(cur: f64, up: bool) -> f64 {
    if up {
        ZOOMS.iter().copied().find(|z| *z > cur + 0.005).unwrap_or(5.0)
    } else {
        ZOOMS.iter().rev().copied().find(|z| *z < cur - 0.005).unwrap_or(0.25)
    }
}

pub fn zoom_pct(z: f64) -> u32 {
    (z * 100.0).round().clamp(25.0, 500.0) as u32
}

/* ---------------- 鍵盤快捷鍵 ---------------- */

/// 瀏覽器的快捷鍵（Windows 的按鍵代碼）→ 指令。回傳 None 的交給網頁或 WebView2 自己處理
/// （重新整理、尋找、列印、上一頁／下一頁、開發人員工具這些 WebView2 本來就會做）
pub fn shortcut(vk: u32, ctrl: bool, shift: bool, alt: bool) -> Option<&'static str> {
    const TAB: u32 = 0x09;
    const PGUP: u32 = 0x21;
    const PGDN: u32 = 0x22;
    const HOME: u32 = 0x24;
    const DEL: u32 = 0x2E;
    const F4: u32 = 0x73;
    const F6: u32 = 0x75;
    const F10: u32 = 0x79;
    const F11: u32 = 0x7A;
    const ESC: u32 = 0x1B;
    let key = |c: char| vk == c as u32;
    match (ctrl, shift, alt) {
        (true, false, false) => match vk {
            _ if key('T') => Some("tab.new"),
            _ if key('N') => Some("win.new"),
            _ if key('W') => Some("tab.close"),
            F4 => Some("tab.close"),
            TAB | PGDN => Some("tab.next"),
            PGUP => Some("tab.prev"),
            0x31 | 0x61 => Some("tab.select:1"),
            0x32 | 0x62 => Some("tab.select:2"),
            0x33 | 0x63 => Some("tab.select:3"),
            0x34 | 0x64 => Some("tab.select:4"),
            0x35 | 0x65 => Some("tab.select:5"),
            0x36 | 0x66 => Some("tab.select:6"),
            0x37 | 0x67 => Some("tab.select:7"),
            0x38 | 0x68 => Some("tab.select:8"),
            0x39 | 0x69 => Some("tab.last"),
            _ if key('L') => Some("omni.focus"),
            _ if key('K') || key('E') => Some("omni.search"),
            _ if key('D') => Some("bm.add"),
            _ if key('H') => Some("page.history"),
            _ if key('J') => Some("page.downloads"),
            _ if key('U') => Some("source"),
            // Ctrl + 加號／減號／0（主鍵盤與數字鍵盤）
            0xBB | 0x6B => Some("zoom.in"),
            0xBD | 0x6D => Some("zoom.out"),
            0x30 | 0x60 => Some("zoom.reset"),
            _ => None,
        },
        (true, true, false) => match vk {
            _ if key('N') => Some("win.incognito"),
            _ if key('W') => Some("win.close"),
            _ if key('T') => Some("tab.reopen"),
            TAB => Some("tab.prev"),
            _ if key('B') => Some("bm.bar"),
            _ if key('A') => Some("tab.search"),
            _ if key('O') => Some("bm.manager"),
            DEL => Some("page.cleardata"),
            0xBB => Some("zoom.in"),
            _ => None,
        },
        (false, false, true) => match vk {
            _ if key('D') => Some("omni.focus"),
            HOME => Some("nav.home"),
            F4 => Some("win.close"),
            _ if key('F') || key('E') => Some("menu.main"),
            _ => None,
        },
        (false, false, false) => match vk {
            F6 => Some("omni.focus"),
            F10 => Some("menu.main"),
            F11 => Some("win.fullscreen"),
            _ => None,
        },
        (false, true, false) => match vk {
            ESC => Some("taskmgr"),
            _ => None,
        },
        _ => None,
    }
}

/// 工具列（程式自己的頁面）有鍵盤時的快捷鍵：除了上面那些，網頁會自己處理的（重新整理、尋找、列印…）
/// 也要轉給目前的網頁，不然會作用在工具列自己身上
pub fn shortcut_bar(vk: u32, ctrl: bool, shift: bool, alt: bool) -> Option<&'static str> {
    if let Some(c) = shortcut(vk, ctrl, shift, alt) {
        return Some(c);
    }
    const LEFT: u32 = 0x25;
    const RIGHT: u32 = 0x27;
    const F3: u32 = 0x72;
    const F5: u32 = 0x74;
    const F12: u32 = 0x7B;
    let key = |c: char| vk == c as u32;
    match (ctrl, shift, alt) {
        (true, false, false) => match vk {
            _ if key('R') => Some("nav.reload"),
            F5 => Some("nav.reload"),
            _ if key('F') || key('G') => Some("find"),
            _ if key('P') => Some("print"),
            _ if key('S') => Some("save"),
            _ if key('O') => Some("noop"),
            _ => None,
        },
        (true, true, false) => match vk {
            _ if key('R') => Some("nav.reload"),
            _ if key('I') || key('J') || key('C') => Some("noop"),
            _ if key('P') || key('S') || key('G') => Some("noop"),
            _ => None,
        },
        (false, false, true) => match vk {
            LEFT => Some("nav.back"),
            RIGHT => Some("nav.forward"),
            _ => None,
        },
        (false, false, false) => match vk {
            F5 => Some("nav.reload"),
            F3 => Some("find"),
            F12 => Some("noop"),
            _ => None,
        },
        (false, true, false) => match vk {
            F5 => Some("nav.reload"),
            F3 => Some("find"),
            _ => None,
        },
        _ => None,
    }
}

/// 按住不放時不要一直重複的指令（關分頁、開視窗這類）
pub fn no_repeat(cmd: &str) -> bool {
    !(cmd.starts_with("tab.next") || cmd.starts_with("tab.prev") || cmd.starts_with("zoom."))
}

/* ---------------- 歷史記錄 ---------------- */

/// 一次造訪：網址、標題、時間（Unix 毫秒）
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Visit {
    pub u: String,
    pub t: String,
    pub at: u64,
}

#[derive(Debug, Clone, Default)]
pub struct Stat {
    pub title: String,
    pub count: u32,
    pub last: u64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct HistFile {
    pub visits: Vec<Visit>,
    /// 網站 → 圖示網址（逛過的網站都記，建議清單、新分頁、歷史記錄頁用）
    pub icons: HashMap<String, String>,
}

/// 最多留幾筆造訪記錄
pub const MAX_VISITS: usize = 20_000;
const MAX_ICONS: usize = 3_000;

#[derive(Debug, Default)]
pub struct History {
    /// 舊 → 新
    pub visits: Vec<Visit>,
    pub icons: HashMap<String, String>,
    index: HashMap<String, Stat>,
}

/// 同一個網頁：不管結尾的 / 與 # 後面
pub fn norm_url(u: &str) -> &str {
    let u = u.split('#').next().unwrap_or(u);
    u.trim_end_matches('/')
}

impl History {
    pub fn from_file(f: HistFile) -> Self {
        let mut h = History { visits: f.visits, icons: f.icons, index: HashMap::new() };
        h.visits.retain(|v| !v.u.is_empty());
        h.rebuild();
        h
    }

    pub fn to_file(&self) -> HistFile {
        HistFile { visits: self.visits.clone(), icons: self.icons.clone() }
    }

    fn rebuild(&mut self) {
        self.index.clear();
        for v in &self.visits {
            let s = self.index.entry(norm_url(&v.u).to_string()).or_default();
            s.count += 1;
            if v.at >= s.last {
                s.last = v.at;
                if !v.t.is_empty() {
                    s.title = v.t.clone();
                }
            }
        }
    }

    pub fn stat(&self, url: &str) -> Option<&Stat> {
        self.index.get(norm_url(url))
    }

    pub fn len(&self) -> usize {
        self.visits.len()
    }

    pub fn is_empty(&self) -> bool {
        self.visits.is_empty()
    }

    /// 記一次造訪。剛剛才記過同一個網址（重新整理、同一頁跳轉）就只更新時間
    pub fn add(&mut self, url: &str, title: &str, now: u64) {
        if url.is_empty() {
            return;
        }
        if let Some(last) = self.visits.last_mut() {
            if norm_url(&last.u) == norm_url(url) && now.saturating_sub(last.at) < 30_000 {
                last.at = now;
                if !title.is_empty() {
                    last.t = title.to_string();
                }
                if let Some(s) = self.index.get_mut(norm_url(url)) {
                    s.last = now;
                    if !title.is_empty() {
                        s.title = title.to_string();
                    }
                }
                return;
            }
        }
        self.visits.push(Visit { u: url.to_string(), t: title.to_string(), at: now });
        let s = self.index.entry(norm_url(url).to_string()).or_default();
        s.count += 1;
        s.last = now;
        if !title.is_empty() {
            s.title = title.to_string();
        }
        if self.visits.len() > MAX_VISITS + 500 {
            let cut = self.visits.len() - MAX_VISITS;
            self.visits.drain(..cut);
            self.rebuild();
        }
    }

    /// 網頁的標題出來了：補到最近那次造訪上。有變動回傳 true
    pub fn set_title(&mut self, url: &str, title: &str) -> bool {
        if title.is_empty() {
            return false;
        }
        let key = norm_url(url).to_string();
        let Some(v) = self.visits.iter_mut().rev().take(50).find(|v| norm_url(&v.u) == key) else { return false };
        if v.t == title {
            return false;
        }
        v.t = title.to_string();
        if let Some(s) = self.index.get_mut(&key) {
            s.title = title.to_string();
        }
        true
    }

    pub fn set_icon(&mut self, host: &str, icon: &str) -> bool {
        if host.is_empty() || icon.is_empty() || self.icons.get(host).map(String::as_str) == Some(icon) {
            return false;
        }
        if self.icons.len() >= MAX_ICONS && !self.icons.contains_key(host) {
            // 太多了：把歷史記錄裡已經沒有的網站清掉
            let alive: std::collections::HashSet<String> = self.visits.iter().map(|v| host_of(&v.u)).collect();
            self.icons.retain(|h, _| alive.contains(h));
            if self.icons.len() >= MAX_ICONS {
                return false;
            }
        }
        self.icons.insert(host.to_string(), icon.to_string());
        true
    }

    pub fn icon_for(&self, url: &str) -> String {
        self.icons.get(&host_of(url)).cloned().unwrap_or_default()
    }

    /// 歷史記錄頁：新 → 舊，可以搜尋（每個關鍵字都要出現在網址或標題裡）。before = 只要比這個時間舊的
    pub fn search(&self, q: &str, before: Option<u64>, limit: usize) -> Vec<Visit> {
        let terms: Vec<String> = q.split_whitespace().map(|t| t.to_lowercase()).collect();
        let mut out = Vec::new();
        for v in self.visits.iter().rev() {
            if before.is_some_and(|b| v.at >= b) {
                continue;
            }
            if !terms.is_empty() {
                let hay = format!("{}\n{}", decode_unicode(&v.u).to_lowercase(), v.t.to_lowercase());
                if !terms.iter().all(|t| hay.contains(t.as_str())) {
                    continue;
                }
            }
            out.push(v.clone());
            if out.len() >= limit {
                break;
            }
        }
        out
    }

    /// 刪掉指定的造訪（網址 + 時間）
    pub fn remove(&mut self, items: &[(String, u64)]) -> usize {
        let before = self.visits.len();
        self.visits.retain(|v| !items.iter().any(|(u, at)| *at == v.at && *u == v.u));
        let n = before - self.visits.len();
        if n > 0 {
            self.rebuild();
        }
        n
    }

    /// 刪掉某個網址的所有記錄
    pub fn remove_url(&mut self, url: &str) -> usize {
        let key = norm_url(url).to_string();
        let before = self.visits.len();
        self.visits.retain(|v| norm_url(&v.u) != key);
        let n = before - self.visits.len();
        if n > 0 {
            self.rebuild();
        }
        n
    }

    /// 清除某個時間之後的記錄（since = 0 就是全部）
    pub fn clear_since(&mut self, since: u64) -> usize {
        let before = self.visits.len();
        self.visits.retain(|v| v.at < since);
        let n = before - self.visits.len();
        if n > 0 {
            self.rebuild();
        }
        if since == 0 {
            self.icons.clear();
        }
        n
    }

    /// 最常去的網站（新分頁的捷徑）：每個網站只留一個最常去的網址
    pub fn top_sites(&self, n: usize) -> Vec<(String, String, u32)> {
        let mut best: HashMap<String, (String, String, u32, u32)> = HashMap::new();
        for (u, s) in &self.index {
            let host = host_of(u);
            if host.is_empty() {
                continue;
            }
            let host = host.strip_prefix("www.").unwrap_or(&host).to_string();
            let e = best.entry(host).or_insert_with(|| (String::new(), String::new(), 0, 0));
            e.3 += s.count;
            // 同一個網站裡挑最常去的；一樣多就挑網址短的（通常是首頁）
            if s.count > e.2 || (s.count == e.2 && (e.0.is_empty() || u.len() < e.0.len())) {
                e.0 = u.clone();
                e.1 = s.title.clone();
                e.2 = s.count;
            }
        }
        let mut list: Vec<(String, String, u32)> = best.into_values().map(|(u, t, _, total)| (u, t, total)).collect();
        list.sort_by(|a, b| b.2.cmp(&a.2).then_with(|| a.0.cmp(&b.0)));
        list.truncate(n);
        list
    }

    pub fn stats(&self) -> impl Iterator<Item = (&String, &Stat)> {
        self.index.iter()
    }
}

/* ---------------- 下載記錄 ---------------- */

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Dl {
    pub id: u64,
    pub url: String,
    /// 存到哪裡（完整路徑）
    pub path: String,
    /// 總大小（位元組；不知道就 -1）
    pub total: i64,
    pub received: i64,
    /// progress（下載中）/ done（完成）/ canceled（取消）/ failed（失敗）
    pub state: String,
    /// 開始、結束的時間（Unix 毫秒）
    pub at: u64,
    pub end: u64,
    /// 檔案已經被刪除或移走
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub gone: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Downloads {
    pub list: Vec<Dl>,
    pub next: u64,
}

const MAX_DL: usize = 500;

impl Downloads {
    /// 上次沒下載完的（程式被關掉）當作取消
    pub fn fix_loaded(&mut self) {
        for d in &mut self.list {
            if d.state == "progress" {
                d.state = "canceled".into();
            }
        }
        self.next = self.next.max(self.list.iter().map(|d| d.id).max().unwrap_or(0));
    }

    pub fn start(&mut self, url: &str, path: &str, total: i64, now: u64) -> u64 {
        self.next += 1;
        let id = self.next;
        self.list.push(Dl { id, url: url.to_string(), path: path.to_string(), total, received: 0, state: "progress".into(), at: now, end: 0, gone: false });
        if self.list.len() > MAX_DL {
            let cut = self.list.len() - MAX_DL;
            self.list.drain(..cut);
        }
        id
    }

    pub fn get_mut(&mut self, id: u64) -> Option<&mut Dl> {
        self.list.iter_mut().find(|d| d.id == id)
    }

    pub fn active(&self) -> usize {
        self.list.iter().filter(|d| d.state == "progress").count()
    }

    /// 下載頁：新 → 舊，可以搜尋檔名或網址
    pub fn search(&self, q: &str) -> Vec<Dl> {
        let q = q.trim().to_lowercase();
        self.list
            .iter()
            .rev()
            .filter(|d| q.is_empty() || d.path.to_lowercase().contains(&q) || d.url.to_lowercase().contains(&q))
            .cloned()
            .collect()
    }

    pub fn remove(&mut self, id: u64) -> bool {
        let n = self.list.len();
        self.list.retain(|d| d.id != id || d.state == "progress");
        self.list.len() != n
    }

    /// 清除記錄（下載中的留著）。since = 0 就是全部
    pub fn clear_since(&mut self, since: u64) -> usize {
        let n = self.list.len();
        self.list.retain(|d| d.state == "progress" || d.at < since);
        n - self.list.len()
    }
}

/// 檔案路徑的檔名
pub fn file_name(path: &str) -> &str {
    path.rsplit(['\\', '/']).next().unwrap_or(path)
}

/* ---------------- 最近關閉的分頁 ---------------- */

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct ClosedTab {
    pub url: String,
    pub title: String,
    pub icon: String,
    /// 原本在哪個視窗、第幾個
    pub win: String,
    pub index: usize,
    pub at: u64,
}

pub const MAX_CLOSED: usize = 25;

pub fn push_closed(list: &mut Vec<ClosedTab>, t: ClosedTab) {
    if t.url.is_empty() || is_newtab(&t.url) {
        return;
    }
    list.push(t);
    if list.len() > MAX_CLOSED {
        let cut = list.len() - MAX_CLOSED;
        list.drain(..cut);
    }
}

/* ---------------- 網址列的建議 ---------------- */

#[derive(Debug, Clone, Default, Serialize, PartialEq)]
pub struct Sug {
    /// url（輸入的網址）/ search（搜尋輸入的字）/ history / bookmark / suggest（搜尋引擎的建議）
    pub kind: &'static str,
    /// 主要文字（網址或搜尋的字）
    pub text: String,
    /// 次要文字（網頁標題、「Google 搜尋」）
    pub desc: String,
    /// 按下去要去的網址
    pub url: String,
    /// 用方向鍵選到時填進網址列的文字
    pub fill: String,
    /// 網站圖示
    pub icon: String,
    /// 只有第一項會有：接在輸入的字後面自動補完的部分
    pub inline: String,
}

fn search_sug(e: &Engine, text: &str, kind: &'static str) -> Sug {
    Sug { kind, text: text.to_string(), desc: format!("{} 搜尋", e.name), url: search_url(e, text), fill: text.to_string(), ..Sug::default() }
}

/// 輸入的字去掉 https://、www.，只有網域時去掉最後的 /（比對網域用）
fn strip_input(s: &str) -> String {
    let s = s.trim().to_lowercase();
    let s = s.strip_prefix("https://").or_else(|| s.strip_prefix("http://")).unwrap_or(&s);
    let s = s.strip_prefix("www.").unwrap_or(s);
    let s = if s.ends_with('/') && s.matches('/').count() == 1 { &s[..s.len() - 1] } else { s };
    s.to_string()
}

struct Cand {
    url: String,
    title: String,
    count: u32,
    last: u64,
    bookmark: bool,
}

/// 不靠網路的建議：輸入的網址／搜尋、歷史記錄、書籤。allow_inline = 可以自動補完（刪字的時候不要）
pub fn suggest_local(input: &str, hist: &History, bookmarks: &[(String, String)], e: &Engine, now: u64, allow_inline: bool, max: usize) -> Vec<Sug> {
    let text = input.trim();
    if text.is_empty() {
        return Vec::new();
    }
    let Some((dest, is_search)) = resolve_input(text, e) else { return Vec::new() };
    let typed = if is_search {
        search_sug(e, text, "search")
    } else {
        Sug { kind: "url", text: display_url(&dest), desc: String::new(), url: dest.clone(), fill: text.to_string(), ..Sug::default() }
    };

    // 候選：歷史記錄 + 書籤（同一個網址合併）
    let mut cands: HashMap<String, Cand> = HashMap::new();
    for (u, s) in hist.stats() {
        cands.insert(u.clone(), Cand { url: u.clone(), title: s.title.clone(), count: s.count, last: s.last, bookmark: false });
    }
    for (title, url) in bookmarks {
        let key = norm_url(url).to_string();
        let c = cands.entry(key.clone()).or_insert_with(|| Cand { url: key, title: String::new(), count: 0, last: 0, bookmark: true });
        c.bookmark = true;
        if c.title.is_empty() {
            c.title = title.clone();
        }
    }

    let needle = strip_input(text);
    let terms: Vec<String> = text.to_lowercase().split_whitespace().map(str::to_string).collect();
    let single = terms.len() == 1 && !needle.is_empty();
    let mut scored: Vec<(i64, bool, Cand)> = Vec::new();
    for (_, c) in cands {
        if !c.url.starts_with("http") {
            continue;
        }
        let disp = display_url(&c.url).to_lowercase();
        let title = c.title.to_lowercase();
        let host = disp.split('/').next().unwrap_or("").to_string();
        let mut score: i64 = 0;
        let mut prefix = false;
        if single && disp.starts_with(&needle) {
            // 網域（或網域＋路徑）的開頭就是輸入的字
            prefix = true;
            score += 1000;
            if disp == host {
                score += 200;
            }
        } else if single && host.split('.').skip(1).any(|part| part.starts_with(&needle)) && host.contains(&needle) {
            // 子網域後面的部分開頭符合（輸入 google 對到 mail.google.com）
            score += 500;
        } else {
            let hay_url = format!("{} {}", disp, c.url.to_lowercase());
            let hit = |t: &String| hay_url.contains(t.as_str()) || title.contains(t.as_str()) || (single && hay_url.contains(&needle));
            if !terms.iter().all(hit) {
                continue;
            }
            let in_title_word = terms.iter().all(|t| title.split(|ch: char| !ch.is_alphanumeric()).any(|w| w.starts_with(t.as_str())));
            score += if in_title_word { 320 } else if terms.iter().all(|t| title.contains(t.as_str())) { 220 } else { 180 };
        }
        score += (c.count.min(60) as i64) * 6;
        let age = now.saturating_sub(c.last);
        if c.last > 0 {
            score += if age < 86_400_000 { 120 } else if age < 7 * 86_400_000 { 60 } else if age < 30 * 86_400_000 { 20 } else { 0 };
        }
        if c.bookmark {
            score += 80;
        }
        score -= (disp.len() as i64) / 8;
        scored.push((score, prefix, c));
    }
    scored.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.2.url.len().cmp(&b.2.url.len())).then_with(|| a.2.url.cmp(&b.2.url)));

    let to_sug = |c: &Cand| {
        let disp = display_url(&c.url);
        Sug {
            kind: if c.bookmark { "bookmark" } else { "history" },
            text: disp.clone(),
            desc: c.title.clone(),
            url: c.url.clone(),
            fill: disp,
            icon: hist.icon_for(&c.url),
            inline: String::new(),
        }
    };

    let mut out: Vec<Sug> = Vec::new();
    // 自動補完：最符合的那個網址開頭就是輸入的字 → 它排第一，輸入的字後面接上剩下的部分
    let mut used_top = false;
    if allow_inline && single && !text.contains(' ') {
        if let Some((_, true, c)) = scored.first() {
            let disp = display_url(&c.url);
            let lower = disp.to_lowercase();
            let typed_l = text.to_lowercase();
            // 輸入的字（可能含 https://、www.）要能直接接上
            let needle_len = needle.chars().count();
            if lower.starts_with(&needle) && disp.chars().count() > needle_len && !typed_l.ends_with('/') {
                let mut s = to_sug(c);
                s.inline = disp.chars().skip(needle_len).collect();
                s.fill = format!("{text}{}", s.inline);
                out.push(s);
                used_top = true;
            }
        }
    }
    out.push(typed);
    let limit = max.min(6);
    for (i, (_, _, c)) in scored.iter().enumerate() {
        if out.len() >= limit {
            break;
        }
        if used_top && i == 0 {
            continue;
        }
        if norm_url(&c.url) == norm_url(&dest) && !is_search {
            // 跟「輸入的網址」是同一個：把標題補上去就好
            if let Some(t) = out.iter_mut().find(|s| s.kind == "url") {
                if t.desc.is_empty() {
                    t.desc = c.title.clone();
                    t.icon = hist.icon_for(&c.url);
                }
            }
            continue;
        }
        out.push(to_sug(c));
    }
    out
}

/// 把搜尋引擎的建議接到後面（重複的不要），最多 max 項
pub fn merge_remote(mut list: Vec<Sug>, remote: &[String], e: &Engine, max: usize) -> Vec<Sug> {
    for r in remote {
        if list.len() >= max {
            break;
        }
        let dup = list.iter().any(|s| (s.kind == "search" || s.kind == "suggest") && s.text.eq_ignore_ascii_case(r));
        if dup {
            continue;
        }
        list.push(search_sug(e, r, "suggest"));
    }
    list
}

/// 這段字要不要拿去問搜尋引擎的建議（像網址、內部頁面、太長的就不問）
pub fn wants_remote(input: &str) -> bool {
    let s = input.trim();
    !(s.is_empty() || s.len() > 200 || s.contains("://") || s.starts_with("chrome:") || s.starts_with("view-source:") || s.contains('/') || s.contains('@'))
}

/* ---------------- 網頁右鍵選單 ---------------- */

#[derive(Debug, Clone, PartialEq)]
pub enum CtxAct {
    /// 執行 WebView2 的指令
    Native(i32),
    OpenTab(String),
    OpenWindow(String),
    OpenIncognito(String),
    Search(String),
    ViewSource(String),
    None,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CtxEntry {
    pub title: String,
    pub shortcut: String,
    pub enabled: bool,
    pub checked: bool,
    pub sep: bool,
    pub act: CtxAct,
    pub children: Vec<CtxEntry>,
}

impl CtxEntry {
    fn sep() -> Self {
        CtxEntry { title: String::new(), shortcut: String::new(), enabled: false, checked: false, sep: true, act: CtxAct::None, children: Vec::new() }
    }
    fn custom(title: impl Into<String>, shortcut: &str, act: CtxAct) -> Self {
        CtxEntry { title: title.into(), shortcut: shortcut.to_string(), enabled: true, checked: false, sep: false, act, children: Vec::new() }
    }
}

/// WebView2 的項目名稱 → Chrome 的說法與快捷鍵（沒列到的沿用 WebView2 自己的文字）
fn chrome_label(name: &str, target: &CtxTarget) -> Option<(&'static str, &'static str)> {
    let media = if target.kind == 3 { "音訊" } else { "影片" };
    Some(match name {
        "back" => ("上一頁", "Alt+向左鍵"),
        "forward" => ("下一頁", "Alt+向右鍵"),
        "reload" => ("重新載入", "Ctrl+R"),
        "saveAs" => ("另存新檔…", "Ctrl+S"),
        "print" => ("列印…", "Ctrl+P"),
        "inspectElement" => ("檢查", ""),
        "undo" => ("復原", "Ctrl+Z"),
        "redo" => ("重做", "Ctrl+Shift+Z"),
        "cut" => ("剪下", "Ctrl+X"),
        "copy" => ("複製", "Ctrl+C"),
        "paste" => ("貼上", "Ctrl+V"),
        "pasteAndMatchStyle" => ("以純文字貼上", "Ctrl+Shift+V"),
        "selectAll" => ("全選", "Ctrl+A"),
        "saveLinkAs" => ("另存連結為…", ""),
        "copyLinkLocation" => ("複製連結網址", ""),
        "saveImageAs" => ("另存圖片…", ""),
        "copyImage" => ("複製圖片", ""),
        "copyImageLocation" => ("複製圖片位址", ""),
        "saveMediaAs" => (if media == "音訊" { "另存音訊…" } else { "另存影片…" }, ""),
        "copyMediaLocation" | "copyAudioLocation" | "copyVideoLocation" => (if media == "音訊" { "複製音訊位址" } else { "複製影片位址" }, ""),
        "copyVideoFrame" => ("複製影片畫面", ""),
        "loop" => ("循環播放", ""),
        "showAllControls" | "controls" => ("顯示控制項", ""),
        "pictureInPicture" => ("子母畫面", ""),
        "emoji" => ("表情符號", "Win+句號"),
        "copyLinkToHighlight" => ("複製醒目顯示文字的連結", ""),
        _ => return None,
    })
}

/// Edge 才有、Chrome 沒有的項目（不顯示）；開新視窗的連結項目改用自己的
fn hidden(name: &str) -> bool {
    let n = name.to_lowercase();
    matches!(
        n.as_str(),
        "webcapture" | "webselect" | "share" | "createqrcode" | "sendtabtoself" | "readaloud" | "openinimmersivereader" | "addtocollections" | "openlinkinnewwindow"
            | "openlinkinsplitwindow" | "openlinkininprivatewindow" | "openimageinnewwindow" | "openmediainnewwindow" | "magnify" | "visualsearch"
    ) || n.starts_with("search")
        || n.starts_with("sidebar")
        || n.contains("copilot")
        || n.contains("bing")
}

fn convert(items: &[NativeItem], target: &CtxTarget) -> Vec<CtxEntry> {
    let mut out = Vec::new();
    for it in items {
        if it.kind == 3 {
            out.push(CtxEntry::sep());
            continue;
        }
        if hidden(&it.name) {
            continue;
        }
        let (title, shortcut) = match chrome_label(&it.name, target) {
            Some((t, s)) => (t.to_string(), s.to_string()),
            None => (it.label.replace('&', ""), it.shortcut.clone()),
        };
        if title.trim().is_empty() {
            continue;
        }
        let children = if it.kind == 4 { convert(&it.children, target) } else { Vec::new() };
        if it.kind == 4 && children.iter().all(|c| c.sep) {
            continue;
        }
        out.push(CtxEntry { title, shortcut, enabled: it.enabled, checked: it.checked, sep: false, act: CtxAct::Native(it.id), children });
    }
    out
}

/// 分隔線不要在最前面、最後面，也不要連續兩條
fn tidy(list: Vec<CtxEntry>) -> Vec<CtxEntry> {
    let mut out: Vec<CtxEntry> = Vec::new();
    for mut e in list {
        if e.sep && out.last().is_none_or(|l| l.sep) {
            continue;
        }
        if !e.children.is_empty() {
            e.children = tidy(std::mem::take(&mut e.children));
        }
        out.push(e);
    }
    while out.last().is_some_and(|l| l.sep) {
        out.pop();
    }
    out
}

fn short(s: &str, n: usize) -> String {
    let t: String = s.split_whitespace().collect::<Vec<_>>().join(" ");
    if t.chars().count() > n {
        format!("{}…", t.chars().take(n).collect::<String>())
    } else {
        t
    }
}

fn is_web(u: &str) -> bool {
    u.starts_with("http://") || u.starts_with("https://")
}

/// 用 WebView2 給的項目排出 Chrome 樣子的右鍵選單：文字改成 Chrome 的說法、拿掉 Edge 專用的、
/// 補上「在新分頁中開啟連結」「透過 Google 搜尋」「檢視網頁原始碼」這些
pub fn build_context(target: &CtxTarget, items: &[NativeItem], e: &Engine, incognito: bool) -> Vec<CtxEntry> {
    let mut list = convert(items, target);
    let pos = |list: &Vec<CtxEntry>, names: &[&str]| -> Option<usize> {
        // 用轉換後的標題找（名稱已經換成 Chrome 的說法）
        list.iter().position(|x| names.iter().any(|n| x.title == *n))
    };

    // 圖片：最前面加「在新分頁中開啟圖片」
    if target.kind == 1 && is_web(&target.src) {
        let at = pos(&list, &["另存圖片…"]).unwrap_or(0);
        list.insert(at, CtxEntry::custom("在新分頁中開啟圖片", "", CtxAct::OpenTab(target.src.clone())));
    }
    // 影片、音訊：加「在新分頁中開啟」
    if (target.kind == 3 || target.kind == 4) && is_web(&target.src) {
        let title = if target.kind == 3 { "在新分頁中開啟音訊" } else { "在新分頁中開啟影片" };
        let at = pos(&list, &["另存影片…", "另存音訊…"]).unwrap_or(0);
        list.insert(at, CtxEntry::custom(title, "", CtxAct::OpenTab(target.src.clone())));
    }
    // 選取的文字：複製後面加「透過 Google 搜尋」
    let sel = target.selection.trim();
    if !sel.is_empty() {
        let entry = CtxEntry::custom(format!("透過 {} 搜尋「{}」", e.name, short(sel, 24)), "", CtxAct::Search(sel.to_string()));
        match pos(&list, &["複製"]) {
            Some(i) => list.insert(i + 1, entry),
            None => list.insert(0, entry),
        }
    }
    // 連結：最前面加開啟的三項
    if is_web(&target.link) {
        let mut head = vec![
            CtxEntry::custom("在新分頁中開啟連結", "", CtxAct::OpenTab(target.link.clone())),
            CtxEntry::custom("在新視窗中開啟連結", "", CtxAct::OpenWindow(target.link.clone())),
        ];
        if !incognito {
            head.push(CtxEntry::custom("在無痕式視窗中開啟連結", "", CtxAct::OpenIncognito(target.link.clone())));
        }
        head.push(CtxEntry::sep());
        head.extend(list);
        list = head;
    }
    // 一般網頁（沒點到連結、圖片、文字、輸入框）：「檢查」前面加「檢視網頁原始碼」
    if target.kind == 0 && !target.editable && target.link.is_empty() && sel.is_empty() && is_web(&target.page) {
        let entry = CtxEntry::custom("檢視網頁原始碼", "Ctrl+U", CtxAct::ViewSource(target.page.clone()));
        match pos(&list, &["檢查"]) {
            Some(i) => list.insert(i, entry),
            None => {
                list.push(CtxEntry::sep());
                list.push(entry);
            }
        }
    }
    tidy(list)
}

/* ---------------- 選單小視窗的位置 ---------------- */

/// 小視窗要擺在哪裡（螢幕實體像素）
#[derive(Clone)]
pub enum Place {
    /// 貼在按鈕下面（左、上、右、下）；right = 靠按鈕的右邊對齊
    Below { rect: (i32, i32, i32, i32), right: bool },
    Cursor(i32, i32),
    /// 上一層的某一項旁邊：上一層的範圍、那一項的上緣。
    /// left = 上一層自己是往左開的：這一層也優先往左，一路往同一個方向展開，才不會蓋到更上面那幾層
    Side { parent: (i32, i32, i32, i32), top: i32, left: bool },
}

/// 算出小視窗的範圍：超出螢幕就往上或往另一邊擺
pub fn place_rect(place: &Place, w: i32, h: i32, wa: Option<(i32, i32, i32, i32)>, overlap: i32) -> (i32, i32, i32, i32) {
    let (wl, wt, wr, wb) = wa.unwrap_or((i32::MIN / 4, i32::MIN / 4, i32::MAX / 4, i32::MAX / 4));
    let h = if wa.is_some() { h.min((wb - wt - 8).max(60)) } else { h };
    let w = if wa.is_some() { w.min((wr - wl - 8).max(60)) } else { w };
    let (mut x, mut y);
    match place {
        Place::Below { rect: (l, t, r, b), right } => {
            x = if *right { r - w } else { *l };
            y = *b;
            if y + h > wb && t - h >= wt {
                y = t - h;
            }
        }
        Place::Cursor(cx, cy) => {
            x = *cx;
            y = *cy;
            if x + w > wr {
                x = cx - w;
            }
            if y + h > wb {
                y = if cy - h >= wt { cy - h } else { wb - h };
            }
        }
        Place::Side { parent: (pl, _, pr, _), top, left } => {
            let (right_x, left_x) = (pr - overlap, pl - w + overlap);
            x = if *left {
                // 左邊放不下才換右邊
                if left_x >= wl { left_x } else { right_x }
            } else if right_x + w > wr {
                left_x
            } else {
                right_x
            };
            y = *top;
        }
    }
    x = x.min(wr - w).max(wl);
    y = y.min(wb - h).max(wt);
    (x, y, x + w, y + h)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn popup_placement() {
        let wa = Some((0, 0, 1000, 800));
        // 按鈕下面，靠左對齊
        assert_eq!(place_rect(&Place::Below { rect: (100, 50, 140, 80), right: false }, 200, 300, wa, 3), (100, 80, 300, 380));
        // 靠右對齊（⋮ 選單）
        assert_eq!(place_rect(&Place::Below { rect: (900, 50, 940, 80), right: true }, 200, 300, wa, 3), (740, 80, 940, 380));
        // 下面放不下就放上面
        assert_eq!(place_rect(&Place::Below { rect: (100, 700, 140, 730), right: false }, 200, 300, wa, 3), (100, 400, 300, 700));
        // 超出右邊就往左靠
        assert_eq!(place_rect(&Place::Below { rect: (900, 50, 940, 80), right: false }, 200, 300, wa, 3).0, 800);
        // 滑鼠位置；右邊、下面放不下就往左、往上
        assert_eq!(place_rect(&Place::Cursor(10, 10), 200, 300, wa, 3), (10, 10, 210, 310));
        assert_eq!(place_rect(&Place::Cursor(950, 700), 200, 300, wa, 3), (750, 400, 950, 700));
        // 下一層：在上一層右邊（重疊一點）；放不下就放左邊
        assert_eq!(place_rect(&Place::Side { parent: (100, 100, 300, 500), top: 150, left: false }, 200, 300, wa, 3), (297, 150, 497, 450));
        assert_eq!(place_rect(&Place::Side { parent: (700, 100, 950, 500), top: 150, left: false }, 200, 300, wa, 3), (503, 150, 703, 450));
        // 上一層是往左開的：右邊雖然放得下，還是繼續往左（不蓋到更上面那一層）
        assert_eq!(place_rect(&Place::Side { parent: (503, 150, 703, 450), top: 200, left: true }, 200, 100, wa, 3), (306, 200, 506, 300));
        // 往左放不下了：換右邊
        assert_eq!(place_rect(&Place::Side { parent: (50, 150, 250, 450), top: 200, left: true }, 200, 100, wa, 3), (247, 200, 447, 300));
        // 比螢幕還高：縮到螢幕的高度
        let r = place_rect(&Place::Cursor(10, 10), 200, 2000, wa, 3);
        assert_eq!((r.1, r.3), (8, 800));
    }


    #[test]
    fn internal_urls_round_trip() {
        let l = virtual_to_local("chrome://history/?q=楓之谷").unwrap();
        assert!(l.as_str().starts_with(ui_base()));
        assert!(l.as_str().contains("web-ui.html?p=history&q="));
        assert_eq!(local_to_virtual(&l).unwrap(), Url::parse("chrome://history/?q=楓之谷").unwrap().to_string());
        let s = virtual_to_local("chrome://settings/clearBrowserData").unwrap();
        assert_eq!(local_to_virtual(&s).unwrap(), "chrome://settings/clearBrowserData");
        assert_eq!(local_to_virtual(&virtual_to_local("chrome://newtab").unwrap()).unwrap(), NEWTAB);
        assert_eq!(virtual_page("chrome://new-tab-page/"), Some("newtab"));
        assert!(virtual_to_local("chrome://flags").is_none());
        assert!(virtual_to_local("https://www.google.com/").is_none());
        // 不是內部頁面的本機頁面（例如主程式的 index.html）不算
        assert!(local_to_virtual(&Url::parse(&format!("{}index.html", ui_base())).unwrap()).is_none());
        assert!(local_to_virtual(&Url::parse(&format!("{}web-ui.html?p=evil", ui_base())).unwrap()).is_none());
        assert!(local_to_virtual(&Url::parse("https://example.com/web-ui.html?p=history").unwrap()).is_none());
        assert!(is_app_url(&Url::parse(&format!("{}index.html", ui_base())).unwrap()));
        assert!(!is_app_url(&Url::parse("https://tauri.localhost.evil.com/").unwrap()));
        assert!(is_newtab("chrome://newtab/"));
        assert!(!is_newtab("chrome://history/"));
    }

    #[test]
    fn input_resolution() {
        let g = engine("google");
        let go = |s: &str| resolve_input(s, g).unwrap();
        assert_eq!(go("youtube.com"), ("https://youtube.com/".to_string(), false));
        assert_eq!(go("https://www.twitch.tv/x"), ("https://www.twitch.tv/x".to_string(), false));
        assert_eq!(go("localhost:3000"), ("http://localhost:3000/".to_string(), false));
        assert_eq!(go("192.168.1.1/admin"), ("http://192.168.1.1/admin".to_string(), false));
        assert_eq!(go("chrome://history"), ("chrome://history/".to_string(), false));
        assert_eq!(go("view-source:example.com"), ("view-source:https://example.com/".to_string(), false));
        let (u, s) = go("楓之谷 攻略");
        assert!(s && u.starts_with("https://www.google.com/search?q="));
        // 不是網頁的網址當作搜尋
        assert!(go("file:///c:/x").1);
        assert!(go("javascript:alert(1)").1);
        assert!(go("什麼.是").0.starts_with("https://"));
        assert!(go("a b.com").1);
        assert!(resolve_input("   ", g).is_none());
        let (u, s) = resolve_input("rust lang", engine("duckduckgo")).unwrap();
        assert!(s && u == "https://duckduckgo.com/?q=rust+lang");
        assert_eq!(engine("不存在").id, "google");
    }

    #[test]
    fn url_display() {
        assert_eq!(display_url("https://www.youtube.com/"), "youtube.com");
        assert_eq!(display_url("https://openapi.nexon.com/my-application/355707/"), "openapi.nexon.com/my-application/355707/");
        assert_eq!(display_url("http://example.com/a?b=1"), "example.com/a?b=1");
        assert_eq!(display_url("chrome://settings/"), "chrome://settings/");
        assert_eq!(display_url("chrome://newtab/"), "");
        assert_eq!(display_url("https://zh.wikipedia.org/wiki/%E6%A5%93%E4%B9%8B%E8%B0%B7"), "zh.wikipedia.org/wiki/楓之谷");
        // 英數符號的編碼不還原；壞掉的編碼原樣留著
        assert_eq!(decode_unicode("a%20b%2Fc"), "a%20b%2Fc");
        assert_eq!(decode_unicode("x%E6%A5y"), "x%E6%A5y");
        assert_eq!(decode_unicode("100%"), "100%");
        assert_eq!(decode_unicode("%E6%A5%93%20%E8%B0%B7"), "楓%20谷");
        assert_eq!(decode_unicode("%zz%E6"), "%zz%E6");
    }

    #[test]
    fn zoom_levels() {
        assert_eq!(zoom_step(1.0, true), 1.1);
        assert_eq!(zoom_step(1.0, false), 0.9);
        assert_eq!(zoom_step(5.0, true), 5.0);
        assert_eq!(zoom_step(0.25, false), 0.25);
        assert_eq!(zoom_step(1.2, true), 1.25);
        assert_eq!(zoom_pct(2.0 / 3.0), 67);
        let mut s = Settings::default();
        assert_eq!(s.zoom_for("https://x.com/a"), 1.0);
        assert!(s.set_zoom("https://x.com/a", 1.25));
        assert!(!s.set_zoom("https://x.com/b", 1.25));
        assert_eq!(s.zoom_for("https://x.com/zzz"), 1.25);
        assert_eq!(s.zoom_for("https://y.com/"), 1.0);
        // 設回預設值就不用記
        assert!(s.set_zoom("https://x.com/", 1.0));
        assert!(s.zoom.is_empty());
        assert!(!s.set_zoom("about:blank", 1.5));
        assert!(s.set_zoom("chrome://history/", 1.1));
        assert_eq!(s.zoom_for("chrome://settings/"), 1.1);
    }

    #[test]
    fn settings_defaults_survive_old_files() {
        let s: Settings = serde_json::from_str("{}").unwrap();
        assert_eq!(s, Settings::default());
        let s: Settings = serde_json::from_str(r#"{"engine":"bing","show_bm":false}"#).unwrap();
        assert_eq!(s.engine, "bing");
        assert!(!s.show_bm && s.suggest && s.home_button);
    }

    #[test]
    fn shortcuts() {
        assert_eq!(shortcut('T' as u32, true, false, false), Some("tab.new"));
        assert_eq!(shortcut('T' as u32, true, true, false), Some("tab.reopen"));
        assert_eq!(shortcut('W' as u32, true, false, false), Some("tab.close"));
        assert_eq!(shortcut(0x09, true, false, false), Some("tab.next"));
        assert_eq!(shortcut(0x09, true, true, false), Some("tab.prev"));
        assert_eq!(shortcut('3' as u32, true, false, false), Some("tab.select:3"));
        assert_eq!(shortcut('9' as u32, true, false, false), Some("tab.last"));
        assert_eq!(shortcut('L' as u32, true, false, false), Some("omni.focus"));
        assert_eq!(shortcut('D' as u32, false, false, true), Some("omni.focus"));
        assert_eq!(shortcut(0x2E, true, true, false), Some("page.cleardata"));
        assert_eq!(shortcut(0x7A, false, false, false), Some("win.fullscreen"));
        assert_eq!(shortcut(0x73, false, false, true), Some("win.close"));
        assert_eq!(shortcut(0x73, true, false, false), Some("tab.close"));
        assert_eq!(shortcut('A' as u32, true, true, false), Some("tab.search"));
        // 交給 WebView2 自己處理的
        assert_eq!(shortcut('R' as u32, true, false, false), None);
        assert_eq!(shortcut('S' as u32, true, false, false), None);
        // 工具列有鍵盤時：重新整理、尋找要轉給網頁
        assert_eq!(shortcut_bar('R' as u32, true, false, false), Some("nav.reload"));
        assert_eq!(shortcut_bar(0x74, false, false, false), Some("nav.reload"));
        assert_eq!(shortcut_bar('F' as u32, true, false, false), Some("find"));
        assert_eq!(shortcut_bar('T' as u32, true, false, false), Some("tab.new"));
        assert_eq!(shortcut_bar('C' as u32, true, false, false), None);
        assert_eq!(shortcut_bar('A' as u32, true, false, false), None);
        assert_eq!(shortcut_bar(0x25, false, false, true), Some("nav.back"));
        assert_eq!(shortcut('F' as u32, true, false, false), None);
        assert_eq!(shortcut('P' as u32, true, false, false), None);
        assert_eq!(shortcut(0x74, false, false, false), None);
        assert_eq!(shortcut('C' as u32, true, false, false), None);
        assert_eq!(shortcut('T' as u32, false, false, false), None);
        assert!(no_repeat("tab.close") && !no_repeat("tab.next") && !no_repeat("zoom.in"));
    }

    fn hist(items: &[(&str, &str, u64)]) -> History {
        let mut h = History::default();
        for (u, t, at) in items {
            h.add(u, t, *at);
        }
        h
    }

    #[test]
    fn history_store() {
        let mut h = hist(&[("https://a.com/", "A", 1_000_000), ("https://b.com/x", "B", 2_000_000), ("https://a.com/", "A2", 3_000_000)]);
        assert_eq!(h.len(), 3);
        assert_eq!(h.stat("https://a.com").unwrap().count, 2);
        assert_eq!(h.stat("https://a.com/").unwrap().title, "A2");
        // 30 秒內同一個網址不重複記
        h.add("https://a.com/#top", "", 3_010_000);
        assert_eq!(h.len(), 3);
        assert_eq!(h.visits.last().unwrap().at, 3_010_000);
        assert_eq!(h.visits.last().unwrap().t, "A2");
        // 標題晚一點才出來
        h.add("https://c.com/", "", 4_000_000);
        assert!(h.set_title("https://c.com/", "C 的標題"));
        assert!(!h.set_title("https://c.com/", "C 的標題"));
        assert_eq!(h.stat("https://c.com/").unwrap().title, "C 的標題");
        // 搜尋：新 → 舊
        let all = h.search("", None, 10);
        assert_eq!(all.iter().map(|v| v.u.as_str()).collect::<Vec<_>>(), ["https://c.com/", "https://a.com/", "https://b.com/x", "https://a.com/"]);
        assert_eq!(h.search("標題", None, 10).len(), 1);
        assert_eq!(h.search("a.com", None, 10).len(), 2);
        assert_eq!(h.search("", Some(3_010_000), 10).len(), 2);
        assert_eq!(h.search("", None, 1).len(), 1);
        // 刪除
        assert_eq!(h.remove(&[("https://b.com/x".to_string(), 2_000_000)]), 1);
        assert!(h.stat("https://b.com/x").is_none());
        assert_eq!(h.remove_url("https://a.com"), 2);
        assert_eq!(h.len(), 1);
        assert_eq!(h.clear_since(0), 1);
        assert!(h.is_empty());
        // 存檔再讀回來
        let h = hist(&[("https://a.com/", "A", 1), ("https://a.com/", "A", 100_000)]);
        let back = History::from_file(serde_json::from_str(&serde_json::to_string(&h.to_file()).unwrap()).unwrap());
        assert_eq!(back.stat("https://a.com/").unwrap().count, 2);
    }

    #[test]
    fn history_cap_and_top_sites() {
        let mut h = History::default();
        for i in 0..(MAX_VISITS + 600) {
            h.add(&format!("https://s{}.com/", i % 50), "t", (i as u64 + 1) * 60_000);
        }
        assert!(h.len() <= MAX_VISITS + 500);
        let mut h = hist(&[
            ("https://www.youtube.com/", "YouTube", 1),
            ("https://www.youtube.com/watch?v=1", "影片", 100_000),
            ("https://www.youtube.com/", "YouTube", 200_000),
            ("https://forum.gamer.com.tw/", "巴哈", 300_000),
        ]);
        let top = h.top_sites(8);
        assert_eq!(top[0].0, "https://www.youtube.com");
        assert_eq!(top[0].2, 3);
        assert_eq!(top.len(), 2);
        assert!(h.set_icon("www.youtube.com", "https://www.youtube.com/favicon.ico"));
        assert!(!h.set_icon("www.youtube.com", "https://www.youtube.com/favicon.ico"));
        assert_eq!(h.icon_for("https://www.youtube.com/watch?v=1"), "https://www.youtube.com/favicon.ico");
    }

    #[test]
    fn downloads_store() {
        let mut d = Downloads::default();
        let a = d.start("https://x.com/a.zip", "C:\\Users\\me\\Downloads\\a.zip", 100, 10);
        let b = d.start("https://x.com/b.png", "C:\\Users\\me\\Downloads\\b.png", -1, 20);
        assert_eq!(d.active(), 2);
        d.get_mut(a).unwrap().state = "done".into();
        assert_eq!(d.active(), 1);
        assert_eq!(d.search("B.PNG").len(), 1);
        assert_eq!(d.search("")[0].id, b);
        // 下載中的不能移除、也不會被清掉
        assert!(!d.remove(b));
        assert!(d.remove(a));
        assert_eq!(d.clear_since(0), 0);
        let mut back: Downloads = serde_json::from_str(&serde_json::to_string(&d).unwrap()).unwrap();
        back.fix_loaded();
        assert_eq!(back.list[0].state, "canceled");
        assert_eq!(back.start("u", "p", 0, 30), b + 1);
        assert_eq!(file_name("C:\\a\\b.zip"), "b.zip");
        assert_eq!(file_name("/a/b.zip"), "b.zip");
    }

    #[test]
    fn closed_tabs() {
        let mut list = Vec::new();
        push_closed(&mut list, ClosedTab { url: "chrome://newtab/".into(), ..ClosedTab::default() });
        push_closed(&mut list, ClosedTab::default());
        assert!(list.is_empty());
        for i in 0..40 {
            push_closed(&mut list, ClosedTab { url: format!("https://a.com/{i}"), ..ClosedTab::default() });
        }
        assert_eq!(list.len(), MAX_CLOSED);
        assert_eq!(list.last().unwrap().url, "https://a.com/39");
    }

    #[test]
    fn omnibox_suggestions() {
        let now = 10 * 86_400_000;
        let mut h = hist(&[
            ("https://www.youtube.com/", "YouTube", now - 1000),
            ("https://www.youtube.com/watch?v=abc", "楓之谷 打王影片 - YouTube", now - 5000),
            ("https://forum.gamer.com.tw/A.php?bsn=7650", "新楓之谷 哈啦板 - 巴哈姆特", now - 86_400_000 * 3),
            ("https://mail.google.com/mail/u/0/", "Gmail", now - 86_400_000),
        ]);
        h.add("https://www.youtube.com/", "YouTube", now - 500_000);
        let g = engine("google");
        let bms = vec![("Nexon".to_string(), "https://openapi.nexon.com/".to_string())];
        // 打網域的開頭：自動補完，第一項就是那個網站
        let s = suggest_local("you", &h, &bms, g, now, true, 8);
        assert_eq!(s[0].url, "https://www.youtube.com");
        assert_eq!(s[0].inline, "tube.com");
        assert_eq!(s[0].fill, "youtube.com");
        assert_eq!(s[1].kind, "search");
        assert_eq!(s[1].text, "you");
        assert!(s.iter().any(|x| x.url.contains("watch?v=abc")));
        // 刪字的時候不補完：第一項是輸入的字
        let s = suggest_local("you", &h, &bms, g, now, false, 8);
        assert_eq!(s[0].kind, "search");
        assert!(s[0].inline.is_empty());
        // 中文搜尋標題
        let s = suggest_local("楓之谷", &h, &bms, g, now, true, 8);
        assert_eq!(s[0].kind, "search");
        assert_eq!(s[0].desc, "Google 搜尋");
        assert_eq!(s.iter().filter(|x| x.kind == "history").count(), 2);
        // 兩個關鍵字都要符合
        let s = suggest_local("楓之谷 巴哈", &h, &bms, g, now, true, 8);
        assert_eq!(s.iter().filter(|x| x.kind == "history").count(), 1);
        // 書籤也會出現；輸入的是完整網址時第一項是網址
        let s = suggest_local("nexon", &h, &bms, g, now, true, 8);
        assert!(s.iter().any(|x| x.kind == "bookmark" && x.desc == "Nexon"));
        let s = suggest_local("gmail", &h, &bms, g, now, true, 8);
        assert!(s.iter().any(|x| x.url.starts_with("https://mail.google.com")));
        let s = suggest_local("google", &h, &bms, g, now, true, 8);
        assert!(s.iter().any(|x| x.url.starts_with("https://mail.google.com")));
        let s = suggest_local("https://www.youtube.com/", &h, &bms, g, now, true, 8);
        assert_eq!(s[0].kind, "url");
        assert_eq!(s[0].desc, "YouTube");
        assert!(!s[1..].iter().any(|x| norm_url(&x.url) == "https://www.youtube.com"));
        let s = suggest_local("example.org", &h, &bms, g, now, true, 8);
        assert_eq!((s[0].kind, s[0].url.as_str()), ("url", "https://example.org/"));
        assert!(suggest_local("  ", &h, &bms, g, now, true, 8).is_empty());
        // 搜尋引擎的建議接在後面，不重複、不超過上限
        let s = suggest_local("楓之谷", &h, &bms, g, now, true, 8);
        let n = s.len();
        let m = merge_remote(s, &["楓之谷".into(), "楓之谷 職業".into(), "楓之谷 官網".into()], g, 8);
        assert_eq!(m.len(), n + 2);
        assert_eq!(m.last().unwrap().kind, "suggest");
        assert!(m.last().unwrap().url.starts_with("https://www.google.com/search?q="));
        assert!(wants_remote("楓之谷") && !wants_remote("https://a.com") && !wants_remote("a.com/b") && !wants_remote("chrome://history"));
        assert_eq!(parse_suggest(r#"["楓",["楓之谷","楓葉"],[],[],{}]"#), ["楓之谷", "楓葉"]);
        assert_eq!(parse_suggest(")]}'\n[\"a\",[\"ab\"]]"), ["ab"]);
        assert!(parse_suggest("<html>").is_empty());
    }

    fn ni(id: i32, name: &str, label: &str) -> NativeItem {
        NativeItem { id, name: name.into(), label: label.into(), kind: 0, enabled: true, ..NativeItem::default() }
    }
    fn nsep() -> NativeItem {
        NativeItem { kind: 3, ..NativeItem::default() }
    }

    #[test]
    fn context_menu_page() {
        let items = vec![
            ni(1, "back", "返回"),
            ni(2, "forward", "下一頁"),
            ni(3, "reload", "重新整理"),
            nsep(),
            ni(4, "saveAs", "另存新檔"),
            ni(5, "print", "列印"),
            nsep(),
            ni(6, "webCapture", "網頁擷取"),
            ni(7, "share", "分享"),
            nsep(),
            ni(8, "inspectElement", "檢查"),
        ];
        let t = CtxTarget { page: "https://a.com/".into(), ..CtxTarget::default() };
        let m = build_context(&t, &items, engine("google"), false);
        let titles: Vec<&str> = m.iter().map(|x| if x.sep { "-" } else { x.title.as_str() }).collect();
        assert_eq!(titles, ["上一頁", "下一頁", "重新載入", "-", "另存新檔…", "列印…", "-", "檢視網頁原始碼", "檢查"]);
        assert_eq!(m[0].act, CtxAct::Native(1));
        assert_eq!(m[0].shortcut, "Alt+向左鍵");
        assert_eq!(m[7].act, CtxAct::ViewSource("https://a.com/".into()));
    }

    #[test]
    fn context_menu_link_image_selection() {
        let items = vec![
            ni(1, "openLinkInNewWindow", "在新視窗中開啟連結"),
            ni(2, "saveLinkAs", "另存連結"),
            ni(3, "copyLinkLocation", "複製連結"),
            nsep(),
            ni(4, "saveImageAs", "另存影像"),
            ni(5, "copyImage", "複製影像"),
            ni(6, "copyImageLocation", "複製影像連結"),
            nsep(),
            ni(9, "未知的項目", "某個 &Edge 功能"),
            ni(7, "inspectElement", "檢查"),
        ];
        let t = CtxTarget { kind: 1, link: "https://b.com/x".into(), src: "https://b.com/i.png".into(), page: "https://a.com/".into(), ..CtxTarget::default() };
        let m = build_context(&t, &items, engine("google"), false);
        let titles: Vec<&str> = m.iter().map(|x| if x.sep { "-" } else { x.title.as_str() }).collect();
        assert_eq!(
            titles,
            ["在新分頁中開啟連結", "在新視窗中開啟連結", "在無痕式視窗中開啟連結", "-", "另存連結為…", "複製連結網址", "-", "在新分頁中開啟圖片", "另存圖片…", "複製圖片", "複製圖片位址", "-", "某個 Edge 功能", "檢查"]
        );
        assert_eq!(m[0].act, CtxAct::OpenTab("https://b.com/x".into()));
        assert_eq!(m[7].act, CtxAct::OpenTab("https://b.com/i.png".into()));
        // 無痕視窗裡不顯示「在無痕式視窗中開啟」
        let m = build_context(&t, &items, engine("google"), true);
        assert!(!m.iter().any(|x| x.title.contains("無痕")));

        let items = vec![ni(1, "copy", "複製"), ni(2, "searchWeb", "在網路上搜尋"), nsep(), ni(3, "print", "列印"), ni(4, "inspectElement", "檢查")];
        let t = CtxTarget { kind: 2, selection: "  新楓之谷 第六轉 技能 核心 強化 順序 推薦 整理  ".into(), page: "https://a.com/".into(), ..CtxTarget::default() };
        let m = build_context(&t, &items, engine("bing"), false);
        let titles: Vec<&str> = m.iter().map(|x| if x.sep { "-" } else { x.title.as_str() }).collect();
        assert_eq!(titles[0], "複製");
        assert!(titles[1].starts_with("透過 Bing 搜尋「新楓之谷 第六轉") && titles[1].ends_with("…」"));
        assert_eq!(&titles[2..], ["-", "列印…", "檢查"]);
        assert_eq!(m[1].act, CtxAct::Search("新楓之谷 第六轉 技能 核心 強化 順序 推薦 整理".into()));
    }

    #[test]
    fn context_menu_editable_and_submenu() {
        let spell = NativeItem { id: 20, name: "spellcheck".into(), label: "拼字檢查".into(), kind: 4, enabled: true, children: vec![ni(21, "x", "建議一"), nsep()], ..NativeItem::default() };
        let empty_sub = NativeItem { id: 30, name: "y".into(), label: "空的".into(), kind: 4, enabled: true, children: vec![nsep()], ..NativeItem::default() };
        let mut paste = ni(3, "paste", "貼上");
        paste.enabled = false;
        let items = vec![nsep(), spell, empty_sub, nsep(), nsep(), ni(1, "cut", "剪下"), ni(2, "copy", "複製"), paste, ni(4, "selectAll", "全選"), nsep()];
        let t = CtxTarget { kind: 0, editable: true, page: "https://a.com/".into(), ..CtxTarget::default() };
        let m = build_context(&t, &items, engine("google"), false);
        let titles: Vec<&str> = m.iter().map(|x| if x.sep { "-" } else { x.title.as_str() }).collect();
        assert_eq!(titles, ["拼字檢查", "-", "剪下", "複製", "貼上", "全選"]);
        assert_eq!(m[0].children.len(), 1);
        assert!(!m[4].enabled);
        assert_eq!(m[4].shortcut, "Ctrl+V");
    }
}
