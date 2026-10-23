// 發行版不顯示主控台視窗
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod audio;
mod autostart;
mod avatar;
mod elevate;
mod fg;
mod input;
mod itemup;
mod maple_api;
mod patches;
mod notify;
mod scale;
mod update;
mod watch;
mod web;
mod winsys;
#[allow(dead_code)]
mod wz;
mod serial;
mod sha256;
mod timers;
mod toast;

use serial::{AppState, Settings};
use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::sync::{mpsc, Arc, Mutex};
use std::time::Duration;
use tauri::{AppHandle, Emitter, Manager, State, WebviewWindow, WindowEvent};

struct Shared {
    state: Mutex<AppState>,
    path: PathBuf,
}

type Ctx<'a> = State<'a, Arc<Shared>>;

/// 主視窗被使用者調整的大小（畫面像素），背景執行緒每秒寫入一次
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
    let (w, h) = (st.settings.view_w, st.settings.view_h);
    // 問過一次就不再問（前端沒送也保留）
    let asked = st.settings.close_asked || settings.close_asked;
    st.settings = Settings { view_w: w, view_h: h, close_asked: asked, ..settings };
    st.save(&ctx.path);
}

/// 第一次關閉時的選擇：記住後直接照選的做（縮小到右下角或結束程式）
#[tauri::command]
fn close_choice(app: AppHandle, ctx: Ctx, tray: bool) {
    {
        let mut st = ctx.state.lock().unwrap();
        st.settings.close_tray = tray;
        st.settings.close_asked = true;
        st.save(&ctx.path);
    }
    if tray {
        if let Some(w) = app.get_webview_window("main") {
            let _ = w.hide();
        }
    } else {
        app.exit(0);
    }
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

fn setup_main_window(app: &tauri::App, st: &AppState) {
    let Some(win) = app.get_webview_window("main") else { return };
    #[cfg(windows)]
    if let Ok(h) = win.hwnd() {
        fg::set_main_hwnd(h.0 as isize);
    }
    // 跟螢幕等比例：大小用畫面像素記錄（HD 和 4K 上看起來一樣）
    let saved = st.settings.view_w.zip(st.settings.view_h);
    scale::setup_main(&win, saved);
    let _ = win.center();
    // 開機自動啟動時只放在右下角，不打開主視窗
    if !autostart::launched_at_boot() {
        let _ = win.show();
        let _ = win.set_focus();
    }
}

/// Windows 11：設定標題列與邊框顏色，和程式內容的底色區分開（Windows 10 會忽略）
#[tauri::command]
fn set_frame(window: WebviewWindow, dark: bool, caption: Option<String>, border: Option<String>) {
    // "#rrggbb" → (r, g, b)
    let hex = |s: &Option<String>| -> Option<(u32, u32, u32)> {
        let t = s.as_deref()?.trim().trim_start_matches('#');
        if t.len() != 6 {
            return None;
        }
        let v = u32::from_str_radix(t, 16).ok()?;
        Some((v >> 16 & 0xFF, v >> 8 & 0xFF, v & 0xFF))
    };
    let (custom_caption, custom_border) = (hex(&caption), hex(&border));
    #[cfg(windows)]
    {
        use windows_sys::Win32::Graphics::Dwm::DwmSetWindowAttribute;
        const DWMWA_BORDER_COLOR: u32 = 34;
        const DWMWA_CAPTION_COLOR: u32 = 35;
        const DWMWA_TEXT_COLOR: u32 = 36;
        // COLORREF 是 0x00BBGGRR
        let rgb = |r: u32, g: u32, b: u32| (b << 16) | (g << 8) | r;
        let (mut caption, text, mut border) = if dark {
            (rgb(0x0f, 0x11, 0x16), rgb(0xe8, 0xec, 0xf3), rgb(0x3d, 0x5a, 0x99))
        } else {
            (rgb(0xd9, 0xe1, 0xec), rgb(0x1d, 0x24, 0x33), rgb(0x8f, 0xa6, 0xc8))
        };
        // 主題「角色色彩」：外框跟著角色的顏色
        if let Some((r, g, b)) = custom_caption {
            caption = rgb(r, g, b);
        }
        if let Some((r, g, b)) = custom_border {
            border = rgb(r, g, b);
        }
        if let Ok(h) = window.hwnd() {
            for (attr, color) in [(DWMWA_CAPTION_COLOR, caption), (DWMWA_TEXT_COLOR, text), (DWMWA_BORDER_COLOR, border)] {
                unsafe {
                    DwmSetWindowAttribute(h.0 as _, attr, &color as *const u32 as _, 4);
                }
            }
        }
    }
    #[cfg(not(windows))]
    let _ = (window, dark, custom_caption, custom_border);
}

/// 從右下角圖示叫出主視窗
fn show_main(app: &AppHandle) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.show();
        let _ = w.unminimize();
        let _ = w.set_focus();
    }
}

