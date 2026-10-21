//! 程式自己的通知：螢幕右下角的置頂小視窗（不搶焦點），遊戲在視窗化或無邊框全螢幕時也看得到。
//! Windows 通知發不出去時才會用到；樣子照 Windows 11 的通知（notify.html）。

use crate::scale;
use serde::Serialize;
use std::sync::Mutex;
use tauri::{AppHandle, Emitter, Manager, PhysicalPosition, PhysicalSize, WebviewUrl, WebviewWindow, WebviewWindowBuilder};

pub const LABEL: &str = "notify";
/// 視窗大小（畫面像素，含四周 10 的陰影空間；跟螢幕等比例換算成實際像素）
const W: f64 = 384.0;
const H: f64 = 182.0;

#[derive(Serialize, Clone)]
pub struct Note {
    pub title: String,
    pub body: String,
    /// "warn"（斷線、關閉）、"info"（更新等）
    pub kind: String,
    /// 每則通知的編號，前端用來判斷是不是新的
    pub id: u64,
}

static PENDING: Mutex<Option<Note>> = Mutex::new(None);
static SEQ: Mutex<u64> = Mutex::new(0);

fn show_no_activate(w: &WebviewWindow, visible: bool) {
    #[cfg(windows)]
    {
        use windows_sys::Win32::UI::WindowsAndMessaging::{ShowWindow, SW_HIDE, SW_SHOWNOACTIVATE};
        if let Ok(h) = w.hwnd() {
            unsafe {
                ShowWindow(h.0 as _, if visible { SW_SHOWNOACTIVATE } else { SW_HIDE });
            }
            return;
        }
    }
    let _ = if visible { w.show() } else { w.hide() };
}

fn window(app: &AppHandle) -> Option<WebviewWindow> {
    if let Some(w) = app.get_webview_window(LABEL) {
        return Some(w);
    }
    let w = WebviewWindowBuilder::new(app, LABEL, WebviewUrl::App("notify.html".into()))
        .title("工具箱通知")
        .decorations(false)
        .transparent(true)
        .shadow(false)
        .always_on_top(true)
        .skip_taskbar(true)
        .resizable(false)
        .focused(false)
        // 點通知不會搶走遊戲的焦點；也不會因為被啟用而畫出系統標題列
        .focusable(false)
        .visible(false)
        .inner_size(W, H)
        .build()
        .ok()?;
    // 和計時器浮動視窗一樣，攔下系統畫標題列（右上角 - 口 x）與邊框的訊息
    crate::timers::install_overlay_subclass(app, &w);
    Some(w)
}

/// 放到主螢幕右下角（工作列上方），大小跟螢幕等比例
fn place(app: &AppHandle, w: &WebviewWindow) {
    let Ok(Some(m)) = app.primary_monitor() else { return };
    let r = scale::ratio(&m);
    let wa = *m.work_area();
    let (ww, wh) = ((W * r).round() as i32, (H * r).round() as i32);
    // 卡片四周已經有 10 的透明空間，再留 2 → 卡片離螢幕邊緣 12（和 Windows 通知一樣）
    let margin = (2.0 * r).round() as i32;
    let _ = w.set_size(PhysicalSize::new(ww as u32, wh as u32));
    let _ = w.set_position(PhysicalPosition::new(
        wa.position.x + wa.size.width as i32 - ww - margin,
        wa.position.y + wa.size.height as i32 - wh - margin,
    ));
    scale::apply(w.as_ref());
}

/// 跳出通知
pub fn show(app: &AppHandle, title: &str, body: &str, kind: &str) {
    let id = {
        let mut s = SEQ.lock().unwrap_or_else(|e| e.into_inner());
        *s += 1;
        *s
    };
    let note = Note { title: title.into(), body: body.into(), kind: kind.into(), id };
    *PENDING.lock().unwrap_or_else(|e| e.into_inner()) = Some(note.clone());
    let Some(w) = window(app) else { return };
    place(app, &w);
    let _ = app.emit_to(LABEL, "notify", note);
    // 在主執行緒顯示：排在攔截標題列的設定之後，第一次跳出時也不會畫出 - 口 x
    let w2 = w.clone();
    if app.run_on_main_thread(move || show_no_activate(&w2, true)).is_err() {
        show_no_activate(&w, true);
    }
}

/// 通知頁面剛載入時取回最新一則（視窗第一次建立時，事件可能比頁面早到）
#[tauri::command]
pub fn notify_pending() -> Option<Note> {
    PENDING.lock().unwrap_or_else(|e| e.into_inner()).clone()
}

#[tauri::command]
pub fn notify_hide(app: AppHandle) {
    if let Some(w) = app.get_webview_window(LABEL) {
        show_no_activate(&w, false);
    }
}

/// 點通知：打開主視窗
#[tauri::command]
pub fn notify_open(app: AppHandle) {
    notify_hide(app.clone());
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.unminimize();
        let _ = w.show();
        let _ = w.set_focus();
    }
}

/// 建立視窗不能在同步指令裡做（Windows 會卡死），所以用 async
#[tauri::command]
pub async fn notify_test(app: AppHandle) {
    show(&app, "測試通知", "斷線或遊戲關閉時，會像這樣在右下角通知你。", "warn");
}

/// 前端要跳通知時用（例如發現新版本）
#[tauri::command]
pub async fn notify_show(app: AppHandle, title: String, body: String, kind: Option<String>) {
    show(&app, &title, &body, kind.as_deref().unwrap_or("info"));
}
