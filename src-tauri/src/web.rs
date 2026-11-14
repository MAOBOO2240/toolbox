//! 瀏覽器視窗：外觀與操作比照 Chrome，可以邊玩遊戲邊用。
//!
//! 每個視窗 = 一個無框視窗：上面（分頁列、工具列、書籤列）是本機頁面 web.html，
//! 下面是網頁（每個分頁一個 webview，只顯示目前的分頁；其他分頁點到時才載入）。
//! - 視窗設成「不搶焦點」：滑鼠點影片、按暫停、拖進度時，鍵盤一直留在遊戲。
//! - 點到網頁或網址列的輸入框要打字時，才把鍵盤交給視窗；按 Enter、點別的地方或滑鼠移出視窗就還給原本的視窗（遊戲）。
//! - 選單、網址列的建議、提示框用另外的置頂小視窗顯示（可以超出瀏覽器視窗的範圍），見 web/pop.rs。
//! - 新分頁、歷史記錄、下載、書籤、擴充功能、設定是程式自己的頁面（chrome://…），見 web/pages.rs。
//! - 外部網頁不能呼叫程式的任何指令（沒有設定 remote 權限，Tauri 會直接拒絕）；
//!   程式自己的頁面只有 web-ui.html 可以開在分頁裡，而且不能被別的網頁內嵌。

use crate::fg;
use crate::webcore;
use crate::webev::{self, DlEv, Ev};
#[cfg(windows)]
use crate::webnative;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicIsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock};
use std::time::{Duration, Instant};
use tauri::webview::{NewWindowResponse, PageLoadEvent, WebviewBuilder};
use tauri::window::{Color, WindowBuilder};
use tauri::{
    AppHandle, Emitter, Manager, PhysicalPosition, PhysicalSize, Url, Webview, WebviewUrl, WebviewWindow, WebviewWindowBuilder, Window,
    WindowEvent,
};

pub mod pages;
pub mod pop;

/// 新視窗、新分頁開的頁面
pub const HOME: &str = webcore::NEWTAB;
/// 上面幾排的高度（邏輯像素，要跟 web.html 一致；量自 Chrome）
const STRIP_H: f64 = 48.0;
const STRIP_MAX_H: f64 = 40.0;
const TOOL_H: f64 = 46.0;
const BMS_H: f64 = 34.0;
const LINE_H: f64 = 1.0;
/// 左、右、下留給拉大小的邊（邏輯像素）
const EDGE: f64 = 5.0;
/// 網頁裡有輸入框被選取時，標題最後面會加上兩個看不見的字元
const MARK: char = '\u{2063}';

/// 非 Windows（只為了編譯測試）：跟 webnative.rs 一樣的函式，什麼都不做
#[cfg(not(windows))]
#[allow(dead_code)]
mod webnative {
    use crate::webev::{Opts, Sink};
    pub struct Wv;
    pub fn attach(_w: &Wv, _opts: Opts, _sink: Sink) {}
    pub fn ctx_finish(_seq: u64, _pick: Option<i32>) {}
    pub fn ctx_default(_seq: u64) {}
    pub fn go_back(_w: &Wv) {}
    pub fn go_forward(_w: &Wv) {}
    pub fn stop(_w: &Wv) {}
    pub fn can_go(_w: &Wv) -> (bool, bool) {
        (false, false)
    }
    pub fn set_muted(_w: &Wv, _muted: bool) {}
    pub fn set_zoom(_w: &Wv, _z: f64) {}
    pub fn set_password_save(_w: &Wv, _on: bool) {}
    pub fn set_download_dir(_w: &Wv, _dir: &str) {}
    pub fn task_manager(_w: &Wv) {}
    pub fn save_as(_w: &Wv) {}
    pub fn toggle_downloads(_w: &Wv) {}
    pub fn find(_w: &Wv) -> bool {
        false
    }
    pub fn version(_w: &Wv) -> String {
        String::new()
    }
    pub fn dl_cancel(_key: u64) {}
    pub fn dl_pause(_key: u64, _pause: bool) {}
    pub fn clear_data(_w: &Wv, _kinds: i32, _since_ms: Option<u64>, done: Box<dyn FnOnce(bool) + Send + 'static>) {
        done(false)
    }
    pub fn nav_entries(_w: &Wv, done: Box<dyn FnOnce(String) + Send + 'static>) {
        done(String::new())
    }
    pub fn nav_to(_w: &Wv, _entry: i64) {}
    pub fn clear_site(_w: &Wv, _origins: &[String]) {}
    pub fn round_corners(_hwnd: isize) {}
    pub fn open_devtools(_w: &Wv, _own_page: fn(&str) -> bool) -> bool {
        false
    }
    pub struct NewWin;
    impl NewWin {
        pub fn from_own(&self) -> bool {
            true
        }
        pub fn adopt(&self, _w: &Wv) -> bool {
            false
        }
    }
    pub fn take_new_window(_same: impl Fn(&str) -> bool) -> Option<NewWin> {
        None
    }
}

/// 注入外部網頁的小腳本：只負責「現在有沒有輸入框被選取」，用標題最後的隱形記號告訴程式。
/// 不會讀取網頁內容，也不會送出任何資料。
const TYPING_JS: &str = r#"(() => {
  if (window.__mbTyping) return;
  window.__mbTyping = true;
  const NOTXT = /^(button|checkbox|radio|range|submit|reset|file|color|image|hidden)$/i;
  const deep = (a) => { while (a && a.shadowRoot && a.shadowRoot.activeElement) a = a.shadowRoot.activeElement; return a; };
  const editable = (el) => !!el && el.nodeType === 1 && (el.isContentEditable || el.tagName === "TEXTAREA" || (el.tagName === "INPUT" && !NOTXT.test(el.type || "text")));
  const target = (e) => (e.composedPath ? e.composedPath()[0] : e.target);
  const top = window.top === window;
  let mine = false, frame = false, sent = null, busy = false;
  const M = "⁣⁣";
  const apply = () => {
    if (!top) return;
    const on = mine || frame;
    const t = document.title || "";
    const has = t.endsWith(M);
    if (on === has) return;
    busy = true;
    document.title = on ? t + M : t.slice(0, -M.length);
    busy = false;
  };
  // 內嵌的框架（例如登入框）：選到輸入框時告訴最上層的網頁
  const report = () => {
    if (top) { apply(); return; }
    if (sent === mine) return;
    sent = mine;
    try { window.top.postMessage({ __mbTyping: mine }, "*"); } catch (_) {}
  };
  const set = (v) => { if (v !== mine) { mine = v; report(); } };
  const check = () => set(editable(deep(document.activeElement)) && (top || document.hasFocus()));
  document.addEventListener("pointerdown", (e) => {
    const t = target(e);
    if (top && !(t && t.tagName === "IFRAME")) { frame = false; }
    if (editable(t)) set(true); else { setTimeout(check, 0); apply(); }
  }, true);
  document.addEventListener("focusin", check, true);
  document.addEventListener("focusout", () => setTimeout(check, 0), true);
  if (!top) window.addEventListener("blur", () => set(false));
  // 從遊戲點過來打字時（程式會設定 __mbGame）：單行輸入框按 Enter 送出後取消選取，鍵盤還給遊戲。
  // 一般使用（例如聊天室連續打字）不受影響；選字中的 Enter 不算
  document.addEventListener("keydown", (e) => {
    let game = !!window.__mbGame;
    try { game = game || !!window.top.__mbGame; } catch (_) {}
    if (!game) return;
    if (e.key !== "Enter" || e.isComposing || e.keyCode === 229) return;
    const t = deep(document.activeElement);
    if (!t || t.tagName !== "INPUT" || !editable(t)) return;
    setTimeout(() => { if (deep(document.activeElement) === t) t.blur(); set(false); }, 150);
  }, true);
  // 程式結束打字時呼叫：打字狀態一律清掉（視窗已經不在前景時，取消選取不會觸發事件，狀態會停在「打字中」，
  // 下次再點輸入框就不會把鍵盤交給 web）。內嵌的框架也一起清
  const resetFrames = () => {
    for (let i = 0; i < window.frames.length; i++) { try { window.frames[i].postMessage({ __mbReset: true }, "*"); } catch (_) {} }
  };
  window.__mbReset = () => { mine = false; frame = false; sent = null; apply(); resetFrames(); };
  if (!top) {
    window.addEventListener("message", (e) => { if (e.data && e.data.__mbReset === true) { mine = false; sent = null; resetFrames(); } });
  }
  if (top) {
    window.addEventListener("message", (e) => {
      if (e.data && typeof e.data.__mbTyping === "boolean") { frame = e.data.__mbTyping; apply(); }
    });
    // 只看 <title> 有沒有被網頁改掉（看整個 head 的話，很多網站常常改 head，會一直觸發）
    let watched = null;
    const titleOb = new MutationObserver(() => { if (!busy) apply(); });
    const watch = () => {
      const t = document.querySelector("title");
      if (t && t !== watched) { watched = t; titleOb.disconnect(); titleOb.observe(t, { childList: true, characterData: true, subtree: true }); }
      if (!watched && document.head) new MutationObserver(watch).observe(document.head, { childList: true });
    };
    if (document.head) watch(); else document.addEventListener("DOMContentLoaded", watch, { once: true });
  }
})();"#;

/// 擴充功能的小視窗裡：chrome.tabs.query 問「目前的分頁」時回答目前顯示的網頁那個分頁
/// （WebView2 把每個網頁當成各自的視窗；分頁編號是真的，擴充功能之後對那個分頁做的事都有效）
const EXT_TABS_JS: &str = r#"(() => {
  const PAGE = __PAGE_URL__;
  const same = (a, b) => { const n = (u) => String(u || "").replace(/#.*$/, "").replace(/\/+$/, ""); return n(a) === n(b); };
  try {
    const t = chrome.tabs, w = chrome.windows;
    const q = t.query.bind(t);
    const host = (u) => { try { return new URL(u).host; } catch (_) { return ""; } };
    const mine = () => q({}).then((tabs) => tabs.find((x) => same(x.url, PAGE)) || tabs.find((x) => host(x.url) && host(x.url) === host(PAGE)) || null);
    const wants = (info) => !!info && (info.active || info.currentWindow || info.lastFocusedWindow || info.windowId !== undefined);
    const call = (p, cb) => { if (typeof cb === "function") { p.then(cb, () => cb([])); return undefined; } return p; };
    t.query = (info, cb) => call(wants(info) ? mine().then((m) => (m ? [m] : q(info))) : q(info), cb);
    if (w) {
      const cur = (cb) => call(mine().then((m) => (m ? { id: m.windowId, focused: true, type: "normal", tabs: [m] } : w.getCurrent ? w.getCurrent() : null)), cb);
      w.getCurrent = (_a, cb) => cur(typeof _a === "function" ? _a : cb);
      w.getLastFocused = (_a, cb) => cur(typeof _a === "function" ? _a : cb);
    }
    if (t.getCurrent) {
      t.getCurrent = (cb) => call(mine(), cb);
    }
  } catch (_) {}
})();"#;

/* ---------------- 設定檔 ---------------- */

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Tab {
    pub id: String,
    /// 網址；程式自己的頁面記成 chrome://…（例如 chrome://newtab/）
    pub url: String,
    pub title: String,
    pub icon: String,
    /// 固定的分頁（排在最左邊、只顯示圖示）
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub pinned: bool,
    /// 靜音
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub muted: bool,
}

impl Tab {
    fn new(id: String, url: String) -> Self {
        Tab { id, url, ..Tab::default() }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct WebWin {
    pub id: String,
    /// 目前分頁的網址與標題（給主程式顯示）
    pub url: String,
    pub title: String,
    /// 一般大小時的位置與大小（實體像素）
    pub x: i32,
    pub y: i32,
    pub w: u32,
    pub h: u32,
    /// 不透明度 30～100
    pub opacity: u32,
    /// 鎖定：滑鼠直接穿過視窗點到遊戲
    pub locked: bool,
    /// 只在遊戲（或本程式）在前景時顯示
    pub game_only: bool,
    /// 程式關閉時是開著的 → 下次開啟程式時自動打開
    pub open: bool,
    /// 縮小（暫時藏起來，在 web 工具裡再叫出來）
    pub min: bool,
    /// 放到最大
    pub max: bool,
    pub tabs: Vec<Tab>,
    pub active: String,
    pub next_tab: u32,
    /// 無痕式視窗：不留歷史記錄、不跟一般視窗共用登入狀態，關掉就消失（不會存到設定檔）
    #[serde(skip)]
    pub incognito: bool,
}

impl Default for WebWin {
    fn default() -> Self {
        Self {
            id: String::new(),
            url: HOME.into(),
            title: String::new(),
            x: 200,
            y: 150,
            // 0 = 第一次打開時用預設大小 1280×720（依 Windows 的縮放比例）
            w: 0,
            h: 0,
            opacity: 100,
            locked: false,
            game_only: false,
            open: false,
            min: false,
            max: false,
            tabs: Vec::new(),
            active: String::new(),
            next_tab: 0,
            incognito: false,
        }
    }
}

/// 書籤：url 空白的是資料夾
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Bm {
    pub id: String,
    pub title: String,
    pub url: String,
    pub children: Vec<Bm>,
    /// 從 Chrome 同步來的：Chrome 書籤的編號（Chrome 改名、刪除時跟著變）
    #[serde(skip_serializing_if = "String::is_empty")]
    pub src: String,
    /// 在這裡改過名：同步時不要用 Chrome 的名稱蓋掉
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub renamed: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Bookmarks {
    /// 書籤列
    pub bar: Vec<Bm>,
    /// 書籤列右邊的「所有書籤」
    pub other: Vec<Bm>,
    pub next_id: u64,
    /// 網站 → 圖示網址（逛過有加書籤的網站時記下來）
    pub icons: HashMap<String, String>,
    /// 自動從 Chrome 匯入（Chrome 的書籤有變動就跟著加進來）
    #[serde(default = "yes")]
    pub sync: bool,
    /// 從哪個 Chrome 使用者匯入（空白 = 自動選）
    pub profile: String,
    /// 在這裡刪掉的書籤（網址或「folder:名稱」），自動匯入時不要再加回來
    pub ignored: Vec<String>,
}

fn yes() -> bool {
    true
}

impl Default for Bookmarks {
    fn default() -> Self {
        Self {
            bar: Vec::new(),
            other: Vec::new(),
            next_id: 0,
            icons: HashMap::new(),
            sync: true,
            profile: String::new(),
            ignored: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct WebFile {
    pub next_id: u32,
    pub wins: Vec<WebWin>,
    pub bookmarks: Bookmarks,
    /// 瀏覽器的設定（搜尋引擎、首頁按鈕、書籤列、縮放…）
    pub settings: webcore::Settings,
    /// 最近關閉的分頁（Ctrl+Shift+T 重新開啟）
    pub closed: Vec<webcore::ClosedTab>,
}

struct Store {
    file: WebFile,
    path: PathBuf,
    dirty: bool,
}

static STORE: OnceLock<Mutex<Store>> = OnceLock::new();

fn st() -> MutexGuard<'static, Store> {
    STORE.get().expect("web 尚未初始化").lock().unwrap_or_else(|e| e.into_inner())
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

/// 舊資料補齊：沒有分頁就用原本的網址開一個；目前分頁不存在就選第一個；固定的分頁排前面
fn fix_win(w: &mut WebWin) {
    if w.tabs.is_empty() {
        let url = if w.url.trim().is_empty() { HOME.to_string() } else { w.url.clone() };
        w.next_tab += 1;
        w.tabs.push(Tab { title: w.title.clone(), ..Tab::new(format!("t{}", w.next_tab), url) });
    }
    for t in &mut w.tabs {
        if t.url.trim().is_empty() {
            t.url = HOME.to_string();
        }
    }
    w.tabs.sort_by_key(|t| !t.pinned);
    let biggest = w.tabs.iter().filter_map(|t| t.id.strip_prefix('t')?.parse::<u32>().ok()).max().unwrap_or(0);
    w.next_tab = w.next_tab.max(biggest);
    if !w.tabs.iter().any(|t| t.id == w.active) {
        w.active = w.tabs[0].id.clone();
    }
    if let Some(t) = w.tabs.iter().find(|t| t.id == w.active) {
        w.url = t.url.clone();
        w.title = t.title.clone();
    }
}

fn fix_bm_ids(b: &mut Bookmarks) {
    fn walk(list: &[Bm], m: &mut u64) {
        for x in list {
            if let Some(n) = x.id.strip_prefix('b').and_then(|s| s.parse::<u64>().ok()) {
                *m = (*m).max(n);
            }
            walk(&x.children, m);
        }
    }
    let mut m = b.next_id;
    walk(&b.bar, &mut m);
    walk(&b.other, &mut m);
    b.next_id = m;
}

fn load(path: &Path) -> WebFile {
    let mut f: WebFile =
        std::fs::read_to_string(path).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default();
    for w in &mut f.wins {
        fix_win(w);
    }
    fix_bm_ids(&mut f.bookmarks);
    // 書籤一律自動跟 Chrome 同步（以前可以關掉）
    f.bookmarks.sync = true;
    f
}

fn write_json<T: Serialize>(path: &Path, data: &T, pretty: bool) {
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let text = if pretty { serde_json::to_string_pretty(data) } else { serde_json::to_string(data) };
    if let Ok(s) = text {
        let tmp = path.with_extension("tmp");
        if std::fs::write(&tmp, s).is_ok() {
            let _ = std::fs::rename(&tmp, path);
        }
    }
}

fn save(path: &Path, file: &WebFile) {
    write_json(path, file, true);
}

fn settings() -> webcore::Settings {
    st().file.settings.clone()
}

fn engine() -> &'static webcore::Engine {
    webcore::engine(&st().file.settings.engine)
}

fn is_incognito(wid: &str) -> bool {
    st().file.wins.iter().any(|w| w.id == wid && w.incognito)
}

fn edit<T>(id: &str, f: impl FnOnce(&mut WebWin) -> T) -> Option<T> {
    let mut s = st();
    let r = s.file.wins.iter_mut().find(|w| w.id == id).map(f);
    if r.is_some() {
        s.dirty = true;
    }
    r
}

fn get(id: &str) -> Option<WebWin> {
    st().file.wins.iter().find(|w| w.id == id).cloned()
}

fn bm<T>(f: impl FnOnce(&mut Bookmarks) -> T) -> T {
    let mut s = st();
    s.dirty = true;
    f(&mut s.file.bookmarks)
}

fn bm_read<T>(f: impl FnOnce(&Bookmarks) -> T) -> T {
    f(&st().file.bookmarks)
}

fn open_ids() -> Vec<String> {
    st().file.wins.iter().filter(|w| w.open).map(|w| w.id.clone()).collect()
}

/* ---------------- 書籤工具 ---------------- */

fn bm_find<'a>(list: &'a [Bm], id: &str) -> Option<&'a Bm> {
    for b in list {
        if b.id == id {
            return Some(b);
        }
        if let Some(x) = bm_find(&b.children, id) {
            return Some(x);
        }
    }
    None
}

fn bm_find_any<'a>(b: &'a Bookmarks, id: &str) -> Option<&'a Bm> {
    bm_find(&b.bar, id).or_else(|| bm_find(&b.other, id))
}

fn bm_find_mut<'a>(list: &'a mut [Bm], id: &str) -> Option<&'a mut Bm> {
    for b in list.iter_mut() {
        if b.id == id {
            return Some(b);
        }
        if let Some(x) = bm_find_mut(&mut b.children, id) {
            return Some(x);
        }
    }
    None
}

fn bm_remove(list: &mut Vec<Bm>, id: &str) -> Option<Bm> {
    if let Some(i) = list.iter().position(|b| b.id == id) {
        return Some(list.remove(i));
    }
    for b in list.iter_mut() {
        if let Some(x) = bm_remove(&mut b.children, id) {
            return Some(x);
        }
    }
    None
}

fn same_url(a: &str, b: &str) -> bool {
    a.trim_end_matches('/') == b.trim_end_matches('/')
}

fn bm_url_id(list: &[Bm], url: &str) -> Option<String> {
    for b in list {
        if !b.url.is_empty() && same_url(&b.url, url) {
            return Some(b.id.clone());
        }
        if let Some(x) = bm_url_id(&b.children, url) {
            return Some(x);
        }
    }
    None
}

fn count_links(list: &[Bm]) -> usize {
    list.iter().map(|b| if b.url.is_empty() { count_links(&b.children) } else { 1 }).sum()
}

fn host_of(url: &str) -> Option<String> {
    Url::parse(url).ok()?.host_str().map(str::to_string)
}

fn has_host(list: &[Bm], host: &str) -> bool {
    list.iter().any(|b| (!b.url.is_empty() && host_of(&b.url).as_deref() == Some(host)) || has_host(&b.children, host))
}

/// Chrome 書籤的編號（guid，舊版 Chrome 用 id）
fn chrome_key(v: &Value) -> String {
    v["guid"].as_str().or_else(|| v["id"].as_str()).map(|s| format!("c:{s}")).unwrap_or_default()
}

/// Chrome 書籤檔裡的一個節點 → 書籤（只收網頁網址）
fn from_chrome(v: &Value, next: &mut u64) -> Option<Bm> {
    let title = v["name"].as_str().unwrap_or("").to_string();
    match v["type"].as_str()? {
        "url" => {
            let url = v["url"].as_str()?;
            if !web_url(&Url::parse(url).ok()?) {
                return None;
            }
            *next += 1;
            Some(Bm { id: format!("b{}", *next), title, url: url.to_string(), src: chrome_key(v), ..Bm::default() })
        }
        "folder" => {
            let children: Vec<Bm> = v["children"]
                .as_array()
                .map(|a| a.iter().filter_map(|c| from_chrome(c, next)).collect())
                .unwrap_or_default();
            *next += 1;
            Some(Bm { id: format!("b{}", *next), title, children, src: chrome_key(v), ..Bm::default() })
        }
        _ => None,
    }
}

/// 在這裡刪過的書籤（Chrome 編號，或舊版記的網址、「folder:名稱」）
fn is_ignored(b: &Bm, ignored: &[String]) -> bool {
    ignored.iter().any(|i| {
        (!b.src.is_empty() && *i == b.src)
            || (b.url.is_empty() && *i == format!("folder:{}", b.title))
            || (!b.url.is_empty() && same_url(i, &b.url))
    })
}

/// 拿掉在這裡刪過的書籤
fn drop_ignored(list: Vec<Bm>, ignored: &[String]) -> Vec<Bm> {
    list.into_iter()
        .filter(|b| !is_ignored(b, ignored))
        .map(|mut b| {
            b.children = drop_ignored(std::mem::take(&mut b.children), ignored);
            b
        })
        .collect()
}

/// 跟 Chrome 同步一層書籤：Chrome 改名、改網址、刪除都跟著變，新的加在最後；
/// 在這裡改過名的維持；在這裡刪掉的不再加回；自己加的（不是從 Chrome 來的）不動。回傳新增幾個
fn sync_level(dst: &mut Vec<Bm>, src: Vec<Bm>, ignored: &[String]) -> usize {
    // 舊版匯入的（沒有 Chrome 編號）：用網址或資料夾名稱認領，不要重複
    for s in &src {
        if s.src.is_empty() || dst.iter().any(|d| d.src == s.src) {
            continue;
        }
        let same = |d: &&mut Bm| {
            d.src.is_empty()
                && d.url.is_empty() == s.url.is_empty()
                && if s.url.is_empty() { d.title == s.title } else { same_url(&d.url, &s.url) }
        };
        if let Some(d) = dst.iter_mut().find(same) {
            d.src = s.src.clone();
        }
    }
    let pos: HashMap<String, usize> = src.iter().enumerate().map(|(i, s)| (s.src.clone(), i)).collect();
    let mut left: Vec<Option<Bm>> = src.into_iter().map(Some).collect();
    let mut added = 0;
    let mut keep = Vec::new();
    for mut d in std::mem::take(dst) {
        if d.src.is_empty() {
            keep.push(d);
            continue;
        }
        // Chrome 裡已經沒有（刪掉或搬走）→ 這裡也拿掉
        let Some(s) = pos.get(&d.src).and_then(|&i| left[i].take()) else { continue };
        if !d.renamed {
            d.title = s.title;
        }
        if d.url.is_empty() {
            added += sync_level(&mut d.children, s.children, ignored);
        } else if !s.url.is_empty() {
            d.url = s.url;
        }
        keep.push(d);
    }
    for s in left.into_iter().flatten() {
        if s.src.is_empty() || is_ignored(&s, ignored) {
            continue;
        }
        let mut s = s;
        s.children = drop_ignored(std::mem::take(&mut s.children), ignored);
        added += if s.url.is_empty() { count_links(&s.children) } else { 1 };
        keep.push(s);
    }
    *dst = keep;
    added
}