/// 右下角（系統匣）圖示：左鍵開啟，右鍵選單可以開啟或結束
fn setup_tray(app: &tauri::App) -> tauri::Result<()> {
    use tauri::menu::{Menu, MenuItem};
    use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
    let show = MenuItem::with_id(app, "show", "開啟工具箱", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "結束工具箱", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&show, &quit])?;
    let mut tray = TrayIconBuilder::with_id("main").tooltip("工具箱").menu(&menu).show_menu_on_left_click(false);
    if let Some(icon) = app.default_window_icon() {
        tray = tray.icon(icon.clone());
    }
    tray.on_menu_event(|app, e| match e.id.as_ref() {
        "show" => show_main(app),
        "quit" => app.exit(0),
        _ => {}
    })
    .on_tray_icon_event(|tray, e| {
        if let TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. } = e {
            show_main(tray.app_handle());
        }
    })
    .build(app)?;
    Ok(())
}

/// 自動更新後新版本會帶 --after-update=舊版本的程序編號 開啟：先等舊版本完全結束，
/// 否則「只能開一個」的檢查會以為程式已經開著而直接關掉新版本
fn wait_after_update() {
    let Some(pid) = std::env::args().find_map(|a| a.strip_prefix("--after-update=").and_then(|p| p.parse::<u32>().ok())) else { return };
    #[cfg(windows)]
    unsafe {
        use windows_sys::Win32::Foundation::CloseHandle;
        use windows_sys::Win32::System::Threading::{OpenProcess, WaitForSingleObject, PROCESS_SYNCHRONIZE};
        let h = OpenProcess(PROCESS_SYNCHRONIZE, 0, pid);
        if !h.is_null() {
            WaitForSingleObject(h, 15_000);
            CloseHandle(h);
        }
    }
    #[cfg(not(windows))]
    let _ = pid;
}

