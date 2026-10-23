//! web 浮動視窗：置頂的小瀏覽器，邊玩遊戲邊看影片。
//!
//! 每個視窗 = 一個無框視窗，上面是工具列（本機頁面 web.html），下面是外部網頁。
//! - 視窗設成「不搶焦點」：滑鼠點影片、按暫停、拖進度時，鍵盤一直留在遊戲。
//! - 點到網頁或工具列的輸入框要打字時，才把鍵盤交給視窗；離開輸入框或滑鼠移出視窗就還給原本的視窗（遊戲）。
//! - 外部網頁不能呼叫程式的任何指令（沒有設定 remote 權限，Tauri 會直接拒絕）。

use crate::fg;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, MutexGuard, OnceLock};
use std::time::{Duration, Instant};
use tauri::webview::{NewWindowResponse, PageLoadEvent, WebviewBuilder};
use tauri::window::{Color, WindowBuilder};
use tauri::{
    AppHandle, Emitter, LogicalPosition, LogicalSize, Manager, PhysicalPosition, PhysicalSize, Url, Webview, WebviewUrl,
    Window, WindowEvent,
};

pub const HOME: &str = "https://www.google.com/";
/// 工具列高度（邏輯像素）
const BAR_H: f64 = 38.0;
/// 左、右、下留給拉大小的邊（邏輯像素）
const EDGE: f64 = 5.0;
/// 網頁裡有輸入框被選取時，標題最後面會加上這兩個看不見的字元
const MARK: char = '\u{2063}';

/// 注入外部網頁的小腳本：只負責「現在有沒有輸入框被選取」，用標題最後的隱形記號告訴程式。
/// 不會讀取網頁內容，也不會送出任何資料。
const TYPING_JS: &str = r#"(() => {
  if (window.top !== window) return;
  const M = "\u2063\u2063";
  const NOTXT = /^(button|checkbox|radio|range|submit|reset|file|color|image|hidden)$/i;
  let on = false, busy = false;
  const deep = () => { let a = document.activeElement; while (a && a.shadowRoot && a.shadowRoot.activeElement) a = a.shadowRoot.activeElement; return a; };
  const editable = (el) => !!el && (el.isContentEditable || el.tagName === "TEXTAREA" || (el.tagName === "INPUT" && !NOTXT.test(el.type || "text")));
  const apply = () => {
    const t = document.title || "";
    const has = t.endsWith(M);
    if (on === has) return;
    busy = true;
    document.title = on ? t + M : t.slice(0, -M.length);
    busy = false;
  };
  const check = () => { const v = editable(deep()); if (v !== on) { on = v; apply(); } };
  document.addEventListener("focusin", check, true);
  document.addEventListener("focusout", () => setTimeout(check, 0), true);
  const watch = () => {
    if (!document.head) return;
    new MutationObserver(() => { if (!busy) apply(); }).observe(document.head, { childList: true, subtree: true, characterData: true });
  };
  if (document.head) watch(); else document.addEventListener("DOMContentLoaded", watch, { once: true });
})();"#;

/* ---------------- 設定檔 ---------------- */

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct WebWin {
    pub id: String,
    pub url: String,
    pub title: String,
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
}