/// 合併匯入的書籤：同一層已經有相同網址就略過；同名資料夾合併在一起。回傳新增了幾個書籤
fn merge(dst: &mut Vec<Bm>, src: Vec<Bm>) -> usize {
    let mut added = 0;
    for s in src {
        if s.url.is_empty() {
            if let Some(d) = dst.iter_mut().find(|d| d.url.is_empty() && d.title == s.title) {
                added += merge(&mut d.children, s.children);
            } else {
                added += count_links(&s.children);
                dst.push(s);
            }
        } else if !dst.iter().any(|d| !d.url.is_empty() && same_url(&d.url, &s.url)) {
            added += 1;
            dst.push(s);
        }
    }
    added
}

/* ---------------- 執行狀態 ---------------- */

#[derive(Debug, Default)]
struct Rt {
    hwnd: isize,
    /// 網頁裡有輸入框被選取
    page_typing: bool,
    page_off_at: Option<Instant>,
    /// 網址列（或書籤改名）的輸入框被選取
    bar_typing: bool,
    /// 目前鍵盤在這個視窗
    typing: bool,
    /// 滑鼠移出視窗的時間（打字中才看）
    out_since: Option<Instant>,
    /// 開始打字前在前景的視窗（打完要還給它）
    prev_fg: isize,
    /// 載入中的分頁
    loading: HashSet<String>,
    /// 剛選到網頁／工具列輸入框的時間（遊戲在前景時才用來開始打字）
    page_on_at: Option<Instant>,
    bar_on_at: Option<Instant>,
    /// 開始打字的時間
    typing_since: Option<Instant>,
    /// 網頁進入全螢幕（影片全螢幕）：網頁蓋滿整個視窗、工具列藏起來
    full: bool,
    /// 視窗目前所在螢幕的縮放比例（跨到不同縮放比例的螢幕時，照比例換算大小）
    sf: f64,
    /// 各分頁：上一頁／下一頁能不能按
    nav: HashMap<String, (bool, bool)>,
    /// 正在播放聲音的分頁
    audio: HashSet<String>,
    /// 各分頁目前的縮放
    zoom: HashMap<String, f64>,
    /// 程式自己剛設定的縮放（收到縮放通知時，用來分辨是不是使用者用滾輪調的）
    zoom_set: HashMap<String, f64>,
    /// F11 全螢幕
    fs: bool,
    /// 全螢幕時滑鼠移到最上面：暫時顯示工具列
    reveal: bool,
    /// 工具列載入好了（可以收通知了）
    bar_ready: bool,
    /// 工具列還沒載入好時就要求把鍵盤交給網址列：等它好了再做（值 = 要不要全選）
    want_omni: Option<bool>,
    /// 正在「檢視原始碼」的分頁 → 被檢視的網址。
    /// WebView2 回報這種分頁的網址時不會帶 view-source: 開頭，要自己記著才分得出來
    source: HashMap<String, String>,
    /// 開過開發人員工具的分頁：之後不能再載入程式自己的頁面（見 nav_check）
    devtools: HashSet<String>,
    /// 目前顯示程式自己頁面（設定、新分頁…）的分頁：不能開開發人員工具
    own_page: HashSet<String>,
    /// 正要前往程式自己頁面的分頁（還沒真的換過去也不能開開發人員工具）
    heading_own: HashSet<String>,
    /// 分頁是從哪個分頁開出來的（關掉時回到那個分頁；中途切到別的分頁就不算了）
    opener: HashMap<String, String>,
}

static RT: Mutex<Option<HashMap<String, Rt>>> = Mutex::new(None);
/// 目前在打字的視窗（攔截訊息時要查，所以另外放）
static TYPING_HWND: Mutex<Vec<isize>> = Mutex::new(Vec::new());
/// 放到最大的視窗（不能拉邊改大小）
static GAME_FG: AtomicBool = AtomicBool::new(false);

fn rt<T>(f: impl FnOnce(&mut HashMap<String, Rt>) -> T) -> T {
    let mut g = lock(&RT);
    f(g.get_or_insert_with(HashMap::new))
}

fn win_label(wid: &str) -> String {
    format!("web-{wid}")
}
fn bar_label(wid: &str) -> String {
    format!("webbar-{wid}")
}
fn page_label(wid: &str, tid: &str) -> String {
    format!("webpg-{wid}-{tid}")
}

/// 從工具列的 webview 名稱取回視窗編號
fn id_of(wv: &Webview) -> Option<String> {
    wv.label().strip_prefix("webbar-").map(str::to_string)
}

/// 從分頁網頁的 webview 名稱取回（視窗編號, 分頁編號）
fn page_of(wv: &Webview) -> Option<(String, String)> {
    let rest = wv.label().strip_prefix("webpg-")?;
    let (wid, tid) = rest.rsplit_once('-')?;
    Some((wid.to_string(), tid.to_string()))
}

/// 呼叫指令的是哪個瀏覽器視窗（工具列，或開在分頁裡的內部頁面）
fn owner_of(wv: &Webview) -> Option<String> {
    id_of(wv).or_else(|| page_of(wv).map(|(w, _)| w))
}

#[derive(Serialize, Clone)]
pub struct TabView {
    id: String,
    url: String,
    title: String,
    icon: String,
    pinned: bool,
    muted: bool,
    audio: bool,
    loading: bool,
}

/// 工具列上「下載」按鈕的狀態
#[derive(Serialize, Clone, Default)]
pub struct DlBadge {
    /// 這次開啟程式後有沒有下載過（有才顯示按鈕）
    any: bool,
    active: usize,
    /// 下載中的進度（0～100；不知道大小是 -1）
    pct: i32,
}

#[derive(Serialize, Clone)]
pub struct BarState {
    id: String,
    tabs: Vec<TabView>,
    active: String,
    max: bool,
    /// F11 全螢幕
    full: bool,
    incognito: bool,
    back: bool,
    forward: bool,
    /// 目前分頁的網址，與網址列沒在打字時顯示的樣子
    url: String,
    display: String,
    /// newtab / internal / source / ext / secure / insecure / local / other
    kind: &'static str,
    /// 縮放（百分比）；zoomed = 跟預設值不一樣（網址列顯示放大鏡）
    zoom: u32,
    zoomed: bool,
    show_bm: bool,
    home: bool,
    bookmarked: bool,
    /// 可以重新開啟的已關閉分頁數
    closed: usize,
    engine: &'static str,
    dl: DlBadge,
}

#[derive(Serialize, Clone)]
pub struct BmPayload {
    bar: Vec<Bm>,
    other: Vec<Bm>,
    icons: HashMap<String, String>,
}

#[derive(Serialize, Clone)]
pub struct BarInit {
    state: BarState,
    bm: BmPayload,
    exts: Vec<ExtInfo>,
    /// 一載入好就把鍵盤交給網址列（值 = 要不要全選）
    focus: Option<bool>,
}

fn url_kind(u: &str) -> &'static str {
    if webcore::is_newtab(u) {
        "newtab"
    } else if webcore::virtual_page(u).is_some() {
        "internal"
    } else if u.starts_with("view-source:") {
        "source"
    } else if u.starts_with("chrome-extension://") {
        "ext"
    } else if u.starts_with("https://") {
        "secure"
    } else if u.starts_with("http://") {
        // 這台電腦自己的網址（localhost、127.x.x.x）：跟 Chrome 一樣不標「不安全」
        let local = Url::parse(u).ok().and_then(|p| p.host_str().map(str::to_lowercase)).is_some_and(|h| {
            h == "localhost" || h.ends_with(".localhost") || h == "[::1]" || h.parse::<std::net::Ipv4Addr>().is_ok_and(|ip| ip.is_loopback())
        });
        if local {
            "local"
        } else {
            "insecure"
        }
    } else {
        "other"
    }
}

fn bar_state(wid: &str) -> Option<BarState> {
    let (w, set, closed, bookmarked) = {
        let s = st();
        let w = s.file.wins.iter().find(|w| w.id == wid)?.clone();
        let url = w.tabs.iter().find(|t| t.id == w.active).map(|t| t.url.clone()).unwrap_or_default();
        let b = &s.file.bookmarks;
        let bookmarked = url_kind(&url) != "newtab" && !url.is_empty() && (bm_url_id(&b.bar, &url).is_some() || bm_url_id(&b.other, &url).is_some());
        (w, s.file.settings.clone(), s.file.closed.len(), bookmarked)
    };
    let url = w.tabs.iter().find(|t| t.id == w.active).map(|t| t.url.clone()).unwrap_or_default();
    let (loading, audio, nav, zoom, full) = rt(|m| {
        m.get(wid)
            .map(|r| (r.loading.clone(), r.audio.clone(), r.nav.get(&w.active).copied().unwrap_or((false, false)), r.zoom.get(&w.active).copied(), r.fs))
            .unwrap_or_default()
    });
    let z = zoom.unwrap_or_else(|| set.zoom_for(&url));
    let tabs = w
        .tabs
        .iter()
        .map(|t| TabView {
            id: t.id.clone(),
            url: t.url.clone(),
            title: t.title.clone(),
            icon: t.icon.clone(),
            pinned: t.pinned,
            muted: t.muted,
            audio: audio.contains(&t.id),
            loading: loading.contains(&t.id),
        })
        .collect();
    Some(BarState {
        id: w.id,
        tabs,
        active: w.active,
        max: w.max,
        full,
        incognito: w.incognito,
        back: nav.0,
        forward: nav.1,
        display: webcore::display_url(&url),
        kind: url_kind(&url),
        url,
        zoom: webcore::zoom_pct(z),
        zoomed: (z - set.zoom_for("")).abs() > 0.005,
        show_bm: set.show_bm,
        home: set.home_button,
        bookmarked,
        closed,
        engine: webcore::engine(&set.engine).name,
        dl: dl_badge(),
    })
}

fn bm_payload() -> BmPayload {
    bm_read(|b| BmPayload { bar: b.bar.clone(), other: b.other.clone(), icons: b.icons.clone() })
}

fn emit_state(app: &AppHandle, wid: &str) {
    if let Some(s) = bar_state(wid) {
        let _ = app.emit_to(bar_label(wid).as_str(), "web-state", s);
    }
}

/// 每個開著的視窗都更新工具列（設定、下載狀態這種大家共用的東西變了）
fn emit_state_all(app: &AppHandle) {
    for id in open_ids() {
        emit_state(app, &id);
    }
}

fn emit_main(app: &AppHandle) {
    let _ = app.emit_to("main", "web-changed", ());
}

/// 書籤改了：通知每個瀏覽器視窗與主程式
fn emit_bm(app: &AppHandle) {
    let p = bm_payload();
    for id in open_ids() {
        let _ = app.emit_to(bar_label(&id).as_str(), "web-bm", p.clone());
        emit_state(app, &id);
    }
    emit_main(app);
}

/// 請工具列做一件需要知道按鈕位置的事（打開主選單、書籤提示框、縮放提示框…）
fn ask_bar(app: &AppHandle, wid: &str, what: &str) {
    // 帶著視窗編號：別的視窗的工具列也會收到這個事件，要認得不是給自己的
    let _ = app.emit_to(bar_label(wid).as_str(), "web-ask", serde_json::json!({ "id": wid, "what": what }));
}

/* ---------------- 網址 ---------------- */

/// 一般網頁（http、https，而且不是程式自己的頁面）
fn web_url(u: &Url) -> bool {
    matches!(u.scheme(), "http" | "https") && !webcore::is_app_url(u)
}

fn parse_web(s: &str) -> Option<Url> {
    Url::parse(s).ok().filter(web_url)
}

fn ntp_local() -> Url {
    webcore::virtual_to_local(webcore::NEWTAB).unwrap_or_else(|| Url::parse("about:blank").unwrap())
}

/// 分頁記的網址 → 真正要載入的網址（chrome://… 是程式自己的頁面；擴充功能的頁面、檢視原始碼也可以）
fn page_url(s: &str) -> Url {
    let s = s.trim();
    if s.is_empty() {
        return ntp_local();
    }
    if let Some(u) = webcore::virtual_to_local(s) {
        return u;
    }
    Url::parse(s).ok().filter(|u| web_url(u) || matches!(u.scheme(), "chrome-extension" | "view-source")).unwrap_or_else(ntp_local)
}

/// 網頁目前真正的網址 → 分頁要記的（內部頁面記成 chrome://…；程式自己的其他頁面、about:blank 不記）
fn tab_url(u: &Url) -> Option<String> {
    if let Some(v) = webcore::local_to_virtual(u) {
        return Some(v);
    }
    (web_url(u) || matches!(u.scheme(), "chrome-extension" | "view-source")).then(|| u.to_string())
}

/// 分頁裡可以開的網址（外部網頁不能把分頁導到程式自己的其他頁面，例如主程式的畫面）
fn nav_allowed(u: &Url) -> bool {
    !webcore::is_app_target(u) || webcore::local_to_virtual(u).is_some()
}

/* ---------------- 視窗、分頁 ---------------- */

fn show_bm() -> bool {
    st().file.settings.show_bm
}

/// 上面幾排（分頁列、工具列、書籤列、底線）的高度（邏輯像素）。max = 放到最大或全螢幕（分頁列矮一點）
fn bar_h(max: bool) -> f64 {
    (if max { STRIP_MAX_H } else { STRIP_H }) + TOOL_H + if show_bm() { BMS_H } else { 0.0 } + LINE_H
}

/// 上面幾排的範圍（實體像素）
fn bar_rect(sf: f64, size: PhysicalSize<u32>, h: f64) -> (PhysicalPosition<i32>, PhysicalSize<u32>) {
    (PhysicalPosition::new(0, 0), PhysicalSize::new(size.width.max(1), (h * sf).round() as u32))
}

/// 網頁的範圍（實體像素）：左、右、下留一點邊給拉大小；放到最大時不用留
fn page_rect(sf: f64, size: PhysicalSize<u32>, max: bool, h: f64) -> (PhysicalPosition<i32>, PhysicalSize<u32>) {
    let edge = if max { 0 } else { (EDGE * sf).round() as u32 };
    let top = (h * sf).round() as u32;
    (
        PhysicalPosition::new(edge as i32, top as i32),
        PhysicalSize::new(size.width.saturating_sub(2 * edge).max(1), size.height.saturating_sub(top + edge).max(1)),
    )
}

fn bounds(pos: PhysicalPosition<i32>, size: PhysicalSize<u32>) -> tauri::Rect {
    tauri::Rect { position: pos.into(), size: size.into() }
}

/// 視窗目前的排法：(工具列的範圍, 工具列要不要顯示, 網頁的範圍)
#[allow(clippy::type_complexity)]
fn geometry(wid: &str, win: &Window, size: PhysicalSize<u32>) -> ((PhysicalPosition<i32>, PhysicalSize<u32>), bool, (PhysicalPosition<i32>, PhysicalSize<u32>)) {
    let sf = win.scale_factor().unwrap_or(1.0).max(0.5);
    let (full, fs, reveal) = rt(|m| m.get(wid).map(|r| (r.full, r.fs, r.reveal)).unwrap_or_default());
    let max = win.is_maximized().unwrap_or(false) || fs;
    let h = bar_h(max);
    let hide_bar = full || (fs && !reveal);
    let page = if hide_bar { (PhysicalPosition::new(0, 0), size) } else { page_rect(sf, size, max, h) };
    (bar_rect(sf, size, h), !hide_bar, page)
}

/// 視窗大小改變：工具列和所有分頁的網頁一次放到正確位置（一個網頁只設定一次，不會中途變成奇怪的大小）
fn layout(app: &AppHandle, wid: &str, win: &Window, size: PhysicalSize<u32>) {
    let ((bp, bs), show_bar, (pos, sz)) = geometry(wid, win, size);
    if let Some(b) = app.get_webview(&bar_label(wid)) {
        let _ = b.set_bounds(bounds(bp, bs));
        let _ = if show_bar { b.show() } else { b.hide() };
    }
    for t in get(wid).map(|w| w.tabs).unwrap_or_default() {
        if let Some(p) = app.get_webview(&page_label(wid, &t.id)) {
            let _ = p.set_bounds(bounds(pos, sz));
        }
    }
}

/// 重新排一次（書籤列開關、全螢幕這些讓工具列高度改變的時候）
fn relayout(app: &AppHandle, wid: &str) {
    if let Some(win) = app.get_window(&win_label(wid)) {
        if let Ok(size) = win.inner_size() {
            if size.width > 0 && size.height > 0 {
                layout(app, wid, &win, size);
            }
        }
    }
}

/// 瀏覽器視窗的網頁用自己的瀏覽器資料夾（登入狀態、擴充功能都放這裡）
fn profile_dir() -> PathBuf {
    WEB_DIR.get().cloned().unwrap_or_default().join("web-profile")
}

/// 從 Chrome 同步來的擴充功能
fn ext_dir() -> PathBuf {
    WEB_DIR.get().cloned().unwrap_or_default().join("web-ext")
}

static WEB_DIR: OnceLock<PathBuf> = OnceLock::new();
/// 載入擴充功能失敗過（例如 WebView2 太舊）：之後不再載入，避免網頁開不起來
static EXT_BROKEN: AtomicBool = AtomicBool::new(false);
static POPUP_N: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);

/// 在主執行緒對一個網頁（或工具列）的 WebView2 做事
#[cfg(windows)]
fn with_wv(wv: &Webview, f: impl FnOnce(&webnative::Wv) + Send + 'static) {
    let _ = wv.with_webview(move |pw| {
        let controller = pw.controller();
        if let Ok(core) = unsafe { controller.CoreWebView2() } {
            f(&webnative::Wv { controller, core });
        }
    });
}

