// 發行版不顯示主控台視窗
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod audio;
mod avatar;
mod fg;
mod input;
mod maple_api;
#[allow(dead_code)]
mod wz;
mod serial;
mod timers;

use serial::{AppState, Settings};
use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::sync::{mpsc, Arc, Mutex};
use std::time::Duration;
use tauri::{AppHandle, Emitter, Manager, PhysicalSize, State, WebviewWindow, WindowEvent};

struct Shared {
    state: Mutex<AppState>,
    path: PathBuf,
}

type Ctx<'a> = State<'a, Arc<Shared>>;

/// 主視窗被使用者調整的大小，背景執行緒每秒寫入一次
static PENDING_SIZE: Mutex<Option<(u32, u32)>> = Mutex::new(None);

fn set_clipboard(text: &str) -> Result<(), String> {
    // 剪貼簿可能暫時被其他程式占用，重試幾次
    let mut last_err = String::new();
    for _ in 0..5 {
        match arboard::Clipboard::new().and_then(|mut c| c.set_text(text.to_string())) {
            Ok(()) => return Ok(()),
            Err(e) => {
                last_err = e.to_string();
                std::thread::sleep(Duration::from_millis(40));
            }
        }
    }
    Err(format!("無法寫入剪貼簿：{last_err}"))
}

fn get_clipboard() -> Option<String> {
    for _ in 0..5 {
        match arboard::Clipboard::new().and_then(|mut c| c.get_text()) {
            Ok(t) => return Some(t),
            Err(arboard::Error::ContentNotAvailable) => return None,
            Err(_) => std::thread::sleep(Duration::from_millis(40)),
        }
    }
    None
}

/// 啟用中時，把目前序號放到剪貼簿
fn sync_clipboard(st: &AppState) -> Result<(), String> {
    if st.active {
        if let Some(cur) = st.current() {
            set_clipboard(cur)?;
        }
    }
    Ok(())
}

fn commit(shared: &Shared, st: &AppState) {
    input::PASTE_ACTIVE.store(st.active, Ordering::SeqCst);
    st.save(&shared.path);
}

/* ---------------- 序號複製 ---------------- */

#[tauri::command]
fn get_state(ctx: Ctx) -> AppState {
    ctx.state.lock().unwrap().clone()
}

/// 解析文字並取代舊清單，進度歸零，自動切換預設開啟
#[tauri::command]
fn extract(ctx: Ctx, raw: String) -> Result<AppState, String> {
    let mut st = ctx.state.lock().unwrap();
    st.serials = serial::extract(&raw);
    st.raw = raw;
    st.index = 0;
    st.active = !st.serials.is_empty();
    commit(&ctx, &st);
    sync_clipboard(&st)?;
    Ok(st.clone())
}

#[tauri::command]
fn set_index(ctx: Ctx, index: usize) -> Result<AppState, String> {
    let mut st = ctx.state.lock().unwrap();
    st.index = index.min(st.serials.len());
    commit(&ctx, &st);
    sync_clipboard(&st)?;
    Ok(st.clone())
}

#[tauri::command]
fn set_active(ctx: Ctx, active: bool) -> Result<AppState, String> {
    let mut st = ctx.state.lock().unwrap();
    st.active = active && !st.serials.is_empty();
    commit(&ctx, &st);
    sync_clipboard(&st)?;
    Ok(st.clone())
}

#[tauri::command]
fn copy_text(text: String) -> Result<(), String> {
    set_clipboard(&text)
}

#[tauri::command]
fn clear_list(ctx: Ctx) -> AppState {
    let mut st = ctx.state.lock().unwrap();
    st.raw.clear();
    st.serials.clear();
    st.index = 0;
    st.active = false;
    commit(&ctx, &st);
    st.clone()
}

#[tauri::command]
fn save_settings(ctx: Ctx, settings: Settings) {
    let mut st = ctx.state.lock().unwrap();
    // 視窗大小由後端管理，前端送來的不採用
    let (w, h) = (st.settings.win_w, st.settings.win_h);
    st.settings = Settings { win_w: w, win_h: h, ..settings };
    st.save(&ctx.path);
}

/// 背景執行緒：收到 Ctrl+V 通知後，確認剪貼簿確實是目前序號才切換到下一組
fn paste_worker(app: AppHandle, shared: Arc<Shared>, rx: mpsc::Receiver<()>) {
    while rx.recv().is_ok() {
        // 等目標程式讀完剪貼簿再換
        std::thread::sleep(Duration::from_millis(150));
        while rx.try_recv().is_ok() {}

        let mut st = shared.state.lock().unwrap();
        if !st.active {
            continue;
        }
        let Some(cur) = st.current().map(str::to_owned) else { continue };
        // 使用者若自行複製了別的東西，就不切換
        match get_clipboard() {
            Some(clip) if clip.trim() == cur => {}
            _ => continue,
        }
        st.index += 1;
        if let Some(next) = st.current() {
            let _ = set_clipboard(next);
        }
        st.save(&shared.path);
        let _ = app.emit("state-changed", st.clone());
    }
}

