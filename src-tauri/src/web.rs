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
    AppHandle, Emitter, LogicalPosition, LogicalSize, Manager, PhysicalPosition, PhysicalSize, Url, Webview, WebviewUrl,
    WebviewWindow, WebviewWindowBuilder, Window, WindowEvent,
};

pub const HOME: &str = "https://www.google.com/";
/// 上面三排的高度（邏輯像素，要跟 web.html 一致）
const TABS_H: f64 = 34.0;
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
  if (window.top !== window) return;
  const M = "\u2063\u2063";
  const NOTXT = /^(button|checkbox|radio|range|submit|reset|file|color|image|hidden)$/i;
  let on = false, busy = false;
  const deep = (a) => { while (a && a.shadowRoot && a.shadowRoot.activeElement) a = a.shadowRoot.activeElement; return a; };
  const editable = (el) => !!el && el.nodeType === 1 && (el.isContentEditable || el.tagName === "TEXTAREA" || (el.tagName === "INPUT" && !NOTXT.test(el.type || "text")));
  const apply = () => {
    const t = document.title || "";
    const has = t.endsWith(M);
    if (on === has) return;
    busy = true;
    document.title = on ? t + M : t.slice(0, -M.length);
    busy = false;
  };
  const set = (v) => { if (v !== on) { on = v; apply(); } };
  const check = () => set(editable(deep(document.activeElement)));
  // 點下去的那一刻就判斷（視窗沒有焦點時，網頁不一定會收到 focus 事件）
  document.addEventListener("pointerdown", (e) => {
    const t = e.composedPath ? e.composedPath()[0] : e.target;
    if (editable(t)) set(true); else setTimeout(check, 0);
  }, true);
  document.addEventListener("focusin", check, true);
  document.addEventListener("focusout", () => setTimeout(check, 0), true);
  // 單行輸入框按 Enter（選字中的 Enter 不算）：送出後取消選取，鍵盤還給遊戲
  document.addEventListener("keydown", (e) => {
    if (e.key !== "Enter" || e.isComposing || e.keyCode === 229) return;
    const t = deep(document.activeElement);
    if (!t || t.tagName !== "INPUT" || !editable(t)) return;
    setTimeout(() => { if (deep(document.activeElement) === t) t.blur(); set(false); }, 150);
  }, true);
  const watch = () => {
    if (!document.head) return;
    new MutationObserver(() => { if (!busy) apply(); }).observe(document.head, { childList: true, subtree: true, characterData: true });
  };
  if (document.head) watch(); else document.addEventListener("DOMContentLoaded", watch, { once: true });
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
            w: 860,
            h: 560,
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
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Bookmarks {
    /// 書籤列
    pub bar: Vec<Bm>,
    /// 書籤列右邊的「所有書籤」
    pub other: Vec<Bm>,
    pub next_id: u64,
    /// 網站 → 圖示網址（逛過有加書籤的網站時記下來）
    pub icons: HashMap<String, String>,
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
            Some(Bm { id: format!("b{}", *next), title, url: url.to_string(), children: Vec::new() })
        }
        "folder" => {
            let children: Vec<Bm> = v["children"]
                .as_array()
                .map(|a| a.iter().filter_map(|c| from_chrome(c, next)).collect())
                .unwrap_or_default();
            *next += 1;
            Some(Bm { id: format!("b{}", *next), title, url: String::new(), children })
        }
        _ => None,
    }
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
}

static RT: Mutex<Option<HashMap<String, Rt>>> = Mutex::new(None);
/// 目前在打字的視窗（攔截訊息時要查，所以另外放）
static TYPING_HWND: Mutex<Vec<isize>> = Mutex::new(Vec::new());
/// 放到最大的視窗（不能拉邊改大小）
static MAXED_HWND: Mutex<Vec<isize>> = Mutex::new(Vec::new());
static GAME_FG: AtomicBool = AtomicBool::new(false);

fn rt<T>(f: impl FnOnce(&mut HashMap<String, Rt>) -> T) -> T {
    let mut g = lock(&RT);
    f(g.get_or_insert_with(HashMap::new))
}