#[cfg(not(windows))]
fn with_wv(_wv: &Webview, _f: impl FnOnce(&webnative::Wv) + Send + 'static) {}

/// WebView2 通知的事情先排隊，由一個背景執行緒照順序處理（不可以在通知裡面直接動視窗）
static EVQ: OnceLock<Mutex<std::sync::mpsc::Sender<(String, String, Ev)>>> = OnceLock::new();

fn push_ev(wid: &str, tid: &str, ev: Ev) {
    if let Some(tx) = EVQ.get() {
        let _ = lock(tx).send((wid.to_string(), tid.to_string(), ev));
    }
}

/// 掛上快捷鍵、右鍵選單、下載、縮放…（page = 一般網頁；false = 工具列）
fn attach_native(wv: &Webview, wid: &str, tid: &str, page: bool) {
    let (w, t) = (wid.to_string(), tid.to_string());
    let sink: webev::Sink = Arc::new(move |ev| push_ev(&w, &t, ev));
    let (set, incognito) = {
        let s = st();
        (s.file.settings.clone(), s.file.wins.iter().any(|x| x.id == wid && x.incognito))
    };
    let opts = webev::Opts {
        page,
        accel: if page { webcore::shortcut } else { webcore::shortcut_bar },
        no_repeat: webcore::no_repeat,
        own_page: webcore::is_app_str,
        web_first: page && tab_of(wid, tid).is_some_and(|t| parse_web(&t.url).is_some()),
        save_passwords: set.save_passwords && !incognito,
        download_dir: download_dir(wv.app_handle()),
    };
    with_wv(wv, move |w| webnative::attach(w, opts, sink));
}

/// 下載要存的資料夾：設定裡沒指定就是 Windows 的「下載」。
/// WebView2 會把這個資料夾記在瀏覽器資料裡，所以「恢復預設」時也要明確設回去，不然會一直用上次選的
pub(crate) fn download_dir(app: &AppHandle) -> String {
    let set = st().file.settings.download_dir.clone();
    if !set.trim().is_empty() {
        return set;
    }
    app.path().download_dir().map(|p| p.to_string_lossy().into_owned()).unwrap_or_default()
}

/// 登入用的小視窗（網頁用 window.open 指定大小開的）：開成真的視窗，才能打字、登入完會自己關掉。
/// wid = 開它的網頁所在的瀏覽器視窗
fn popup_window(app: &AppHandle, wid: &str, url: &Url, features: tauri::webview::NewWindowFeatures) -> Option<WebviewWindow> {
    let n = POPUP_N.fetch_add(1, Ordering::SeqCst) + 1;
    let label = format!("webpop-{n}");
    let (a2, w2) = (app.clone(), wid.to_string());
    // 要跟網頁用同一個設定檔、同樣的環境選項：WebView2 才會把它當成同一個瀏覽器開出來的視窗
    // （開它的網頁才拿得到 window.opener、登入結果才送得回去；Google 登入 X.com 就是靠這個）
    let win = WebviewWindowBuilder::new(app, &label, WebviewUrl::External("about:blank".parse().ok()?))
        .data_directory(profile_dir())
        .browser_extensions_enabled(true)
        // 無痕式視窗開的小視窗也要是無痕的：不然 WebView2 不接受，小視窗會一片空白、網站以為被擋掉
        .incognito(is_incognito(wid))
        .window_features(features)
        .title(url.host_str().unwrap_or("登入"))
        .focused(true)
        // 小視窗不能開程式自己的頁面
        .on_navigation(|u| !webcore::is_app_target(u))
        .on_document_title_changed(|w, title| {
            let _ = w.set_title(&title);
        })
        // 小視窗裡會開新視窗的連結（說明、忘記密碼、建立帳戶…）：指定大小的再開一個小視窗，
        // 其他的開在原本那個瀏覽器視窗的新分頁，並把那個視窗叫到前面（跟 Chrome 一樣）
        .on_new_window(move |u, features| {
            if webcore::is_app_target(&u) {
                return NewWindowResponse::Deny;
            }
            if features.size().is_some() {
                return match popup_window(&a2, &w2, &u, features) {
                    Some(window) => NewWindowResponse::Create { window },
                    None => NewWindowResponse::Deny,
                };
            }
            if web_url(&u) {
                let (app, wid) = (a2.clone(), w2.clone());
                std::thread::spawn(move || {
                    let target = match add_tab(&app, &wid, u.to_string(), true, None) {
                        Ok(_) => Some(wid.clone()),
                        // 原本的視窗已經關了：開一個新的
                        Err(_) => new_window(&app, Some(u.to_string()), is_incognito(&wid), None).ok(),
                    };
                    let hwnd = target.map(|id| rt(|m| m.get(&id).map(|r| r.hwnd).unwrap_or(0))).unwrap_or(0);
                    if hwnd != 0 {
                        let _ = app.run_on_main_thread(move || win32::activate(hwnd));
                    }
                });
            }
            NewWindowResponse::Deny
        })
        .build()
        .ok()?;
    // 網頁自己關視窗（window.close()，登入完成時）：WebView2 只會把裡面的網頁收掉，視窗要自己關，
    // 不然會留一個空白的視窗在畫面上
    {
        let (a3, l3) = (app.clone(), label.clone());
        with_wv(win.as_ref(), move |w| {
            webnative_close_hook(w, a3, l3);
        });
    }
    // 遊戲在前景時點網頁不會切換前景，Windows 就不讓新開的小視窗變成前景（打字會跑到遊戲）：主動叫到前景
    #[cfg(windows)]
    if let Ok(h) = win.hwnd() {
        let h = h.0 as isize;
        let _ = app.run_on_main_thread(move || win32::activate(h));
    }
    Some(win)
}

fn page_builder(app: &AppHandle, wid: &str, tid: &str, url: Url, ext: Option<PathBuf>, incognito: bool) -> WebviewBuilder<tauri::Wry> {
    let (a1, a2, a3, a4) = (app.clone(), app.clone(), app.clone(), app.clone());
    let (w1, w2, w3, w4) = (wid.to_string(), wid.to_string(), wid.to_string(), wid.to_string());
    let (t1, t2, t3, t4) = (tid.to_string(), tid.to_string(), tid.to_string(), tid.to_string());
    // 新分頁不要搶走焦點（網址列要能直接打字）
    let blank = webcore::local_to_virtual(&url).is_some_and(|v| webcore::is_newtab(&v));
    let mut b = WebviewBuilder::new(page_label(wid, tid), WebviewUrl::External(url))
        .data_directory(profile_dir())
        .browser_extensions_enabled(true)
        .incognito(incognito)
        .zoom_hotkeys_enabled(true)
        .focused(!blank);
    if let Some(dir) = ext {
        b = b.extensions_path(dir);
    }
    // 使用者腳本（document-start 的）：網頁一開始就跑；程式自己的頁面不跑
    let us = crate::userscripts::start_script();
    if !us.is_empty() {
        let origin = webcore::ui_base().trim_end_matches('/');
        b = b.initialization_script_for_all_frames(format!("if (location.origin !== {origin:?}) {{ {us} }}"));
    }
    b.initialization_script_for_all_frames(TYPING_JS)
        // 讓網頁自己的拖放（例如上傳檔案、排序清單）可以正常使用
        .disable_drag_drop_handler()
        .on_navigation(move |u| nav_check(&a4, &w4, &t4, u))
        .on_page_load(move |wv, p| {
            on_page(&a1, &w1, &t1, &wv, p.url(), matches!(p.event(), PageLoadEvent::Started));
        })
        .on_document_title_changed(move |_wv, raw| on_title(&a2, &w2, &t2, raw))
        // 登入用的小視窗開成真的視窗；其他開新分頁的連結：在這個視窗開一個新分頁
        .on_new_window(move |u, features| {
            // WebView2 這次「開新視窗」的要求（每次都要拿，順序才對得上）
            let req = webnative::take_new_window(|raw| Url::parse(raw).is_ok_and(|x| x == u));
            if webcore::is_app_target(&u) {
                return NewWindowResponse::Deny;
            }
            if features.size().is_some() {
                return match popup_window(&a3, &w3, &u, features) {
                    Some(window) => NewWindowResponse::Create { window },
                    None => NewWindowResponse::Allow,
                };
            }
            // 跟 Chrome 一樣：Ctrl + 點或中鍵點 = 在背景開新分頁；Shift + 點 = 開新視窗
            let (ctrl, shift, middle) = win32::click_mods();
            let plain = !(shift && !ctrl && !middle) && !((ctrl || middle) && !shift);
            // 同一個網址已經開著：切過去就好，不要一直開新分頁
            let same = if web_url(&u) { get(&w3).and_then(|w| w.tabs.into_iter().find(|t| same_url(&t.url, u.as_str()))).map(|t| t.id) } else { None };
            // 一般的情況：開成新分頁，而且跟開它的網頁連著（登入頁、付款頁靠這個把結果傳回來）。
            // 網站先開空白分頁再前往（about:blank）、開自己產生的檔案（blob:）也算
            let linkable = web_url(&u) || u.as_str() == "about:blank" || u.scheme() == "blob";
            if let Some(req) = req {
                if plain && same.is_none() && linkable && !req.from_own() && linked_tab(&a3, &w3, &t3, &u, &features, req) {
                    return NewWindowResponse::Deny;
                }
            }
            if web_url(&u) {
                let (app, wid, tid) = (a3.clone(), w3.clone(), t3.clone());
                std::thread::spawn(move || {
                    if shift && !ctrl && !middle {
                        let _ = new_window(&app, Some(u.to_string()), is_incognito(&wid), Some(&wid));
                        return;
                    }
                    if (ctrl || middle) && !shift {
                        let _ = add_tab(&app, &wid, u.to_string(), false, Some(&tid));
                        return;
                    }
                    match same {
                        Some(t) => {
                            let _ = select_tab(&app, &wid, &t);
                        }
                        None => {
                            if let Ok(new) = add_tab(&app, &wid, u.to_string(), true, Some(&tid)) {
                                set_opener(&wid, &new, &tid);
                            }
                        }
                    }
                });
            }
            NewWindowResponse::Deny
        })
}

/// 登入小視窗的網頁自己關視窗時，把整個視窗關掉
#[cfg(windows)]
fn webnative_close_hook(w: &webnative::Wv, app: AppHandle, label: String) {
    webnative::on_close_requested(w, move || {
        let (app, label) = (app.clone(), label.clone());
        // 不可以在通知裡面直接動視窗
        std::thread::spawn(move || {
            if let Some(win) = app.get_webview_window(&label) {
                let _ = win.close();
            }
        });
    });
}

#[cfg(not(windows))]
fn webnative_close_hook(_w: &webnative::Wv, _app: AppHandle, _label: String) {}

/// 記下分頁是從哪個分頁開出來的（關掉時回到那個分頁）
fn set_opener(wid: &str, tid: &str, opener: &str) {
    rt(|m| {
        if let Some(r) = m.get_mut(wid) {
            r.opener.insert(tid.to_string(), opener.to_string());
        }
    });
}

/// 分頁要前往某個網址：可不可以。順便記下這個分頁是不是在程式自己的頁面上。
/// 「開過開發人員工具」和「在自己的頁面上」放在同一把鎖裡判斷，兩件事不會同時成立：
/// 程式自己的頁面可以呼叫程式內部的功能，開發人員工具開在上面等於把內部整個攤開
fn nav_check(app: &AppHandle, wid: &str, tid: &str, u: &Url) -> bool {
    if !nav_allowed(u) {
        return false;
    }
    let own = webcore::is_app_target(u);
    let blocked = rt(|m| {
        let Some(r) = m.get_mut(wid) else { return false };
        if own && r.devtools.contains(tid) {
            return true;
        }
        // 正要前往程式自己的頁面：還沒真的換過去就先記（前往途中也不能開開發人員工具），改成前往別的網頁才拿掉。
        // 「已經顯示著自己的頁面」另外記（見 on_own_page），要等真的換到別的網頁才拿掉
        if own {
            r.heading_own.insert(tid.to_string());
        } else {
            r.heading_own.remove(tid);
        }
        false
    });
    if blocked {
        // 開過開發人員工具的分頁：程式自己的頁面改開在新分頁
        if let Some(v) = webcore::local_to_virtual(u) {
            let (app, wid, tid) = (app.clone(), wid.to_string(), tid.to_string());
            std::thread::spawn(move || {
                let _ = add_tab(&app, &wid, v, true, Some(&tid));
            });
        }
        return false;
    }
    true
}

/// 分頁真的換到程式自己的頁面了
fn on_own_page(wid: &str, tid: &str) {
    rt(|m| {
        if let Some(r) = m.get_mut(wid) {
            r.own_page.insert(tid.to_string());
        }
    });
}

/// 分頁真的換到一般網頁了：不再算「在自己的頁面上」
fn left_own_page(wid: &str, tid: &str) {
    rt(|m| {
        if let Some(r) = m.get_mut(wid) {
            r.own_page.remove(tid);
        }
    });
}

/// 這個分頁可不可以開開發人員工具（只有一般網頁可以）；可以就記下「開過」
pub(crate) fn devtools_mark(wid: &str, tid: &str) -> bool {
    if !tab_of(wid, tid).is_some_and(|t| parse_web(&t.url).is_some()) {
        return false;
    }
    rt(|m| {
        m.get_mut(wid).is_some_and(|r| {
            if r.own_page.contains(tid) || r.heading_own.contains(tid) {
                return false;
            }
            r.devtools.insert(tid.to_string());
            true
        })
    })
}

fn devtools_used(wid: &str, tid: &str) -> bool {
    rt(|m| m.get(wid).is_some_and(|r| r.devtools.contains(tid)))
}

/// 開發人員工具（F12）：開成另一個視窗
fn open_devtools(app: &AppHandle, wid: &str, tid: &str) {
    if !devtools_mark(wid, tid) {
        return;
    }
    if let Some(p) = app.get_webview(&page_label(wid, tid)) {
        with_wv(&p, |w| {
            webnative::open_devtools(w, webcore::is_app_str);
        });
    }
}

/// 網頁要開新分頁（window.open 沒指定大小、開新分頁的連結、送到新分頁的表單）：開成新分頁，並且把分頁交還給
/// WebView2 當成這次開出來的視窗——網站拿得到開出來的分頁（不會以為被擋掉）、新分頁拿得到 window.opener、
/// 表單資料也帶得過去。要在主執行緒呼叫（WebView2 通知「要開新視窗」的當下）。
/// 回傳 false = 沒辦法這樣開（呼叫的人改用原本的方式：自己開一個分頁載入那個網址）
#[cfg(windows)]
fn linked_tab(app: &AppHandle, wid: &str, opener: &str, u: &Url, features: &tauri::webview::NewWindowFeatures, req: webnative::NewWin) -> bool {
    let Some(win) = app.get_window(&win_label(wid)) else { return false };
    let Ok(blank) = Url::parse("about:blank") else { return false };
    let incognito = is_incognito(wid);
    // 還沒載入前，分頁先記要去的網址（空白頁就是 about:blank）
    let shown = tab_url(u).unwrap_or_else(|| "about:blank".to_string());
    let Some(tid) = edit(wid, |w| {
        w.next_tab += 1;
        let tid = format!("t{}", w.next_tab);
        let pinned = w.tabs.iter().filter(|t| t.pinned).count();
        let pos = w.tabs.iter().position(|t| t.id == opener).map(|i| i + 1).unwrap_or(w.tabs.len()).max(pinned);
        w.tabs.insert(pos, Tab::new(tid.clone(), shown));
        tid
    }) else {
        return false;
    };
    let drop_tab = |tid: &str| {
        edit(wid, |w| w.tabs.retain(|t| t.id != tid));
    };
    let size = win.inner_size().unwrap_or(PhysicalSize::new(860, 560));
    let (_, _, (pos, sz)) = geometry(wid, &win, size);
    // 一定要跟開它的網頁同一個環境、同一個設定檔，而且還沒載入過別的網址
    let b = page_builder(app, wid, &tid, blank, None, incognito).with_environment(features.opener().environment.clone());
    let made = match win.add_child(b, pos, sz) {
        Ok(w) => w,
        Err(e) => {
            eprintln!("新分頁建立失敗：{e}");
            drop_tab(&tid);
            return false;
        }
    };
    attach_native(&made, wid, &tid, true);
    let ok = Arc::new(AtomicBool::new(false));
    let ok2 = ok.clone();
    with_wv(&made, move |w| ok2.store(req.adopt(w), Ordering::SeqCst));
    if !ok.load(Ordering::SeqCst) {
        // 交不出去：這個分頁不要了，讓呼叫的人用原本的方式開
        let _ = made.close();
        drop_tab(&tid);
        return false;
    }
    rt(|m| {
        if let Some(r) = m.get_mut(wid) {
            r.opener.insert(tid.clone(), opener.to_string());
            // 從開過開發人員工具的分頁開出來的，也當成開過（開發人員工具可能會自動跟著開）
            if r.devtools.contains(opener) {
                r.devtools.insert(tid.clone());
            }
        }
    });
    // 切過去（顯示它、收起原本的、更新工具列）交給背景做
    let (app, wid) = (app.clone(), wid.to_string());
    std::thread::spawn(move || {
        let _ = select_tab(&app, &wid, &tid);
    });
    true
}

#[cfg(not(windows))]
fn linked_tab(_app: &AppHandle, _wid: &str, _opener: &str, _u: &Url, _features: &tauri::webview::NewWindowFeatures, _req: webnative::NewWin) -> bool {
    false
}

/// 網頁載入完成後：取得網站圖示的網址與標題（只回傳這兩樣，不做其他事）
const INFO_JS: &str = r#"(() => {
  let best = "", score = -1;
  for (const l of document.querySelectorAll("link[rel]")) {
    const rel = (l.getAttribute("rel") || "").toLowerCase();
    if (!rel.split(/\s+/).includes("icon") && !rel.includes("apple-touch-icon")) continue;
    if (!l.href) continue;
    const size = parseInt((l.getAttribute("sizes") || "").split(/x/i)[0], 10) || 16;
    const s = (rel.includes("apple") ? 0 : 1000) + (100 - Math.min(99, Math.abs(32 - size)));
    if (s > score) { score = s; best = l.href; }
  }
  return { icon: best || (location.origin + "/favicon.ico"), title: document.title || "" };
})()"#;

/// 把這個網址該用的縮放套到網頁上（縮放是照網站記的）
fn apply_zoom(wv: &Webview, wid: &str, tid: &str, url: &str) {
    let z = st().file.settings.zoom_for(url);
    rt(|m| {
        if let Some(r) = m.get_mut(wid) {
            r.zoom.insert(tid.to_string(), z);
            r.zoom_set.insert(tid.to_string(), z);
        }
    });
    with_wv(wv, move |w| webnative::set_zoom(w, z));
}

fn on_page(app: &AppHandle, wid: &str, tid: &str, wv: &Webview, url: &Url, started: bool) {
    if webcore::is_app_target(url) {
        on_own_page(wid, tid);
        // 保險：開過開發人員工具的分頁不應該載入得了程式自己的頁面（nav_check 會擋）。
        // 萬一還是載入了：換一個新的分頁來開，這個關掉（開發人員工具會跟著關）
        if devtools_used(wid, tid) {
            let (app2, w2, t2) = (app.clone(), wid.to_string(), tid.to_string());
            let v = webcore::local_to_virtual(url).unwrap_or_else(|| HOME.to_string());
            std::thread::spawn(move || {
                let _ = add_tab(&app2, &w2, v, true, Some(&t2));
                let _ = close_tab(&app2, &w2, &t2);
            });
            return;
        }
    } else if !started {
        left_own_page(wid, tid);
    }
    let shown = shown_url(wid, tid, url, started);
    // 檢視原始碼的頁面：不跑使用者腳本、不記到歷史記錄
    let viewing = shown.as_deref().is_some_and(|s| s.starts_with("view-source:"));
    if let Some(s) = &shown {
        edit(wid, |w| {
            if let Some(t) = w.tabs.iter_mut().find(|t| t.id == tid) {
                // 換到別的網站：舊網站的圖示先拿掉
                if started && webcore::zoom_key(&t.url) != webcore::zoom_key(s) {
                    t.icon.clear();
                }
                t.url = s.clone();
            }
            if w.active == tid {
                w.url = s.clone();
            }
        });
    }
    rt(|m| {
        if let Some(r) = m.get_mut(wid) {
            if started {
                r.loading.insert(tid.to_string());
            } else {
                r.loading.remove(tid);
            }
        }
    });
    if started {
        if let Some(s) = &shown {
            apply_zoom(wv, wid, tid, s);
        }
    } else if web_url(url) && !viewing {
        // 使用者腳本（載入完成後跑的）
        for code in crate::userscripts::scripts_for(url.as_str()) {
            let _ = wv.eval(&code);
        }
        let (app2, w2, t2, u2) = (app.clone(), wid.to_string(), tid.to_string(), url.to_string());
        let _ = wv.eval_with_callback(INFO_JS, move |res| {
            let v: Value = serde_json::from_str(&res).unwrap_or_default();
            let title = v["title"].as_str().unwrap_or("").trim_end_matches(MARK).trim().to_string();
            // 記到歷史記錄（無痕式視窗不記）
            if !is_incognito(&w2) {
                hist_edit(|h| h.add(&u2, &title, webcore::now_ms()));
            }
            if let Some(icon) = v["icon"].as_str() {
                set_icon(&app2, &w2, &t2, icon.to_string());
            }
        });
    }
    emit_state(app, wid);
    emit_main(app);
}

/// 網址變了但沒有重新載入（YouTube 這類不換頁就換網址的網站）
fn on_source(app: &AppHandle, wid: &str, tid: &str, raw: &str) {
    if webcore::is_app_str(raw) {
        on_own_page(wid, tid);
    } else {
        left_own_page(wid, tid);
    }
    let Some(shown) = Url::parse(raw).ok().and_then(|u| shown_url(wid, tid, &u, false)) else { return };
    let loading = rt(|m| m.get(wid).is_some_and(|r| r.loading.contains(tid)));
    let changed = edit(wid, |w| {
        let mut title = None;
        if let Some(t) = w.tabs.iter_mut().find(|t| t.id == tid) {
            if t.url != shown {
                t.url = shown.clone();
                title = Some(t.title.clone());
            }
        }
        if title.is_some() && w.active == tid {
            w.url = shown.clone();
        }
        title
    })
    .flatten();
    let Some(title) = changed else { return };
    if !loading && parse_web(&shown).is_some() && !is_incognito(wid) {
        hist_edit(|h| h.add(&shown, &title, webcore::now_ms()));
    }
    emit_state(app, wid);
    emit_main(app);
}

fn set_icon(app: &AppHandle, wid: &str, tid: &str, icon: String) {
    let ok = icon.starts_with("https://")
        || icon.starts_with("http://")
        || (icon.starts_with("data:image/") && icon.len() <= 16_000);
    if !ok || icon.len() > 16_000 {
        return;
    }
    let (changed, url) = edit(wid, |w| match w.tabs.iter_mut().find(|t| t.id == tid) {
        Some(t) => {
            // 內部頁面用自己的圖示
            if webcore::virtual_page(&t.url).is_some() {
                return (false, String::new());
            }
            let ch = t.icon != icon;
            t.icon = icon.clone();
            (ch, t.url.clone())
        }
        None => (false, String::new()),
    })
    .unwrap_or((false, String::new()));
    if url.is_empty() {
        return;
    }
    // 歷史記錄、建議清單、新分頁的捷徑用：記住這個網站的圖示
    if !is_incognito(wid) {
        let host = webcore::host_of(&url);
        hist_edit(|h| {
            h.set_icon(&host, &icon);
        });
    }
    // 書籤用：記住這個網站的圖示（只記有加入書籤的網站）
    let learned = host_of(&url).is_some_and(|host| {
        let mut s = st();
        let b = &mut s.file.bookmarks;
        if !(has_host(&b.bar, &host) || has_host(&b.other, &host)) || b.icons.get(&host) == Some(&icon) {
            return false;
        }
        b.icons.insert(host, icon.clone());
        s.dirty = true;
        true
    });
    if learned {
        emit_bm(app);
    }
    if changed {
        emit_state(app, wid);
    }
}

fn on_title(app: &AppHandle, wid: &str, tid: &str, raw: String) {
    let typing = raw.ends_with(MARK);
    let title = raw.trim_end_matches(MARK).trim().to_string();
    let Some(w) = get(wid) else { return };
    if w.active == tid {
        rt(|m| {
            if let Some(r) = m.get_mut(wid) {
                if r.page_typing && !typing {
                    r.page_off_at = Some(Instant::now());
                }
                if !r.page_typing && typing {
                    r.page_on_at = Some(Instant::now());
                }
                r.page_typing = typing;
            }
        });
    }
    let changed = edit(wid, |w| {
        let mut ch = None;
        if let Some(t) = w.tabs.iter_mut().find(|t| t.id == tid) {
            if t.title != title {
                t.title = title.clone();
                ch = Some(t.url.clone());
            }
        }
        if w.active == tid {
            w.title = title.clone();
        }
        ch
    })
    .flatten();
    if let Some(url) = changed {
        if !title.is_empty() && parse_web(&url).is_some() && !w.incognito {
            hist_edit(|h| {
                h.set_title(&url, &title);
            });
        }
        emit_state(app, wid);
        emit_main(app);
        // 工作列上顯示目前分頁的標題
        if get(wid).is_some_and(|w| w.active == tid) {
            if let Some(win) = app.get_window(&win_label(wid)) {
                let _ = win.set_title(&window_title(&title, w.incognito));
            }
        }
    }
}

fn window_title(title: &str, incognito: bool) -> String {
    let t = if title.is_empty() { "新分頁" } else { title };
    if incognito {
        format!("{t} - 無痕模式")
    } else {
        t.to_string()
    }
}

/// 擴充功能裝進瀏覽器後會一直留著；只有新的、更新過的（放在「待安裝」資料夾）才裝一次。
/// （每開一個分頁都重新裝的話，擴充功能會以為剛更新，一直開「已更新」的分頁）
fn ext_pending() -> Option<PathBuf> {
    if EXT_BROKEN.load(Ordering::SeqCst) {
        return None;
    }
    let st = ext_state();
    if st.pending.is_empty() {
        return None;
    }
    let dir = ext_dir().join(&st.pending);
    let any = std::fs::read_dir(&dir).map(|mut d| d.any(|e| e.is_ok_and(|e| e.path().join("manifest.json").is_file()))).unwrap_or(false);
    any.then_some(dir)
}

/// 待安裝的都裝好了
fn ext_mark_loaded() {
    let _g = lock(&EXT_LOCK);
    let mut st = ext_state();
    let pending = std::mem::take(&mut st.pending);
    for m in st.exts.values_mut() {
        if m.dir.starts_with(&format!("{pending}/")) {
            m.loaded = true;
        }
    }
    save_ext_state(&st);
}

/// 加上分頁的網頁；載入擴充功能失敗就不載入再試一次
fn add_page(app: &AppHandle, win: &Window, wid: &str, tid: &str, url: Url, pos: PhysicalPosition<i32>, size: PhysicalSize<u32>) -> Result<(), String> {
    source_mark(wid, tid, &url);
    let incognito = is_incognito(wid);
    // 無痕式視窗不載入擴充功能
    let ext = if incognito { None } else { ext_pending() };
    let had = ext.is_some();
    let made = match win.add_child(page_builder(app, wid, tid, url.clone(), ext, incognito), pos, size) {
        Ok(w) => {
            if had {
                ext_mark_loaded();
            }
            Ok(w)
        }
        Err(e) if had => {
            eprintln!("擴充功能載入失敗：{e}");
            EXT_BROKEN.store(true, Ordering::SeqCst);
            win.add_child(page_builder(app, wid, tid, url, None, incognito), pos, size).map_err(|e| format!("網頁建立失敗：{e}"))
        }
        Err(e) => Err(format!("網頁建立失敗：{e}")),
    }?;
    attach_native(&made, wid, tid, true);
    // 靜音的分頁維持靜音
    if get(wid).is_some_and(|w| w.tabs.iter().any(|t| t.id == tid && t.muted)) {
        with_wv(&made, |w| webnative::set_muted(w, true));
    }
    Ok(())
}

/// 視窗的全螢幕狀態（網頁的影片全螢幕，或 F11）變了：套用並重新排
fn apply_full(app: &AppHandle, wid: &str) {
    pop::hide_of(app, wid);
    close_ext_popup(app, wid);
    let Some(win) = app.get_window(&win_label(wid)) else { return };
    let on = rt(|m| m.get(wid).is_some_and(|r| r.full || r.fs));
    if win.is_fullscreen().unwrap_or(false) != on {
        let _ = win.set_fullscreen(on);
    }
    relayout(app, wid);
    emit_state(app, wid);
}

/// 網頁按影片的全螢幕：整個視窗變全螢幕、網頁蓋滿；離開時還原
fn set_fullscreen(app: &AppHandle, wid: &str, on: bool) {
    let changed = rt(|m| m.get_mut(wid).map(|r| std::mem::replace(&mut r.full, on) != on)).unwrap_or(false);
    if changed {
        apply_full(app, wid);
    }
}

/// F11：整個視窗全螢幕（滑鼠移到最上面會暫時顯示工具列）
fn toggle_fs(app: &AppHandle, wid: &str) {
    let ok = rt(|m| {
        m.get_mut(wid).map(|r| {
            r.fs = !r.fs;
            r.reveal = false;
        })
    })
    .is_some();
    if ok {
        apply_full(app, wid);
    }
}

/// 建立分頁的網頁（要在主執行緒以外呼叫）
/// 分頁要前往某個網址：是「檢視原始碼」（view-source:…）就記下被檢視的網址，不是就取消
fn source_mark(wid: &str, tid: &str, target: &Url) {
    let inner = (target.scheme() == "view-source").then(|| target.as_str().trim_start_matches("view-source:")).and_then(|u| Url::parse(u).ok()).map(|u| u.to_string());
    rt(|m| {
        if let Some(r) = m.get_mut(wid) {
            match inner {
                Some(u) => {
                    r.source.insert(tid.to_string(), u);
                }
                None => {
                    r.source.remove(tid);
                }
            }
        }
    });
}

/// WebView2 回報的網址 → 分頁要記的網址（正在檢視原始碼的那個網址，補回 view-source: 開頭）。
/// leaving = 開始前往別的網址了：不再是檢視原始碼
fn shown_url(wid: &str, tid: &str, reported: &Url, leaving: bool) -> Option<String> {
    let s = tab_url(reported)?;
    let viewing = rt(|m| {
        let r = m.get_mut(wid)?;
        match r.source.get(tid) {
            Some(u) if *u == s => Some(true),
            Some(_) if leaving => {
                r.source.remove(tid);
                Some(false)
            }
            _ => Some(false),
        }
    })
    .unwrap_or(false);
    Some(if viewing { format!("view-source:{s}") } else { s })
}

fn create_page(app: &AppHandle, win: &Window, wid: &str, tid: &str) -> Result<(), String> {
    let url = get(wid).and_then(|w| w.tabs.into_iter().find(|t| t.id == tid)).map(|t| t.url).unwrap_or_default();
    let url = page_url(&url);
    let size = win.inner_size().unwrap_or(PhysicalSize::new(860, 560));
    let (_, _, (pos, sz)) = geometry(wid, win, size);
    add_page(app, win, wid, tid, url, pos, sz)
}

