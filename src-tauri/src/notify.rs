//! 程式自己的通知：螢幕右下角的置頂小視窗（不搶焦點），遊戲在視窗化或無邊框全螢幕時也看得到。

use serde::Serialize;
use std::sync::Mutex;
use tauri::{AppHandle, Emitter, Manager, PhysicalPosition, PhysicalSize, WebviewUrl, WebviewWindow, WebviewWindowBuilder};

pub const LABEL: &str = "notify";
const W: f64 = 380.0;
const H: f64 = 118.0;

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
    WebviewWindowBuilder::new(app, LABEL, WebviewUrl::App("notify.html".into()))
        .title("工具箱通知")
        .decorations(false)
        .transparent(true)
        .shadow(false)
        .always_on_top(true)
        .skip_taskbar(true)
        .resizable(false)
        .focused(false)
        .visible(false)
        .inner_size(W, H)
        .build()
        .ok()
}

/// 放到主螢幕右下角（工作列上方）
fn place(app: &AppHandle, w: &WebviewWindow) {
    let Ok(Some(m)) = app.primary_monitor() else { return };
    let scale = m.scale_factor();
    let (mw, mh) = (m.size().width as i32, m.size().height as i32);
    let (mx, my) = (m.position().x, m.position().y);
    let (ww, wh) = ((W * scale) as i32, (H * scale) as i32);
    let margin = (16.0 * scale) as i32;
    let taskbar = (52.0 * scale) as i32;
    let _ = w.set_size(PhysicalSize::new(ww as u32, wh as u32));
    let _ = w.set_position(PhysicalPosition::new(mx + mw - ww - margin, my + mh - wh - margin - taskbar));
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
    show_no_activate(&w, true);
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