fn hwnd_of(wid: &str) -> isize {
    rt(|m| m.get(wid).map(|r| r.hwnd).unwrap_or(0))
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

/* ---------------- 視窗、分頁 ---------------- */

fn page_rect(sf: f64, size: PhysicalSize<u32>) -> (LogicalPosition<f64>, LogicalSize<f64>) {
    let (w, h) = (size.width as f64 / sf, size.height as f64 / sf);
    (LogicalPosition::new(EDGE, BAR_H), LogicalSize::new((w - 2.0 * EDGE).max(1.0), (h - BAR_H - EDGE).max(1.0)))
}

fn layout(app: &AppHandle, wid: &str, win: &Window, size: PhysicalSize<u32>) {
    let sf = win.scale_factor().unwrap_or(1.0).max(0.5);
    if let Some(b) = app.get_webview(&bar_label(wid)) {
        let _ = b.set_position(LogicalPosition::new(0.0, 0.0));
        let _ = b.set_size(LogicalSize::new((size.width as f64 / sf).max(1.0), BAR_H));
    }
    let (pos, sz) = page_rect(sf, size);
    for t in get(wid).map(|w| w.tabs).unwrap_or_default() {
        if let Some(p) = app.get_webview(&page_label(wid, &t.id)) {
            let _ = p.set_position(pos);
            let _ = p.set_size(sz);
        }
    }
}

/// 視窗跑到所有螢幕外面時移回主螢幕
fn on_screen(win: &Window, x: i32, y: i32, w: u32, h: u32) -> (i32, i32) {
    let Ok(monitors) = win.available_monitors() else { return (x, y) };
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
    let base = win.primary_monitor().ok().flatten().map(|m| *m.position()).unwrap_or(PhysicalPosition::new(0, 0));
    (base.x + 200, base.y + 150)
}

/// 視窗所在螢幕的可用範圍（扣掉工作列）
fn work_area(win: &Window, x: i32, y: i32) -> Option<(PhysicalPosition<i32>, PhysicalSize<u32>)> {
    let m = win
        .monitor_from_point(x as f64 + 40.0, y as f64 + 20.0)
        .ok()
        .flatten()
        .or_else(|| win.current_monitor().ok().flatten())
        .or_else(|| win.primary_monitor().ok().flatten())?;
    let wa = m.work_area();
    Some((wa.position, wa.size))
}

fn page_builder(app: &AppHandle, wid: &str, tid: &str, url: Url) -> WebviewBuilder<tauri::Wry> {
    let (a1, a2, a3) = (app.clone(), app.clone(), app.clone());
    let (w1, w2, w3) = (wid.to_string(), wid.to_string(), wid.to_string());
    let (t1, t2, t3) = (tid.to_string(), tid.to_string(), tid.to_string());
    WebviewBuilder::new(page_label(wid, tid), WebviewUrl::External(url))
        .initialization_script(TYPING_JS)
        // 讓網頁自己的拖放（例如上傳檔案、排序清單）可以正常使用
        .disable_drag_drop_handler()
        .on_page_load(move |wv, p| {
            on_page(&a1, &w1, &t1, &wv, p.url(), matches!(p.event(), PageLoadEvent::Started));
        })
        .on_document_title_changed(move |_wv, raw| on_title(&a2, &w2, &t2, raw))
        // 開新分頁的連結：在這個視窗開一個新分頁
        .on_new_window(move |u, _features| {
            if web_url(&u) {
                let (app, wid, tid) = (a3.clone(), w3.clone(), t3.clone());
                std::thread::spawn(move || {
                    let _ = new_tab(&app, &wid, Some(u), true, Some(&tid));
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
    }
}

/// 建立分頁的網頁（要在主執行緒以外呼叫）
fn create_page(app: &AppHandle, win: &Window, wid: &str, tid: &str) -> Result<(), String> {
    let url = get(wid).and_then(|w| w.tabs.into_iter().find(|t| t.id == tid)).map(|t| t.url).unwrap_or_default();
    let url = parse_web(&url).unwrap_or_else(|| Url::parse(HOME).unwrap());
    let sf = win.scale_factor().unwrap_or(1.0).max(0.5);
    let size = win.inner_size().unwrap_or(PhysicalSize::new(860, 560));
    let (pos, sz) = page_rect(sf, size);
    win.add_child(page_builder(app, wid, tid, url), pos, sz).map_err(|e| format!("網頁建立失敗：{e}"))?;
    Ok(())
}

fn select_tab(app: &AppHandle, wid: &str, tid: &str) -> Result<(), String> {
    let prev = edit(wid, |w| {
        let t = w.tabs.iter().find(|t| t.id == tid)?.clone();
        w.url = t.url;
        w.title = t.title;
        Some(std::mem::replace(&mut w.active, tid.to_string()))
    })
    .flatten()
    .ok_or("找不到這個分頁")?;
    if let Some(win) = app.get_window(&win_label(wid)) {
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
    let url = url.map(|u| u.to_string()).unwrap_or_else(|| HOME.to_string());
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
            show(&win, true);
        }
        emit_main(app);
        return Ok(());
    }

    let win = WindowBuilder::new(app, &label)
        .title("web")
        .decorations(false)
        .always_on_top(true)
        .skip_taskbar(true)
        .resizable(true)
        .maximizable(false)
        .minimizable(false)
        .focused(false)
        // 點了也不會變成作用中的視窗：鍵盤留在遊戲
        .focusable(false)
        .visible(false)
        .background_color(Color(30, 34, 43, 255))
        .build()
        .map_err(|e| format!("視窗建立失敗：{e}"))?;
    let _ = win.set_min_size(Some(LogicalSize::new(380.0, 240.0)));
    let (x, y) = on_screen(&win, cfg.x, cfg.y, cfg.w.max(380), cfg.h.max(240));
    let (pos, size) = if cfg.max {
        work_area(&win, x, y).unwrap_or((PhysicalPosition::new(x, y), PhysicalSize::new(cfg.w, cfg.h)))
    } else {
        (PhysicalPosition::new(x, y), PhysicalSize::new(cfg.w.max(380), cfg.h.max(240)))
    };
    let _ = win.set_size(size);
    let _ = win.set_position(pos);

    #[cfg(windows)]
    let hwnd = win.hwnd().map(|h| h.0 as isize).unwrap_or(0);
    #[cfg(not(windows))]
    let hwnd = 0isize;
    rt(|m| m.insert(id.to_string(), Rt { hwnd, ..Rt::default() }));
    if cfg.max {
        lock(&MAXED_HWND).push(hwnd);
    }
    install_subclass(app, hwnd);

    let sf = win.scale_factor().unwrap_or(1.0).max(0.5);
    // 上面：分頁列、網址列、書籤列（本機頁面）
    let bar = WebviewBuilder::new(bar_label(id), WebviewUrl::App("web.html".into())).disable_drag_drop_handler();
    win.add_child(bar, LogicalPosition::new(0.0, 0.0), LogicalSize::new((size.width as f64 / sf).max(1.0), BAR_H))
        .map_err(|e| format!("工具列建立失敗：{e}"))?;
    // 下面：目前分頁的網頁（其他分頁點到時才載入）
    let url = parse_web(&cfg.url).unwrap_or_else(|| Url::parse(HOME).unwrap());
    let (ppos, psz) = page_rect(sf, size);
    win.add_child(page_builder(app, id, &cfg.active, url), ppos, psz).map_err(|e| format!("網頁建立失敗：{e}"))?;

    let (app2, id2, win2) = (app.clone(), id.to_string(), win.clone());
    win.on_window_event(move |ev| match ev {
        WindowEvent::Resized(s) if s.width > 0 && s.height > 0 => {
            layout(&app2, &id2, &win2, *s);
            let (w, h) = (s.width, s.height);
            edit(&id2, |c| {
                if !c.max {
                    (c.w, c.h) = (w, h);
                }
            });
        }
        WindowEvent::Moved(p) if p.x > -10000 && p.y > -10000 => {
            let (x, y) = (p.x, p.y);
            edit(&id2, |c| {
                if !c.max {
                    (c.x, c.y) = (x, y);
                }
            });
        }
        WindowEvent::Destroyed => {
            let hwnd = rt(|m| m.remove(&id2).map(|r| r.hwnd).unwrap_or(0));
            lock(&TYPING_HWND).retain(|h| *h != hwnd);
            lock(&MAXED_HWND).retain(|h| *h != hwnd);
            emit_main(&app2);
        }
        _ => {}
    });

    edit(id, |w| {
        w.open = true;
        if !w.max {
            (w.x, w.y) = (x, y);
        }
    });
    if !cfg.min {
        show(&win, true);
    }
    emit_main(app);
    Ok(())
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

fn toggle_max(app: &AppHandle, wid: &str) {
    let Some(win) = app.get_window(&win_label(wid)) else { return };
    let Some(cfg) = get(wid) else { return };
    let hwnd = hwnd_of(wid);
    if cfg.max {
        edit(wid, |w| w.max = false);
        lock(&MAXED_HWND).retain(|h| *h != hwnd);
        let _ = win.set_size(PhysicalSize::new(cfg.w, cfg.h));
        let _ = win.set_position(PhysicalPosition::new(cfg.x, cfg.y));
    } else if let Some((p, s)) = work_area(&win, cfg.x, cfg.y) {
        edit(wid, |w| w.max = true);
        lock(&MAXED_HWND).push(hwnd);
        let _ = win.set_position(p);
        let _ = win.set_size(s);
    }
    emit_state(app, wid);
}

/// 顯示（不搶焦點）或隱藏
fn show(win: &Window, visible: bool) {
    #[cfg(windows)]
    {
        use windows_sys::Win32::UI::WindowsAndMessaging::{ShowWindow, SW_HIDE, SW_SHOWNOACTIVATE};
        if let Ok(h) = win.hwnd() {
            unsafe { ShowWindow(h.0 as _, if visible { SW_SHOWNOACTIVATE } else { SW_HIDE }) };
            return;
        }
    }
    let _ = if visible { win.show() } else { win.hide() };
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
    use super::{lock, MAXED_HWND, TYPING_HWND};
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

    fn typing(h: HWND) -> bool {
        lock(&TYPING_HWND).contains(&(h as isize))
    }

    fn maxed(h: HWND) -> bool {
        lock(&MAXED_HWND).contains(&(h as isize))
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

    /// 被啟用了（網頁元件自己搶、或拖曳造成的）：把前景還給原本的視窗
    unsafe fn give_back_later(hwnd: HWND, prev: HWND) {
        if prev.is_null() || IsWindow(prev) == 0 {
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
            // 沒在打字時，點到視窗不要變成作用中
            WM_MOUSEACTIVATE if !typing(hwnd) => MA_NOACTIVATE as LRESULT,
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
                if prev != 0 && !typing(hwnd) && GetForegroundWindow() == hwnd {
                    PostMessageW(hwnd, WM_REFOCUS, prev as WPARAM, 0);
                }
                DefSubclassProc(hwnd, msg, wp, lp)
            }
            WM_REFOCUS => {
                if !typing(hwnd) && IsWindow(wp as HWND) != 0 {
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
                if (wp & 0xFFFF) != 0 && !prev.is_null() && IsWindow(prev) != 0 {
                    PostMessageW(hwnd, WM_REFOCUS, prev as WPARAM, 0);
                }
                DefSubclassProc(hwnd, msg, wp, lp)
            }
            WM_REFOCUS => {
                if IsWindow(wp as HWND) != 0 {
                    SetForegroundWindow(wp as HWND);
                }
                0
            }
            _ => DefSubclassProc(hwnd, msg, wp, lp),
        }
    }

    pub fn subclass(h: isize) {
        unsafe { SetWindowSubclass(h as HWND, Some(proc_), SUBCLASS_ID, 0) };
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
            if want != ex {
                SetWindowLongPtrW(hwnd, GWL_EXSTYLE, want as isize);
            }
            if layered {
                SetLayeredWindowAttributes(hwnd, 0, alpha, LWA_ALPHA);
            }
            SetWindowPos(
                hwnd,
                std::ptr::null_mut(),
                0,
                0,
                0,
                0,
                SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE | SWP_FRAMECHANGED,
            );
        }
    }

    pub fn show(h: isize, visible: bool) {
        unsafe { ShowWindow(h as HWND, if visible { SW_SHOWNOACTIVATE } else { SW_HIDE }) };
    }

    /// 選單：放到指定位置、蓋在所有視窗最上面、顯示（不搶焦點）
    pub fn show_at(h: isize, x: i32, y: i32, w: i32, hh: i32) {
        unsafe { SetWindowPos(h as HWND, HWND_TOPMOST, x, y, w, hh, SWP_NOACTIVATE | SWP_SHOWWINDOW) };
    }
}

fn install_subclass(app: &AppHandle, hwnd: isize) {
    #[cfg(windows)]
    if hwnd != 0 {
        let _ = app.run_on_main_thread(move || win32::subclass(hwnd));
    }
    #[cfg(not(windows))]
    let _ = (app, hwnd);
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

/// 鍵盤還給原本的視窗（遊戲）
fn leave_typing(app: &AppHandle, id: &str, hwnd: isize) {
    #[cfg(windows)]
    {
        lock(&TYPING_HWND).retain(|h| *h != hwnd);
        let prev = rt(|m| m.get_mut(id).map(|r| std::mem::take(&mut r.prev_fg)).unwrap_or(0));
        let locked = get(id).map(|w| w.locked).unwrap_or(false);
        let alpha = alpha_of(id);
        let _ = app.run_on_main_thread(move || {
            if win32::foreground() == hwnd || win32::is_own(win32::foreground()) {
                win32::give_back(prev);
            }
            win32::apply_style(hwnd, true, locked, alpha);
        });
    }
    #[cfg(not(windows))]
    let _ = (app, id, hwnd);
}

/* ---------------- 背景執行緒 ---------------- */

pub fn init(app: &AppHandle, path: PathBuf) {
    let file = load(&path);
    let reopen: Vec<String> = file.wins.iter().filter(|w| w.open).map(|w| w.id.clone()).collect();
    let _ = STORE.set(Mutex::new(Store { file, path, dirty: false }));
    let app1 = app.clone();
    std::thread::spawn(move || {
        // 上次關閉程式時開著的視窗：重新打開
        std::thread::sleep(Duration::from_millis(1500));
        for id in reopen {
            if let Err(e) = open_window(&app1, &id, false) {
                eprintln!("{e}");
            }
        }
        let mut last_save = Instant::now();
        loop {
            std::thread::sleep(Duration::from_millis(100));
            tick(&app1);
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
                let Some((rect, shown)) = cur else {
                    seen = None;
                    std::thread::sleep(Duration::from_millis(120));
                    continue;
                };
                let down = win32::mouse_down();
                if seen != Some(shown) {
                    // 剛打開：打開它的那一下不算
                    seen = Some(shown);
                } else if down && !prev_down {
                    let (x, y) = win32::cursor();
                    if x < rect.0 || x >= rect.2 || y < rect.1 || y >= rect.3 {
                        hide_menu(&app2, true);
                    }
                }
                prev_down = down;
                std::thread::sleep(Duration::from_millis(20));
            }
        });
    }
}

enum Action {
    Enter(bool),
    Leave,
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
    let game_fg = GAME_FG.load(Ordering::SeqCst);
    let now = Instant::now();

    for cfg in open {
        let id = cfg.id.clone();
        // 先在鎖內決定要做什麼，鎖外再動視窗
        let decided = rt(|m| {
            let r = m.get_mut(&id)?;
            if r.hwnd == 0 {
                return None;
            }
            let page_grace = r.page_off_at.is_some_and(|t| now.duration_since(t) < Duration::from_millis(250));
            let mut want = !cfg.locked && !cfg.min && (r.bar_typing || r.page_typing || (r.typing && page_grace));
            #[cfg(windows)]
            {
                // 打字中滑鼠移出視窗超過 0.4 秒：鍵盤還給遊戲
                if r.typing && win32::cursor_outside(r.hwnd) {
                    let since = *r.out_since.get_or_insert(now);
                    if now.duration_since(since) >= Duration::from_millis(400) {
                        want = false;
                        r.page_typing = false;
                        r.bar_typing = false;
                        r.out_since = None;
                    }
                } else {
                    r.out_since = None;
                }
            }
            if !want {
                r.bar_typing = r.bar_typing && !cfg.min && !cfg.locked;
            }
            let act = if want && !r.typing {
                r.typing = true;
                Some(Action::Enter(r.bar_typing))
            } else if !want && r.typing {
                r.typing = false;
                r.bar_typing = false;
                r.page_typing = false;
                Some(Action::Leave)
            } else {
                None
            };
            Some((r.hwnd, r.typing, act))
        });
        let Some((hwnd, typing, act)) = decided else { continue };
        match act {
            Some(Action::Enter(from_bar)) => enter_typing(app, &id, hwnd, from_bar),
            Some(Action::Leave) => {
                leave_typing(app, &id, hwnd);
                // 網頁與網址列的輸入框也取消選取
                if let Some(p) = active_page(app, &id) {
                    let _ = p.eval("(() => { const a = document.activeElement; if (a && a !== document.body) a.blur(); })()");
                }
                let _ = app.emit_to(bar_label(&id).as_str(), "web-blur", id.clone());
            }
            None => {}
        }
        #[cfg(windows)]
        {
            if !typing {
                win32::apply_style(hwnd, true, cfg.locked, alpha_of(&id));
            }
            let visible = !cfg.min && (!cfg.game_only || game_fg || typing || info.is_self);
            if win32::visible(hwnd) != visible {
                win32::show(hwnd, visible);
                if !visible {
                    hide_menu_of(app, &id);
                }
            }
        }
        #[cfg(not(windows))]
        let _ = (typing, game_fg);
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

#[derive(Serialize)]
pub struct ChromeProfile {
    dir: String,
    name: String,
    account: String,
    count: usize,
}

fn chrome_root() -> Option<PathBuf> {
    std::env::var_os("LOCALAPPDATA").map(|p| PathBuf::from(p).join("Google").join("Chrome").join("User Data"))
}

fn read_json(path: &Path) -> Option<Value> {
    serde_json::from_str(&std::fs::read_to_string(path).ok()?).ok()
}

/// Chrome 的使用者（每個登入的 Google 帳號一個）與書籤數量
#[tauri::command]
pub fn web_chrome_profiles() -> Result<Vec<ChromeProfile>, String> {
    let root = chrome_root().ok_or("找不到 Chrome 的資料夾")?;
    let state = read_json(&root.join("Local State")).unwrap_or(Value::Null);
    let cache = state["profile"]["info_cache"].as_object().cloned().unwrap_or_default();
    let mut dirs: Vec<String> = cache.keys().cloned().collect();
    if !dirs.iter().any(|d| d == "Default") {
        dirs.push("Default".into());
    }
    let mut out = Vec::new();
    for dir in dirs {
        let Some(v) = read_json(&root.join(&dir).join("Bookmarks")) else { continue };
        let mut n = 0;
        let count = ["bookmark_bar", "other", "synced"]
            .iter()
            .filter_map(|k| from_chrome(&v["roots"][*k], &mut n))
            .map(|b| count_links(&b.children))
            .sum();
        let info = cache.get(&dir).cloned().unwrap_or(Value::Null);
        out.push(ChromeProfile {
            name: info["name"].as_str().filter(|s| !s.is_empty()).unwrap_or(&dir).to_string(),
            account: info["user_name"].as_str().unwrap_or("").to_string(),
            dir,
            count,
        });
    }
    if out.is_empty() {
        return Err("找不到 Chrome 的書籤（這台電腦可能沒有安裝 Chrome，或還沒有任何書籤）".into());
    }
    out.sort_by_key(|p| (p.dir != "Default", p.dir.clone()));
    Ok(out)
}

/// 從 Chrome 匯入書籤（書籤列 → 書籤列；其他書籤、行動裝置書籤 → 所有書籤）。回傳新增幾個
#[tauri::command]
pub fn web_chrome_import(app: AppHandle, dir: String) -> Result<usize, String> {
    if dir.is_empty() || !dir.chars().all(|c| c.is_ascii_alphanumeric() || c == ' ' || c == '_' || c == '-') {
        return Err("Chrome 使用者名稱不正確".into());
    }
    let root = chrome_root().ok_or("找不到 Chrome 的資料夾")?;
    let v = read_json(&root.join(&dir).join("Bookmarks")).ok_or("讀不到這個使用者的書籤")?;
    let added = bm(|b| {
        let mut n = b.next_id;
        let bar = from_chrome(&v["roots"]["bookmark_bar"], &mut n).map(|f| f.children).unwrap_or_default();
        let mut other = from_chrome(&v["roots"]["other"], &mut n).map(|f| f.children).unwrap_or_default();
        if let Some(mut synced) = from_chrome(&v["roots"]["synced"], &mut n) {
            if !synced.children.is_empty() {
                synced.title = "行動裝置書籤".into();
                other.push(synced);
            }
        }
        b.next_id = n;
        merge(&mut b.bar, bar) + merge(&mut b.other, other)
    });
    emit_bm(&app);
    Ok(added)
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
            edit(&id, |w| w.min = true);
            if let Some(win) = app.get_window(&win_label(&id)) {
                show(&win, false);
            }
            emit_main(&app);
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
            let _ = bm_remove(&mut b.bar, &bid).or_else(|| bm_remove(&mut b.other, &bid));
            return false;
        }
        b.next_id += 1;
        let host = host_of(&tab.url).unwrap_or_default();
        let title = if tab.title.trim().is_empty() { host.clone() } else { tab.title.clone() };
        b.bar.push(Bm { id: format!("b{}", b.next_id), title, url: tab.url.clone(), children: Vec::new() });
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
        }
    });
    emit_bm(&app);
}

fn bm_delete(app: &AppHandle, id: &str) {
    bm(|b| {
        let _ = bm_remove(&mut b.bar, id).or_else(|| bm_remove(&mut b.other, id));
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
        _ => None,
    };
    let Some(((items, acts), width)) = built else { return Ok(()) };
    let win = app.get_window(&win_label(&wid)).ok_or("找不到這個視窗")?;
    let sf = win.scale_factor().unwrap_or(1.0).max(0.5);
    let anchor = match (kind.as_str(), rect) {
        ("folder" | "other" | "overflow", Some([l, t, r, b])) => {
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
    let picked = lock(&MENU).as_ref().filter(|s| s.visible).map(|s| (s.owner.clone(), s.acts.get(index).cloned()));
    let Some((owner, Some(act))) = picked else { return };
    let only_right = matches!(act, Act::TabRight(_));
    if button == 2 {
        if let Act::Link(id) | Act::Folder(id) = &act {
            if let Some((items, acts)) = bm_read(|b| bm_find_any(b, id).map(bm_ctx)) {
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
    fn old_window_data_gets_a_tab() {
        let mut w: WebWin = serde_json::from_str(r#"{"id":"w1","url":"https://www.youtube.com/","title":"YouTube","open":true}"#).unwrap();
        fix_win(&mut w);
        assert_eq!(w.tabs.len(), 1);
        assert_eq!(w.tabs[0].url, "https://www.youtube.com/");
        assert_eq!(w.active, w.tabs[0].id);
        assert_eq!(w.next_tab, 1);
    }
}