fn select_tab(app: &AppHandle, wid: &str, tid: &str) -> Result<(), String> {
    close_ext_popup(app, wid);
    pop::hide_of(app, wid);
    let (prev, title, incognito) = edit(wid, |w| {
        let t = w.tabs.iter().find(|t| t.id == tid)?.clone();
        w.url = t.url;
        w.title = t.title.clone();
        Some((std::mem::replace(&mut w.active, tid.to_string()), t.title, w.incognito))
    })
    .flatten()
    .ok_or("找不到這個分頁")?;
    if let Some(win) = app.get_window(&win_label(wid)) {
        // 工作列上的標題跟著換成這個分頁的
        let _ = win.set_title(&window_title(&title, incognito));
        if app.get_webview(&page_label(wid, tid)).is_none() {
            create_page(app, &win, wid, tid)?;
        }
        if let Some(p) = app.get_webview(&page_label(wid, tid)) {
            let _ = p.show();
        }
        if prev != tid {
            if let Some(p) = app.get_webview(&page_label(wid, &prev)) {
                let _ = p.hide();
            }
        }
    }
    rt(|m| {
        if let Some(r) = m.get_mut(wid) {
            r.page_typing = false;
            // 切到別的分頁了：其他分頁「是從哪個分頁開的」不再算（跟 Chrome 一樣，關掉時不會跳回去）
            r.opener.retain(|child, _| child == tid);
        }
    });
    // 這個視窗正在用（它是前景視窗）：鍵盤交給新顯示的網頁，快捷鍵、捲動才會作用在它上面。
    // 不然關掉分頁（Ctrl+W）之後鍵盤沒有人接，再按快捷鍵都沒反應。遊戲在前景時不動鍵盤
    if is_foreground(wid) {
        if let Some(p) = app.get_webview(&page_label(wid, tid)) {
            let _ = p.set_focus();
        }
    }
    emit_state(app, wid);
    emit_main(app);
    Ok(())
}

/// 開新分頁（url 是分頁要記的網址，例如 https://… 或 chrome://newtab/）。
/// after = 放在哪個分頁後面（沒有就放最後）；固定的分頁永遠在最前面
fn add_tab(app: &AppHandle, wid: &str, url: String, activate: bool, after: Option<&str>) -> Result<String, String> {
    let url = if url.trim().is_empty() { HOME.to_string() } else { url };
    let tid = edit(wid, |w| {
        w.next_tab += 1;
        let tid = format!("t{}", w.next_tab);
        let pinned = w.tabs.iter().filter(|t| t.pinned).count();
        let pos = after.and_then(|a| w.tabs.iter().position(|t| t.id == a)).map(|i| i + 1).unwrap_or(w.tabs.len()).max(pinned);
        let title = webcore::virtual_page(&url).map(webcore::ui_title).unwrap_or("").to_string();
        w.tabs.insert(pos, Tab { title, ..Tab::new(tid.clone(), url) });
        tid
    })
    .ok_or("找不到這個視窗")?;
    if activate {
        select_tab(app, wid, &tid)?;
    } else {
        emit_state(app, wid);
    }
    Ok(tid)
}

/// 開新分頁：沒給網址就是「新分頁」頁面
fn new_tab(app: &AppHandle, wid: &str, url: Option<Url>, activate: bool, after: Option<&str>) -> Result<String, String> {
    add_tab(app, wid, url.map(|u| u.to_string()).unwrap_or_default(), activate, after)
}

/// 把分頁導到某個網址（分頁要記的網址：https://…、chrome://…、view-source:…）
fn navigate_tab(app: &AppHandle, wid: &str, tid: &str, target: &str) {
    let url = page_url(target);
    match app.get_webview(&page_label(wid, tid)) {
        Some(p) => {
            source_mark(wid, tid, &url);
            let _ = p.navigate(url);
        }
        None => {
            // 還沒載入過的分頁：改掉它記的網址，點到時就會開這個
            let shown = tab_url(&url).unwrap_or_else(|| HOME.to_string());
            edit(wid, |w| {
                if let Some(t) = w.tabs.iter_mut().find(|t| t.id == tid) {
                    t.url = shown;
                }
            });
            emit_state(app, wid);
        }
    }
}

/// 鍵盤交給網址列：select = 把裡面的字全選
/// 本程式的視窗（主程式或瀏覽器）在前景。遊戲在前景時不可以自己去動鍵盤焦點（鍵盤要留在遊戲）
fn own_foreground() -> bool {
    #[cfg(windows)]
    {
        win32::is_own(win32::foreground())
    }
    #[cfg(not(windows))]
    {
        true
    }
}

/// 這個瀏覽器視窗是不是前景視窗（使用者正在用它）
fn is_foreground(wid: &str) -> bool {
    #[cfg(windows)]
    {
        let h = rt(|m| m.get(wid).map(|r| r.hwnd).unwrap_or(0));
        h != 0 && win32::foreground() == h
    }
    #[cfg(not(windows))]
    {
        let _ = wid;
        false
    }
}

/// 鍵盤焦點交給某個分頁的網頁（只有本程式在前景時才做）
fn focus_page(app: &AppHandle, wid: &str, tid: &str) {
    if own_foreground() {
        if let Some(p) = app.get_webview(&page_label(wid, tid)) {
            let _ = p.set_focus();
        }
    }
}

fn focus_omni(app: &AppHandle, wid: &str, select: bool) {
    // 遊戲在前景時不直接搬焦點：工具列會通知「要打字」，再照原本的方式把鍵盤交過來
    if own_foreground() {
        if let Some(bar) = app.get_webview(&bar_label(wid)) {
            let _ = bar.set_focus();
        }
    }
    // 工具列還沒載入好（剛開的視窗）：等它好了再做
    let ready = rt(|m| match m.get_mut(wid) {
        Some(r) if !r.bar_ready => {
            r.want_omni = Some(select);
            false
        }
        _ => true,
    });
    if ready {
        let _ = app.emit_to(bar_label(wid).as_str(), "web-omni-focus", serde_json::json!({ "id": wid, "select": select }));
    }
}

fn close_tab(app: &AppHandle, wid: &str, tid: &str) -> Result<(), String> {
    enum After {
        Last,
        Select(String),
        Stay,
    }
    let opener = rt(|m| m.get(wid).and_then(|r| r.opener.get(tid).cloned()));
    let after = edit(wid, |w| {
        let i = w.tabs.iter().position(|t| t.id == tid)?;
        let t = w.tabs.remove(i);
        // 空白頁（網站先開空白分頁、還沒前往就關掉的）不用記到「最近關閉」
        let closed = (!w.incognito && t.url != "about:blank")
            .then(|| webcore::ClosedTab { url: t.url, title: t.title, icon: t.icon, win: w.id.clone(), index: i, at: webcore::now_ms() });
        if w.tabs.is_empty() {
            return Some((After::Last, closed));
        }
        if w.active == tid {
            // 從別的分頁開出來的（登入頁這類）：關掉後回到開它的那個分頁；不然是右邊那個
            let back = opener.filter(|o| w.tabs.iter().any(|t| t.id == *o));
            return Some((After::Select(back.unwrap_or_else(|| w.tabs[i.min(w.tabs.len() - 1)].id.clone())), closed));
        }
        Some((After::Stay, closed))
    })
    .flatten();
    let Some((after, closed)) = after else { return Ok(()) };
    if let Some(c) = closed {
        let mut s = st();
        webcore::push_closed(&mut s.file.closed, c);
        s.dirty = true;
    }
    if let Some(p) = app.get_webview(&page_label(wid, tid)) {
        let _ = p.close();
    }
    rt(|m| {
        if let Some(r) = m.get_mut(wid) {
            r.loading.remove(tid);
            r.nav.remove(tid);
            r.audio.remove(tid);
            r.zoom.remove(tid);
            r.zoom_set.remove(tid);
            r.source.remove(tid);
            r.devtools.remove(tid);
            r.own_page.remove(tid);
            r.heading_own.remove(tid);
            r.opener.remove(tid);
            r.opener.retain(|_, o| o != tid);
        }
    });
    match after {
        // 最後一個分頁關掉：整個視窗關閉
        After::Last => close_window(app, wid),
        After::Select(n) => select_tab(app, wid, &n)?,
        After::Stay => {
            emit_state(app, wid);
            emit_main(app);
        }
    }
    Ok(())
}

/// 重新開啟最近關閉的分頁（index = 最近關閉清單裡從新到舊的第幾個）
fn reopen_tab(app: &AppHandle, wid: &str, index: usize) {
    let c = {
        let mut s = st();
        let n = s.file.closed.len();
        if index >= n {
            return;
        }
        s.dirty = true;
        s.file.closed.remove(n - 1 - index)
    };
    // 原本的視窗還開著就放回原本的位置；不然開在目前的視窗最後面
    let home = get(&c.win).is_some_and(|w| w.open && !w.incognito);
    let target = if home { c.win.clone() } else { wid.to_string() };
    if let Ok(tid) = add_tab(app, &target, c.url, true, None) {
        let title = c.title.clone();
        let shown = edit(&target, |w| {
            let i = w.tabs.iter().position(|t| t.id == tid)?;
            let mut t = w.tabs.remove(i);
            t.title = c.title;
            t.icon = c.icon;
            let pinned = w.tabs.iter().filter(|x| x.pinned).count();
            let at = if home { c.index.clamp(pinned, w.tabs.len()) } else { w.tabs.len() };
            w.tabs.insert(at, t);
            // 它是目前的分頁：視窗（工作列上）的標題也換成它的
            (w.active == tid).then(|| {
                w.title = title.clone();
                w.incognito
            })
        })
        .flatten();
        if let (Some(incognito), Some(win)) = (shown, app.get_window(&win_label(&target))) {
            let _ = win.set_title(&window_title(&title, incognito));
        }
        emit_state(app, &target);
    }
}

/// 固定／取消固定分頁：固定的排在最左邊
fn pin_tab(app: &AppHandle, wid: &str, tid: &str) {
    edit(wid, |w| {
        let Some(i) = w.tabs.iter().position(|t| t.id == tid) else { return };
        let mut t = w.tabs.remove(i);
        t.pinned = !t.pinned;
        let pinned = w.tabs.iter().filter(|x| x.pinned).count();
        w.tabs.insert(pinned, t);
    });
    emit_state(app, wid);
}

fn mute_tab(app: &AppHandle, wid: &str, tid: &str) {
    let muted = edit(wid, |w| {
        w.tabs.iter_mut().find(|t| t.id == tid).map(|t| {
            t.muted = !t.muted;
            t.muted
        })
    })
    .flatten();
    if let (Some(m), Some(p)) = (muted, app.get_webview(&page_label(wid, tid))) {
        with_wv(&p, move |w| webnative::set_muted(w, m));
    }
    emit_state(app, wid);
}

/// 開一個新的瀏覽器視窗（url = 第一個分頁要開的網址；near = 開在哪個視窗旁邊）
fn new_window(app: &AppHandle, url: Option<String>, incognito: bool, near: Option<&str>) -> Result<String, String> {
    let base = near.and_then(get);
    let w = {
        let mut s = st();
        s.file.next_id += 1;
        let n = s.file.next_id;
        // 新視窗稍微錯開，不要完全疊在一起
        let k = (s.file.wins.len() % 6) as i32;
        let (x, y, ww, hh) = match &base {
            Some(b) => (b.x + 30, b.y + 30, b.w, b.h),
            None => (200 + k * 40, 150 + k * 40, 0, 0),
        };
        let mut w = WebWin { id: format!("w{n}"), x, y, w: ww, h: hh, incognito, url: url.unwrap_or_else(|| HOME.to_string()), ..WebWin::default() };
        fix_win(&mut w);
        s.file.wins.push(w.clone());
        s.dirty = true;
        w
    };
    open_window(app, &w.id, true)?;
    // 使用者是在本程式（主程式或另一個瀏覽器視窗）裡開的：新視窗拿到鍵盤、放到最前面；
    // 遊戲在前景時不搶（維持邊玩邊用）
    #[cfg(windows)]
    {
        let hwnd = rt(|m| m.get(&w.id).map(|r| r.hwnd).unwrap_or(0));
        if hwnd != 0 {
            let _ = app.run_on_main_thread(move || {
                if win32::is_own(win32::foreground()) {
                    win32::activate(hwnd);
                }
            });
        }
    }
    Ok(w.id)
}

/// 最近一次從 Chrome 同步擴充功能的時間（開視窗時同步，但不要太頻繁）
static EXT_SYNCED: Mutex<Option<Instant>> = Mutex::new(None);

/// 開視窗前：從 Chrome 同步書籤與擴充功能（每次開啟都會做；剛做過就不重複）
fn sync_on_open(app: &AppHandle) {
    let due = {
        let mut g = lock(&EXT_SYNCED);
        if g.is_none_or(|t| t.elapsed() >= Duration::from_secs(20)) {
            *g = Some(Instant::now());
            true
        } else {
            false
        }
    };
    if due {
        let _ = sync_extensions();
        let _ = import_chrome(app, true);
    }
}

/// 打開視窗。reveal = 縮小的也叫出來（從瀏覽器工具打開時）；程式剛啟動自動打開時維持縮小
fn open_window(app: &AppHandle, id: &str, reveal: bool) -> Result<(), String> {
    edit(id, |w| {
        fix_win(w);
        if reveal {
            w.min = false;
        }
    })
    .ok_or("找不到這個視窗")?;
    let cfg = get(id).ok_or("找不到這個視窗")?;
    let label = win_label(id);
    if app.get_window(&label).is_some() {
        // 已經開著（可能縮小了）：叫出來
        edit(id, |w| {
            w.open = true;
            w.min = false;
        });
        if let Some(win) = app.get_window(&label) {
            #[cfg(windows)]
            if let Ok(h) = win.hwnd() {
                // 「只在遊戲畫面時顯示」藏起來的：叫出來
                win32::show(h.0 as isize, true);
            }
            if win.is_minimized().unwrap_or(false) {
                let _ = win.unminimize();
            }
        }
        emit_main(app);
        return Ok(());
    }
    // 每次開啟都先跟 Chrome 同步（新的擴充功能要在網頁建立前準備好）
    sync_on_open(app);

    // 位置與大小（實體像素）：新視窗預設 1280×720（依 Windows 的縮放比例），螢幕放不下就縮小一點
    let mut cfg = cfg;
    let sf0 = app
        .monitor_from_point(cfg.x as f64 + 40.0, cfg.y as f64 + 20.0)
        .ok()
        .flatten()
        .or_else(|| app.primary_monitor().ok().flatten())
        .map(|m| m.scale_factor())
        .unwrap_or(1.0)
        .max(0.5);
    if cfg.w == 0 || cfg.h == 0 {
        let (mut w, mut h) = ((1280.0 * sf0).round() as u32, (720.0 * sf0).round() as u32);
        if let Some((p, sz)) = app_work_area(app, cfg.x, cfg.y) {
            w = w.min(sz.width.saturating_sub(40)).max(380);
            h = h.min(sz.height.saturating_sub(40)).max(240);
            // 放不下就往左上移，整個視窗留在螢幕裡
            cfg.x = cfg.x.min(p.x + sz.width as i32 - w as i32 - 20).max(p.x + 10);
            cfg.y = cfg.y.min(p.y + sz.height as i32 - h as i32 - 20).max(p.y + 10);
        }
        (cfg.w, cfg.h) = (w, h);
    }
    let (w, h) = (cfg.w.max(380), cfg.h.max(240));
    let (x, y) = app_on_screen(app, cfg.x, cfg.y, w, h);
    edit(id, |c| (c.w, c.h, c.x, c.y) = (w, h, x, y));

    // 建立時就顯示（不搶焦點）：視窗的顯示狀態一律交給 Tauri 管，放到最大、縮小、跨螢幕才不會出錯
    let mut builder = WindowBuilder::new(app, &label)
        .title(window_title(&cfg.title, cfg.incognito))
        .decorations(false)
        // 跟一般視窗一樣：工作列有圖示、可以縮小到工作列、可以被其他視窗蓋住
        .skip_taskbar(false)
        .resizable(true)
        .maximizable(true)
        .minimizable(true)
        .focused(false)
        .visible(true)
        .position(x as f64 / sf0, y as f64 / sf0)
        .inner_size(w as f64 / sf0, h as f64 / sf0)
        .min_inner_size(380.0, 240.0)
        // 跟 Chrome 的視窗框一樣的顏色（網頁旁邊留給拉大小的細邊會看到）
        .background_color(Color(31, 32, 32, 255));
    if let Some(icon) = app.default_window_icon() {
        builder = builder.icon(icon.clone()).map_err(|e| e.to_string())?;
    }
    let win = builder.build().map_err(|e| format!("視窗建立失敗：{e}"))?;
    // 換算有誤差時以實體像素為準
    if win.outer_position().ok() != Some(PhysicalPosition::new(x, y)) {
        let _ = win.set_position(PhysicalPosition::new(x, y));
    }
    if win.inner_size().ok() != Some(PhysicalSize::new(w, h)) {
        let _ = win.set_size(PhysicalSize::new(w, h));
    }

    #[cfg(windows)]
    let hwnd = win.hwnd().map(|h| h.0 as isize).unwrap_or(0);
    #[cfg(not(windows))]
    let hwnd = 0isize;
    rt(|m| m.insert(id.to_string(), Rt { hwnd, sf: sf0, ..Rt::default() }));
    install_subclass(app, hwnd, (x, y, w, h));

    let size = win.inner_size().unwrap_or(PhysicalSize::new(w, h));
    let ((bp, bs), _, (ppos, psz)) = geometry(id, &win, size);
    // 上面：分頁列、工具列、書籤列（本機頁面）
    let bar = WebviewBuilder::new(bar_label(id), WebviewUrl::App("web.html".into())).disable_drag_drop_handler();
    let bar = win.add_child(bar, bp, bs).map_err(|e| format!("工具列建立失敗：{e}"))?;
    attach_native(&bar, id, "", false);
    // 下面：目前分頁的網頁（其他分頁點到時才載入）
    let url = page_url(&cfg.url);
    add_page(app, &win, id, &cfg.active, url, ppos, psz)?;

    let (app2, id2, win2) = (app.clone(), id.to_string(), win.clone());
    win.on_window_event(move |ev| match ev {
        // 放到最大（按鈕、按兩下、拖到螢幕頂端）時不記大小，還原後回到原本的大小
        WindowEvent::Resized(s) if s.width > 0 && s.height > 0 => {
            close_ext_popup(&app2, &id2);
            pop::hide_of(&app2, &id2);
            layout(&app2, &id2, &win2, *s);
            let max = win2.is_maximized().unwrap_or(false);
            let (w, h) = (s.width, s.height);
            let (hwnd, full) = rt(|m| m.get(&id2).map(|r| (r.hwnd, r.full || r.fs)).unwrap_or((0, false)));
            // 螢幕關掉、拔掉時 Windows 會把視窗縮小塞進剩下的螢幕：那不是使用者調的，不記錄（螢幕回來後會放回原本的大小）
            let record = crate::scale::user_change(hwnd) && !full;
            let changed = edit(&id2, |c| {
                if !max && record {
                    (c.w, c.h) = (w, h);
                }
                c.min = false;
                !full && std::mem::replace(&mut c.max, max) != max
            });
            if changed == Some(true) {
                emit_state(&app2, &id2);
            }
        }
        // 縮小到工作列（大小變成 0）
        WindowEvent::Resized(_) => {
            pop::hide_of(&app2, &id2);
            let min = win2.is_minimized().unwrap_or(false);
            if edit(&id2, |c| std::mem::replace(&mut c.min, min) != min) == Some(true) {
                emit_main(&app2);
            }
        }
        // 跨到縮放比例不同的螢幕：視窗大小照比例換算（有時 Windows 只通知、沒有幫忙改大小，自己算一次才不會忽大忽小）
        WindowEvent::ScaleFactorChanged { scale_factor, .. } => {
            let new_sf = scale_factor.max(0.5);
            let old_sf = rt(|m| m.get_mut(&id2).map(|r| std::mem::replace(&mut r.sf, new_sf))).unwrap_or(new_sf);
            if (new_sf - old_sf).abs() > 0.001 && !win2.is_maximized().unwrap_or(false) && !win2.is_minimized().unwrap_or(false) {
                if let Some((w, h)) = get(&id2).map(|c| (c.w, c.h)) {
                    let want = PhysicalSize::new(((w as f64) / old_sf * new_sf).round() as u32, ((h as f64) / old_sf * new_sf).round() as u32);
                    let (w3, id3, app3) = (win2.clone(), id2.clone(), app2.clone());
                    std::thread::spawn(move || {
                        std::thread::sleep(Duration::from_millis(150));
                        let full = rt(|m| m.get(&id3).is_some_and(|r| r.full || r.fs));
                        if full || w3.is_maximized().unwrap_or(false) {
                            return;
                        }
                        if w3.inner_size().ok() != Some(want) {
                            let _ = w3.set_size(want);
                        }
                        if let Ok(size) = w3.inner_size() {
                            layout(&app3, &id3, &w3, size);
                        }
                    });
                }
            }
        }
        WindowEvent::Moved(p) if p.x > -10000 && p.y > -10000 => {
            close_ext_popup(&app2, &id2);
            pop::hide_of(&app2, &id2);
            let (x, y) = (p.x, p.y);
            let max = win2.is_maximized().unwrap_or(false);
            let (hwnd, full) = rt(|m| m.get(&id2).map(|r| (r.hwnd, r.full || r.fs)).unwrap_or((0, false)));
            let record = crate::scale::user_change(hwnd) && !full;
            edit(&id2, |c| {
                if !max && !c.max && record {
                    (c.x, c.y) = (x, y);
                }
            });
        }
        // 從工作列關閉、Alt+F4：照一般的關閉流程走（記下狀態、留最後一張畫面）
        WindowEvent::CloseRequested { api, .. } => {
            api.prevent_close();
            let (app3, id3) = (app2.clone(), id2.clone());
            std::thread::spawn(move || close_window(&app3, &id3));
        }
        WindowEvent::Destroyed => {
            let hwnd = rt(|m| m.remove(&id2).map(|r| r.hwnd).unwrap_or(0));
            lock(&TYPING_HWND).retain(|h| *h != hwnd);
            shot_keep(&id2);
            emit_main(&app2);
        }
        _ => {}
    });

    edit(id, |w| w.open = true);
    if cfg.max {
        let _ = win.maximize();
    }
    if cfg.min {
        let _ = win.minimize();
    }
    emit_main(app);
    // 選單用的小視窗先準備好，第一次按選單才不用等
    let app3 = app.clone();
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(1200));
        pop::warm_up(&app3);
    });
    Ok(())
}

/// 視窗跑到所有螢幕外面時移回主螢幕
fn app_on_screen(app: &AppHandle, x: i32, y: i32, w: u32, h: u32) -> (i32, i32) {
    let Ok(monitors) = app.available_monitors() else { return (x, y) };
    if monitors.is_empty() {
        return (x, y);
    }
    let seen = monitors.iter().any(|m| {
        let (mp, ms) = (m.position(), m.size());
        let x1 = x.max(mp.x);
        let y1 = y.max(mp.y);
        let x2 = (x + w as i32).min(mp.x + ms.width as i32);
        let y2 = (y + h as i32).min(mp.y + ms.height as i32);
        x2 - x1 >= 80 && y2 - y1 >= 40
    });
    if seen {
        return (x, y);
    }
    let base = app.primary_monitor().ok().flatten().map(|m| *m.position()).unwrap_or(PhysicalPosition::new(0, 0));
    (base.x + 100, base.y + 80)
}

/// 某個位置所在螢幕的可用範圍（扣掉工作列）
fn app_work_area(app: &AppHandle, x: i32, y: i32) -> Option<(PhysicalPosition<i32>, PhysicalSize<u32>)> {
    let m = app
        .monitor_from_point(x as f64 + 40.0, y as f64 + 20.0)
        .ok()
        .flatten()
        .or_else(|| app.primary_monitor().ok().flatten())?;
    let wa = m.work_area();
    Some((wa.position, wa.size))
}

fn close_window(app: &AppHandle, id: &str) {
    pop::hide_of(app, id);
    // 關之前再抓最後一張
    #[cfg(windows)]
    if let Some(h) = app.get_window(&win_label(id)).and_then(|w| w.hwnd().ok()).map(|h| h.0 as isize) {
        shot_take(id, h);
    }
    shot_keep(id);
    let incognito = edit(id, |w| {
        w.open = false;
        w.min = false;
        w.incognito
    })
    .unwrap_or(false);
    if let Some(win) = app.get_window(&win_label(id)) {
        let _ = win.destroy();
    }
    if incognito {
        // 無痕式視窗關掉就消失
        shot_forget(id);
        let mut s = st();
        s.file.wins.retain(|w| w.id != id);
    }
    // 最後一個視窗關了：歷史記錄馬上存檔
    if open_ids().is_empty() {
        flush_side(true);
    }
    emit_main(app);
}

/// 放到最大／還原（用 Windows 本身的放到最大，所以拖到螢幕頂端也會放大、拖離頂端會還原）
fn toggle_max(app: &AppHandle, wid: &str) {
    let Some(win) = app.get_window(&win_label(wid)) else { return };
    let _ = if win.is_maximized().unwrap_or(false) { win.unmaximize() } else { win.maximize() };
}

fn active_page(app: &AppHandle, wid: &str) -> Option<Webview> {
    let w = get(wid)?;
    app.get_webview(&page_label(wid, &w.active))
}

