//! web 浮動視窗：置頂的小瀏覽器，邊玩遊戲邊看影片。
//!
//! 每個視窗 = 一個無框視窗：上面三排（分頁列、網址列、書籤列）是本機頁面 web.html，
//! 下面是外部網頁（每個分頁一個 webview，只顯示目前的分頁；其他分頁點到時才載入）。
//! - 視窗設成「不搶焦點」：滑鼠點影片、按暫停、拖進度時，鍵盤一直留在遊戲。
//! - 點到網頁或網址列的輸入框要打字時，才把鍵盤交給視窗；按 Enter、點別的地方或滑鼠移出視窗就還給原本的視窗（遊戲）。
//! - 書籤資料夾、右鍵選單用另一個置頂的小視窗顯示（可以超出 web 視窗的範圍）。
//! - 外部網頁不能呼叫程式的任何指令（沒有設定 remote 權限，Tauri 會直接拒絕）。

use crate::fg;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicIsize, Ordering};
use std::sync::{Mutex, MutexGuard, OnceLock};
use std::time::{Duration, Instant};
use tauri::webview::{NewWindowResponse, PageLoadEvent, WebviewBuilder};
use tauri::window::{Color, WindowBuilder};
use tauri::{
    AppHandle, Emitter, Manager, PhysicalPosition, PhysicalSize, Url, Webview, WebviewUrl,
    WebviewWindow, WebviewWindowBuilder, Window, WindowEvent,
};

pub const HOME: &str = "https://www.google.com/";
/// 上面三排的高度（邏輯像素，要跟 web.html 一致）
const TABS_H: f64 = 40.0;
const TOOL_H: f64 = 38.0;
const BMS_H: f64 = 30.0;
const BAR_H: f64 = TABS_H + TOOL_H + BMS_H;
/// 左、右、下留給拉大小的邊（邏輯像素）
const EDGE: f64 = 5.0;
/// 網頁裡有輸入框被選取時，標題最後面會加上兩個看不見的字元
const MARK: char = '\u{2063}';
const MENU_LABEL: &str = "webmenu";
/// 選單的尺寸（邏輯像素，要跟 web-menu.html 一致）
const MENU_ITEM_H: f64 = 30.0;
const MENU_SEP_H: f64 = 9.0;
const MENU_PAD: f64 = 6.0;
const MENU_MAX_H: f64 = 520.0;

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

/// 網頁載入完成後取得網站圖示的網址（只回傳網址，不做其他事）
const ICON_JS: &str = r#"(() => {
  let best = "", score = -1;
  for (const l of document.querySelectorAll("link[rel]")) {
    const rel = (l.getAttribute("rel") || "").toLowerCase();
    if (!rel.split(/\s+/).includes("icon") && !rel.includes("apple-touch-icon")) continue;
    if (!l.href) continue;
    const size = parseInt((l.getAttribute("sizes") || "").split(/x/i)[0], 10) || 16;
    const s = (rel.includes("apple") ? 0 : 1000) + (100 - Math.min(99, Math.abs(32 - size)));
    if (s > score) { score = s; best = l.href; }
  }
  return best || (location.origin + "/favicon.ico");
})()"#;