impl Default for WebWin {
    fn default() -> Self {
        Self {
            id: String::new(),
            url: HOME.into(),
            title: String::new(),
            x: 200,
            y: 150,
            w: 720,
            h: 460,
            opacity: 100,
            locked: false,
            game_only: false,
            open: false,
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct WebFile {
    pub next_id: u32,
    pub wins: Vec<WebWin>,
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

fn load(path: &Path) -> WebFile {
    std::fs::read_to_string(path).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default()
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

fn edit(id: &str, f: impl FnOnce(&mut WebWin)) {
    let mut s = st();
    if let Some(w) = s.file.wins.iter_mut().find(|w| w.id == id) {
        f(w);
        s.dirty = true;
    }
}

fn get(id: &str) -> Option<WebWin> {
    st().file.wins.iter().find(|w| w.id == id).cloned()
}

/* ---------------- 執行狀態 ---------------- */

#[derive(Debug, Default)]
struct Rt {
    hwnd: isize,
    /// 網頁裡有輸入框被選取
    page_typing: bool,
    page_off_at: Option<Instant>,
    /// 工具列的網址列被選取
    bar_typing: bool,
    /// 目前鍵盤在這個視窗
    typing: bool,
    /// 滑鼠移出視窗的時間（打字中才看）
    out_since: Option<Instant>,
    /// 開始打字前在前景的視窗（打完要還給它）
    prev_fg: isize,
}

static RT: Mutex<Option<HashMap<String, Rt>>> = Mutex::new(None);
/// 目前在打字的視窗（攔截訊息時要查，所以另外放）
static TYPING_HWND: Mutex<Vec<isize>> = Mutex::new(Vec::new());
static GAME_FG: AtomicBool = AtomicBool::new(false);

fn rt<T>(f: impl FnOnce(&mut HashMap<String, Rt>) -> T) -> T {
    let mut g = RT.lock().unwrap_or_else(|e| e.into_inner());
    f(g.get_or_insert_with(HashMap::new))
}

fn win_label(id: &str) -> String {
    format!("web-{id}")
}
fn bar_label(id: &str) -> String {
    format!("webbar-{id}")
}
fn page_label(id: &str) -> String {
    format!("webpg-{id}")
}

/// 從工具列的 webview 名稱取回視窗編號
fn id_of(wv: &Webview) -> Option<String> {
    wv.label().strip_prefix("webbar-").map(str::to_string)
}

#[derive(Serialize, Clone)]
pub struct BarState {
    id: String,
    url: String,
    title: String,
    loading: bool,
}

fn emit_bar(app: &AppHandle, id: &str, loading: bool) {
    if let Some(w) = get(id) {
        let _ = app.emit_to(bar_label(id).as_str(), "web-state", BarState { id: w.id, url: w.url, title: w.title, loading });
    }
}

fn emit_main(app: &AppHandle) {
    let _ = app.emit_to("main", "web-changed", ());
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

/* ---------------- 建立視窗 ---------------- */

fn layout(app: &AppHandle, id: &str, win: &Window, size: PhysicalSize<u32>) {
    let sf = win.scale_factor().unwrap_or(1.0).max(0.5);
    let (w, h) = (size.width as f64 / sf, size.height as f64 / sf);
    if let Some(b) = app.get_webview(&bar_label(id)) {
        let _ = b.set_position(LogicalPosition::new(0.0, 0.0));
        let _ = b.set_size(LogicalSize::new(w.max(1.0), BAR_H));
    }
    if let Some(p) = app.get_webview(&page_label(id)) {
        let _ = p.set_position(LogicalPosition::new(EDGE, BAR_H));
        let _ = p.set_size(LogicalSize::new((w - 2.0 * EDGE).max(1.0), (h - BAR_H - EDGE).max(1.0)));
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

fn open_window(app: &AppHandle, id: &str) -> Result<(), String> {
    let Some(cfg) = get(id) else { return Err("找不到這個視窗".into()) };
    let label = win_label(id);
    if let Some(win) = app.get_window(&label) {
        edit(id, |w| w.open = true);
        show(&win, true);
        return Ok(());
    }
    let url = Url::parse(&cfg.url).ok().filter(web_url).unwrap_or_else(|| Url::parse(HOME).unwrap());

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
    let _ = win.set_min_size(Some(LogicalSize::new(260.0, 160.0)));
    let (x, y) = on_screen(&win, cfg.x, cfg.y, cfg.w.max(260), cfg.h.max(160));
    let _ = win.set_size(PhysicalSize::new(cfg.w.max(260), cfg.h.max(160)));
    let _ = win.set_position(PhysicalPosition::new(x, y));

    #[cfg(windows)]
    let hwnd = win.hwnd().map(|h| h.0 as isize).unwrap_or(0);
    #[cfg(not(windows))]
    let hwnd = 0isize;
    rt(|m| m.insert(id.to_string(), Rt { hwnd, ..Rt::default() }));
    install_subclass(app, hwnd);

    let sf = win.scale_factor().unwrap_or(1.0).max(0.5);
    let size = win.inner_size().unwrap_or(PhysicalSize::new(cfg.w, cfg.h));
    let (lw, lh) = (size.width as f64 / sf, size.height as f64 / sf);

    // 上面：工具列（本機頁面）
    let bar = WebviewBuilder::new(bar_label(id), WebviewUrl::App("web.html".into()));
    win.add_child(bar, LogicalPosition::new(0.0, 0.0), LogicalSize::new(lw.max(1.0), BAR_H))
        .map_err(|e| format!("工具列建立失敗：{e}"))?;

    // 下面：外部網頁
    let (a1, a2, a3) = (app.clone(), app.clone(), app.clone());
    let (i1, i2, i3) = (id.to_string(), id.to_string(), id.to_string());
    let page = WebviewBuilder::new(page_label(id), WebviewUrl::External(url))
        .initialization_script(TYPING_JS)
        .on_page_load(move |_wv, p| {
            let u = p.url();
            if web_url(u) {
                let s = u.to_string();
                edit(&i1, |w| w.url = s);
            }
            emit_bar(&a1, &i1, matches!(p.event(), PageLoadEvent::Started));
            emit_main(&a1);
        })
        .on_document_title_changed(move |_wv, raw| {
            let typing = raw.ends_with(MARK);
            let title = raw.trim_end_matches(MARK).to_string();
            rt(|m| {
                if let Some(r) = m.get_mut(&i2) {
                    if r.page_typing && !typing {
                        r.page_off_at = Some(Instant::now());
                    }
                    r.page_typing = typing;
                }
            });
            let changed = get(&i2).map(|w| w.title != title).unwrap_or(false);
            if changed {
                edit(&i2, |w| w.title = title);
                emit_bar(&a2, &i2, false);
                emit_main(&a2);
            }
        })
        // 開新分頁的連結直接在同一個視窗打開
        .on_new_window(move |u, _features| {
            if web_url(&u) {
                let (app, label) = (a3.clone(), page_label(&i3));
                std::thread::spawn(move || {
                    if let Some(p) = app.get_webview(&label) {
                        let _ = p.navigate(u);
                    }
                });
            }
            NewWindowResponse::Deny
        });
    win.add_child(
        page,
        LogicalPosition::new(EDGE, BAR_H),
        LogicalSize::new((lw - 2.0 * EDGE).max(1.0), (lh - BAR_H - EDGE).max(1.0)),
    )
    .map_err(|e| format!("網頁建立失敗：{e}"))?;

    let (app2, id2, win2) = (app.clone(), id.to_string(), win.clone());
    win.on_window_event(move |ev| match ev {
        WindowEvent::Resized(s) if s.width > 0 && s.height > 0 => {
            layout(&app2, &id2, &win2, *s);
            let (w, h) = (s.width, s.height);
            edit(&id2, |c| (c.w, c.h) = (w, h));
        }
        WindowEvent::Moved(p) if p.x > -10000 && p.y > -10000 => {
            let (x, y) = (p.x, p.y);
            edit(&id2, |c| (c.x, c.y) = (x, y));
        }
        WindowEvent::Destroyed => {
            let hwnd = rt(|m| m.remove(&id2).map(|r| r.hwnd).unwrap_or(0));
            TYPING_HWND.lock().unwrap_or_else(|e| e.into_inner()).retain(|h| *h != hwnd);
            emit_main(&app2);
        }
        _ => {}
    });

    edit(id, |w| {
        w.open = true;
        (w.x, w.y) = (x, y);
    });
    show(&win, true);
    emit_main(app);
    Ok(())
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

/* ---------------- Windows：攔截訊息、焦點、樣式 ---------------- */

#[cfg(windows)]
mod win32 {
    use super::TYPING_HWND;
    use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
    use windows_sys::Win32::System::Threading::{AttachThreadInput, GetCurrentProcessId, GetCurrentThreadId};
    use windows_sys::Win32::UI::Shell::{DefSubclassProc, SetWindowSubclass};
    use windows_sys::Win32::UI::WindowsAndMessaging::*;

    const WM_REFOCUS: u32 = WM_APP + 0x57;
    const SUBCLASS_ID: usize = 0x4D42_5745;

    fn typing(h: HWND) -> bool {
        TYPING_HWND.lock().unwrap_or_else(|e| e.into_inner()).contains(&(h as isize))
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

    unsafe extern "system" fn proc_(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM, _id: usize, _d: usize) -> LRESULT {
        match msg {
            // 不畫系統標題列與邊框；整個視窗都是內容區
            WM_NCPAINT | 0x00AE | 0x00AF => 0,
            WM_NCACTIVATE => 1,
            WM_NCCALCSIZE if wp != 0 => 0,
            // 左、右、下的邊可以拉大小（只有沒被網頁蓋住的那一條邊會收到這個訊息）
            WM_NCHITTEST => {
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
            // 萬一還是被啟用了（網頁元件自己搶），馬上把前景還給原本的視窗
            WM_ACTIVATE => {
                let active = (wp & 0xFFFF) != 0;
                let prev = lp as HWND;
                if active && !typing(hwnd) && !prev.is_null() && IsWindow(prev) != 0 {
                    PostMessageW(hwnd, WM_REFOCUS, prev as WPARAM, 0);
                }
                DefSubclassProc(hwnd, msg, wp, lp)
            }
            WM_REFOCUS => {
                if !typing(hwnd) {
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
}

fn install_subclass(app: &AppHandle, hwnd: isize) {
    #[cfg(windows)]
    if hwnd != 0 {
        let _ = app.run_on_main_thread(move || win32::subclass(hwnd));
    }
    #[cfg(not(windows))]
    let _ = (app, hwnd);
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
        TYPING_HWND.lock().unwrap_or_else(|e| e.into_inner()).push(hwnd);
        let locked = get(id).map(|w| w.locked).unwrap_or(false);
        let alpha = alpha_of(id);
        let target = if from_bar { bar_label(id) } else { page_label(id) };
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
        TYPING_HWND.lock().unwrap_or_else(|e| e.into_inner()).retain(|h| *h != hwnd);
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

fn alpha_of(id: &str) -> u8 {
    let o = get(id).map(|w| w.opacity).unwrap_or(100).clamp(30, 100);
    ((o * 255 + 50) / 100) as u8
}

/* ---------------- 背景執行緒 ---------------- */

pub fn init(app: &AppHandle, path: PathBuf) {
    let file = load(&path);
    let reopen: Vec<String> = file.wins.iter().filter(|w| w.open).map(|w| w.id.clone()).collect();
    let _ = STORE.set(Mutex::new(Store { file, path, dirty: false }));
    let app = app.clone();
    std::thread::spawn(move || {
        // 上次關閉程式時開著的視窗：重新打開
        std::thread::sleep(Duration::from_millis(1500));
        for id in reopen {
            if let Err(e) = open_window(&app, &id) {
                eprintln!("{e}");
            }
        }
        let mut last_save = Instant::now();
        loop {
            std::thread::sleep(Duration::from_millis(100));
            tick(&app);
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
            let mut want = !cfg.locked && (r.bar_typing || r.page_typing || (r.typing && page_grace));
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
            let act = if want && !r.typing {
                r.typing = true;
                Some(Action::Enter(r.bar_typing))
            } else if !want && r.typing {
                r.typing = false;
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
                if let Some(p) = app.get_webview(&page_label(&id)) {
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
            let visible = !cfg.game_only || game_fg || typing || info.is_self;
            if win32::visible(hwnd) != visible {
                win32::show(hwnd, visible);
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
        let w = WebWin { id: format!("w{n}"), x: 200 + k * 40, y: 150 + k * 40, ..WebWin::default() };
        s.file.wins.push(w.clone());
        s.dirty = true;
        w
    };
    open_window(&app, &w.id)?;
    Ok(get(&w.id).unwrap_or(w))
}

#[tauri::command]
pub async fn web_open(app: AppHandle, id: String) -> Result<(), String> {
    open_window(&app, &id)
}

fn close_window(app: &AppHandle, id: &str) {
    edit(id, |w| w.open = false);
    if let Some(win) = app.get_window(&win_label(id)) {
        let _ = win.destroy();
    }
    emit_main(app);
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

/* ---------------- 指令：工具列 ---------------- */

#[tauri::command]
pub fn web_bar_init(webview: Webview) -> Option<BarState> {
    let id = id_of(&webview)?;
    get(&id).map(|w| BarState { id: w.id, url: w.url, title: w.title, loading: false })
}

#[tauri::command]
pub async fn web_nav(app: AppHandle, webview: Webview, action: String, input: Option<String>) -> Result<(), String> {
    let id = id_of(&webview).ok_or("不是 web 視窗")?;
    let page = app.get_webview(&page_label(&id)).ok_or("網頁不存在")?;
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

/// 工具列的網址列被選取／取消選取
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
pub async fn web_close_self(app: AppHandle, webview: Webview) {
    if let Some(id) = id_of(&webview) {
        close_window(&app, &id);
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
}