fn open_bm(app: &AppHandle, wid: &str, id: &str, new: bool) {
    let Some(url) = bm_read(|b| bm_find_any(b, id).map(|x| x.url.clone())).and_then(|u| parse_web(&u)) else { return };
    if new {
        let _ = new_tab(app, wid, Some(url), false, None);
    } else if let Some(w) = get(wid) {
        navigate_tab(app, wid, &w.active, url.as_str());
    }
}

/* ---------------- 視窗縮圖（卡片上顯示最後一幀畫面） ---------------- */

/// 每個開著的視窗最近一次抓到的畫面（PNG）；關閉時存成檔案，卡片顯示
static SHOTS: Mutex<Option<HashMap<String, Vec<u8>>>> = Mutex::new(None);
const SHOT_W: u32 = 480;
const SHOT_EVERY: Duration = Duration::from_secs(2);

fn shot_path(id: &str) -> Option<PathBuf> {
    WEB_DIR.get().map(|d| d.join("shots").join(format!("{id}.png")))
}

fn encode_png(w: u32, h: u32, px: &[u8]) -> Option<Vec<u8>> {
    let mut buf = Vec::new();
    {
        let mut enc = png::Encoder::new(&mut buf, w, h);
        enc.set_color(png::ColorType::Rgba);
        enc.set_depth(png::BitDepth::Eight);
        enc.set_compression(png::Compression::Fast);
        let mut wr = enc.write_header().ok()?;
        wr.write_image_data(px).ok()?;
    }
    Some(buf)
}

/// 抓一張視窗畫面放在記憶體（縮小的視窗不抓，留上一張）
fn shot_take(id: &str, hwnd: isize) {
    if hwnd == 0 || win32::iconic(hwnd) {
        return;
    }
    let Some((w, h, px)) = win32::capture(hwnd, SHOT_W) else { return };
    // 整張全黑（例如被遮住、還沒畫出來）就不要
    if px.chunks(4).step_by(97).all(|p| p[0] < 8 && p[1] < 8 && p[2] < 8) {
        return;
    }
    if let Some(png) = encode_png(w, h, &px) {
        let mut g = lock(&SHOTS);
        g.get_or_insert_with(HashMap::new).insert(id.to_string(), png);
    }
}

/// 視窗關了：把最後一張存成檔案
fn shot_keep(id: &str) {
    let png = lock(&SHOTS).as_mut().and_then(|m| m.remove(id));
    let (Some(png), Some(path)) = (png, shot_path(id)) else { return };
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let _ = std::fs::write(path, png);
}

fn shot_forget(id: &str) {
    if let Some(m) = lock(&SHOTS).as_mut() {
        m.remove(id);
    }
    if let Some(p) = shot_path(id) {
        let _ = std::fs::remove_file(p);
    }
}

/// 卡片要顯示的畫面：開著的用最近抓到的，關了的用關閉時存的（data URL；沒有就空字串）
#[tauri::command]
pub fn web_shot(id: String) -> String {
    use base64::Engine;
    let mem = lock(&SHOTS).as_ref().and_then(|m| m.get(&id).cloned());
    let png = mem.or_else(|| shot_path(&id).and_then(|p| std::fs::read(p).ok()));
    match png {
        Some(b) => format!("data:image/png;base64,{}", base64::engine::general_purpose::STANDARD.encode(&b)),
        None => String::new(),
    }
}

/* ---------------- 網站資料、用 Chrome 開啟 ---------------- */

/// 這個網址要清的來源：本身、去掉子網域的主網域、www.主網域，各有 http / https
fn site_origins(url: &str) -> Vec<String> {
    let Some(host) = host_of(url) else { return Vec::new() };
    let host = host.to_lowercase();
    let mut hosts = vec![host.clone()];
    if host.parse::<std::net::IpAddr>().is_err() && host != "localhost" {
        let labels: Vec<&str> = host.split('.').collect();
        // 像 co.uk、com.tw 這種第二層網域要多留一層
        let n = if labels.len() >= 3 && matches!(labels[labels.len() - 2], "co" | "com" | "net" | "org" | "gov" | "edu" | "ac") { 3 } else { 2 };
        let base = if labels.len() > n { labels[labels.len() - n..].join(".") } else { host.clone() };
        hosts.push(base.clone());
        hosts.push(format!("www.{base}"));
    }
    let mut out = Vec::new();
    for h in hosts {
        if h.is_empty() {
            continue;
        }
        for scheme in ["https", "http"] {
            let o = format!("{scheme}://{h}");
            if !out.contains(&o) {
                out.push(o);
            }
        }
    }
    out
}

/// 把分頁（沒指定就是目前的分頁）的網址丟給電腦上的 Chrome 開
fn open_tab_in_chrome(wid: &str, tid: Option<&str>) {
    let Some(w) = get(wid) else { return };
    let tid = tid.map(str::to_string).unwrap_or_else(|| w.active.clone());
    let url = w.tabs.iter().find(|t| t.id == tid).map(|t| t.url.clone()).unwrap_or(w.url);
    if parse_web(&url).is_none() {
        return;
    }
    if let Err(e) = crate::winsys::open_in_chrome(&url) {
        eprintln!("用 Chrome 開啟失敗：{e}");
    }
}

/// 清掉分頁網站的 Cookie 與儲存資料，然後重新整理
fn clear_site(app: &AppHandle, owner: &str, tid: &str) {
    let Some(url) = get(owner).and_then(|w| w.tabs.into_iter().find(|t| t.id == tid)).map(|t| t.url) else { return };
    if parse_web(&url).is_none() {
        return;
    }
    let origins = site_origins(&url);
    if origins.is_empty() {
        return;
    }
    let label = page_label(owner, tid);
    let Some(wv) = app.get_webview(&label) else { return };
    with_wv(&wv, move |w| webnative::clear_site(w, &origins));
    // 清完（最多等一秒多）再重新整理，不靠上面的回呼
    let app = app.clone();
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(1200));
        if let Some(p) = app.get_webview(&label) {
            let _ = p.reload();
        }
    });
}

/* ---------------- 歷史記錄、下載記錄（另外存檔） ---------------- */

struct Side {
    dir: PathBuf,
    hist: webcore::History,
    hist_dirty: bool,
    hist_saved: Instant,
    dl: webcore::Downloads,
    dl_dirty: bool,
    /// WebView2 的下載編號 → 下載記錄的編號
    dl_keys: HashMap<u64, u64>,
    /// 這次開啟程式後有沒有下載過
    dl_any: bool,
}

static SIDE: OnceLock<Mutex<Side>> = OnceLock::new();

fn side_edit<T>(f: impl FnOnce(&mut Side) -> T) -> Option<T> {
    Some(f(&mut *lock(SIDE.get()?)))
}

fn hist_edit<T>(f: impl FnOnce(&mut webcore::History) -> T) -> Option<T> {
    side_edit(|s| {
        s.hist_dirty = true;
        f(&mut s.hist)
    })
}

fn hist_read<T>(f: impl FnOnce(&webcore::History) -> T) -> Option<T> {
    side_edit(|s| f(&s.hist))
}

fn dl_edit<T>(f: impl FnOnce(&mut webcore::Downloads) -> T) -> Option<T> {
    side_edit(|s| {
        s.dl_dirty = true;
        f(&mut s.dl)
    })
}

fn load_side(dir: &Path) {
    let read = |name: &str| std::fs::read_to_string(dir.join(name)).ok();
    let hist: webcore::HistFile = read("web-history.json").and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default();
    let mut dl: webcore::Downloads = read("web-downloads.json").and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default();
    dl.fix_loaded();
    let _ = SIDE.set(Mutex::new(Side {
        dir: dir.to_path_buf(),
        hist: webcore::History::from_file(hist),
        hist_dirty: false,
        hist_saved: Instant::now(),
        dl,
        dl_dirty: false,
        dl_keys: HashMap::new(),
        dl_any: false,
    }));
}

/// 有變動就存檔：歷史記錄比較大，最多 15 秒存一次（force = 馬上存）
fn flush_side(force: bool) {
    let jobs = side_edit(|s| {
        let mut jobs: Vec<(PathBuf, String)> = Vec::new();
        if s.hist_dirty && (force || s.hist_saved.elapsed() >= Duration::from_secs(15)) {
            s.hist_dirty = false;
            s.hist_saved = Instant::now();
            if let Ok(t) = serde_json::to_string(&s.hist.to_file()) {
                jobs.push((s.dir.join("web-history.json"), t));
            }
        }
        if s.dl_dirty {
            s.dl_dirty = false;
            if let Ok(t) = serde_json::to_string(&s.dl) {
                jobs.push((s.dir.join("web-downloads.json"), t));
            }
        }
        jobs
    })
    .unwrap_or_default();
    for (path, text) in jobs {
        let tmp = path.with_extension("tmp");
        if std::fs::write(&tmp, text).is_ok() {
            let _ = std::fs::rename(&tmp, &path);
        }
    }
}

fn dl_badge() -> DlBadge {
    side_edit(|s| {
        let active: Vec<&webcore::Dl> = s.dl.list.iter().filter(|d| d.state == "progress").collect();
        let (got, total) = active.iter().fold((0i64, 0i64), |a, d| (a.0 + d.received.max(0), a.1 + d.total.max(0)));
        let known = !active.is_empty() && active.iter().all(|d| d.total > 0);
        DlBadge { any: s.dl_any, active: active.len(), pct: if known && total > 0 { ((got * 100) / total).clamp(0, 100) as i32 } else { -1 } }
    })
    .unwrap_or_default()
}

fn on_download(app: &AppHandle, d: DlEv) {
    let now = webcore::now_ms();
    side_edit(|s| {
        s.dl_dirty = true;
        match d {
            DlEv::Start { key, url, path, total } => {
                let id = s.dl.start(&url, &path, total, now);
                s.dl_keys.insert(key, id);
                s.dl_any = true;
            }
            DlEv::Progress { key, received, total } => {
                if let Some(x) = s.dl_keys.get(&key).copied().and_then(|id| s.dl.get_mut(id)) {
                    x.received = received;
                    if total > 0 {
                        x.total = total;
                    }
                }
            }
            DlEv::Done { key, state, path, received } => {
                if let Some(x) = s.dl_keys.remove(&key).and_then(|id| s.dl.get_mut(id)) {
                    x.state = state.to_string();
                    x.end = now;
                    if received > 0 {
                        x.received = received;
                    }
                    if !path.is_empty() {
                        x.path = path;
                    }
                }
            }
        }
    });
    emit_state_all(app);
}

/* ---------------- WebView2 通知的事情、指令 ---------------- */

fn handle_ev(app: &AppHandle, wid: &str, tid: &str, ev: Ev) {
    match ev {
        Ev::Key(cmd) => run_cmd(app, wid, (!tid.is_empty()).then_some(tid), cmd),
        Ev::Nav { back, forward } => {
            let changed = rt(|m| m.get_mut(wid).map(|r| r.nav.insert(tid.to_string(), (back, forward)) != Some((back, forward)))).unwrap_or(false);
            if changed {
                emit_state(app, wid);
            }
        }
        Ev::Source(u) => on_source(app, wid, tid, &u),
        Ev::Favicon(u) => set_icon(app, wid, tid, u),
        Ev::Audio { playing, muted } => {
            rt(|m| {
                if let Some(r) = m.get_mut(wid) {
                    if playing {
                        r.audio.insert(tid.to_string());
                    } else {
                        r.audio.remove(tid);
                    }
                }
            });
            edit(wid, |w| {
                if let Some(t) = w.tabs.iter_mut().find(|t| t.id == tid) {
                    t.muted = muted;
                }
            });
            emit_state(app, wid);
        }
        Ev::Zoom(z) => on_zoom(app, wid, tid, z),
        Ev::Fullscreen(on) => set_fullscreen(app, wid, on),
        Ev::Context { seq, target, items } => pop::open_context(app, wid, tid, seq, target, items),
        Ev::Download(d) => on_download(app, d),
        Ev::CloseRequested => {
            let _ = close_tab(app, wid, tid);
        }
    }
}

fn tab_of(wid: &str, tid: &str) -> Option<Tab> {
    get(wid)?.tabs.into_iter().find(|t| t.id == tid)
}

/// WebView2 通知縮放變了
fn on_zoom(app: &AppHandle, wid: &str, tid: &str, z: f64) {
    let mine = rt(|m| {
        m.get_mut(wid).and_then(|r| {
            r.zoom.insert(tid.to_string(), z);
            r.zoom_set.get(tid).copied()
        })
    });
    if mine.is_some_and(|p| (p - z).abs() < 0.002) {
        emit_state(app, wid);
        return;
    }
    // 使用者用 Ctrl + 滾輪調的：記住這個網站的縮放
    let Some(tab) = tab_of(wid, tid) else { return };
    {
        let mut s = st();
        if s.file.settings.set_zoom(&tab.url, z) {
            s.dirty = true;
        }
    }
    rt(|m| {
        if let Some(r) = m.get_mut(wid) {
            r.zoom_set.insert(tid.to_string(), z);
        }
    });
    emit_state(app, wid);
    if get(wid).is_some_and(|w| w.active == tid) {
        ask_bar(app, wid, "zoom");
    }
}

/// 設定分頁的縮放（快捷鍵、選單）：記住這個網站的縮放並套用
fn set_tab_zoom(app: &AppHandle, wid: &str, tid: &str, z: f64) {
    let Some(tab) = tab_of(wid, tid) else { return };
    let z = z.clamp(0.25, 5.0);
    {
        let mut s = st();
        if s.file.settings.set_zoom(&tab.url, z) {
            s.dirty = true;
        }
    }
    rt(|m| {
        if let Some(r) = m.get_mut(wid) {
            r.zoom.insert(tid.to_string(), z);
            r.zoom_set.insert(tid.to_string(), z);
        }
    });
    if let Some(p) = app.get_webview(&page_label(wid, tid)) {
        with_wv(&p, move |w| webnative::set_zoom(w, z));
    }
    emit_state(app, wid);
}

fn tab_zoom(wid: &str, tid: &str) -> f64 {
    let cur = rt(|m| m.get(wid).and_then(|r| r.zoom.get(tid).copied()));
    cur.unwrap_or_else(|| {
        let url = tab_of(wid, tid).map(|t| t.url).unwrap_or_default();
        st().file.settings.zoom_for(&url)
    })
}

/// 開程式自己的頁面（歷史記錄、設定…）：目前是新分頁就直接換過去；已經開著就切過去；不然開新分頁
fn open_internal(app: &AppHandle, wid: &str, v: &str) {
    let Some(w) = get(wid) else { return };
    let page = webcore::virtual_page(v);
    if w.tabs.iter().any(|t| t.id == w.active && webcore::is_newtab(&t.url)) {
        navigate_tab(app, wid, &w.active, v);
        return;
    }
    if let Some(t) = w.tabs.iter().find(|t| page.is_some() && webcore::virtual_page(&t.url) == page) {
        let (tid, same) = (t.id.clone(), t.url == v);
        let _ = select_tab(app, wid, &tid);
        if !same {
            navigate_tab(app, wid, &tid, v);
        }
        return;
    }
    let _ = add_tab(app, wid, v.to_string(), true, None);
}

fn copy_text(text: &str) {
    let _ = arboard::Clipboard::new().and_then(|mut c| c.set_text(text.to_string()));
}

/// 目前的網頁加入書籤列（已經加過就不動）。回傳書籤的編號
fn bookmark_add(app: &AppHandle, wid: &str) -> Option<String> {
    let w = get(wid)?;
    let tab = w.tabs.into_iter().find(|t| t.id == w.active)?;
    parse_web(&tab.url)?;
    let (id, added) = bm(|b| {
        if let Some(id) = bm_url_id(&b.bar, &tab.url).or_else(|| bm_url_id(&b.other, &tab.url)) {
            return (id, false);
        }
        b.ignored.retain(|i| !same_url(i, &tab.url));
        b.next_id += 1;
        let host = host_of(&tab.url).unwrap_or_default();
        let title = if tab.title.trim().is_empty() { host.clone() } else { tab.title.clone() };
        let id = format!("b{}", b.next_id);
        b.bar.push(Bm { id: id.clone(), title, url: tab.url.clone(), ..Bm::default() });
        if !tab.icon.is_empty() && !host.is_empty() {
            b.icons.insert(host, tab.icon.clone());
        }
        (id, true)
    });
    if added {
        emit_bm(app);
    }
    Some(id)
}

/// 目前網頁的書籤（有的話）移除
fn bookmark_remove(app: &AppHandle, wid: &str) {
    let Some(url) = get(wid).and_then(|w| w.tabs.into_iter().find(|t| t.id == w.active)).map(|t| t.url) else { return };
    let removed = bm(|b| {
        let Some(id) = bm_url_id(&b.bar, &url).or_else(|| bm_url_id(&b.other, &url)) else { return false };
        if let Some(x) = bm_remove(&mut b.bar, &id).or_else(|| bm_remove(&mut b.other, &id)) {
            remember_deleted(b, &x);
        }
        true
    });
    if removed {
        emit_bm(app);
    }
}

/// 執行一個瀏覽器指令（快捷鍵、選單、工具列按鈕共用）。tid = 指令針對的分頁（沒給就是目前的分頁）
pub(crate) fn run_cmd(app: &AppHandle, wid: &str, tid: Option<&str>, cmd: &str) {
    let Some(w) = get(wid) else { return };
    let tid = tid.filter(|t| w.tabs.iter().any(|x| x.id == *t)).map(str::to_string).unwrap_or_else(|| w.active.clone());
    let tab = w.tabs.iter().find(|t| t.id == tid).cloned().unwrap_or_default();
    let page = || app.get_webview(&page_label(wid, &tid));
    let index = w.tabs.iter().position(|t| t.id == w.active).unwrap_or(0);
    if cmd != "esc" && cmd != "noop" {
        pop::hide_of(app, wid);
    }
    match cmd {
        "noop" => {}
        "esc" => pop::hide_of(app, wid),
        "tab.new" => {
            if add_tab(app, wid, HOME.to_string(), true, None).is_ok() {
                focus_omni(app, wid, false);
            }
        }
        "tab.new_right" => {
            if add_tab(app, wid, HOME.to_string(), true, Some(&tid)).is_ok() {
                focus_omni(app, wid, false);
            }
        }
        "tab.close" => {
            let _ = close_tab(app, wid, &tid);
        }
        "tab.reopen" => reopen_tab(app, wid, 0),
        "tab.next" | "tab.prev" => {
            let n = w.tabs.len();
            if n > 1 {
                let i = if cmd == "tab.next" { (index + 1) % n } else { (index + n - 1) % n };
                let _ = select_tab(app, wid, &w.tabs[i].id);
            }
        }
        "tab.last" => {
            if let Some(t) = w.tabs.last() {
                let _ = select_tab(app, wid, &t.id);
            }
        }
        "tab.dup" => {
            let _ = add_tab(app, wid, tab.url.clone(), true, Some(&tid));
        }
        "tab.pin" => pin_tab(app, wid, &tid),
        "tab.mute" => mute_tab(app, wid, &tid),
        "tab.reload" | "nav.reload" => {
            if let Some(p) = page() {
                let _ = p.reload();
            }
        }
        "tab.to_window" => {
            if w.tabs.len() > 1 && new_window(app, Some(tab.url.clone()), w.incognito, Some(wid)).is_ok() {
                let _ = close_tab(app, wid, &tid);
            }
        }
        "tab.close_others" | "tab.close_right" => {
            let Some(i) = w.tabs.iter().position(|x| x.id == tid) else { return };
            let only_right = cmd == "tab.close_right";
            let close: Vec<String> =
                w.tabs.iter().enumerate().filter(|(j, x)| x.id != tid && !x.pinned && (!only_right || *j > i)).map(|(_, x)| x.id.clone()).collect();
            let _ = select_tab(app, wid, &tid);
            for c in close {
                let _ = close_tab(app, wid, &c);
            }
        }
        "tab.chrome" => open_tab_in_chrome(wid, Some(&tid)),
        "site.clear" => clear_site(app, wid, &tid),
        "win.new" | "win.incognito" => {
            if let Ok(id) = new_window(app, None, cmd == "win.incognito", Some(wid)) {
                focus_omni(app, &id, false);
            }
        }
        "win.close" => close_window(app, wid),
        "win.min" => {
            if let Some(win) = app.get_window(&win_label(wid)) {
                let _ = win.minimize();
            }
        }
        "win.max" => toggle_max(app, wid),
        "win.fullscreen" => toggle_fs(app, wid),
        "exit" => {
            for id in open_ids() {
                close_window(app, &id);
            }
        }
        "nav.back" => {
            if let Some(p) = page() {
                with_wv(&p, webnative::go_back);
            }
        }
        "nav.forward" => {
            if let Some(p) = page() {
                with_wv(&p, webnative::go_forward);
            }
        }
        "nav.stop" => {
            if let Some(p) = page() {
                with_wv(&p, webnative::stop);
            }
        }
        "nav.home" => {
            let home = st().file.settings.home_url.clone();
            let target = webcore::resolve_input(&home, engine()).map(|(u, _)| u).unwrap_or_else(|| HOME.to_string());
            navigate_tab(app, wid, &tid, &target);
        }
        "omni.focus" | "omni.search" => focus_omni(app, wid, true),
        "menu.main" => ask_bar(app, wid, "main"),
        "tab.search" => ask_bar(app, wid, "tabs"),
        "bm.add" => {
            if bookmark_add(app, wid).is_some() {
                ask_bar(app, wid, "bookmark");
            }
        }
        "bm.remove" => bookmark_remove(app, wid),
        "bm.bar" => {
            {
                let mut s = st();
                s.file.settings.show_bm = !s.file.settings.show_bm;
                s.dirty = true;
            }
            for id in open_ids() {
                relayout(app, &id);
                emit_state(app, &id);
            }
        }
        "bm.manager" => open_internal(app, wid, "chrome://bookmarks/"),
        "bm.all_tabs" => {
            let links: Vec<(String, String)> = w.tabs.iter().filter(|t| parse_web(&t.url).is_some()).map(|t| (t.title.clone(), t.url.clone())).collect();
            if !links.is_empty() {
                bm(|b| {
                    let mut next = b.next_id;
                    let children = links
                        .into_iter()
                        .map(|(title, url)| {
                            next += 1;
                            Bm { id: format!("b{next}"), title, url, ..Bm::default() }
                        })
                        .collect();
                    next += 1;
                    b.bar.push(Bm { id: format!("b{next}"), title: "已儲存的分頁".into(), children, ..Bm::default() });
                    b.next_id = next;
                });
                emit_bm(app);
            }
        }
        "page.history" => open_internal(app, wid, "chrome://history/"),
        "page.downloads" => open_internal(app, wid, "chrome://downloads/"),
        "page.settings" => open_internal(app, wid, "chrome://settings/"),
        "page.extensions" => open_internal(app, wid, "chrome://extensions/"),
        "page.cleardata" => open_internal(app, wid, "chrome://settings/clearBrowserData"),
        "zoom.in" | "zoom.out" | "zoom.reset" => {
            let cur = tab_zoom(wid, &tid);
            let z = match cmd {
                "zoom.in" => webcore::zoom_step(cur, true),
                "zoom.out" => webcore::zoom_step(cur, false),
                _ => st().file.settings.zoom_for(""),
            };
            set_tab_zoom(app, wid, &tid, z);
            if w.active == tid {
                ask_bar(app, wid, "zoom");
            }
        }
        "print" => {
            if let Some(p) = page() {
                let _ = p.print();
            }
        }
        "find" => {
            if let Some(p) = page() {
                with_wv(&p, |w| {
                    webnative::find(w);
                });
            }
        }
        "save" => {
            if let Some(p) = page() {
                with_wv(&p, webnative::save_as);
            }
        }
        "source" => {
            if parse_web(&tab.url).is_some() {
                let _ = add_tab(app, wid, format!("view-source:{}", tab.url), true, Some(&tid));
            }
        }
        "taskmgr" => {
            if let Some(p) = page() {
                with_wv(&p, webnative::task_manager);
            }
        }
        "devtools" => open_devtools(app, wid, &tid),
        "dl.toggle" => {
            if let Some(p) = active_page(app, wid) {
                with_wv(&p, webnative::toggle_downloads);
            }
        }
        "copy.url" => copy_text(&tab.url),
        _ => {
            if let Some(n) = cmd.strip_prefix("tab.select:").and_then(|n| n.parse::<usize>().ok()) {
                if let Some(t) = n.checked_sub(1).and_then(|i| w.tabs.get(i)) {
                    let _ = select_tab(app, wid, &t.id);
                }
            }
        }
    }
}

/* ---------------- Windows：攔截訊息、焦點、樣式 ---------------- */