/* ---------------- 設定檔 ---------------- */

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Tab {
    pub id: String,
    pub url: String,
    pub title: String,
    pub icon: String,
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

/// 舊資料補齊：沒有分頁就用原本的網址開一個；目前分頁不存在就選第一個
fn fix_win(w: &mut WebWin) {
    if w.tabs.is_empty() {
        let url = if w.url.trim().is_empty() { HOME.to_string() } else { w.url.clone() };
        w.next_tab += 1;
        w.tabs.push(Tab { id: format!("t{}", w.next_tab), url, title: w.title.clone(), icon: String::new() });
    }
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
    f
}

fn save(path: &Path, file: &WebFile) {
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    if let Ok(s) = serde_json::to_string_pretty(file) {
        let tmp = path.with_extension("tmp");
        if std::fs::write(&tmp, s).is_ok() {
            let _ = std::fs::rename(&tmp, path);
        }
    }
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

#[derive(Serialize, Clone)]
pub struct BarState {
    id: String,
    tabs: Vec<Tab>,
    active: String,
    loading: Vec<String>,
    max: bool,
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
}

fn bar_state(wid: &str) -> Option<BarState> {
    let w = get(wid)?;
    let loading = rt(|m| m.get(wid).map(|r| r.loading.iter().cloned().collect()).unwrap_or_default());
    Some(BarState { id: w.id, tabs: w.tabs, active: w.active, loading, max: w.max })
}

fn bm_payload() -> BmPayload {
    bm_read(|b| BmPayload { bar: b.bar.clone(), other: b.other.clone(), icons: b.icons.clone() })
}

fn emit_state(app: &AppHandle, wid: &str) {
    if let Some(s) = bar_state(wid) {
        let _ = app.emit_to(bar_label(wid).as_str(), "web-state", s);
    }
}

fn emit_main(app: &AppHandle) {
    let _ = app.emit_to("main", "web-changed", ());
}

/// 書籤改了：通知每個 web 視窗與主程式
fn emit_bm(app: &AppHandle) {
    let p = bm_payload();
    for id in open_ids() {
        let _ = app.emit_to(bar_label(&id).as_str(), "web-bm", p.clone());
    }
    emit_main(app);
}

/* ---------------- 網址 ---------------- */

/// 網址列輸入的文字 → 網址；不像網址的就用 Google 搜尋
pub fn to_url(input: &str) -> Option<Url> {
    let s = input.trim();
    if s.is_empty() {
        return None;
    }
    if let Ok(u) = Url::parse(s) {
        if matches!(u.scheme(), "http" | "https") {
            return Some(u);
        }
    }
    let looks_like_host = !s.contains(char::is_whitespace)
        && (s.contains('.') || s.starts_with("localhost"))
        && !s.starts_with('.')
        && !s.ends_with('.');
    if looks_like_host {
        if let Ok(u) = Url::parse(&format!("https://{s}")) {
            return Some(u);
        }
    }
    let mut u = Url::parse("https://www.google.com/search").ok()?;
    u.query_pairs_mut().append_pair("q", s);
    Some(u)
}

fn web_url(u: &Url) -> bool {
    matches!(u.scheme(), "http" | "https")
}

fn parse_web(s: &str) -> Option<Url> {
    Url::parse(s).ok().filter(web_url)
}

/// 分頁要開的網址：空白 = 新分頁（空白頁）；擴充功能的頁面也可以
fn page_url(s: &str) -> Url {
    if s.trim().is_empty() {
        return Url::parse("about:blank").unwrap();
    }
    Url::parse(s)
        .ok()
        .filter(|u| web_url(u) || u.scheme() == "chrome-extension")
        .unwrap_or_else(|| Url::parse(HOME).unwrap())
}

/* ---------------- 視窗、分頁 ---------------- */

/// 上面三排的範圍（實體像素）
fn bar_rect(sf: f64, size: PhysicalSize<u32>) -> (PhysicalPosition<i32>, PhysicalSize<u32>) {
    (PhysicalPosition::new(0, 0), PhysicalSize::new(size.width.max(1), (BAR_H * sf).round() as u32))
}

/// 網頁的範圍（實體像素）：左、右、下留一點邊給拉大小；放到最大時不用留
fn page_rect(sf: f64, size: PhysicalSize<u32>, max: bool) -> (PhysicalPosition<i32>, PhysicalSize<u32>) {
    let edge = if max { 0 } else { (EDGE * sf).round() as u32 };
    let top = (BAR_H * sf).round() as u32;
    (
        PhysicalPosition::new(edge as i32, top as i32),
        PhysicalSize::new(size.width.saturating_sub(2 * edge).max(1), size.height.saturating_sub(top + edge).max(1)),
    )
}

fn bounds(pos: PhysicalPosition<i32>, size: PhysicalSize<u32>) -> tauri::Rect {
    tauri::Rect { position: pos.into(), size: size.into() }
}

/// 視窗大小改變：工具列和所有分頁的網頁一次放到正確位置（一個網頁只設定一次，不會中途變成奇怪的大小）
fn layout(app: &AppHandle, wid: &str, win: &Window, size: PhysicalSize<u32>) {
    let sf = win.scale_factor().unwrap_or(1.0).max(0.5);
    let max = win.is_maximized().unwrap_or(false);
    let full = rt(|m| m.get(wid).is_some_and(|r| r.full));
    if let Some(b) = app.get_webview(&bar_label(wid)) {
        let (p, s) = bar_rect(sf, size);
        let _ = b.set_bounds(bounds(p, s));
        let _ = if full { b.hide() } else { b.show() };
    }
    let (pos, sz) = if full { (PhysicalPosition::new(0, 0), size) } else { page_rect(sf, size, max) };
    for t in get(wid).map(|w| w.tabs).unwrap_or_default() {
        if let Some(p) = app.get_webview(&page_label(wid, &t.id)) {
            let _ = p.set_bounds(bounds(pos, sz));
        }
    }
}

/// web 視窗的網頁用自己的瀏覽器資料夾（登入狀態、擴充功能都放這裡）
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


/// 登入用的小視窗（網頁用 window.open 指定大小開的）：開成真的視窗，才能打字、登入完會自己關掉
fn popup_window(app: &AppHandle, url: &Url, features: tauri::webview::NewWindowFeatures) -> Option<WebviewWindow> {
    let n = POPUP_N.fetch_add(1, Ordering::SeqCst) + 1;
    let win = WebviewWindowBuilder::new(app, format!("webpop-{n}"), WebviewUrl::External("about:blank".parse().ok()?))
        .window_features(features)
        .title(url.host_str().unwrap_or("登入"))
        .focused(true)
        .on_document_title_changed(|w, title| {
            let _ = w.set_title(&title);
        })
        .build()
        .ok()?;
    // 遊戲在前景時點網頁不會切換前景，Windows 就不讓新開的小視窗變成前景（打字會跑到遊戲）：主動叫到前景
    #[cfg(windows)]
    if let Ok(h) = win.hwnd() {
        let h = h.0 as isize;
        let _ = app.run_on_main_thread(move || win32::activate(h));
    }
    Some(win)
}

fn page_builder(app: &AppHandle, wid: &str, tid: &str, url: Url, ext: Option<PathBuf>) -> WebviewBuilder<tauri::Wry> {
    let (a1, a2, a3) = (app.clone(), app.clone(), app.clone());
    let (w1, w2, w3) = (wid.to_string(), wid.to_string(), wid.to_string());
    let (t1, t2, t3) = (tid.to_string(), tid.to_string(), tid.to_string());
    // 按「+」開的空白分頁不要搶走焦點（網址列要能直接打字）
    let blank = url.as_str() == "about:blank";
    let mut b = WebviewBuilder::new(page_label(wid, tid), WebviewUrl::External(url))
        .data_directory(profile_dir())
        .browser_extensions_enabled(true)
        .focused(!blank);
    if let Some(dir) = ext {
        b = b.extensions_path(dir);
    }
    b.initialization_script_for_all_frames(TYPING_JS)
        // 讓網頁自己的拖放（例如上傳檔案、排序清單）可以正常使用
        .disable_drag_drop_handler()
        .on_page_load(move |wv, p| {
            on_page(&a1, &w1, &t1, &wv, p.url(), matches!(p.event(), PageLoadEvent::Started));
        })
        .on_document_title_changed(move |_wv, raw| on_title(&a2, &w2, &t2, raw))
        // 登入用的小視窗開成真的視窗；其他開新分頁的連結：在這個視窗開一個新分頁
        .on_new_window(move |u, features| {
            if features.size().is_some() {
                return match popup_window(&a3, &u, features) {
                    Some(window) => NewWindowResponse::Create { window },
                    None => NewWindowResponse::Allow,
                };
            }
            if web_url(&u) {
                let (app, wid, tid) = (a3.clone(), w3.clone(), t3.clone());
                std::thread::spawn(move || {
                    // 同一個網址已經開著：切過去就好，不要一直開新分頁
                    let same = get(&wid).and_then(|w| w.tabs.into_iter().find(|t| same_url(&t.url, u.as_str()))).map(|t| t.id);
                    match same {
                        Some(t) => {
                            let _ = select_tab(&app, &wid, &t);
                        }
                        None => {
                            let _ = new_tab(&app, &wid, Some(u), true, Some(&tid));
                        }
                    }
                });
            }
            NewWindowResponse::Deny
        })
}

fn on_page(app: &AppHandle, wid: &str, tid: &str, wv: &Webview, url: &Url, started: bool) {
    if web_url(url) {
        let s = url.to_string();
        edit(wid, |w| {
            if let Some(t) = w.tabs.iter_mut().find(|t| t.id == tid) {
                t.url = s.clone();
            }
            if w.active == tid {
                w.url = s;
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
    if !started {
        let (app2, w2, t2) = (app.clone(), wid.to_string(), tid.to_string());
        let _ = wv.eval_with_callback(ICON_JS, move |res| {
            if let Ok(icon) = serde_json::from_str::<String>(&res) {
                set_icon(&app2, &w2, &t2, icon);
            }
        });
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
            let ch = t.icon != icon;
            t.icon = icon.clone();
            (ch, t.url.clone())
        }
        None => (false, String::new()),
    })
    .unwrap_or((false, String::new()));
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
        let mut ch = false;
        if let Some(t) = w.tabs.iter_mut().find(|t| t.id == tid) {
            if t.title != title {
                t.title = title.clone();
                ch = true;
            }
        }
        if w.active == tid {
            w.title = title.clone();
        }
        ch
    })
    .unwrap_or(false);
    if changed {
        emit_state(app, wid);
        emit_main(app);
        // 工作列上顯示目前分頁的標題
        if get(wid).is_some_and(|w| w.active == tid) {
            if let Some(win) = app.get_window(&win_label(wid)) {
                let _ = win.set_title(if title.is_empty() { "瀏覽器" } else { &title });
            }
        }
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
    let ext = ext_pending();
    let had = ext.is_some();
    let made = match win.add_child(page_builder(app, wid, tid, url.clone(), ext), pos, size) {
        Ok(w) => {
            if had {
                ext_mark_loaded();
            }
            Ok(w)
        }
        Err(e) if had => {
            eprintln!("擴充功能載入失敗：{e}");
            EXT_BROKEN.store(true, Ordering::SeqCst);
            win.add_child(page_builder(app, wid, tid, url, None), pos, size).map_err(|e| format!("網頁建立失敗：{e}"))
        }
        Err(e) => Err(format!("網頁建立失敗：{e}")),
    }?;
    watch_fullscreen(app, wid, &made);
    Ok(())
}

/// 網頁按影片的全螢幕：整個視窗變全螢幕、網頁蓋滿；離開時還原
#[cfg(windows)]
fn watch_fullscreen(app: &AppHandle, wid: &str, wv: &Webview) {
    use webview2_com::ContainsFullScreenElementChangedEventHandler;
    let (app2, wid2) = (app.clone(), wid.to_string());
    let _ = wv.with_webview(move |pw| unsafe {
        let Ok(core) = pw.controller().CoreWebView2() else { return };
        let mut token = 0i64;
        let _ = core.add_ContainsFullScreenElementChanged(
            &ContainsFullScreenElementChangedEventHandler::create(Box::new(move |core, _| {
                let Some(core) = core else { return Ok(()) };
                let mut on = 0i32;
                if core.ContainsFullScreenElement(&mut on as *mut i32 as *mut _).is_ok() {
                    let (app3, wid3) = (app2.clone(), wid2.clone());
                    std::thread::spawn(move || set_fullscreen(&app3, &wid3, on != 0));
                }
                Ok(())
            })),
            &mut token,
        );
    });
}

#[cfg(not(windows))]
fn watch_fullscreen(_app: &AppHandle, _wid: &str, _wv: &Webview) {}

fn set_fullscreen(app: &AppHandle, wid: &str, on: bool) {
    let changed = rt(|m| m.get_mut(wid).map(|r| std::mem::replace(&mut r.full, on) != on)).unwrap_or(false);
    if !changed {
        return;
    }
    hide_menu_of(app, wid);
    close_ext_popup(app, wid);
    let Some(win) = app.get_window(&win_label(wid)) else { return };
    let _ = win.set_fullscreen(on);
    if let Ok(size) = win.inner_size() {
        layout(app, wid, &win, size);
    }
}

/// 建立分頁的網頁（要在主執行緒以外呼叫）
fn create_page(app: &AppHandle, win: &Window, wid: &str, tid: &str) -> Result<(), String> {
    let url = get(wid).and_then(|w| w.tabs.into_iter().find(|t| t.id == tid)).map(|t| t.url).unwrap_or_default();
    let url = page_url(&url);
    let sf = win.scale_factor().unwrap_or(1.0).max(0.5);
    let size = win.inner_size().unwrap_or(PhysicalSize::new(860, 560));
    let (pos, sz) = page_rect(sf, size, win.is_maximized().unwrap_or(false));
    add_page(app, win, wid, tid, url, pos, sz)
}

fn select_tab(app: &AppHandle, wid: &str, tid: &str) -> Result<(), String> {
    close_ext_popup(app, wid);
    let (prev, title) = edit(wid, |w| {
        let t = w.tabs.iter().find(|t| t.id == tid)?.clone();
        w.url = t.url;
        w.title = t.title.clone();
        Some((std::mem::replace(&mut w.active, tid.to_string()), t.title))
    })
    .flatten()
    .ok_or("找不到這個分頁")?;
    if let Some(win) = app.get_window(&win_label(wid)) {
        // 工作列上的標題跟著換成這個分頁的
        let _ = win.set_title(if title.is_empty() { "瀏覽器" } else { &title });
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
        }
    });
    emit_state(app, wid);
    emit_main(app);
    Ok(())
}

/// 開新分頁：after = 放在哪個分頁後面（沒有就放最後）
fn new_tab(app: &AppHandle, wid: &str, url: Option<Url>, activate: bool, after: Option<&str>) -> Result<String, String> {
    // 按「+」開的新分頁是空白頁，網址列等你輸入
    let url = url.map(|u| u.to_string()).unwrap_or_default();
    let tid = edit(wid, |w| {
        w.next_tab += 1;
        let tid = format!("t{}", w.next_tab);
        let pos = after.and_then(|a| w.tabs.iter().position(|t| t.id == a)).map(|i| i + 1).unwrap_or(w.tabs.len());
        w.tabs.insert(pos, Tab { id: tid.clone(), url, title: String::new(), icon: String::new() });
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

fn close_tab(app: &AppHandle, wid: &str, tid: &str) -> Result<(), String> {
    enum After {
        Last,
        Select(String),
        Stay,
    }
    let after = edit(wid, |w| {
        let i = w.tabs.iter().position(|t| t.id == tid)?;
        w.tabs.remove(i);
        if w.tabs.is_empty() {
            return Some(After::Last);
        }
        if w.active == tid {
            return Some(After::Select(w.tabs[i.min(w.tabs.len() - 1)].id.clone()));
        }
        Some(After::Stay)
    })
    .flatten();
    let Some(after) = after else { return Ok(()) };
    if let Some(p) = app.get_webview(&page_label(wid, tid)) {
        let _ = p.close();
    }
    rt(|m| {
        if let Some(r) = m.get_mut(wid) {
            r.loading.remove(tid);
        }
    });
    match after {
        // 最後一個分頁關掉：整個視窗關閉（下次打開是首頁）
        After::Last => close_window(app, wid),
        After::Select(n) => select_tab(app, wid, &n)?,
        After::Stay => {
            emit_state(app, wid);
            emit_main(app);
        }
    }
    Ok(())
}

/// 打開視窗。reveal = 縮小的也叫出來（從 web 工具打開時）；程式剛啟動自動打開時維持縮小
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
        .title(if cfg.title.is_empty() { "瀏覽器".to_string() } else { cfg.title.clone() })
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
        .background_color(Color(30, 34, 43, 255));
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
    rt(|m| m.insert(id.to_string(), Rt { hwnd, ..Rt::default() }));
    install_subclass(app, hwnd, (x, y, w, h));

    let sf = win.scale_factor().unwrap_or(sf0).max(0.5);
    let size = win.inner_size().unwrap_or(PhysicalSize::new(w, h));
    // 上面：分頁列、網址列、書籤列（本機頁面）
    let bar = WebviewBuilder::new(bar_label(id), WebviewUrl::App("web.html".into())).disable_drag_drop_handler();
    let (bp, bs) = bar_rect(sf, size);
    win.add_child(bar, bp, bs).map_err(|e| format!("工具列建立失敗：{e}"))?;
    // 下面：目前分頁的網頁（其他分頁點到時才載入）
    let url = page_url(&cfg.url);
    let (ppos, psz) = page_rect(sf, size, false);
    add_page(app, &win, id, &cfg.active, url, ppos, psz)?;

    let (app2, id2, win2) = (app.clone(), id.to_string(), win.clone());
    win.on_window_event(move |ev| match ev {
        // 放到最大（按鈕、按兩下、拖到螢幕頂端）時不記大小，還原後回到原本的大小
        WindowEvent::Resized(s) if s.width > 0 && s.height > 0 => {
            close_ext_popup(&app2, &id2);
            layout(&app2, &id2, &win2, *s);
            let max = win2.is_maximized().unwrap_or(false);
            let (w, h) = (s.width, s.height);
            let changed = edit(&id2, |c| {
                if !max {
                    (c.w, c.h) = (w, h);
                }
                c.min = false;
                std::mem::replace(&mut c.max, max) != max
            });
            if changed == Some(true) {
                emit_state(&app2, &id2);
            }
        }
        // 縮小到工作列（大小變成 0）
        WindowEvent::Resized(_) => {
            let min = win2.is_minimized().unwrap_or(false);
            if edit(&id2, |c| std::mem::replace(&mut c.min, min) != min) == Some(true) {
                emit_main(&app2);
            }
        }
        WindowEvent::Moved(p) if p.x > -10000 && p.y > -10000 => {
            close_ext_popup(&app2, &id2);
            let (x, y) = (p.x, p.y);
            let max = win2.is_maximized().unwrap_or(false);
            edit(&id2, |c| {
                if !max && !c.max {
                    (c.x, c.y) = (x, y);
                }
            });
        }
        WindowEvent::Destroyed => {
            let hwnd = rt(|m| m.remove(&id2).map(|r| r.hwnd).unwrap_or(0));
            lock(&TYPING_HWND).retain(|h| *h != hwnd);
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
    hide_menu_of(app, id);
    edit(id, |w| {
        w.open = false;
        w.min = false;
    });
    if let Some(win) = app.get_window(&win_label(id)) {
        let _ = win.destroy();
    }
    emit_main(app);
}

/// 放到最大／還原（用 Windows 本身的放到最大，所以拖到螢幕頂端也會放大、拖離頂端會還原）
fn toggle_max(app: &AppHandle, wid: &str) {
    let Some(win) = app.get_window(&win_label(wid)) else { return };
    let _ = if win.is_maximized().unwrap_or(false) { win.unmaximize() } else { win.maximize() };
    let _ = app;
}

fn active_page(app: &AppHandle, wid: &str) -> Option<Webview> {
    let w = get(wid)?;
    app.get_webview(&page_label(wid, &w.active))
}

fn open_bm(app: &AppHandle, wid: &str, id: &str, new: bool) {
    let Some(url) = bm_read(|b| bm_find_any(b, id).map(|x| x.url.clone())).and_then(|u| parse_web(&u)) else { return };
    if new {
        let _ = new_tab(app, wid, Some(url), false, None);
    } else if let Some(p) = active_page(app, wid) {
        let _ = p.navigate(url);
    }
}

/* ---------------- 選單（另一個置頂小視窗） ---------------- */

#[derive(Serialize, Clone, Default)]
pub struct MenuItem {
    /// link / folder / back / act / sep / empty
    kind: &'static str,
    title: String,
    /// link：網站圖示網址；act：圖示名稱
    icon: String,
    /// link：網址；folder：裡面有幾個書籤
    sub: String,
    danger: bool,
}

#[derive(Serialize, Clone)]
pub struct MenuPayload {
    items: Vec<MenuItem>,
}

#[derive(Clone)]
enum Act {
    Link(String),
    Folder(String),
    Back,
    NewTab(String),
    Rename(String),
    Delete(String),
    TabReload(String),
    TabClose(String),
    TabOthers(String),
    TabRight(String),
    /// 擴充功能：點了開它的選單頁面（沒有就開選項）
    Ext(String),
    ExtPage(String),
    ExtToggle(String, bool),
    ExtSync,
    Nothing,
}

struct MenuSt {
    owner: String,
    key: String,
    items: Vec<MenuItem>,
    acts: Vec<Act>,
    stack: Vec<(Vec<MenuItem>, Vec<Act>)>,
    /// 選單要貼著的範圍（螢幕實體像素：左、上、右、下）
    anchor: (i32, i32, i32, i32),
    width: f64,
    sf: f64,
    /// 選單實際的範圍
    rect: (i32, i32, i32, i32),
    visible: bool,
    shown_at: Instant,
}

static MENU: Mutex<Option<MenuSt>> = Mutex::new(None);
/// 最近一次因為點到外面而關掉的選單（同一個按鈕再點一下＝關掉，不要又打開）
static MENU_CLOSED: Mutex<Option<(String, Instant)>> = Mutex::new(None);
static MENU_HWND: AtomicIsize = AtomicIsize::new(0);

fn item(kind: &'static str, title: impl Into<String>, icon: &str) -> MenuItem {
    MenuItem { kind, title: title.into(), icon: icon.to_string(), ..MenuItem::default() }
}

fn bm_items(list: &[Bm], icons: &HashMap<String, String>) -> (Vec<MenuItem>, Vec<Act>) {
    if list.is_empty() {
        return (vec![item("empty", "（沒有書籤）", "")], vec![Act::Nothing]);
    }
    let mut items = Vec::new();
    let mut acts = Vec::new();
    for b in list {
        if b.url.is_empty() {
            let mut it = item("folder", b.title.clone(), "");
            it.sub = count_links(&b.children).to_string();
            items.push(it);
            acts.push(Act::Folder(b.id.clone()));
        } else {
            let icon = host_of(&b.url).and_then(|h| icons.get(&h).cloned()).unwrap_or_default();
            let title = if b.title.trim().is_empty() { b.url.clone() } else { b.title.clone() };
            let mut it = item("link", title, &icon);
            it.sub = b.url.clone();
            items.push(it);
            acts.push(Act::Link(b.id.clone()));
        }
    }
    (items, acts)
}

fn bm_ctx(b: &Bm) -> (Vec<MenuItem>, Vec<Act>) {
    let id = b.id.clone();
    if b.url.is_empty() {
        let mut del = item("act", format!("刪除資料夾（含 {} 個書籤）", count_links(&b.children)), "delete");
        del.danger = true;
        (vec![item("act", "改名", "rename"), item("sep", "", ""), del], vec![Act::Rename(id.clone()), Act::Nothing, Act::Delete(id)])
    } else {
        let mut del = item("act", "刪除", "delete");
        del.danger = true;
        (
            vec![item("act", "在新分頁開啟", "newtab"), item("sep", "", ""), item("act", "改名", "rename"), del],
            vec![Act::NewTab(id.clone()), Act::Nothing, Act::Rename(id.clone()), Act::Delete(id)],
        )
    }
}

fn ext_items() -> (Vec<MenuItem>, Vec<Act>) {
    let list = ext_list();
    let mut items = vec![item("head", "擴充功能", "")];
    let mut acts = vec![Act::Nothing];
    if list.is_empty() {
        items.push(item("empty", "（沒有擴充功能）", ""));
        acts.push(Act::Nothing);
    }
    for e in &list {
        let mut it = item("ext", e.name.clone(), &e.icon);
        it.sub = if e.off { "已停用".into() } else if e.popup.is_empty() && e.options.is_empty() { "自動執行".into() } else { String::new() };
        items.push(it);
        acts.push(Act::Ext(e.id.clone()));
    }
    items.push(item("sep", "", ""));
    acts.push(Act::Nothing);
    items.push(item("act", "從 Chrome 重新同步", "reload"));
    acts.push(Act::ExtSync);
    (items, acts)
}

/// 擴充功能的右鍵選單：開啟選單頁面、選項、停用／啟用
fn ext_ctx(id: &str) -> Option<(Vec<MenuItem>, Vec<Act>)> {
    let e = ext_list().into_iter().find(|e| e.id == id)?;
    let mut items = Vec::new();
    let mut acts = Vec::new();
    if !e.popup.is_empty() {
        items.push(item("act", "開啟擴充功能選單", "newtab"));
        acts.push(Act::ExtPage(e.popup.clone()));
    }
    if !e.options.is_empty() {
        items.push(item("act", "選項", "rename"));
        acts.push(Act::ExtPage(e.options.clone()));
    }
    if !items.is_empty() {
        items.push(item("sep", "", ""));
        acts.push(Act::Nothing);
    }
    if e.off {
        items.push(item("act", "啟用（重新開啟視窗後生效）", "reload"));
        acts.push(Act::ExtToggle(e.id, false));
    } else {
        let mut it = item("act", "停用（重新開啟視窗後生效）", "close");
        it.danger = true;
        items.push(it);
        acts.push(Act::ExtToggle(e.id, true));
    }
    Some((items, acts))
}

fn tab_ctx(tid: &str) -> (Vec<MenuItem>, Vec<Act>) {
    let t = tid.to_string();
    (
        vec![
            item("act", "重新整理", "reload"),
            item("sep", "", ""),
            item("act", "關閉分頁", "close"),
            item("act", "關閉其他分頁", "close"),
            item("act", "關閉右側分頁", "close"),
        ],
        vec![Act::TabReload(t.clone()), Act::Nothing, Act::TabClose(t.clone()), Act::TabOthers(t.clone()), Act::TabRight(t)],
    )
}

fn menu_height(items: &[MenuItem]) -> f64 {
    let h: f64 = items.iter().map(|i| if i.kind == "sep" { MENU_SEP_H } else { MENU_ITEM_H }).sum();
    (h + MENU_PAD * 2.0).min(MENU_MAX_H)
}

fn menu_window(app: &AppHandle) -> Result<WebviewWindow, String> {
    if let Some(w) = app.get_webview_window(MENU_LABEL) {
        return Ok(w);
    }
    let w = WebviewWindowBuilder::new(app, MENU_LABEL, WebviewUrl::App("web-menu.html".into()))
        .title("web 選單")
        .decorations(false)
        .always_on_top(true)
        .skip_taskbar(true)
        .resizable(false)
        .maximizable(false)
        .minimizable(false)
        .focused(false)
        .focusable(false)
        .visible(false)
        .shadow(true)
        .background_color(Color(38, 43, 54, 255))
        .inner_size(280.0, 120.0)
        .build()
        .map_err(|e| format!("選單建立失敗：{e}"))?;
    #[cfg(windows)]
    if let Ok(h) = w.hwnd() {
        let h = h.0 as isize;
        MENU_HWND.store(h, Ordering::SeqCst);
        let _ = app.run_on_main_thread(move || win32::subclass_menu(h));
    }
    Ok(w)
}

/// 顯示選單（位置貼著 anchor，超出螢幕就往上或往左擺）
fn show_menu(app: &AppHandle, mut s: MenuSt) -> Result<(), String> {
    let win = menu_window(app)?;
    let pw = (s.width * s.sf).round() as i32;
    let mut ph = (menu_height(&s.items) * s.sf).round() as i32;
    let (l, t, r, b) = s.anchor;
    let (mut x, mut y) = (l, b);
    let wa = app
        .get_window(&win_label(&s.owner))
        .and_then(|o| o.monitor_from_point(l as f64 + 1.0, t as f64 + 1.0).ok().flatten())
        .map(|m| *m.work_area());
    if let Some(wa) = wa {
        let (wl, wt) = (wa.position.x, wa.position.y);
        let (wr, wb) = (wl + wa.size.width as i32, wt + wa.size.height as i32);
        ph = ph.min((wb - wt - 8).max(60));
        if x + pw > wr {
            x = if r > l { r - pw } else { wr - pw };
        }
        x = x.max(wl);
        if y + ph > wb {
            y = if t - ph >= wt { t - ph } else { (wb - ph).max(wt) };
        }
    }
    s.rect = (x, y, x + pw, y + ph);
    s.visible = true;
    s.shown_at = Instant::now();
    let payload = MenuPayload { items: s.items.clone() };
    *lock(&MENU) = Some(s);
    let _ = app.emit_to(MENU_LABEL, "web-menu-items", payload);
    // 等選單內容換好再顯示，避免先閃一下舊的內容
    std::thread::sleep(Duration::from_millis(40));
    #[cfg(windows)]
    {
        let h = MENU_HWND.load(Ordering::SeqCst);
        if h != 0 {
            win32::show_at(h, x, y, pw, ph);
            // 選單換到縮放比例不同的螢幕時，Windows 會自動放大縮小一次：再設回正確的大小
            std::thread::sleep(Duration::from_millis(60));
            if lock(&MENU).as_ref().is_some_and(|s| s.visible && s.rect == (x, y, x + pw, y + ph)) {
                win32::show_at(h, x, y, pw, ph);
            }
            return Ok(());
        }
    }
    let _ = win.set_position(PhysicalPosition::new(x, y));
    let _ = win.set_size(PhysicalSize::new(pw as u32, ph as u32));
    let _ = win.show();
    Ok(())
}

fn hide_menu(app: &AppHandle, outside: bool) {
    let closed = {
        let mut m = lock(&MENU);
        match m.as_mut() {
            Some(s) if s.visible => {
                s.visible = false;
                Some((s.owner.clone(), s.key.clone()))
            }
            _ => None,
        }
    };
    let Some((owner, key)) = closed else { return };
    if outside {
        *lock(&MENU_CLOSED) = Some((key, Instant::now()));
    }
    #[cfg(windows)]
    {
        let h = MENU_HWND.load(Ordering::SeqCst);
        if h != 0 {
            win32::show(h, false);
        }
    }
    #[cfg(not(windows))]
    if let Some(w) = app.get_webview_window(MENU_LABEL) {
        let _ = w.hide();
    }
    let _ = app.emit_to(bar_label(&owner).as_str(), "web-menu-closed", owner.clone());
}

fn hide_menu_of(app: &AppHandle, wid: &str) {
    let mine = lock(&MENU).as_ref().is_some_and(|s| s.visible && s.owner == wid);
    if mine {
        hide_menu(app, false);
    }
}

/// 選單換成另一層（資料夾、右鍵選單），可以按「返回上一層」回來
fn push_menu(app: &AppHandle, items: Vec<MenuItem>, acts: Vec<Act>) {
    let Some(mut s) = lock(&MENU).take() else { return };
    let old_items = std::mem::take(&mut s.items);
    let old_acts = std::mem::take(&mut s.acts);
    s.stack.push((old_items, old_acts));
    s.items = std::iter::once(item("back", "返回上一層", "")).chain(items).collect();
    s.acts = std::iter::once(Act::Back).chain(acts).collect();
    s.width = s.width.max(220.0);
    let _ = show_menu(app, s);
}

fn pop_menu(app: &AppHandle) {
    let Some(mut s) = lock(&MENU).take() else { return };
    if let Some((items, acts)) = s.stack.pop() {
        s.items = items;
        s.acts = acts;
    }
    let _ = show_menu(app, s);
}

/* ---------------- Windows：攔截訊息、焦點、樣式 ---------------- */

#[cfg(windows)]
mod win32 {
    use super::{lock, TYPING_HWND};
    use std::sync::atomic::{AtomicIsize, Ordering};
    use windows_sys::Win32::UI::Accessibility::{SetWinEventHook, HWINEVENTHOOK};
    use std::sync::Mutex;
    use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
    use windows_sys::Win32::System::Threading::{AttachThreadInput, GetCurrentProcessId, GetCurrentThreadId};
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{GetAsyncKeyState, VK_LBUTTON, VK_MBUTTON, VK_RBUTTON};
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

    /// 選單：放到指定位置、蓋在所有視窗最上面、顯示（不搶焦點）
    pub fn show_at(h: isize, x: i32, y: i32, w: i32, hh: i32) {
        unsafe { SetWindowPos(h as HWND, HWND_TOPMOST, x, y, w, hh, SWP_NOACTIVATE | SWP_SHOWWINDOW) };
    }
}

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
    let h = h.min(size.height as i32 - (BAR_H * sf) as i32 - 8).max(100);
    let x = (anchor.2 - origin.x - w).max(4).min(size.width as i32 - w - 4);
    let y = (anchor.3 - origin.y + 4).max(0).min(size.height as i32 - h);
    let b = WebviewBuilder::new(ext_popup_label(wid), WebviewUrl::External(url))
        .data_directory(profile_dir())
        .browser_extensions_enabled(true)
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
    if let Some(dir) = path.parent() {
        let _ = WEB_DIR.set(dir.to_path_buf());
    }
    #[cfg(windows)]
    let _ = app.run_on_main_thread(win32::hook_foreground);
    let file = load(&path);
    let reopen: Vec<String> = file.wins.iter().filter(|w| w.open).map(|w| w.id.clone()).collect();
    let _ = STORE.set(Mutex::new(Store { file, path, dirty: false }));
    let app1 = app.clone();
    std::thread::spawn(move || {
        // 上次關閉程式時開著的視窗：重新打開（先從 Chrome 同步擴充功能）
        let _ = sync_extensions();
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
            // 自動從 Chrome 匯入：有視窗開著時每 5 秒看一次 Chrome 的書籤檔有沒有變動
            if any_open && last_sync.is_none_or(|t| t.elapsed() >= Duration::from_secs(5)) {
                last_sync = Some(Instant::now());
                if bm_read(|b| b.sync) {
                    let _ = import_chrome(&app1, false);
                }
            }
            let to_save = {
                let mut s = st();
                if s.dirty && last_save.elapsed() >= Duration::from_secs(1) {
                    s.dirty = false;
                    last_save = Instant::now();
                    Some((s.path.clone(), s.file.clone()))
                } else {
                    None
                }
            };
            if let Some((p, f)) = to_save {
                save(&p, &f);
            }
        }
    });

    // 選單開著時：點到選單外面就關掉
    #[cfg(windows)]
    {
        let app2 = app.clone();
        std::thread::spawn(move || {
            let mut prev_down = true;
            let mut seen: Option<Instant> = None;
            loop {
                let cur = lock(&MENU).as_ref().filter(|s| s.visible).map(|s| (s.rect, s.shown_at));
                let pop = lock(&EXT_POPUP).clone();
                if cur.is_none() && pop.is_none() {
                    seen = None;
                    std::thread::sleep(Duration::from_millis(300));
                    continue;
                }
                let down = win32::mouse_down();
                if let Some((rect, shown)) = cur {
                    if seen != Some(shown) {
                        // 剛打開：打開它的那一下不算
                        seen = Some(shown);
                    } else if down && !prev_down {
                        let (x, y) = win32::cursor();
                        if x < rect.0 || x >= rect.2 || y < rect.1 || y >= rect.3 {
                            hide_menu(&app2, true);
                        }
                    }
                }
                // 擴充功能的小視窗：點外面就關
                if let Some((wid, rect)) = pop {
                    if down && !prev_down {
                        let (x, y) = win32::cursor();
                        if x < rect.0 || x >= rect.2 || y < rect.1 || y >= rect.3 {
                            close_ext_popup(&app2, &wid);
                        }
                    }
                }
                prev_down = down;
                std::thread::sleep(Duration::from_millis(20));
            }
        });
    }
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
                        hide_menu_of(app, &id);
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
    st().file.clone()
}

#[tauri::command]
pub async fn web_add(app: AppHandle) -> Result<WebWin, String> {
    let w = {
        let mut s = st();
        s.file.next_id += 1;
        let n = s.file.next_id;
        // 新視窗稍微錯開，不要完全疊在一起
        let k = (s.file.wins.len() % 6) as i32;
        let mut w = WebWin { id: format!("w{n}"), x: 200 + k * 40, y: 150 + k * 40, ..WebWin::default() };
        fix_win(&mut w);
        s.file.wins.push(w.clone());
        s.dirty = true;
        w
    };
    open_window(&app, &w.id, true)?;
    Ok(get(&w.id).unwrap_or(w))
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
    let (sync, chosen) = bm_read(|b| (b.sync, b.profile.clone()));
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

/// 開關自動匯入
#[tauri::command]
pub async fn web_chrome_sync(app: AppHandle, on: bool) {
    bm(|b| b.sync = on);
    if on {
        let _ = import_chrome(&app, true);
    }
    emit_main(&app);
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
        pending: String::new(),
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
    let _g = lock(&EXT_LOCK);
    let base = ext_dir();
    std::fs::create_dir_all(&base).map_err(|e| e.to_string())?;
    let mut st = ext_state();
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

#[tauri::command]
pub async fn web_ext_toggle(id: String, on: bool) -> Result<ExtStatus, String> {
    ext_set_off(&id, !on)?;
    Ok(ExtStatus { list: ext_list(), broken: EXT_BROKEN.load(Ordering::SeqCst) })
}

#[tauri::command]
pub fn web_ext_list() -> ExtStatus {
    ExtStatus { list: ext_list(), broken: EXT_BROKEN.load(Ordering::SeqCst) }
}

/// 重新從 Chrome 同步擴充功能（新開的分頁、視窗才會套用）
#[tauri::command]
pub async fn web_ext_sync() -> Result<ExtStatus, String> {
    let list = sync_extensions()?;
    EXT_BROKEN.store(false, Ordering::SeqCst);
    Ok(ExtStatus { list, broken: false })
}

/* ---------------- 指令：web 視窗上面的三排 ---------------- */

#[tauri::command]
pub fn web_bar_init(webview: Webview) -> Option<BarInit> {
    let id = id_of(&webview)?;
    Some(BarInit { state: bar_state(&id)?, bm: bm_payload() })
}

#[tauri::command]
pub async fn web_nav(app: AppHandle, webview: Webview, action: String, input: Option<String>) -> Result<(), String> {
    let id = id_of(&webview).ok_or("不是 web 視窗")?;
    let page = active_page(&app, &id).ok_or("網頁不存在")?;
    let r = match action.as_str() {
        "back" => page.eval("history.back()"),
        "forward" => page.eval("history.forward()"),
        "reload" => page.reload(),
        "stop" => page.eval("window.stop()"),
        "home" => page.navigate(Url::parse(HOME).unwrap()),
        "go" => match input.as_deref().and_then(to_url) {
            Some(u) => page.navigate(u),
            None => return Ok(()),
        },
        _ => return Ok(()),
    };
    r.map_err(|e| e.to_string())
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
pub async fn web_tab_new(app: AppHandle, webview: Webview) -> Result<(), String> {
    let id = id_of(&webview).ok_or("不是 web 視窗")?;
    new_tab(&app, &id, None, true, None).map(|_| ())
}

#[tauri::command]
pub async fn web_tab_select(app: AppHandle, webview: Webview, tab: String) -> Result<(), String> {
    let id = id_of(&webview).ok_or("不是 web 視窗")?;
    select_tab(&app, &id, &tab)
}

/// 拖曳分頁排順序
#[tauri::command]
pub fn web_tab_move(app: AppHandle, webview: Webview, tab: String, index: usize) {
    let Some(id) = id_of(&webview) else { return };
    edit(&id, |w| {
        if let Some(i) = w.tabs.iter().position(|t| t.id == tab) {
            let t = w.tabs.remove(i);
            let j = index.min(w.tabs.len());
            w.tabs.insert(j, t);
        }
    });
    emit_state(&app, &id);
}

#[tauri::command]
pub async fn web_tab_close(app: AppHandle, webview: Webview, tab: String) -> Result<(), String> {
    let id = id_of(&webview).ok_or("不是 web 視窗")?;
    close_tab(&app, &id, &tab)
}

/// 右上角：縮小（藏起來）、放到最大／還原、關閉
#[tauri::command]
pub async fn web_win(app: AppHandle, webview: Webview, action: String) {
    let Some(id) = id_of(&webview) else { return };
    match action.as_str() {
        "min" => {
            hide_menu_of(&app, &id);
            if let Some(win) = app.get_window(&win_label(&id)) {
                let _ = win.minimize();
            }
        }
        "max" => toggle_max(&app, &id),
        "close" => close_window(&app, &id),
        _ => {}
    }
}

/// 星號：目前的網頁加入書籤列，已經加過就移除。回傳現在是不是書籤
#[tauri::command]
pub async fn web_bm_toggle(app: AppHandle, webview: Webview) -> Result<bool, String> {
    let id = id_of(&webview).ok_or("不是 web 視窗")?;
    let w = get(&id).ok_or("找不到這個視窗")?;
    let tab = w.tabs.into_iter().find(|t| t.id == w.active).ok_or("找不到這個分頁")?;
    if parse_web(&tab.url).is_none() {
        return Ok(false);
    }
    let on = bm(|b| {
        if let Some(bid) = bm_url_id(&b.bar, &tab.url).or_else(|| bm_url_id(&b.other, &tab.url)) {
            if let Some(x) = bm_remove(&mut b.bar, &bid).or_else(|| bm_remove(&mut b.other, &bid)) {
                remember_deleted(b, &x);
            }
            return false;
        }
        b.ignored.retain(|i| !same_url(i, &tab.url));
        b.next_id += 1;
        let host = host_of(&tab.url).unwrap_or_default();
        let title = if tab.title.trim().is_empty() { host.clone() } else { tab.title.clone() };
        b.bar.push(Bm { id: format!("b{}", b.next_id), title, url: tab.url.clone(), ..Bm::default() });
        if !tab.icon.is_empty() && !host.is_empty() {
            b.icons.insert(host, tab.icon.clone());
        }
        true
    });
    emit_bm(&app);
    Ok(on)
}

#[tauri::command]
pub async fn web_bm_open(app: AppHandle, webview: Webview, id: String, new_tab: bool) {
    if let Some(wid) = id_of(&webview) {
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

/// 打開選單：folder（書籤資料夾）、other（所有書籤）、overflow（放不下的書籤）、bm（書籤右鍵）、tab（分頁右鍵）。
/// rect = 按鈕在工具列裡的位置（左、上、右、下）；右鍵選單出現在滑鼠的位置
#[tauri::command]
pub async fn web_menu_open(
    app: AppHandle,
    webview: Webview,
    kind: String,
    key: String,
    target: Option<String>,
    ids: Option<Vec<String>>,
    rect: Option<[f64; 4]>,
) -> Result<(), String> {
    let wid = id_of(&webview).ok_or("不是 web 視窗")?;
    // 同一個按鈕再點一下：關掉
    let same_open = lock(&MENU).as_ref().is_some_and(|s| s.visible && s.key == key);
    if same_open {
        hide_menu(&app, false);
        return Ok(());
    }
    let just_closed = lock(&MENU_CLOSED).as_ref().is_some_and(|(k, t)| *k == key && t.elapsed() < Duration::from_millis(400));
    if just_closed {
        return Ok(());
    }
    let target = target.unwrap_or_default();
    let built = match kind.as_str() {
        "folder" => bm_read(|b| bm_find_any(b, &target).map(|f| bm_items(&f.children, &b.icons))).map(|x| (x, 300.0)),
        "other" => Some((bm_read(|b| bm_items(&b.other, &b.icons)), 300.0)),
        "overflow" => {
            let ids = ids.unwrap_or_default();
            Some((
                bm_read(|b| {
                    let list: Vec<Bm> = b.bar.iter().filter(|x| ids.contains(&x.id)).cloned().collect();
                    bm_items(&list, &b.icons)
                }),
                300.0,
            ))
        }
        "bm" => bm_read(|b| bm_find_any(b, &target).map(bm_ctx)).map(|x| (x, 220.0)),
        "tab" => Some((tab_ctx(&target), 220.0)),
        "ext" => Some((ext_items(), 300.0)),
        _ => None,
    };
    let Some(((items, acts), width)) = built else { return Ok(()) };
    let win = app.get_window(&win_label(&wid)).ok_or("找不到這個視窗")?;
    let sf = win.scale_factor().unwrap_or(1.0).max(0.5);
    let anchor = match (kind.as_str(), rect) {
        ("folder" | "other" | "overflow" | "ext", Some([l, t, r, b])) => {
            let p = win.inner_position().map_err(|e| e.to_string())?;
            let f = |v: f64| (v * sf).round() as i32;
            (p.x + f(l), p.y + f(t), p.x + f(r), p.y + f(b))
        }
        _ => {
            let c = win.cursor_position().map_err(|e| e.to_string())?;
            let (x, y) = (c.x.round() as i32, c.y.round() as i32);
            (x, y, x, y)
        }
    };
    show_menu(
        &app,
        MenuSt {
            owner: wid,
            key,
            items,
            acts,
            stack: Vec::new(),
            anchor,
            width,
            sf,
            rect: (0, 0, 0, 0),
            visible: false,
            shown_at: Instant::now(),
        },
    )
}

#[tauri::command]
pub fn web_menu_init() -> MenuPayload {
    MenuPayload { items: lock(&MENU).as_ref().map(|s| s.items.clone()).unwrap_or_default() }
}

/// 選單裡點了第幾項：button 0 左鍵、1 中鍵（書籤開在新分頁）、2 右鍵（書籤的右鍵選單）
#[tauri::command]
pub async fn web_menu_pick(app: AppHandle, index: usize, button: u8) {
    let picked = lock(&MENU).as_ref().filter(|s| s.visible).map(|s| (s.owner.clone(), s.acts.get(index).cloned(), s.anchor));
    let Some((owner, Some(act), anchor)) = picked else { return };
    let only_right = matches!(act, Act::TabRight(_));
    if button == 2 {
        if let Act::Link(id) | Act::Folder(id) = &act {
            if let Some((items, acts)) = bm_read(|b| bm_find_any(b, id).map(bm_ctx)) {
                push_menu(&app, items, acts);
            }
        }
        if let Act::Ext(id) = &act {
            if let Some((items, acts)) = ext_ctx(id) {
                push_menu(&app, items, acts);
            }
        }
        return;
    }
    match act {
        Act::Link(id) => {
            hide_menu(&app, false);
            open_bm(&app, &owner, &id, button == 1);
        }
        Act::Folder(id) => {
            if let Some((items, acts)) = bm_read(|b| bm_find_any(b, &id).map(|f| bm_items(&f.children, &b.icons))) {
                push_menu(&app, items, acts);
            }
        }
        Act::Back => pop_menu(&app),
        Act::NewTab(id) => {
            hide_menu(&app, false);
            if let Some(url) = bm_read(|b| bm_find_any(b, &id).map(|x| x.url.clone())).and_then(|u| parse_web(&u)) {
                let _ = new_tab(&app, &owner, Some(url), true, None);
            }
        }
        Act::Rename(id) => {
            hide_menu(&app, false);
            let _ = app.emit_to(bar_label(&owner).as_str(), "web-rename", serde_json::json!({ "id": owner, "bm": id }));
        }
        Act::Delete(id) => {
            hide_menu(&app, false);
            bm_delete(&app, &id);
        }
        Act::TabReload(t) => {
            hide_menu(&app, false);
            if let Some(p) = app.get_webview(&page_label(&owner, &t)) {
                let _ = p.reload();
            }
        }
        Act::TabClose(t) => {
            hide_menu(&app, false);
            let _ = close_tab(&app, &owner, &t);
        }
        Act::TabOthers(t) | Act::TabRight(t) => {
            hide_menu(&app, false);
            let tabs = get(&owner).map(|w| w.tabs).unwrap_or_default();
            let Some(i) = tabs.iter().position(|x| x.id == t) else { return };
            let close: Vec<String> = tabs
                .iter()
                .enumerate()
                .filter(|(j, x)| x.id != t && (!only_right || *j > i))
                .map(|(_, x)| x.id.clone())
                .collect();
            let _ = select_tab(&app, &owner, &t);
            for c in close {
                let _ = close_tab(&app, &owner, &c);
            }
        }
        Act::Ext(id) => {
            // 有選單頁面就開；沒有就開選項；都沒有（自動執行的）就顯示右鍵選單
            let e = ext_list().into_iter().find(|e| e.id == id);
            let e = e.filter(|e| !e.off);
            let popup = e.as_ref().and_then(|e| Url::parse(&e.popup).ok());
            let options = e.as_ref().and_then(|e| Url::parse(&e.options).ok());
            if let Some(u) = popup {
                // 像 Chrome 一樣開成小視窗蓋在網頁上，分頁不變，擴充功能才知道目前是哪個網站
                hide_menu(&app, false);
                let _ = open_ext_popup(&app, &owner, u, anchor);
            } else if let Some(u) = options {
                hide_menu(&app, false);
                let _ = new_tab(&app, &owner, Some(u), true, None);
            } else if let Some((items, acts)) = ext_ctx(&id) {
                push_menu(&app, items, acts);
            }
        }
        Act::ExtPage(page) => {
            hide_menu(&app, false);
            if let Ok(u) = Url::parse(&page) {
                if ext_list().iter().any(|e| e.popup == page) {
                    let _ = open_ext_popup(&app, &owner, u, anchor);
                } else {
                    let _ = new_tab(&app, &owner, Some(u), true, None);
                }
            }
        }
        Act::ExtToggle(id, off) => {
            hide_menu(&app, false);
            let _ = ext_set_off(&id, off);
            emit_main(&app);
        }
        Act::ExtSync => {
            hide_menu(&app, false);
            let _ = sync_extensions();
            emit_main(&app);
        }
        Act::Nothing => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn address_bar() {
        assert_eq!(to_url("youtube.com").unwrap().as_str(), "https://youtube.com/");
        assert_eq!(to_url("https://www.twitch.tv/x").unwrap().as_str(), "https://www.twitch.tv/x");
        let s = to_url("楓之谷 攻略").unwrap();
        assert_eq!(s.host_str(), Some("www.google.com"));
        assert!(s.query().unwrap().starts_with("q="));
        assert!(to_url("   ").is_none());
        // 不是網頁的網址當作搜尋
        assert_eq!(to_url("file:///c:/x").unwrap().host_str(), Some("www.google.com"));
        assert_eq!(to_url("javascript:alert(1)").unwrap().host_str(), Some("www.google.com"));
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