fn main() {
    // 發行版：沒有系統管理員權限時，用「以系統管理員身分執行」重新開一次（開機自動啟動也是）
    #[cfg(all(windows, not(debug_assertions)))]
    if elevate::relaunch_if_needed() {
        return;
    }
    wait_after_update();
    tauri::Builder::default()
        // 只能開一個：已經開著時再開一次，就把原本的視窗叫到最前面（開機自動啟動的不叫出來）
        .plugin(tauri_plugin_single_instance::init(|app, args, _cwd| {
            if !args.iter().any(|a| a == autostart::ARG) {
                show_main(app);
            }
        }))
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
            patches::init(dir.join("patches.json"));
            itemup::init(dir.join("item_updates.json"));
            timers::init(app.handle(), dir.join("timers.json"));
            update::init(dir.join("update.json"));
            toast::init(dir.clone());
            watch::init(app.handle(), dir.join("watch.json"));
            web::init(app.handle(), dir.join("web.json"));
            if let Err(e) = setup_tray(app) {
                eprintln!("無法建立右下角圖示：{e}");
            }
            std::thread::spawn(autostart::refresh);

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
                    st.settings.view_w = Some(w);
                    st.settings.view_h = Some(h);
                    st.save(&shared.path);
                }
            });
            Ok(())
        })
        // 跟螢幕等比例：每個頁面載入時套用縮放
        .on_page_load(|webview, _payload| {
            // web 浮動視窗：外部網頁與它的工具列不套用
            if webview.label().starts_with("web") {
                return;
            }
            scale::apply(webview);
        })
        .on_window_event(|window, event| {
            let label = window.label();
            if label == "main" {
                match event {
                    WindowEvent::Resized(size) => {
                        let normal = !window.is_minimized().unwrap_or(false) && !window.is_maximized().unwrap_or(false);
                        if normal {
                            if let Some(w) = window.app_handle().get_webview_window("main") {
                                if let Some((cw, ch)) = scale::main_resized(&w, *size) {
                                    if cw as f64 >= scale::MIN_W - 1.0 && ch as f64 >= scale::MIN_H - 1.0 {
                                        *PENDING_SIZE.lock().unwrap() = Some((cw, ch));
                                    }
                                }
                            }
                        }
                    }
                    // 換到別的螢幕：重新套用等比例縮放
                    WindowEvent::Moved(_) | WindowEvent::ScaleFactorChanged { .. } => scale::main_moved(window.app_handle()),
                    // 關閉主視窗：設定為縮小時藏到右下角；否則連同浮動視窗一起結束程式
                    WindowEvent::CloseRequested { api, .. } => {
                        let (to_tray, asked) = window
                            .app_handle()
                            .try_state::<Arc<Shared>>()
                            .map(|s| {
                                let st = s.state.lock().unwrap();
                                (st.settings.close_tray, st.settings.close_asked)
                            })
                            .unwrap_or((false, true));
                        if !asked {
                            // 第一次關閉：先問要直接關閉還是縮小到右下角
                            api.prevent_close();
                            let _ = window.emit("ask-close", ());
                        } else if to_tray {
                            api.prevent_close();
                            let _ = window.hide();
                        } else {
                            window.app_handle().exit(0);
                        }
                    }
                    _ => {}
                }
            } else if label == timers::OVERLAY_LABEL {
                match event {
                    WindowEvent::Moved(p) => timers::overlay_moved(p.x, p.y),
                    WindowEvent::Resized(s) => timers::overlay_resized(s.width, s.height),
                    _ => {}
                }
            }
            // 浮動視窗、通知換到別的螢幕：重新套用等比例縮放
            if label != "main" && !label.starts_with("web") && matches!(event, WindowEvent::Moved(_) | WindowEvent::ScaleFactorChanged { .. }) {
                if let Some(w) = window.app_handle().get_webview_window(label) {
                    scale::check(w.as_ref());
                }
            }
        })
        .invoke_handler(tauri::generate_handler![
            get_state,
            set_frame,
            extract,
            set_index,
            set_active,
            copy_text,
            clear_list,
            save_settings,
            close_choice,
            timers::get_timers,
            timers::update_timer,
            timers::update_prefs,
            timers::add_timer,
            timers::delete_timer,
            timers::timer_start,
            timers::timer_stop,
            timers::timer_adjust,
            timers::set_master,
            timers::set_game_only,
            timers::reset_overlay,
            timers::preview_sound,
            timers::pick_sound_file,
            maple_api::maple_lookup,
            patches::version_list,
            patches::version_hubs,
            patches::open_bulletin,
            patches::pool_page,
            maple_api::exp_track,
            avatar::beauty_render,
            avatar::beauty_tags,
            avatar::beauty_wz_status,
            avatar::pick_game_dir,
            avatar::beauty_items,
            avatar::beauty_icons,
            avatar::beauty_skins,
            avatar::beauty_styles,
            avatar::item_icons,
            avatar::item_icons_by_id,
            avatar::item_text,
            itemup::item_updates,
            notify::notify_pending,
            notify::notify_hide,
            notify::notify_open,
            notify::notify_test,
            notify::notify_show,
            watch::watch_get,
            watch::watch_set,
            watch::watch_clear_log,
            watch::watch_test,
            watch::watch_preview,
            watch::watch_discord_test,
            toast::toast_status,
            toast::open_notify_settings,
            update::update_info,
            update::update_check,
            update::update_install,
            autostart::autostart_get,
            autostart::autostart_set,
            web::web_list,
            web::web_add,
            web::web_open,
            web::web_close,
            web::web_delete,
            web::web_set,
            web::web_chrome_status,
            web::web_chrome_import,
            web::web_chrome_sync,
            web::web_bar_init,
            web::web_nav,
            web::web_typing,
            web::web_tab_new,
            web::web_tab_select,
            web::web_tab_close,
            web::web_tab_move,
            web::web_ext_list,
            web::web_ext_sync,
            web::web_win,
            web::web_bm_toggle,
            web::web_bm_open,
            web::web_bm_move,
            web::web_bm_rename,
            web::web_bm_delete,
            web::web_menu_open,
            web::web_menu_init,
            web::web_menu_pick
        ])
        .run(tauri::generate_context!())
        .expect("啟動失敗");
}