/// 非 Windows（只為了編譯測試）
#[cfg(not(windows))]
mod win32 {
    pub fn iconic(_h: isize) -> bool {
        false
    }
    pub fn activate(_h: isize) {}
    pub fn capture(_h: isize, _max_w: u32) -> Option<(u32, u32, Vec<u8>)> {
        None
    }
    pub fn click_mods() -> (bool, bool, bool) {
        (false, false, false)
    }
}

#[cfg(windows)]
mod win32 {
    use super::{lock, TYPING_HWND};
    use std::sync::atomic::{AtomicIsize, Ordering};
    use windows_sys::Win32::UI::Accessibility::{SetWinEventHook, HWINEVENTHOOK};
    use std::sync::Mutex;
    use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
    use windows_sys::Win32::System::Threading::{AttachThreadInput, GetCurrentProcessId, GetCurrentThreadId};
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{GetAsyncKeyState, VK_CONTROL, VK_LBUTTON, VK_MBUTTON, VK_RBUTTON, VK_SHIFT};
    use windows_sys::Win32::UI::Shell::{DefSubclassProc, SetWindowSubclass};
    use windows_sys::Win32::UI::WindowsAndMessaging::*;

    const WM_REFOCUS: u32 = WM_APP + 0x57;
    const SUBCLASS_ID: usize = 0x4D42_5745;
    const SUBCLASS_MENU: usize = 0x4D42_574D;

    /// 正在拖曳或拉大小的視窗 → 這段時間被啟用時，原本在前景的視窗（放開後再還回去）
    static MOVING: Mutex<Vec<(isize, isize)>> = Mutex::new(Vec::new());
    /// 新楓之谷的視窗
    pub static GAME_HWND: AtomicIsize = AtomicIsize::new(0);
    /// 遊戲在前景時點了這個視窗（不搶焦點）的時間：這之後馬上被啟用，就是網頁元件自己搶的
    static NOACT_CLICK: Mutex<Option<(isize, std::time::Instant)>> = Mutex::new(None);

    fn is_game(h: HWND) -> bool {
        !h.is_null() && h as isize == GAME_HWND.load(Ordering::SeqCst) && unsafe { IsWindow(h) } != 0
    }

    fn clicked_recently(h: HWND) -> bool {
        lock(&NOACT_CLICK).is_some_and(|(w, t)| w == h as isize && t.elapsed() < std::time::Duration::from_millis(800))
    }

    /// 遊戲剛到前景：馬上把 web 視窗放到遊戲上面（不搶焦點）
    unsafe extern "system" fn on_foreground(_h: HWINEVENTHOOK, _ev: u32, hwnd: HWND, _o: i32, _c: i32, _t: u32, _ms: u32) {
        if hwnd.is_null() || is_own(hwnd as isize) || !crate::fg::current().is_game {
            return;
        }
        GAME_HWND.store(hwnd as isize, Ordering::SeqCst);
        for h in super::web_hwnds() {
            if IsWindowVisible(h as HWND) != 0 && IsIconic(h as HWND) == 0 {
                raise(h);
            }
        }
    }

    /// 放到其他視窗上面（不搶焦點）。
    /// 遊戲在前景時，用 HWND_TOP 拉不到遊戲上面（Windows 不讓背景程式這樣做），先設成置頂再取消置頂就可以
    pub fn raise(h: isize) {
        unsafe {
            let f = SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE;
            SetWindowPos(h as HWND, HWND_TOPMOST, 0, 0, 0, 0, f);
            SetWindowPos(h as HWND, HWND_NOTOPMOST, 0, 0, 0, 0, f);
        }
    }

    /// 要在主執行緒（有訊息迴圈）呼叫
    pub fn hook_foreground() {
        unsafe {
            SetWinEventHook(EVENT_SYSTEM_FOREGROUND, EVENT_SYSTEM_FOREGROUND, std::ptr::null_mut(), Some(on_foreground), 0, 0, WINEVENT_OUTOFCONTEXT);
        }
    }

    fn typing(h: HWND) -> bool {
        lock(&TYPING_HWND).contains(&(h as isize))
    }

    fn maxed(h: HWND) -> bool {
        unsafe { IsZoomed(h) != 0 }
    }

    pub fn is_own(h: isize) -> bool {
        if h == 0 {
            return false;
        }
        let mut pid = 0u32;
        unsafe {
            GetWindowThreadProcessId(h as HWND, &mut pid);
            pid == GetCurrentProcessId()
        }
    }

    /// 別的程式的視窗（例如遊戲）。自己的視窗之間不互相還前景，否則會無限來回卡住
    unsafe fn foreign(h: HWND) -> bool {
        !h.is_null() && IsWindow(h) != 0 && !is_own(h as isize)
    }

    /// 被啟用了（網頁元件自己搶、或拖曳造成的）：把前景還給原本的視窗
    unsafe fn give_back_later(hwnd: HWND, prev: HWND) {
        // 只有「遊戲在前景時點了這個視窗、網頁元件又自己搶走焦點」才還給遊戲；
        // 從工作列、Alt+Tab 切過來就是一般的切換，不還
        if !is_game(prev) || !clicked_recently(hwnd) {
            return;
        }
        let mut mv = lock(&MOVING);
        if let Some(e) = mv.iter_mut().find(|e| e.0 == hwnd as isize) {
            // 拖曳中：放開後再還
            if e.1 == 0 {
                e.1 = prev as isize;
            }
            return;
        }
        drop(mv);
        PostMessageW(hwnd, WM_REFOCUS, prev as WPARAM, 0);
    }

    /// 無框視窗不畫系統標題列與邊框，整個視窗都是內容區
    fn no_frame(msg: u32, wp: WPARAM) -> Option<LRESULT> {
        match msg {
            WM_NCPAINT | 0x00AE | 0x00AF => Some(0),
            WM_NCACTIVATE => Some(1),
            WM_NCCALCSIZE if wp != 0 => Some(0),
            _ => None,
        }
    }

    unsafe extern "system" fn proc_(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM, _id: usize, _d: usize) -> LRESULT {
        // 放到最大時交給 Tauri（tao）處理：內容剛好是螢幕可用範圍，也會留一點給自動隱藏的工作列
        if msg == WM_NCCALCSIZE && wp != 0 && maxed(hwnd) {
            return DefSubclassProc(hwnd, msg, wp, lp);
        }
        if let Some(r) = no_frame(msg, wp) {
            return r;
        }
        match msg {
            // 左、右、下的邊可以拉大小（只有沒被網頁蓋住的那一條邊會收到這個訊息）
            WM_NCHITTEST => {
                if maxed(hwnd) {
                    return HTCLIENT as LRESULT;
                }
                let x = (lp & 0xFFFF) as i16 as i32;
                let y = ((lp >> 16) & 0xFFFF) as i16 as i32;
                let mut r: RECT = std::mem::zeroed();
                GetWindowRect(hwnd, &mut r);
                const Z: i32 = 12;
                let (l, rr, b) = (x < r.left + Z, x >= r.right - Z, y >= r.bottom - Z);
                let code = match (l, rr, b) {
                    (true, _, true) => HTBOTTOMLEFT,
                    (_, true, true) => HTBOTTOMRIGHT,
                    (_, _, true) => HTBOTTOM,
                    (true, _, _) => HTLEFT,
                    (_, true, _) => HTRIGHT,
                    _ => HTCLIENT,
                };
                code as LRESULT
            }
            // 遊戲在前景時點這個視窗：不變成作用中（鍵盤留在遊戲），但要浮到前面
            WM_MOUSEACTIVATE if !typing(hwnd) && is_game(GetForegroundWindow()) => {
                *lock(&NOACT_CLICK) = Some((hwnd as isize, std::time::Instant::now()));
                raise(hwnd as isize);
                MA_NOACTIVATE as LRESULT
            }
            WM_ACTIVATE => {
                let active = (wp & 0xFFFF) != 0;
                if active && !typing(hwnd) {
                    give_back_later(hwnd, lp as HWND);
                }
                DefSubclassProc(hwnd, msg, wp, lp)
            }
            WM_DISPLAYCHANGE => {
                crate::scale::display_changed();
                super::restore_later();
                DefSubclassProc(hwnd, msg, wp, lp)
            }
            WM_ENTERSIZEMOVE => {
                lock(&MOVING).push((hwnd as isize, 0));
                DefSubclassProc(hwnd, msg, wp, lp)
            }
            WM_EXITSIZEMOVE => {
                let prev = {
                    let mut mv = lock(&MOVING);
                    let p = mv.iter().find(|e| e.0 == hwnd as isize).map(|e| e.1).unwrap_or(0);
                    mv.retain(|e| e.0 != hwnd as isize);
                    p
                };
                if prev != 0 && !typing(hwnd) && is_game(prev as HWND) && GetForegroundWindow() == hwnd {
                    PostMessageW(hwnd, WM_REFOCUS, prev as WPARAM, 0);
                }
                DefSubclassProc(hwnd, msg, wp, lp)
            }
            WM_REFOCUS => {
                if !typing(hwnd) && is_game(wp as HWND) && GetForegroundWindow() == hwnd {
                    SetForegroundWindow(wp as HWND);
                }
                0
            }
            _ => DefSubclassProc(hwnd, msg, wp, lp),
        }
    }

    /// 選單視窗：永遠不搶焦點
    unsafe extern "system" fn proc_menu(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM, _id: usize, _d: usize) -> LRESULT {
        if let Some(r) = no_frame(msg, wp) {
            return r;
        }
        match msg {
            WM_MOUSEACTIVATE => MA_NOACTIVATE as LRESULT,
            WM_ACTIVATE => {
                let prev = lp as HWND;
                if (wp & 0xFFFF) != 0 && foreign(prev) {
                    PostMessageW(hwnd, WM_REFOCUS, prev as WPARAM, 0);
                }
                DefSubclassProc(hwnd, msg, wp, lp)
            }
            WM_REFOCUS => {
                if foreign(wp as HWND) && GetForegroundWindow() == hwnd {
                    SetForegroundWindow(wp as HWND);
                }
                0
            }
            _ => DefSubclassProc(hwnd, msg, wp, lp),
        }
    }

    /// rect = 視窗要的位置大小（實體像素）：Tauri 建立無框視窗時留了看不到的邊（內容區比視窗小一圈），
    /// 換成自己的處理後重算一次，整個視窗都是內容區，大小也設回原本的（不然每次重開視窗都會變大一點）
    pub fn subclass(h: isize, rect: Option<(i32, i32, i32, i32)>) {
        unsafe {
            SetWindowSubclass(h as HWND, Some(proc_), SUBCLASS_ID, 0);
            if let Some((x, y, w, hh)) = rect {
                if IsZoomed(h as HWND) == 0 && IsIconic(h as HWND) == 0 {
                    SetWindowPos(h as HWND, std::ptr::null_mut(), x, y, w, hh, SWP_NOZORDER | SWP_NOACTIVATE | SWP_FRAMECHANGED);
                }
            }
        }
    }

    pub fn subclass_menu(h: isize) {
        unsafe { SetWindowSubclass(h as HWND, Some(proc_menu), SUBCLASS_MENU, 0) };
    }

    pub fn foreground() -> isize {
        unsafe { GetForegroundWindow() as isize }
    }

    pub fn alive(h: isize) -> bool {
        h != 0 && unsafe { IsWindow(h as HWND) } != 0
    }

    /// 把視窗叫到前景（要在視窗所屬的執行緒呼叫）
    pub fn activate(h: isize) {
        unsafe {
            let hwnd = h as HWND;
            if SetForegroundWindow(hwnd) != 0 {
                return;
            }
            let fg = GetForegroundWindow();
            let fg_thread = GetWindowThreadProcessId(fg, std::ptr::null_mut());
            let me = GetCurrentThreadId();
            if fg_thread != 0 && fg_thread != me {
                AttachThreadInput(me, fg_thread, 1);
                SetForegroundWindow(hwnd);
                BringWindowToTop(hwnd);
                AttachThreadInput(me, fg_thread, 0);
            }
        }
    }

    pub fn give_back(prev: isize) {
        if alive(prev) {
            unsafe { SetForegroundWindow(prev as HWND) };
        }
    }

    pub fn cursor() -> (i32, i32) {
        unsafe {
            let mut p: POINT = std::mem::zeroed();
            GetCursorPos(&mut p);
            (p.x, p.y)
        }
    }

    /// 視窗的範圍（螢幕實體像素：左、上、右、下）
    pub fn rect(h: isize) -> Option<(i32, i32, i32, i32)> {
        unsafe {
            let mut r: RECT = std::mem::zeroed();
            if GetWindowRect(h as HWND, &mut r) == 0 {
                return None;
            }
            Some((r.left, r.top, r.right, r.bottom))
        }
    }

    pub fn cursor_outside(h: isize) -> bool {
        unsafe {
            let mut p: POINT = std::mem::zeroed();
            let mut r: RECT = std::mem::zeroed();
            if GetCursorPos(&mut p) == 0 || GetWindowRect(h as HWND, &mut r) == 0 {
                return false;
            }
            p.x < r.left || p.x >= r.right || p.y < r.top || p.y >= r.bottom
        }
    }

    /// 點連結開新分頁時的按鍵：(按著 Ctrl, 按著 Shift, 剛按過滑鼠中鍵)
    pub fn click_mods() -> (bool, bool, bool) {
        unsafe {
            let mid = GetAsyncKeyState(VK_MBUTTON as i32);
            (GetAsyncKeyState(VK_CONTROL as i32) < 0, GetAsyncKeyState(VK_SHIFT as i32) < 0, mid < 0 || (mid & 1) != 0)
        }
    }

    /// 滑鼠任一個鍵按著
    pub fn mouse_down() -> bool {
        unsafe { [VK_LBUTTON, VK_RBUTTON, VK_MBUTTON].iter().any(|&k| GetAsyncKeyState(k as i32) < 0) }
    }

    pub fn visible(h: isize) -> bool {
        unsafe { IsWindowVisible(h as HWND) != 0 }
    }

    /// 套用：不搶焦點、滑鼠穿透、不透明度。樣式被別的地方改掉時也會補回來
    pub fn apply_style(h: isize, noactivate: bool, locked: bool, alpha: u8) {
        unsafe {
            let hwnd = h as HWND;
            let ex = GetWindowLongPtrW(hwnd, GWL_EXSTYLE) as u32;
            let layered = locked || alpha < 255;
            let mut want = ex & !(WS_EX_NOACTIVATE | WS_EX_TRANSPARENT | WS_EX_LAYERED);
            if noactivate {
                want |= WS_EX_NOACTIVATE;
            }
            if locked {
                want |= WS_EX_TRANSPARENT;
            }
            if layered {
                want |= WS_EX_LAYERED;
            }
            let mut cur_alpha = 255u8;
            if ex & WS_EX_LAYERED != 0 {
                let mut a = 255u8;
                let mut flags = 0u32;
                if GetLayeredWindowAttributes(hwnd, std::ptr::null_mut(), &mut a, &mut flags) != 0 && flags & LWA_ALPHA != 0 {
                    cur_alpha = a;
                }
            }
            if want == ex && (!layered || cur_alpha == alpha) {
                return;
            }
            // 這幾個樣式都不影響邊框，不用重畫邊框（重畫會讓視窗閃一下）
            if want != ex {
                SetWindowLongPtrW(hwnd, GWL_EXSTYLE, want as isize);
            }
            if layered {
                SetLayeredWindowAttributes(hwnd, 0, alpha, LWA_ALPHA);
            }
        }
    }

    pub fn show(h: isize, visible: bool) {
        unsafe { ShowWindow(h as HWND, if visible { SW_SHOWNA } else { SW_HIDE }) };
    }

    /// 遊戲在前景、而且蓋住這個視窗時：把視窗放到遊戲上面（不搶焦點）。其他程式還是可以蓋住它
    pub fn keep_above(h: isize, game: isize) {
        unsafe {
            if IsIconic(h as HWND) != 0 {
                return;
            }
            let mut w = GetWindow(h as HWND, GW_HWNDPREV);
            let mut n = 0;
            while !w.is_null() && n < 4000 {
                if w as isize == game {
                    raise(h);
                    return;
                }
                w = GetWindow(w, GW_HWNDPREV);
                n += 1;
            }
        }
    }

    /// 讓滑鼠底下的網頁（工具列）重新看一次滑鼠停在什麼東西上：把提示文字收起來。
    /// 按按鈕打開選單之後滑鼠還停在按鈕上，按鈕的提示文字不會自己消失，會蓋住剛打開的選單。
    /// 工具列在按下去的時候已經先把那個按鈕的提示文字拿掉了，這裡只要讓瀏覽器元件再處理一次滑鼠位置。
    /// 只是傳訊息給自己的視窗，不會動到真正的滑鼠，也不是模擬輸入
    pub fn unhover() {
        unsafe {
            let mut p = POINT { x: 0, y: 0 };
            if GetCursorPos(&mut p) == 0 {
                return;
            }
            let mut h = WindowFromPoint(p);
            if h.is_null() || !is_own(GetAncestor(h, GA_ROOT) as isize) {
                return;
            }
            // 往上找到瀏覽器元件收滑鼠訊息的那個視窗
            for _ in 0..6 {
                if h.is_null() {
                    return;
                }
                let mut name = [0u16; 64];
                let n = GetClassNameW(h, name.as_mut_ptr(), name.len() as i32).clamp(0, 63) as usize;
                if String::from_utf16_lossy(&name[..n]) == "Chrome_WidgetWin_1" {
                    let mut c = p;
                    windows_sys::Win32::Graphics::Gdi::ScreenToClient(h, &mut c);
                    let at = |x: i32, y: i32| (((y as u16 as u32) << 16) | (x as u16 as u32)) as LPARAM;
                    // 位置有變才算有移動：先差一點點，再回到原位
                    PostMessageW(h, WM_MOUSEMOVE, 0, at(c.x + 1, c.y));
                    PostMessageW(h, WM_MOUSEMOVE, 0, at(c.x, c.y));
                    return;
                }
                h = GetParent(h);
            }
        }
    }

    /// 選單：放到指定位置、蓋在所有視窗最上面、顯示（不搶焦點）
    pub fn show_at(h: isize, x: i32, y: i32, w: i32, hh: i32) {
        unsafe { SetWindowPos(h as HWND, HWND_TOPMOST, x, y, w, hh, SWP_NOACTIVATE | SWP_SHOWWINDOW) };
    }

    pub fn iconic(h: isize) -> bool {
        unsafe { IsIconic(h as HWND) != 0 }
    }

    /// 把視窗目前的畫面抓下來（含瀏覽器用顯示卡畫的內容），縮到最多 `max_w` 寬 → (寬, 高, RGBA)
    pub fn capture(h: isize, max_w: u32) -> Option<(u32, u32, Vec<u8>)> {
        use windows_sys::Win32::Graphics::Gdi::*;
        use windows_sys::Win32::Storage::Xps::PrintWindow;
        unsafe {
            let hwnd = h as HWND;
            let mut r = RECT { left: 0, top: 0, right: 0, bottom: 0 };
            if GetWindowRect(hwnd, &mut r) == 0 {
                return None;
            }
            let (w, hh) = ((r.right - r.left).max(0) as u32, (r.bottom - r.top).max(0) as u32);
            if w < 16 || hh < 16 {
                return None;
            }
            let sdc = GetDC(std::ptr::null_mut());
            let mdc = CreateCompatibleDC(sdc);
            let mut bmi: BITMAPINFO = std::mem::zeroed();
            bmi.bmiHeader.biSize = std::mem::size_of::<BITMAPINFOHEADER>() as u32;
            bmi.bmiHeader.biWidth = w as i32;
            bmi.bmiHeader.biHeight = -(hh as i32);
            bmi.bmiHeader.biPlanes = 1;
            bmi.bmiHeader.biBitCount = 32;
            bmi.bmiHeader.biCompression = BI_RGB;
            let mut bits: *mut std::ffi::c_void = std::ptr::null_mut();
            let bmp = CreateDIBSection(sdc, &bmi, DIB_RGB_COLORS, &mut bits, std::ptr::null_mut(), 0);
            let mut out = None;
            if !bmp.is_null() && !bits.is_null() {
                let old = SelectObject(mdc, bmp as HGDIOBJ);
                if PrintWindow(hwnd, mdc, PW_RENDERFULLCONTENT) != 0 {
                    let src = std::slice::from_raw_parts(bits as *const u8, (w * hh * 4) as usize);
                    out = Some(shrink(src, w, hh, max_w));
                }
                SelectObject(mdc, old);
                DeleteObject(bmp as HGDIOBJ);
            }
            DeleteDC(mdc);
            ReleaseDC(std::ptr::null_mut(), sdc);
            out
        }
    }

    /// BGRA → 縮小（取平均）→ RGBA
    fn shrink(src: &[u8], w: u32, h: u32, max_w: u32) -> (u32, u32, Vec<u8>) {
        let k = (w as f64 / max_w as f64).ceil().max(1.0) as u32;
        let (ow, oh) = ((w / k).max(1), (h / k).max(1));
        let mut out = vec![0u8; (ow * oh * 4) as usize];
        let n = (k * k) as u32;
        for oy in 0..oh {
            for ox in 0..ow {
                let (mut r, mut g, mut b) = (0u32, 0u32, 0u32);
                for dy in 0..k {
                    let row = ((oy * k + dy) * w) as usize;
                    for dx in 0..k {
                        let i = (row + (ox * k + dx) as usize) * 4;
                        b += src[i] as u32;
                        g += src[i + 1] as u32;
                        r += src[i + 2] as u32;
                    }
                }
                let o = ((oy * ow + ox) * 4) as usize;
                out[o] = (r / n) as u8;
                out[o + 1] = (g / n) as u8;
                out[o + 2] = (b / n) as u8;
                out[o + 3] = 255;
            }
        }
        (ow, oh, out)
    }
}

static RESTORE_AT: Mutex<Option<Instant>> = Mutex::new(None);

/// 螢幕配置改變後幾秒：把每個瀏覽器視窗放回記錄的大小與位置（Windows 暫時塞進小螢幕時會把它縮小）
fn restore_later() {
    let now = Instant::now();
    *lock(&RESTORE_AT) = Some(now);
    let Some(app) = APP_HANDLE.get().cloned() else { return };
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(2500));
        if *lock(&RESTORE_AT) != Some(now) {
            return;
        }
        // 先把清單複製出來再處理（直接寫在 for 後面的話，整個迴圈都會鎖著設定檔，裡面再讀設定檔就卡死）
        let open: Vec<WebWin> = st().file.wins.iter().filter(|w| w.open).cloned().collect();
        for cfg in open {
            let Some(win) = app.get_window(&win_label(&cfg.id)) else { continue };
            if win.is_minimized().unwrap_or(false) || win.is_maximized().unwrap_or(false) || rt(|m| m.get(&cfg.id).is_some_and(|r| r.full || r.fs)) {
                continue;
            }
            let sf = win.scale_factor().unwrap_or(1.0).max(0.5);
            let old_sf = rt(|m| m.get_mut(&cfg.id).map(|r| std::mem::replace(&mut r.sf, sf))).unwrap_or(sf);
            let (w, h) = (((cfg.w as f64) / old_sf * sf).round() as u32, ((cfg.h as f64) / old_sf * sf).round() as u32);
            let (x, y) = app_on_screen(&app, cfg.x, cfg.y, w, h);
            edit(&cfg.id, |c| (c.w, c.h, c.x, c.y) = (w, h, x, y));
            let _ = win.set_position(PhysicalPosition::new(x, y));
            let _ = win.set_size(PhysicalSize::new(w, h));
            if let Ok(size) = win.inner_size() {
                layout(&app, &cfg.id, &win, size);
            }
        }
    });
}

static APP_HANDLE: OnceLock<AppHandle> = OnceLock::new();

/// 所有開著的 web 視窗
fn web_hwnds() -> Vec<isize> {
    rt(|m| m.values().map(|r| r.hwnd).filter(|h| *h != 0).collect())
}

/// 擴充功能的小視窗：蓋在網頁上（像 Chrome 按擴充功能圖示那樣），目前的分頁不變，擴充功能才看得到「目前網頁」
static EXT_POPUP: Mutex<Option<(String, (i32, i32, i32, i32))>> = Mutex::new(None);

fn ext_popup_label(wid: &str) -> String {
    format!("webext-{wid}")
}

fn close_ext_popup(app: &AppHandle, wid: &str) {
    let open = lock(&EXT_POPUP).as_ref().is_some_and(|(w, _)| w == wid);
    if open {
        *lock(&EXT_POPUP) = None;
    }
    if let Some(p) = app.get_webview(&ext_popup_label(wid)) {
        let _ = p.close();
    }
}

