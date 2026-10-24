//! 跟螢幕等比例顯示：不管 HD 或 4K，畫面內容佔螢幕的比例都一樣（4K 只是更清楚）。
//!
//! 以 1920×1080 的螢幕為基準：HD 上 1 個畫面像素 = 1 個螢幕像素，4K 上 = 2 個，2560×1440 上 = 1.33 個；
//! 但不會比 Windows 的縮放設定小（例如筆電設 125% 就照 125%，字不會變小）。
//! 網頁縮放 = 等比例倍率 ÷ Windows 縮放；主視窗大小用畫面像素記錄，換到別的螢幕時內容大小維持一樣。

use std::collections::HashMap;
use std::sync::Mutex;
use tauri::{AppHandle, Manager, Monitor, PhysicalPosition, PhysicalSize, Runtime, Webview, WebviewWindow};

/// 主視窗預設大小與最小大小（畫面像素）
pub const MAIN_W: f64 = 1366.0;
pub const MAIN_H: f64 = 768.0;
pub const MIN_W: f64 = 900.0;
pub const MIN_H: f64 = 560.0;

/// 螢幕實際像素 w×h、Windows 縮放 scale：1 個畫面像素等於幾個螢幕像素
pub fn ratio_of(w: u32, h: u32, scale: f64) -> f64 {
    let scale = if scale.is_finite() && scale > 0.0 { scale } else { 1.0 };
    let fit = (w as f64 / 1920.0).min(h as f64 / 1080.0);
    let r = if fit.is_finite() && fit > 0.0 { fit.max(scale) } else { scale };
    r.clamp(0.5, 8.0)
}

pub fn ratio(m: &Monitor) -> f64 {
    ratio_of(m.size().width, m.size().height, m.scale_factor())
}

/// 預設的主視窗內容大小（畫面像素）：1366×768，螢幕比較小時用螢幕的八成
pub fn default_view(screen_w: u32, screen_h: u32, r: f64) -> (f64, f64) {
    let (sw, sh) = (screen_w as f64 / r, screen_h as f64 / r);
    (MAIN_W.min(sw * 0.8).max(MIN_W.min(sw * 0.95)), MAIN_H.min(sh * 0.8).max(MIN_H.min(sh * 0.9)))
}

/// 每個視窗最後套用的（等比例倍率, Windows 縮放）
static APPLIED: Mutex<Option<HashMap<String, (f64, f64)>>> = Mutex::new(None);
/// 主視窗目前的內容大小（畫面像素）
static MAIN_VIEW: Mutex<Option<(f64, f64)>> = Mutex::new(None);

fn monitor_of<R: Runtime>(wv: &Webview<R>) -> Option<Monitor> {
    let win = wv.window();
    win.current_monitor().ok().flatten().or_else(|| win.primary_monitor().ok().flatten())
}

/// 網頁所在螢幕的（等比例倍率, Windows 縮放）
fn current<R: Runtime>(wv: &Webview<R>) -> Option<(f64, f64)> {
    let m = monitor_of(wv)?;
    let s = wv.window().scale_factor().unwrap_or_else(|_| m.scale_factor());
    Some((ratio(&m), s))
}

fn same(a: (f64, f64), b: (f64, f64)) -> bool {
    (a.0 - b.0).abs() < 1e-3 && (a.1 - b.1).abs() < 1e-3
}

fn applied(label: &str) -> Option<(f64, f64)> {
    APPLIED.lock().unwrap_or_else(|e| e.into_inner()).as_ref().and_then(|m| m.get(label).copied())
}

/// 套用網頁縮放；回傳等比例倍率
pub fn apply<R: Runtime>(wv: &Webview<R>) -> Option<f64> {
    let (r, s) = current(wv)?;
    let _ = wv.set_zoom(r / s);
    APPLIED.lock().unwrap_or_else(|e| e.into_inner()).get_or_insert_with(HashMap::new).insert(wv.label().to_string(), (r, s));
    Some(r)
}

/// 視窗移動後：換到倍率或縮放不同的螢幕才重新套用
pub fn check<R: Runtime>(wv: &Webview<R>) -> bool {
    let Some(now) = current(wv) else { return false };
    if applied(wv.label()).is_some_and(|a| same(a, now)) {
        return false;
    }
    apply(wv);
    true
}