/* ---------------- 主視窗大小 ---------------- */

/// 依螢幕解析度（實際像素）決定預設視窗大小
fn default_size(win: &WebviewWindow) -> (u32, u32) {
    let monitor = win.current_monitor().ok().flatten().or_else(|| win.primary_monitor().ok().flatten());
    let Some(m) = monitor else { return (1366, 768) };
    let (sw, sh) = (m.size().width, m.size().height);
    let (w, h) = if sw >= 3840 {
        (1920, 1080)
    } else if sw >= 2560 {
        (1600, 900)
    } else if sw >= 1920 {
        (1366, 768)
    } else {
        (sw * 4 / 5, sh * 4 / 5)
    };
    (w.min(sw * 95 / 100), h.min(sh * 90 / 100))
}

fn setup_main_window(app: &tauri::App, st: &AppState) {
    let Some(win) = app.get_webview_window("main") else { return };
    #[cfg(windows)]
    if let Ok(h) = win.hwnd() {
        fg::set_main_hwnd(h.0 as isize);
    }
    let size = match (st.settings.win_w, st.settings.win_h) {
        (Some(w), Some(h)) if w >= 600 && h >= 400 => (w, h),
        _ => default_size(&win),
    };
    let _ = win.set_size(PhysicalSize::new(size.0, size.1));
    let _ = win.center();
    let _ = win.show();
    let _ = win.set_focus();
}

fn main() {
    tauri::Builder::default()
        .setup(|app| {
            let dir = app.path().app_data_dir().expect("無法取得資料夾");
            let path = dir.join("state.json");
            let mut st = AppState::load(&path);
            // 程式開啟時停在主頁，序號複製尚未打開 → 自動切換一律關閉
            st.active = false;
            input::PASTE_ACTIVE.store(false, Ordering::SeqCst);
            setup_main_window(app, &st);

            let shared = Arc::new(Shared { state: Mutex::new(st), path });
            app.manage(shared.clone());

            audio::init();
            maple_api::init(dir.join("maple_cache.json"));
            timers::init(app.handle(), dir.join("timers.json"));

            let (tx, rx) = mpsc::channel();
            input::start(app.handle().clone(), tx);
            let handle = app.handle().clone();
            let s2 = shared.clone();
            std::thread::spawn(move || paste_worker(handle, s2, rx));

            // 每秒把使用者調整過的視窗大小寫入
            std::thread::spawn(move || loop {
                std::thread::sleep(Duration::from_secs(1));
                let pending = PENDING_SIZE.lock().unwrap().take();
                if let Some((w, h)) = pending {
                    let mut st = shared.state.lock().unwrap();
                    st.settings.win_w = Some(w);
                    st.settings.win_h = Some(h);
                    st.save(&shared.path);
                }
            });
            Ok(())
        })
        .on_window_event(|window, event| {
            let label = window.label();
            if label == "main" {
                match event {
                    WindowEvent::Resized(size) => {
                        let normal = !window.is_minimized().unwrap_or(false) && !window.is_maximized().unwrap_or(false);
                        if normal && size.width >= 600 && size.height >= 400 {
                            *PENDING_SIZE.lock().unwrap() = Some((size.width, size.height));
                        }
                    }
                    // 關閉主視窗時連同浮動視窗一起結束程式
                    WindowEvent::CloseRequested { .. } => window.app_handle().exit(0),
                    _ => {}
                }
            } else if label == timers::OVERLAY_LABEL {
                match event {
                    WindowEvent::Moved(p) => timers::overlay_moved(p.x, p.y),
                    WindowEvent::Resized(s) => timers::overlay_resized(s.width, s.height),
                    _ => {}
                }
            }
        })
        .invoke_handler(tauri::generate_handler![
            get_state,
            extract,
            set_index,
            set_active,
            copy_text,
            clear_list,
            save_settings,
            timers::get_timers,
            timers::update_timer,
            timers::update_prefs,
            timers::add_timer,
            timers::delete_timer,
            timers::timer_start,
            timers::timer_stop,
            timers::set_master,
            timers::set_game_only,
            timers::reset_overlay,
            timers::preview_sound,
            timers::pick_sound_file,
            maple_api::maple_lookup,
            avatar::beauty_render,
            avatar::beauty_wz_status,
            avatar::pick_game_dir,
            avatar::beauty_items,
            avatar::beauty_icons,
            avatar::beauty_skins,
            avatar::beauty_styles
        ])
        .run(tauri::generate_context!())
        .expect("啟動失敗");
}