/// 在視窗右上（擴充功能按鈕下面）開擴充功能的頁面；anchor = 貼著的範圍（螢幕實體像素）
fn open_ext_popup(app: &AppHandle, wid: &str, url: Url, anchor: (i32, i32, i32, i32)) -> Result<(), String> {
    close_ext_popup(app, wid);
    let win = app.get_window(&win_label(wid)).ok_or("找不到視窗")?;
    let sf = win.scale_factor().unwrap_or(1.0).max(0.5);
    let size = win.inner_size().map_err(|e| e.to_string())?;
    let origin = win.inner_position().map_err(|e| e.to_string())?;
    let (w, h) = ((400.0 * sf).round() as i32, (560.0 * sf).round() as i32);
    let w = w.min(size.width as i32 - 8).max(100);
    let bar_px = (bar_h(win.is_maximized().unwrap_or(false)) * sf) as i32;
    let h = h.min(size.height as i32 - bar_px - 8).max(100);
    let x = (anchor.2 - origin.x - w).max(4).min(size.width as i32 - w - 4);
    let y = (anchor.3 - origin.y + 4).max(0).min(size.height as i32 - h);
    // 擴充功能問「目前的分頁」時，回答目前顯示的網頁（WebView2 把每個網頁當成各自的視窗，不這樣做擴充功能只會看到自己）
    // 用網頁目前真正的網址（YouTube 這類不換頁就換網址的網站，記錄的網址會舊）
    let page = active_page(app, wid).and_then(|p| p.url().ok()).map(|u| u.to_string()).or_else(|| get(wid).map(|w| w.url)).unwrap_or_default();
    let patch = EXT_TABS_JS.replace("__PAGE_URL__", &serde_json::to_string(&page).unwrap_or_else(|_| "\"\"".into()));
    let b = WebviewBuilder::new(ext_popup_label(wid), WebviewUrl::External(url))
        .data_directory(profile_dir())
        .browser_extensions_enabled(true)
        .initialization_script(&patch)
        .disable_drag_drop_handler();
    win.add_child(b, PhysicalPosition::new(x, y), PhysicalSize::new(w as u32, h as u32)).map_err(|e| e.to_string())?;
    *lock(&EXT_POPUP) = Some((wid.to_string(), (origin.x + x, origin.y + y, origin.x + x + w, origin.y + y + h)));
    Ok(())
}

fn install_subclass(app: &AppHandle, hwnd: isize, rect: (i32, i32, u32, u32)) {
    #[cfg(windows)]
    if hwnd != 0 {
        let (x, y, w, h) = (rect.0, rect.1, rect.2 as i32, rect.3 as i32);
        let _ = app.run_on_main_thread(move || win32::subclass(hwnd, Some((x, y, w, h))));
    }
    #[cfg(not(windows))]
    let _ = (app, hwnd, rect);
}

fn alpha_of(id: &str) -> u8 {
    let o = get(id).map(|w| w.opacity).unwrap_or(100).clamp(30, 100);
    ((o * 255 + 50) / 100) as u8
}

/// 鍵盤交給視窗（打字）
fn enter_typing(app: &AppHandle, id: &str, hwnd: isize, from_bar: bool) {
    #[cfg(windows)]
    {
        let prev = win32::foreground();
        let prev = if win32::is_own(prev) { 0 } else { prev };
        rt(|m| {
            if let Some(r) = m.get_mut(id) {
                r.prev_fg = prev;
            }
        });
        lock(&TYPING_HWND).push(hwnd);
        let locked = get(id).map(|w| w.locked).unwrap_or(false);
        let alpha = alpha_of(id);
        let target = if from_bar {
            bar_label(id)
        } else {
            get(id).map(|w| page_label(id, &w.active)).unwrap_or_default()
        };
        if let Some(p) = active_page(app, id) {
            let _ = p.eval("window.__mbGame = true");
        }
        let app2 = app.clone();
        let _ = app.run_on_main_thread(move || {
            win32::apply_style(hwnd, false, locked, alpha);
            win32::activate(hwnd);
            if let Some(wv) = app2.get_webview(&target) {
                let _ = wv.set_focus();
            }
        });
    }
    #[cfg(not(windows))]
    let _ = (app, id, hwnd, from_bar);
}

/// 打字結束。give_back = 把鍵盤還給原本的視窗（遊戲）；使用者自己切走的就不用還
fn leave_typing(app: &AppHandle, id: &str, hwnd: isize, give_back: bool) {
    if let Some(p) = active_page(app, id) {
        let _ = p.eval("window.__mbGame = false");
    }
    #[cfg(windows)]
    {
        lock(&TYPING_HWND).retain(|h| *h != hwnd);
        let prev = rt(|m| m.get_mut(id).map(|r| std::mem::take(&mut r.prev_fg)).unwrap_or(0));
        if give_back {
            let _ = app.run_on_main_thread(move || {
                if win32::foreground() == hwnd || win32::is_own(win32::foreground()) {
                    win32::give_back(prev);
                }
            });
        }
    }
    #[cfg(not(windows))]
    let _ = (app, id, hwnd, give_back);
}

/* ---------------- 背景執行緒 ---------------- */

pub fn init(app: &AppHandle, path: PathBuf) {
    let _ = APP_HANDLE.set(app.clone());
    if let Some(dir) = path.parent() {
        let _ = WEB_DIR.set(dir.to_path_buf());
        crate::userscripts::init(dir.to_path_buf());
        load_side(dir);
    }
    #[cfg(windows)]
    let _ = app.run_on_main_thread(win32::hook_foreground);
    let file = load(&path);
    let reopen: Vec<String> = file.wins.iter().filter(|w| w.open).map(|w| w.id.clone()).collect();
    let _ = STORE.set(Mutex::new(Store { file, path, dirty: false }));

    // WebView2 通知的事情（快捷鍵、右鍵選單、下載…）：一個背景執行緒照順序處理
    let (tx, rx) = std::sync::mpsc::channel::<(String, String, Ev)>();
    let _ = EVQ.set(Mutex::new(tx));
    let app0 = app.clone();
    std::thread::spawn(move || {
        for (wid, tid, ev) in rx {
            handle_ev(&app0, &wid, &tid, ev);
        }
    });

    let app1 = app.clone();
    std::thread::spawn(move || {
        // 上次關閉程式時開著的視窗：重新打開（打開時會先從 Chrome 同步書籤與擴充功能）
        std::thread::sleep(Duration::from_millis(1500));
        for id in reopen {
            if let Err(e) = open_window(&app1, &id, false) {
                eprintln!("{e}");
            }
        }
        let mut last_save = Instant::now();
        let mut last_sync: Option<Instant> = None;
        loop {
            // 沒有視窗開著時什麼都不用做，放慢檢查
            let any_open = st().file.wins.iter().any(|w| w.open);
            std::thread::sleep(Duration::from_millis(if any_open { 100 } else { 500 }));
            if any_open {
                tick(&app1);
            }
            // 書籤自動跟 Chrome 同步：有視窗開著時每 5 秒看一次 Chrome 的書籤檔有沒有變動
            if any_open && last_sync.is_none_or(|t| t.elapsed() >= Duration::from_secs(5)) {
                last_sync = Some(Instant::now());
                let _ = import_chrome(&app1, false);
            }
            let to_save = {
                let mut s = st();
                if s.dirty && last_save.elapsed() >= Duration::from_secs(1) {
                    s.dirty = false;
                    last_save = Instant::now();
                    let mut f = s.file.clone();
                    // 無痕式視窗不存檔
                    f.wins.retain(|w| !w.incognito);
                    Some((s.path.clone(), f))
                } else {
                    None
                }
            };
            if let Some((p, f)) = to_save {
                save(&p, &f);
            }
            flush_side(false);
        }
    });

    // 選單開著時：點到選單外面就關掉
    pop::watch(app);
}

enum Action {
    /// 遊戲在前景時點了輸入框：鍵盤交給 web（true = 工具列的輸入框）
    Enter(bool),
    /// 打完了：鍵盤還給遊戲
    Leave,
    /// 使用者自己切到別的視窗：打字結束，不用還
    End,
}

fn tick(app: &AppHandle) {
    let open: Vec<WebWin> = st().file.wins.iter().filter(|w| w.open).cloned().collect();
    if open.is_empty() {
        return;
    }
    let info = fg::current();
    if !info.is_self || info.is_main {
        GAME_FG.store(info.is_game, Ordering::SeqCst);
    }
    #[cfg(windows)]
    let fg_hwnd = win32::foreground();
    #[cfg(not(windows))]
    let fg_hwnd = 0isize;
    #[cfg(windows)]
    if info.is_game {
        win32::GAME_HWND.store(fg_hwnd, Ordering::SeqCst);
    }
    let game_fg = GAME_FG.load(Ordering::SeqCst);
    let now = Instant::now();
    let fresh = |t: Option<Instant>| t.is_some_and(|t| now.duration_since(t) < Duration::from_millis(1500));

    // 每 2 秒抓一次每個視窗的畫面（卡片上顯示、關閉時留最後一張）
    static LAST_SHOT: Mutex<Option<Instant>> = Mutex::new(None);
    let take_shots = {
        let mut g = lock(&LAST_SHOT);
        if g.is_none_or(|t| t.elapsed() >= SHOT_EVERY) {
            *g = Some(Instant::now());
            true
        } else {
            false
        }
    };
    if take_shots {
        for cfg in &open {
            #[cfg(windows)]
            if let Some(h) = app.get_window(&win_label(&cfg.id)).and_then(|w| w.hwnd().ok()).map(|h| h.0 as isize) {
                shot_take(&cfg.id, h);
            }
        }
    }

    for cfg in open {
        let id = cfg.id.clone();
        // 先在鎖內決定要做什麼，鎖外再動視窗
        let decided = rt(|m| {
            let r = m.get_mut(&id)?;
            if r.hwnd == 0 {
                return None;
            }
            let mut act = None;
            if !r.typing {
                // 只有「遊戲在前景時點了輸入框」才把鍵盤交給 web；web 本來就在前景時本來就能打字，不用處理
                let page = r.page_typing && fresh(r.page_on_at);
                let bar = r.bar_typing && fresh(r.bar_on_at);
                if info.is_game && !cfg.locked && (page || bar) {
                    r.typing = true;
                    r.typing_since = Some(now);
                    r.out_since = None;
                    act = Some(Action::Enter(bar));
                }
                if page || bar || !(r.page_typing || r.bar_typing) {
                    r.page_on_at = None;
                    r.bar_on_at = None;
                }
            } else {
                let started = r.typing_since.is_some_and(|t| now.duration_since(t) < Duration::from_millis(700));
                let page_grace = r.page_off_at.is_some_and(|t| now.duration_since(t) < Duration::from_millis(250));
                let mut keep = r.bar_typing || r.page_typing || page_grace;
                #[cfg(windows)]
                {
                    // 打字中滑鼠移出視窗超過 0.4 秒：鍵盤還給遊戲
                    if win32::cursor_outside(r.hwnd) {
                        let since = *r.out_since.get_or_insert(now);
                        if now.duration_since(since) >= Duration::from_millis(400) {
                            keep = false;
                        }
                    } else {
                        r.out_since = None;
                    }
                }
                #[cfg(windows)]
                let mine = fg_hwnd == r.hwnd || win32::is_own(fg_hwnd);
                #[cfg(not(windows))]
                let mine = true;
                if !started && !mine {
                    act = Some(Action::End);
                } else if !started && !keep {
                    act = Some(Action::Leave);
                }
                if act.is_some() {
                    r.typing = false;
                    r.typing_since = None;
                    r.page_on_at = None;
                    r.bar_on_at = None;
                }
            }
            Some((r.hwnd, r.typing, act))
        });
        let Some((hwnd, typing, act)) = decided else { continue };
        match act {
            Some(Action::Enter(from_bar)) => enter_typing(app, &id, hwnd, from_bar),
            Some(Action::Leave) => {
                leave_typing(app, &id, hwnd, true);
                // 網頁與網址列的輸入框也取消選取，網頁裡的打字狀態清掉
                if let Some(p) = active_page(app, &id) {
                    let _ = p.eval("(() => { const a = document.activeElement; if (a && a !== document.body) a.blur(); if (window.__mbReset) window.__mbReset(); })()");
                }
                let _ = app.emit_to(bar_label(&id).as_str(), "web-blur", id.clone());
                rt(|m| {
                    if let Some(r) = m.get_mut(&id) {
                        r.page_typing = false;
                        r.bar_typing = false;
                    }
                });
            }
            Some(Action::End) => {
                leave_typing(app, &id, hwnd, false);
                // 使用者自己切走：不取消選取，只把網頁裡的打字狀態清掉（之後從遊戲再點輸入框才會再交給 web）
                if let Some(p) = active_page(app, &id) {
                    let _ = p.eval("window.__mbReset && window.__mbReset()");
                }
                rt(|m| {
                    if let Some(r) = m.get_mut(&id) {
                        r.page_typing = false;
                    }
                });
            }
            None => {}
        }
        #[cfg(windows)]
        {
            // 遊戲在前景：點這個視窗不搶遊戲的鍵盤。其他時候就是一般視窗（點了就切過來，跟 Win11 一樣）
            win32::apply_style(hwnd, info.is_game && !typing, cfg.locked, alpha_of(&id));
            let visible = !cfg.game_only || game_fg || typing || info.is_self;
            if visible && info.is_game {
                win32::keep_above(hwnd, fg_hwnd);
            }
            // 「只在遊戲畫面時顯示」：切到別的程式時藏起來
            if cfg.game_only || !win32::visible(hwnd) {
                if win32::visible(hwnd) != visible {
                    win32::show(hwnd, visible);
                    if !visible {
                        pop::hide_of(app, &id);
                    }
                }
            }
            // F11 全螢幕：滑鼠移到最上面暫時顯示工具列，移開就收起來
            let (fs, reveal, sf) = rt(|m| m.get(&id).map(|r| (r.fs && !r.full, r.reveal, r.sf)).unwrap_or((false, false, 1.0)));
            if fs {
                if let Some((l, t, r, _)) = win32::rect(hwnd) {
                    let (cx, cy) = win32::cursor();
                    let inside = cx >= l && cx < r;
                    let bar_px = (bar_h(true) * sf.max(0.5)).round() as i32;
                    let want = if reveal { (inside && cy < t + bar_px + 12) || pop::open_for(&id) } else { inside && cy <= t + 1 };
                    if want != reveal {
                        rt(|m| {
                            if let Some(r) = m.get_mut(&id) {
                                r.reveal = want;
                            }
                        });
                        relayout(app, &id);
                    }
                }
            }
        }
        #[cfg(not(windows))]
        let _ = (typing, game_fg, fg_hwnd);
    }
}

/* ---------------- 指令：主程式 ---------------- */

#[tauri::command]
pub fn web_list() -> WebFile {
    let mut f = st().file.clone();
    // 無痕式視窗不列在主程式裡（關掉就消失）
    f.wins.retain(|w| !w.incognito);
    f
}

#[tauri::command]
pub async fn web_add(app: AppHandle) -> Result<WebWin, String> {
    let id = new_window(&app, None, false, None)?;
    focus_omni(&app, &id, false);
    get(&id).ok_or_else(|| "找不到這個視窗".to_string())
}

/// 打開視窗（已經開著但縮小了就叫出來）
#[tauri::command]
pub async fn web_open(app: AppHandle, id: String) -> Result<(), String> {
    open_window(&app, &id, true)
}

#[tauri::command]
pub async fn web_close(app: AppHandle, id: String) {
    close_window(&app, &id);
}

#[tauri::command]
pub async fn web_delete(app: AppHandle, id: String) {
    close_window(&app, &id);
    shot_forget(&id);
    let mut s = st();
    s.file.wins.retain(|w| w.id != id);
    s.dirty = true;
}

/// 不透明度、鎖定、只在遊戲時顯示（背景執行緒 0.1 秒內套用）
#[tauri::command]
pub fn web_set(id: String, opacity: Option<u32>, locked: Option<bool>, game_only: Option<bool>) {
    edit(&id, |w| {
        if let Some(o) = opacity {
            w.opacity = o.clamp(30, 100);
        }
        if let Some(l) = locked {
            w.locked = l;
        }
        if let Some(g) = game_only {
            w.game_only = g;
        }
    });
}

/// Chrome 的資料夾（程式以系統管理員身分執行時也找得到同一個使用者的資料夾）
fn chrome_roots() -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = Vec::new();
    let bases = [
        std::env::var_os("LOCALAPPDATA").map(PathBuf::from),
        std::env::var_os("USERPROFILE").map(|p| PathBuf::from(p).join("AppData").join("Local")),
    ];
    for base in bases.into_iter().flatten() {
        let root = base.join("Google").join("Chrome").join("User Data");
        if !out.contains(&root) {
            out.push(root);
        }
    }
    out
}

fn read_json(path: &Path) -> Option<Value> {
    serde_json::from_str(&std::fs::read_to_string(path).ok()?).ok()
}

/// 一個 Chrome 使用者的書籤檔：Bookmarks（這台電腦的）與 AccountBookmarks（登入 Google 帳號存在帳號裡的）
const BM_FILES: [&str; 2] = ["Bookmarks", "AccountBookmarks"];

fn profile_files(root: &Path, dir: &str) -> Vec<PathBuf> {
    BM_FILES.iter().map(|f| root.join(dir).join(f)).filter(|p| p.is_file()).collect()
}

/// 讀一個 Chrome 使用者的書籤 → (書籤列, 所有書籤)
fn read_profile(root: &Path, dir: &str, next: &mut u64) -> Option<(Vec<Bm>, Vec<Bm>)> {
    let files = profile_files(root, dir);
    if files.is_empty() {
        return None;
    }
    let (mut bar, mut other) = (Vec::new(), Vec::new());
    for f in files {
        let Some(v) = read_json(&f) else { continue };
        let b = from_chrome(&v["roots"]["bookmark_bar"], next).map(|f| f.children).unwrap_or_default();
        let mut o = from_chrome(&v["roots"]["other"], next).map(|f| f.children).unwrap_or_default();
        if let Some(mut synced) = from_chrome(&v["roots"]["synced"], next) {
            if !synced.children.is_empty() {
                synced.title = "行動裝置書籤".into();
                o.push(synced);
            }
        }
        merge(&mut bar, b);
        merge(&mut other, o);
    }
    Some((bar, other))
}

/// 書籤檔最後修改的時間（有變動才重新匯入）
fn profile_mtime(root: &Path, dir: &str) -> u128 {
    profile_files(root, dir)
        .iter()
        .filter_map(|p| p.metadata().ok()?.modified().ok())
        .filter_map(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_millis())
        .max()
        .unwrap_or(0)
}

#[derive(Serialize, Clone)]
pub struct ChromeProfile {
    dir: String,
    name: String,
    account: String,
    count: usize,
}

/// 找出所有有書籤的 Chrome 使用者
fn find_profiles() -> Result<(PathBuf, Vec<ChromeProfile>), String> {
    let roots = chrome_roots();
    for root in &roots {
        if !root.is_dir() {
            continue;
        }
        let state = read_json(&root.join("Local State")).unwrap_or(Value::Null);
        let cache = state["profile"]["info_cache"].as_object().cloned().unwrap_or_default();
        // Local State 裡記的使用者，加上資料夾裡實際有書籤檔的
        let mut dirs: Vec<String> = cache.keys().cloned().collect();
        if let Ok(rd) = std::fs::read_dir(root) {
            for e in rd.flatten() {
                let name = e.file_name().to_string_lossy().to_string();
                if !dirs.contains(&name) && !profile_files(root, &name).is_empty() {
                    dirs.push(name);
                }
            }
        }
        let mut out = Vec::new();
        for dir in dirs {
            let mut n = 0;
            let Some((bar, other)) = read_profile(root, &dir, &mut n) else { continue };
            let info = cache.get(&dir).cloned().unwrap_or(Value::Null);
            out.push(ChromeProfile {
                name: info["name"].as_str().filter(|s| !s.is_empty()).unwrap_or(&dir).to_string(),
                account: info["user_name"].as_str().unwrap_or("").to_string(),
                count: count_links(&bar) + count_links(&other),
                dir,
            });
        }
        if !out.is_empty() {
            out.sort_by_key(|p| (p.dir != "Default", p.dir.clone()));
            return Ok((root.clone(), out));
        }
    }
    let shown = roots.first().map(|p| p.display().to_string()).unwrap_or_default();
    Err(format!("找不到 Chrome 的書籤（找過 {shown}）"))
}

/// 要從哪個使用者匯入：選過的優先；沒選過就挑有登入帳號、書籤最多的
fn pick_profile(list: &[ChromeProfile], chosen: &str) -> Option<ChromeProfile> {
    list.iter()
        .find(|p| p.dir == chosen)
        .or_else(|| list.iter().filter(|p| !p.account.is_empty()).max_by_key(|p| p.count))
        .or_else(|| list.iter().max_by_key(|p| p.count))
        .cloned()
}

/// 上次自動匯入時書籤檔的時間（使用者, 時間）
static SYNCED: Mutex<Option<(String, u128)>> = Mutex::new(None);

/// 從 Chrome 匯入（force = 不管有沒有變動都匯入）。回傳 (使用者, 新增幾個)
fn import_chrome(app: &AppHandle, force: bool) -> Result<(ChromeProfile, usize), String> {
    let (root, list) = find_profiles()?;
    let chosen = bm_read(|b| b.profile.clone());
    let p = pick_profile(&list, &chosen).ok_or("找不到 Chrome 的書籤")?;
    let mtime = profile_mtime(&root, &p.dir);
    if !force && lock(&SYNCED).as_ref().is_some_and(|(d, t)| *d == p.dir && *t == mtime) {
        return Ok((p, 0));
    }
    let mut next = bm_read(|b| b.next_id);
    let (bar, other) = read_profile(&root, &p.dir, &mut next).ok_or("讀不到這個使用者的書籤")?;
    let (added, changed) = bm(|b| {
        b.next_id = b.next_id.max(next);
        let before = serde_json::to_string(&(&b.bar, &b.other)).unwrap_or_default();
        let ignored = b.ignored.clone();
        let added = sync_level(&mut b.bar, bar, &ignored) + sync_level(&mut b.other, other, &ignored);
        (added, serde_json::to_string(&(&b.bar, &b.other)).unwrap_or_default() != before)
    });
    *lock(&SYNCED) = Some((p.dir.clone(), mtime));
    if changed {
        emit_bm(app);
    }
    Ok((p, added))
}

#[derive(Serialize)]
pub struct ChromeStatus {
    sync: bool,
    current: Option<ChromeProfile>,
    profiles: Vec<ChromeProfile>,
    error: String,
}

/// 主程式：自動匯入的狀態、目前用哪個 Chrome 使用者、有哪些使用者
#[tauri::command]
pub fn web_chrome_status() -> ChromeStatus {
    let (sync, chosen) = bm_read(|b| (true, b.profile.clone()));
    match find_profiles() {
        Ok((_, profiles)) => ChromeStatus { sync, current: pick_profile(&profiles, &chosen), profiles, error: String::new() },
        Err(e) => ChromeStatus { sync, current: None, profiles: Vec::new(), error: e },
    }
}

/// 選擇從哪個 Chrome 使用者匯入（空白 = 自動選），並馬上匯入。回傳新增幾個
#[tauri::command]
pub async fn web_chrome_import(app: AppHandle, dir: Option<String>) -> Result<usize, String> {
    if let Some(dir) = dir {
        if !dir.chars().all(|c| c.is_ascii_alphanumeric() || c == ' ' || c == '_' || c == '-') {
            return Err("Chrome 使用者名稱不正確".into());
        }
        bm(|b| b.profile = dir);
    }
    let (_, added) = import_chrome(&app, true)?;
    emit_bm(&app);
    Ok(added)
}

/* ---------------- 擴充功能（從 Chrome 複製） ---------------- */

#[derive(Serialize, Clone)]
pub struct ExtInfo {
    id: String,
    name: String,
    version: String,
    /// 在這裡停用了
    off: bool,
    /// 圖示（data: 網址）
    icon: String,
    /// 擴充功能按鈕的選單頁面、選項頁面（要能用固定的編號開啟才有）
    popup: String,
    options: String,
    /// 圖示固定在工具列上
    pinned: bool,
}

#[derive(Serialize)]
pub struct ExtStatus {
    list: Vec<ExtInfo>,
    /// 這台電腦的 WebView2 載入擴充功能失敗過
    broken: bool,
}