/// 主視窗：套用縮放與最小大小，並調成 cw×ch 畫面像素（不超出螢幕可用範圍）
pub fn fit_main<R: Runtime>(w: &WebviewWindow<R>, cw: f64, ch: f64, keep_on_screen: bool) {
    let wv: &Webview<R> = w.as_ref();
    let Some(r) = apply(wv) else { return };
    let Some(m) = monitor_of(wv) else { return };
    let wa = *m.work_area();
    let (aw, ah) = (wa.size.width.max(1) as f64, wa.size.height.max(1) as f64);
    let (min_w, min_h) = ((MIN_W * r).min(aw * 0.95), (MIN_H * r).min(ah * 0.9));
    let _ = w.set_min_size(Some(PhysicalSize::new(min_w.round() as u32, min_h.round() as u32)));
    let pw = (cw * r).min(aw * 0.95).max(min_w).round();
    let ph = (ch * r).min(ah * 0.9).max(min_h).round();
    *MAIN_VIEW.lock().unwrap_or_else(|e| e.into_inner()) = Some((pw / r, ph / r));
    // 標題列與邊框的厚度（外框大小 − 內容大小），用來算調整後的外框
    let frame = match (w.outer_size(), w.inner_size()) {
        (Ok(o), Ok(i)) => (o.width.saturating_sub(i.width) as i32, o.height.saturating_sub(i.height) as i32),
        _ => (0, 0),
    };
    let _ = w.set_size(PhysicalSize::new(pw as u32, ph as u32));
    if keep_on_screen {
        // 變大後超出螢幕就往回移
        if let Ok(pos) = w.outer_position() {
            let (ow, oh) = (pw as i32 + frame.0, ph as i32 + frame.1);
            let (l, t) = (wa.position.x, wa.position.y);
            let (r_edge, b_edge) = (l + wa.size.width as i32, t + wa.size.height as i32);
            let x = pos.x.min(r_edge - ow).max(l);
            let y = pos.y.min(b_edge - oh).max(t);
            if (x, y) != (pos.x, pos.y) {
                let _ = w.set_position(PhysicalPosition::new(x, y));
            }
        }
    }
}

/// 主視窗第一次顯示前：照記錄的內容大小（沒有就用預設）
pub fn setup_main<R: Runtime>(w: &WebviewWindow<R>, saved: Option<(u32, u32)>) {
    let wv: &Webview<R> = w.as_ref();
    let (cw, ch) = match saved {
        Some((sw, sh)) if sw as f64 >= MIN_W && sh as f64 >= MIN_H => (sw as f64, sh as f64),
        _ => match (monitor_of(wv), current(wv)) {
            (Some(m), Some((r, _))) => default_view(m.size().width, m.size().height, r),
            _ => (MAIN_W, MAIN_H),
        },
    };
    fit_main(w, cw, ch, false);
}

/// 使用者調整主視窗大小：回傳要記錄的內容大小（畫面像素）；換螢幕途中的大小改變不算
pub fn main_resized<R: Runtime>(w: &WebviewWindow<R>, size: PhysicalSize<u32>) -> Option<(u32, u32)> {
    let wv: &Webview<R> = w.as_ref();
    #[cfg(windows)]
    if !user_change(w.hwnd().map(|h| h.0 as isize).unwrap_or(0)) {
        return None;
    }
    let now = current(wv)?;
    if !applied(wv.label()).is_some_and(|a| same(a, now)) {
        return None;
    }
    let (cw, ch) = (size.width as f64 / now.0, size.height as f64 / now.0);
    *MAIN_VIEW.lock().unwrap_or_else(|e| e.into_inner()) = Some((cw, ch));
    Some((cw.round() as u32, ch.round() as u32))
}

/// 主視窗移動或 Windows 縮放改變後：換到不同倍率的螢幕時，重新套用縮放並維持內容大小
pub fn main_moved(app: &AppHandle) {
    let app2 = app.clone();
    let _ = app.run_on_main_thread(move || {
        let Some(w) = app2.get_webview_window("main") else { return };
        let wv: &Webview = w.as_ref();
        let Some(now) = current(wv) else { return };
        if applied(wv.label()).is_some_and(|a| same(a, now)) {
            return;
        }
        // 縮小時位置在螢幕外，等還原後再處理；最大化時只換縮放
        if w.is_minimized().unwrap_or(false) {
            return;
        }
        if w.is_maximized().unwrap_or(false) {
            apply(wv);
            return;
        }
        // 拖曳途中不要把視窗拉回螢幕內（使用者還在移動）
        let (cw, ch) = MAIN_VIEW.lock().unwrap_or_else(|e| e.into_inner()).unwrap_or((MAIN_W, MAIN_H));
        fit_main(&w, cw, ch, false);
    });
}

/* ---------------- 螢幕關掉再開（或接上、拔掉螢幕）後把視窗大小還原 ----------------
 * 螢幕關掉時 Windows 會把視窗塞進剩下的（或暫時的小）螢幕，視窗被縮小；
 * 這段時間的大小、位置變動不算使用者調整的，螢幕回來後照記錄的大小放回去。 */

use std::sync::OnceLock;
use std::time::{Duration, Instant};

/// 最近一次螢幕配置改變的時間
static DISPLAY_CHANGED: Mutex<Option<Instant>> = Mutex::new(None);
/// 使用者正在拖曳或拉大小的視窗
static SIZING: Mutex<Vec<isize>> = Mutex::new(Vec::new());
static APP: OnceLock<AppHandle> = OnceLock::new();
/// 螢幕配置改變後多久內的變動不記錄
const SETTLE: Duration = Duration::from_secs(8);

