// 發行版不顯示主控台視窗
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod hook;
mod serial;

use serial::{AppState, Settings};
use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::sync::{mpsc, Arc, Mutex};
use std::time::Duration;
use tauri::{AppHandle, Emitter, Manager, State};

struct Shared {
    state: Mutex<AppState>,
    path: PathBuf,
}

type Ctx<'a> = State<'a, Arc<Shared>>;

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
    hook::ACTIVE.store(st.active, Ordering::SeqCst);
    st.save(&shared.path);
}

#[tauri::command]
fn get_state(ctx: Ctx) -> AppState {
    ctx.state.lock().unwrap().clone()
}

/// 解析文字並取代舊清單，進度歸零
#[tauri::command]
fn extract(ctx: Ctx, raw: String) -> Result<AppState, String> {
    let mut st = ctx.state.lock().unwrap();
    st.serials = serial::extract(&raw);
    st.raw = raw;
    st.index = 0;
    if st.serials.is_empty() {
        st.active = false;
    }
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
    st.settings = settings;
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

fn main() {
    tauri::Builder::default()
        .setup(|app| {
            let dir = app.path().app_data_dir().expect("無法取得資料夾");
            let path = dir.join("state.json");
            let st = AppState::load(&path);
            hook::ACTIVE.store(st.active, Ordering::SeqCst);
            // 重啟後還原：若上次是啟用中，把目前序號放回剪貼簿
            let _ = sync_clipboard(&st);

            let shared = Arc::new(Shared { state: Mutex::new(st), path });
            app.manage(shared.clone());

            let (tx, rx) = mpsc::channel();
            hook::start(tx);
            let handle = app.handle().clone();
            std::thread::spawn(move || paste_worker(handle, shared, rx));
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            get_state,
            extract,
            set_index,
            set_active,
            copy_text,
            clear_list,
            save_settings
        ])
        .run(tauri::generate_context!())
        .expect("啟動失敗");
}