/// 每個擴充功能：版本、放在哪個資料夾（相對 web-ext）、名稱、停用、裝好了沒
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
struct ExtMark {
    ver: String,
    dir: String,
    name: String,
    off: bool,
    loaded: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
struct ExtState {
    exts: std::collections::BTreeMap<String, ExtMark>,
    /// 待安裝的資料夾（新的、更新過的擴充功能放這裡，下次開網頁時安裝）
    pending: String,
    /// Chrome 裡固定在工具列上的擴充功能（同步時讀到的）
    chrome_pins: Vec<String>,
    /// 在這裡自己調整過的固定清單（有調整過就用這個，不再跟 Chrome）
    pins: Option<Vec<String>>,
}

impl ExtState {
    fn pinned(&self, id: &str) -> bool {
        self.pins.as_ref().unwrap_or(&self.chrome_pins).iter().any(|p| p == id)
    }
}

static EXT_LOCK: Mutex<()> = Mutex::new(());

fn ext_state_path() -> PathBuf {
    WEB_DIR.get().cloned().unwrap_or_default().join("web-ext.json")
}

fn ext_state() -> ExtState {
    let Some(v) = read_json(&ext_state_path()) else { return ExtState::default() };
    if v.get("exts").is_some() {
        return serde_json::from_value(v).unwrap_or_default();
    }
    // 舊版：{ 編號: 版本 }，放在 web-ext/編號，已經裝過
    let old: HashMap<String, String> = serde_json::from_value(v).unwrap_or_default();
    ExtState {
        exts: old.into_iter().map(|(id, ver)| (id.clone(), ExtMark { ver, dir: id, loaded: true, ..ExtMark::default() })).collect(),
        ..ExtState::default()
    }
}

fn save_ext_state(st: &ExtState) {
    if let Ok(t) = serde_json::to_string_pretty(st) {
        let _ = std::fs::write(ext_state_path(), t);
    }
}

/// 擴充功能的圖示 → data: 網址（挑 32 像素左右的）
fn ext_icon(dir: &Path, m: &Value) -> String {
    use base64::Engine;
    let Some(icons) = m["icons"].as_object() else { return String::new() };
    let best = icons
        .iter()
        .filter_map(|(k, v)| Some((k.parse::<i64>().ok()?, v.as_str()?)))
        .min_by_key(|(k, _)| (k - 32).abs());
    let Some((_, rel)) = best else { return String::new() };
    let path = dir.join(rel.trim_start_matches('/'));
    let Ok(bytes) = std::fs::read(&path) else { return String::new() };
    if bytes.len() > 200_000 {
        return String::new();
    }
    let mime = match path.extension().and_then(|e| e.to_str()).unwrap_or("").to_lowercase().as_str() {
        "svg" => "image/svg+xml",
        "jpg" | "jpeg" => "image/jpeg",
        "webp" => "image/webp",
        _ => "image/png",
    };
    format!("data:{mime};base64,{}", base64::engine::general_purpose::STANDARD.encode(bytes))
}

fn ext_list() -> Vec<ExtInfo> {
    let st = ext_state();
    let mut out: Vec<ExtInfo> = st
        .exts
        .iter()
        .map(|(id, mk)| {
            let dir = ext_dir().join(&mk.dir);
            let m = read_json(&dir.join("manifest.json"));
            let (icon, popup, options, name) = match &m {
                Some(m) if !mk.off => {
                    // 有 key 的擴充功能編號跟 Chrome 一樣，才能直接開它的頁面
                    let fixed = m["key"].is_string();
                    let page = |v: &Value| v.as_str().filter(|s| fixed && !s.is_empty()).map(|s| format!("chrome-extension://{id}/{}", s.trim_start_matches('/')));
                    let popup = page(&m["action"]["default_popup"])
                        .or_else(|| page(&m["browser_action"]["default_popup"]))
                        .or_else(|| page(&m["page_action"]["default_popup"]))
                        .unwrap_or_default();
                    let options = page(&m["options_ui"]["page"]).or_else(|| page(&m["options_page"])).unwrap_or_default();
                    (ext_icon(&dir, m), popup, options, ext_name(&dir, m))
                }
                _ => (String::new(), String::new(), String::new(), String::new()),
            };
            ExtInfo {
                id: id.clone(),
                name: if name.is_empty() { if mk.name.is_empty() { id.clone() } else { mk.name.clone() } } else { name },
                version: mk.ver.clone(),
                off: mk.off,
                icon,
                popup,
                options,
                pinned: st.pinned(id),
            }
        })
        .collect();
    out.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    out
}

fn version_key(v: &str) -> Vec<u64> {
    v.split(['.', '_']).map(|p| p.parse().unwrap_or(0)).collect()
}

/// 名稱是「__MSG_xxx__」時，到 _locales 找翻譯（繁中優先）
fn ext_name(dir: &Path, manifest: &Value) -> String {
    let raw = manifest["name"].as_str().unwrap_or("").to_string();
    let Some(key) = raw.strip_prefix("__MSG_").and_then(|k| k.strip_suffix("__")) else { return raw };
    let key = key.to_lowercase();
    let default = manifest["default_locale"].as_str().unwrap_or("en").to_string();
    for loc in ["zh_TW", "zh_HK", "zh_CN", default.as_str(), "en"] {
        let Some(m) = read_json(&dir.join("_locales").join(loc).join("messages.json")) else { continue };
        if let Some(obj) = m.as_object() {
            if let Some((_, v)) = obj.iter().find(|(k, _)| k.to_lowercase() == key) {
                if let Some(t) = v["message"].as_str() {
                    return t.to_string();
                }
            }
        }
    }
    raw
}

fn copy_dir(from: &Path, to: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(to)?;
    for e in std::fs::read_dir(from)? {
        let e = e?;
        let name = e.file_name();
        // Chrome 線上商店的簽章資料夾不需要
        if name == "_metadata" {
            continue;
        }
        let ft = e.file_type()?;
        if ft.is_dir() {
            copy_dir(&e.path(), &to.join(&name))?;
        } else if ft.is_file() {
            std::fs::copy(e.path(), to.join(&name))?;
        }
    }
    Ok(())
}

/// Chrome 裡被停用的擴充功能
fn disabled_ext(profile: &Path) -> HashSet<String> {
    let mut out = HashSet::new();
    for f in ["Secure Preferences", "Preferences"] {
        let Some(v) = read_json(&profile.join(f)) else { continue };
        let Some(settings) = v["extensions"]["settings"].as_object() else { continue };
        for (id, x) in settings {
            let reasons = &x["disable_reasons"];
            let off = x["state"].as_i64() == Some(0)
                || reasons.as_array().is_some_and(|a| !a.is_empty())
                || reasons.as_i64().is_some_and(|n| n != 0);
            if off {
                out.insert(id.clone());
            }
        }
    }
    out
}

/// 把 Chrome 裡啟用中的擴充功能複製過來：版本沒變的不動；新的、更新過的放進「待安裝」；
/// Chrome 移除的這裡也移除；在這裡停用的不複製
fn sync_extensions() -> Result<Vec<ExtInfo>, String> {
    let (root, list) = find_profiles()?;
    let chosen = bm_read(|b| b.profile.clone());
    let p = pick_profile(&list, &chosen).ok_or("找不到 Chrome 的使用者")?;
    let profile = root.join(&p.dir);
    let disabled = disabled_ext(&profile);
    let mut want: HashMap<String, (PathBuf, String, String)> = HashMap::new();
    if let Ok(rd) = std::fs::read_dir(profile.join("Extensions")) {
        for e in rd.flatten() {
            let id = e.file_name().to_string_lossy().to_string();
            if id == "Temp" || disabled.contains(&id) || !e.path().is_dir() {
                continue;
            }
            let Ok(vers) = std::fs::read_dir(e.path()) else { continue };
            let best = vers
                .flatten()
                .filter(|v| v.path().join("manifest.json").is_file())
                .max_by_key(|v| version_key(&v.file_name().to_string_lossy()));
            let Some(best) = best else { continue };
            let Some(m) = read_json(&best.path().join("manifest.json")) else { continue };
            // 應用程式、佈景主題不是擴充功能
            if !m["app"].is_null() || !m["theme"].is_null() {
                continue;
            }
            let name = ext_name(&best.path(), &m);
            want.insert(id, (best.path(), best.file_name().to_string_lossy().to_string(), name));
        }
    }
    // Chrome 裡固定在工具列上的擴充功能
    let chrome_pins: Vec<String> = ["Preferences", "Secure Preferences"]
        .iter()
        .filter_map(|f| read_json(&profile.join(f)))
        .find_map(|v| v["extensions"]["pinned_extensions"].as_array().map(|a| a.iter().filter_map(|x| x.as_str().map(str::to_string)).collect()))
        .unwrap_or_default();
    let _g = lock(&EXT_LOCK);
    let base = ext_dir();
    std::fs::create_dir_all(&base).map_err(|e| e.to_string())?;
    let mut st = ext_state();
    st.chrome_pins = chrome_pins;
    // Chrome 已經沒有的：移除
    let gone: Vec<String> = st.exts.keys().filter(|id| !want.contains_key(*id)).cloned().collect();
    for id in gone {
        if let Some(mk) = st.exts.remove(&id) {
            let _ = std::fs::remove_dir_all(base.join(&mk.dir));
        }
    }
    for (id, (src, ver, name)) in &want {
        let mk = st.exts.entry(id.clone()).or_default();
        mk.name = name.clone();
        if mk.off {
            if !mk.dir.is_empty() {
                let _ = std::fs::remove_dir_all(base.join(&mk.dir));
            }
            continue;
        }
        if mk.ver == *ver && !mk.dir.is_empty() && base.join(&mk.dir).join("manifest.json").is_file() {
            continue;
        }
        if st.pending.is_empty() {
            let ms = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis()).unwrap_or(0);
            st.pending = format!("b{ms}");
        }
        let mk = st.exts.get_mut(id).unwrap();
        if !mk.dir.is_empty() {
            let _ = std::fs::remove_dir_all(base.join(&mk.dir));
        }
        let rel = format!("{}/{id}", st.pending);
        if copy_dir(src, &base.join(&rel)).is_ok() {
            *mk = ExtMark { ver: ver.clone(), dir: rel, name: name.clone(), off: false, loaded: false };
        } else {
            let _ = std::fs::remove_dir_all(base.join(&rel));
            mk.dir.clear();
            mk.ver.clear();
        }
    }
    save_ext_state(&st);
    drop(_g);
    Ok(ext_list())
}

/// 在這裡停用／啟用一個擴充功能（重新開啟 web 視窗後生效）
fn ext_set_off(id: &str, off: bool) -> Result<(), String> {
    {
        let _g = lock(&EXT_LOCK);
        let mut st = ext_state();
        let Some(mk) = st.exts.get_mut(id) else { return Err("找不到這個擴充功能".into()) };
        mk.off = off;
        if off && !mk.dir.is_empty() {
            let _ = std::fs::remove_dir_all(ext_dir().join(&mk.dir));
            mk.dir.clear();
            mk.ver.clear();
        }
        save_ext_state(&st);
    }
    if !off {
        sync_extensions()?;
    }
    Ok(())
}

/// 工具列上要顯示圖示的擴充功能
fn ext_pins() -> Vec<ExtInfo> {
    ext_list().into_iter().filter(|e| e.pinned && !e.off).collect()
}

/// 擴充功能清單變了：每個視窗的工具列更新圖示
fn emit_exts(app: &AppHandle) {
    let pins = ext_pins();
    for id in open_ids() {
        let _ = app.emit_to(bar_label(&id).as_str(), "web-exts", pins.clone());
    }
    emit_main(app);
}

/// 固定／取消固定擴充功能的圖示
fn ext_toggle_pin(app: &AppHandle, id: &str) {
    {
        let _g = lock(&EXT_LOCK);
        let mut st = ext_state();
        let mut pins = st.pins.clone().unwrap_or_else(|| st.chrome_pins.clone());
        match pins.iter().position(|p| p == id) {
            Some(i) => {
                pins.remove(i);
            }
            None => pins.push(id.to_string()),
        }
        st.pins = Some(pins);
        save_ext_state(&st);
    }
    emit_exts(app);
}

#[tauri::command]
pub async fn web_ext_toggle(app: AppHandle, id: String, on: bool) -> Result<ExtStatus, String> {
    ext_set_off(&id, !on)?;
    emit_exts(&app);
    Ok(ExtStatus { list: ext_list(), broken: EXT_BROKEN.load(Ordering::SeqCst) })
}

/// 固定／取消固定擴充功能的圖示（擴充功能頁面）
#[tauri::command]
pub async fn web_ext_pin(app: AppHandle, id: String) -> ExtStatus {
    ext_toggle_pin(&app, &id);
    ExtStatus { list: ext_list(), broken: EXT_BROKEN.load(Ordering::SeqCst) }
}

#[tauri::command]
pub fn web_ext_list() -> ExtStatus {
    ExtStatus { list: ext_list(), broken: EXT_BROKEN.load(Ordering::SeqCst) }
}

/// 重新從 Chrome 同步擴充功能（新開的分頁、視窗才會套用）
#[tauri::command]
pub async fn web_ext_sync(app: AppHandle) -> Result<ExtStatus, String> {
    let list = sync_extensions()?;
    EXT_BROKEN.store(false, Ordering::SeqCst);
    emit_exts(&app);
    Ok(ExtStatus { list, broken: false })
}

/* ---------------- 指令：瀏覽器視窗上面的工具列 ---------------- */

#[tauri::command]
pub fn web_bar_init(webview: Webview) -> Option<BarInit> {
    let id = id_of(&webview)?;
    let focus = rt(|m| {
        m.get_mut(&id).and_then(|r| {
            r.bar_ready = true;
            r.want_omni.take()
        })
    });
    Some(BarInit { state: bar_state(&id)?, bm: bm_payload(), exts: ext_pins(), focus })
}

/// 執行瀏覽器指令（工具列的按鈕、內部頁面）。tab = 針對哪個分頁（沒給就是目前的）
#[tauri::command]
pub async fn web_cmd(app: AppHandle, webview: Webview, cmd: String, tab: Option<String>) {
    let Some(id) = owner_of(&webview) else { return };
    let tab = tab.or_else(|| page_of(&webview).map(|(_, t)| t));
    run_cmd(&app, &id, tab.as_deref(), &cmd);
}

/// 網址列按 Enter、新分頁的搜尋框：去輸入的網址（不是網址就搜尋）。new_tab = 開在新分頁（Alt+Enter）
#[tauri::command]
pub async fn web_go(app: AppHandle, webview: Webview, input: String, new_tab: Option<bool>) -> Result<(), String> {
    let id = owner_of(&webview).ok_or("不是瀏覽器視窗")?;
    let Some((target, _)) = webcore::resolve_input(&input, engine()) else { return Ok(()) };
    pop::hide_of(&app, &id);
    let w = get(&id).ok_or("找不到這個視窗")?;
    let tid = page_of(&webview).map(|(_, t)| t).unwrap_or_else(|| w.active.clone());
    if new_tab == Some(true) {
        add_tab(&app, &id, target, true, Some(&tid))?;
        return Ok(());
    }
    navigate_tab(&app, &id, &tid, &target);
    // 鍵盤從網址列交給網頁
    focus_page(&app, &id, &tid);
    Ok(())
}

/// 網址列（或改名框）被選取／取消選取
#[tauri::command]
pub fn web_typing(webview: Webview, on: bool) {
    if let Some(id) = id_of(&webview) {
        rt(|m| {
            if let Some(r) = m.get_mut(&id) {
                if on && !r.bar_typing {
                    r.bar_on_at = Some(Instant::now());
                }
                r.bar_typing = on;
            }
        });
    }
}

#[tauri::command]
pub async fn web_tab_select(app: AppHandle, webview: Webview, tab: String) -> Result<(), String> {
    let id = id_of(&webview).ok_or("不是瀏覽器視窗")?;
    select_tab(&app, &id, &tab)
}

/// 拖曳分頁排順序（固定的分頁只能在固定的那一區裡移動）
#[tauri::command]
pub fn web_tab_move(app: AppHandle, webview: Webview, tab: String, index: usize) {
    let Some(id) = id_of(&webview) else { return };
    edit(&id, |w| {
        if let Some(i) = w.tabs.iter().position(|t| t.id == tab) {
            let t = w.tabs.remove(i);
            let pinned = w.tabs.iter().filter(|x| x.pinned).count();
            let j = if t.pinned { index.min(pinned) } else { index.clamp(pinned, w.tabs.len()) };
            w.tabs.insert(j, t);
        }
    });
    emit_state(&app, &id);
}

#[tauri::command]
pub async fn web_tab_close(app: AppHandle, webview: Webview, tab: String) -> Result<(), String> {
    let id = id_of(&webview).ok_or("不是瀏覽器視窗")?;
    close_tab(&app, &id, &tab)
}

/// 右上角：縮小、放到最大／還原、關閉
#[tauri::command]
pub async fn web_win(app: AppHandle, webview: Webview, action: String) {
    let Some(id) = id_of(&webview) else { return };
    match action.as_str() {
        "min" => run_cmd(&app, &id, None, "win.min"),
        "max" => {
            // 全螢幕時按「還原」是離開全螢幕
            if rt(|m| m.get(&id).is_some_and(|r| r.fs)) {
                toggle_fs(&app, &id);
            } else {
                toggle_max(&app, &id);
            }
        }
        "close" => close_window(&app, &id),
        _ => {}
    }
}

#[tauri::command]
pub async fn web_bm_open(app: AppHandle, webview: Webview, id: String, new_tab: bool) {
    if let Some(wid) = owner_of(&webview) {
        open_bm(&app, &wid, &id, new_tab);
    }
}

/// 拖曳排序（書籤列最上層）
#[tauri::command]
pub fn web_bm_move(app: AppHandle, id: String, index: usize) {
    bm(|b| {
        if let Some(i) = b.bar.iter().position(|x| x.id == id) {
            let x = b.bar.remove(i);
            let j = index.min(b.bar.len());
            b.bar.insert(j, x);
        }
    });
    emit_bm(&app);
}

#[tauri::command]
pub fn web_bm_rename(app: AppHandle, id: String, title: String) {
    let title = title.trim().to_string();
    if title.is_empty() {
        return;
    }
    bm(|b| {
        if let Some(x) = bm_find_mut(&mut b.bar, &id).or_else(|| bm_find_mut(&mut b.other, &id)) {
            x.title = title;
            x.renamed = true;
        }
    });
    emit_bm(&app);
}

/// 記下刪掉的書籤，自動匯入時不要再加回來
fn remember_deleted(b: &mut Bookmarks, x: &Bm) {
    let key = if !x.src.is_empty() {
        x.src.clone()
    } else if x.url.is_empty() {
        format!("folder:{}", x.title)
    } else {
        x.url.clone()
    };
    if !b.ignored.contains(&key) {
        b.ignored.push(key);
    }
}

fn bm_delete(app: &AppHandle, id: &str) {
    bm(|b| {
        if let Some(x) = bm_remove(&mut b.bar, id).or_else(|| bm_remove(&mut b.other, id)) {
            remember_deleted(b, &x);
        }
    });
    emit_bm(app);
}

#[tauri::command]
pub fn web_bm_delete(app: AppHandle, id: String) {
    bm_delete(&app, &id);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clear_site_origins() {
        let o = site_origins("https://x.com/home");
        assert_eq!(o, ["https://x.com", "http://x.com", "https://www.x.com", "http://www.x.com"]);
        let o = site_origins("https://accounts.google.com/signin");
        assert!(o.contains(&"https://accounts.google.com".to_string()));
        assert!(o.contains(&"https://google.com".to_string()));
        assert!(o.contains(&"https://www.google.com".to_string()));
        let o = site_origins("https://shop.example.co.uk/");
        assert!(o.contains(&"https://example.co.uk".to_string()));
        assert!(!o.contains(&"https://co.uk".to_string()));
        assert_eq!(site_origins("http://127.0.0.1:8080/"), ["https://127.0.0.1", "http://127.0.0.1"]);
        assert!(site_origins("about:blank").is_empty());
    }

    #[test]
    fn chrome_import_and_merge() {
        let v: Value = serde_json::from_str(
            r#"{"roots":{"bookmark_bar":{"type":"folder","name":"書籤列","children":[
                {"type":"url","name":"YouTube","url":"https://www.youtube.com/"},
                {"type":"url","name":"小書籤","url":"javascript:alert(1)"},
                {"type":"folder","name":"楓之谷","children":[{"type":"url","name":"官網","url":"https://maplestory.beanfun.com/"}]}
            ]}}}"#,
        )
        .unwrap();
        let mut n = 0;
        let bar = from_chrome(&v["roots"]["bookmark_bar"], &mut n).unwrap().children;
        // javascript: 書籤不匯入
        assert_eq!(bar.len(), 2);
        assert_eq!(count_links(&bar), 2);
        let mut dst = Vec::new();
        assert_eq!(merge(&mut dst, bar.clone()), 2);
        // 再匯入一次：沒有新增
        let again = from_chrome(&v["roots"]["bookmark_bar"], &mut n).unwrap().children;
        assert_eq!(merge(&mut dst, again), 0);
        assert_eq!(dst.len(), 2);
        assert!(bm_url_id(&dst, "https://maplestory.beanfun.com").is_some());
        let fid = dst[1].id.clone();
        assert_eq!(bm_find(&dst, &fid).unwrap().title, "楓之谷");
        assert!(bm_remove(&mut dst, &fid).is_some());
        assert_eq!(count_links(&dst), 1);
    }

    #[test]
    fn account_bookmarks_file_and_ignored() {
        // 登入 Google 帳號的 Chrome 只有 AccountBookmarks、沒有 Bookmarks 也要讀得到
        let root = std::env::temp_dir().join(format!("web_bm_test_{}", std::process::id()));
        let dir = root.join("Default");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("AccountBookmarks"),
            r#"{"roots":{"bookmark_bar":{"type":"folder","name":"","children":[
                {"type":"url","name":"Gmail","url":"https://mail.google.com/"},
                {"type":"url","name":"YouTube","url":"https://www.youtube.com/"}]},
              "other":{"type":"folder","name":"","children":[{"type":"url","name":"X","url":"https://x.com/"}]}}}"#,
        )
        .unwrap();
        let mut n = 0;
        let (bar, other) = read_profile(&root, "Default", &mut n).unwrap();
        assert_eq!((bar.len(), other.len()), (2, 1));
        assert!(profile_mtime(&root, "Default") > 0);
        // 在這裡刪掉的不會再加回來
        let kept = drop_ignored(bar, &["https://www.youtube.com".to_string()]);
        assert_eq!(kept.len(), 1);
        assert_eq!(kept[0].title, "Gmail");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn chrome_sync_follows_renames_and_deletes() {
        let chrome = |items: &str| -> Vec<Bm> {
            let v: Value = serde_json::from_str(&format!(r#"{{"type":"folder","name":"","children":[{items}]}}"#)).unwrap();
            let mut n = 100;
            from_chrome(&v, &mut n).unwrap().children
        };
        let a = r#"{"type":"url","guid":"g1","name":"Gmail","url":"https://mail.google.com/"}"#;
        let b = r#"{"type":"url","guid":"g2","name":"YouTube","url":"https://www.youtube.com/"}"#;
        // 舊版匯入的（沒有 Chrome 編號）會被認領，不會重複
        let mut dst = vec![Bm { id: "b1".into(), title: "Gmail".into(), url: "https://mail.google.com/".into(), ..Bm::default() }];
        assert_eq!(sync_level(&mut dst, chrome(&format!("{a},{b}")), &[]), 1);
        assert_eq!(dst.len(), 2);
        assert_eq!(dst[0].id, "b1");
        // Chrome 改名 → 跟著改
        let a2 = a.replace("\"Gmail\"", "\"信箱\"");
        sync_level(&mut dst, chrome(&format!("{a2},{b}")), &[]);
        assert_eq!(dst[0].title, "信箱");
        // 在這裡改過名的 → 維持
        dst[1].title = "影片".into();
        dst[1].renamed = true;
        let b2 = b.replace("\"YouTube\"", "\"YT\"");
        sync_level(&mut dst, chrome(&format!("{a2},{b2}")), &[]);
        assert_eq!(dst[1].title, "影片");
        // Chrome 刪掉 → 這裡也拿掉；自己加的不動
        dst.push(Bm { id: "b9".into(), title: "自己加的".into(), url: "https://x.com/".into(), ..Bm::default() });
        sync_level(&mut dst, chrome(&b2), &[]);
        assert_eq!(dst.iter().map(|x| x.title.as_str()).collect::<Vec<_>>(), vec!["影片", "自己加的"]);
        // 在這裡刪掉的 → 不再加回
        let mut dst2 = Vec::new();
        assert_eq!(sync_level(&mut dst2, chrome(&format!("{a},{b}")), &["c:g1".to_string()]), 1);
        assert_eq!(dst2[0].title, "YouTube");
    }

    #[test]
    fn old_window_data_gets_a_tab() {
        let mut w: WebWin = serde_json::from_str(r#"{"id":"w1","url":"https://www.youtube.com/","title":"YouTube","open":true}"#).unwrap();
        fix_win(&mut w);
        assert_eq!(w.tabs.len(), 1);
        assert_eq!(w.tabs[0].url, "https://www.youtube.com/");
        assert_eq!(w.active, w.tabs[0].id);
        assert_eq!(w.next_tab, 1);
    }
}