/// 螢幕配置剛改變（視窗大小可能是 Windows 暫時塞的，不是使用者調的）
pub fn settling() -> bool {
    DISPLAY_CHANGED.lock().unwrap_or_else(|e| e.into_inner()).is_some_and(|t| t.elapsed() < SETTLE)
}

pub fn user_sizing(hwnd: isize) -> bool {
    SIZING.lock().unwrap_or_else(|e| e.into_inner()).contains(&hwnd)
}

/// 視窗大小變動算不算使用者調的（拖曳中一定算；螢幕剛變動時不算）
pub fn user_change(hwnd: isize) -> bool {
    user_sizing(hwnd) || !settling()
}

/// 螢幕配置改變了：過幾秒等 Windows 安定後，把主視窗放回記錄的大小
pub fn display_changed() {
    *DISPLAY_CHANGED.lock().unwrap_or_else(|e| e.into_inner()) = Some(Instant::now());
    let Some(app) = APP.get().cloned() else { return };
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(2500));
        // 這段時間又變了：交給最後那一次
        if DISPLAY_CHANGED.lock().unwrap_or_else(|e| e.into_inner()).is_some_and(|t| t.elapsed() < Duration::from_millis(2400)) {
            return;
        }
        let app2 = app.clone();
        let _ = app.run_on_main_thread(move || {
            let Some(w) = app2.get_webview_window("main") else { return };
            if w.is_minimized().unwrap_or(false) || w.is_maximized().unwrap_or(false) || !w.is_visible().unwrap_or(false) {
                return;
            }
            let (cw, ch) = MAIN_VIEW.lock().unwrap_or_else(|e| e.into_inner()).unwrap_or((MAIN_W, MAIN_H));
            fit_main(&w, cw, ch, true);
        });
    });
}

#[cfg(windows)]
mod disp {
    use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
    use windows_sys::Win32::UI::Shell::{DefSubclassProc, SetWindowSubclass};
    use windows_sys::Win32::UI::WindowsAndMessaging::{WM_DISPLAYCHANGE, WM_ENTERSIZEMOVE, WM_EXITSIZEMOVE};

    const ID: usize = 0x4D42_4453;

    unsafe extern "system" fn proc_(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM, _id: usize, _d: usize) -> LRESULT {
        match msg {
            WM_DISPLAYCHANGE => super::display_changed(),
            WM_ENTERSIZEMOVE => super::SIZING.lock().unwrap_or_else(|e| e.into_inner()).push(hwnd as isize),
            WM_EXITSIZEMOVE => super::SIZING.lock().unwrap_or_else(|e| e.into_inner()).retain(|h| *h != hwnd as isize),
            _ => {}
        }
        DefSubclassProc(hwnd, msg, wp, lp)
    }

    pub fn watch(hwnd: isize) {
        unsafe { SetWindowSubclass(hwnd as HWND, Some(proc_), ID, 0) };
    }
}

/// 主視窗：監看螢幕配置改變與使用者拖曳（要在主執行緒呼叫）
pub fn watch_main(app: &AppHandle, hwnd: isize) {
    let _ = APP.set(app.clone());
    #[cfg(windows)]
    disp::watch(hwnd);
    #[cfg(not(windows))]
    let _ = hwnd;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ratio_rules() {
        assert_eq!(ratio_of(1920, 1080, 1.0), 1.0); // HD
        assert_eq!(ratio_of(3840, 2160, 1.5), 2.0); // 4K 150%
        assert_eq!(ratio_of(3840, 2160, 2.0), 2.0); // 4K 200%
        assert_eq!(ratio_of(3840, 2160, 1.0), 2.0); // 4K 100%
        assert!((ratio_of(2560, 1440, 1.0) - 4.0 / 3.0).abs() < 1e-9);
        assert_eq!(ratio_of(1920, 1080, 1.25), 1.25); // 筆電 125%：不會變小
        assert_eq!(ratio_of(1366, 768, 1.0), 1.0); // 小螢幕：不縮小
        assert!((ratio_of(3440, 1440, 1.0) - 4.0 / 3.0).abs() < 1e-9); // 超寬螢幕看高度
    }

    #[test]
    fn default_sizes() {
        assert_eq!(default_view(1920, 1080, 1.0), (1366.0, 768.0));
        assert_eq!(default_view(3840, 2160, 2.0), (1366.0, 768.0));
        let (w, h) = default_view(1366, 768, 1.0);
        assert!((w - 1092.8).abs() < 1e-6 && (h - 614.4).abs() < 1e-6);
        assert_eq!(default_view(1600, 900, 1.0), (1280.0, 720.0));
    }
}
