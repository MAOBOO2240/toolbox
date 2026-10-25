//! 計時器核心：倒數、冷卻、超時、重複、通知，以及統一的浮動視窗。
//!
//! 計時在 Rust 背景執行緒進行（每 100ms 一次），主視窗縮小或在背景時也不會停。
//! 通知、音效、音量與浮動視窗是全局設定；每個計時器只保存自己的時間與重新計時方式。

use crate::{audio, fg, input};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, MutexGuard, OnceLock};
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter, Manager, PhysicalPosition, PhysicalSize, WebviewUrl, WebviewWindow, WebviewWindowBuilder};

const LONG_NOTIFY_INTERVAL: Duration = Duration::from_secs(3);
const TICK: Duration = Duration::from_millis(100);
/// 沒有計時器在用的時候改用這個間隔（總開關關著、或所有計時器都停用，而且沒有在倒數）
const IDLE_TICK: Duration = Duration::from_millis(1000);
pub const OVERLAY_LABEL: &str = "overlay";

/* ---------------- 設定（會保存） ---------------- */

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Overlay {
    pub show: bool,
    pub locked: bool,
    /// 底色不透明度 0.0 ~ 1.0（文字永遠不透明）
    pub bg: f64,
    pub x: i32,
    pub y: i32,
    pub w: u32,
    pub h: u32,
    /// 「全部關閉」前的顯示與鎖定，「全部開啟」時恢復
    pub saved_show: Option<bool>,
    pub saved_locked: Option<bool>,
}

impl Default for Overlay {
    fn default() -> Self {
        Self { show: false, locked: false, bg: 0.35, x: 120, y: 160, w: 340, h: 180, saved_show: None, saved_locked: None }
    }
}

/// 全局設定（設定面板）
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Prefs {
    /// "short" | "long"
    pub notify: String,
    /// "beep1" | "beep2" | "beep3" | "tts" | "file"
    pub sound: String,
    pub sound_file: String,
    /// 0 ~ 200（%）
    pub volume: u32,
    pub overlay: Overlay,
}

impl Default for Prefs {
    fn default() -> Self {
        Self {
            notify: "short".into(),
            sound: "beep1".into(),
            sound_file: String::new(),
            volume: 100,
            overlay: Overlay::default(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct TimerCfg {
    pub id: String,
    pub name: String,
    /// 程式內建的預設項目（現在也可以刪除）
    pub builtin: bool,
    pub duration_ms: u64,
    pub cooldown_ms: u64,
    /// "normal" | "count" | "key"
    pub mode: String,
    /// 次數模式的總輪數，0 = 無限
    pub repeat: u32,
    pub keys: Vec<String>,
    /// 超逾回補（只在指派按鍵模式生效）
    pub overflow_comp: bool,
    pub enabled: bool,
}

impl Default for TimerCfg {
    fn default() -> Self {
        Self {
            id: String::new(),
            name: "新計時器".into(),
            builtin: false,
            duration_ms: 60_000,
            cooldown_ms: 0,
            mode: "key".into(),
            repeat: 0,
            keys: Vec::new(),
            overflow_comp: false,
            enabled: true,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct TimersFile {
    /// 只在新楓之谷為前景時監聽按鍵、顯示浮動視窗
    pub game_only: bool,
    /// 總開關：關閉時所有計時器暫停，但不改動各自的啟用狀態
    pub master_on: bool,
    pub prefs: Prefs,
    pub next_id: u64,
    pub timers: Vec<TimerCfg>,
}

impl Default for TimersFile {
    fn default() -> Self {
        let mk = |id: &str, name: &str, ms: u64, cd: u64| TimerCfg {
            id: id.into(),
            name: name.into(),
            builtin: true,
            duration_ms: ms,
            cooldown_ms: cd,
            ..TimerCfg::default()
        };
        Self {
            game_only: true,
            master_on: true,
            prefs: Prefs::default(),
            next_id: 4,
            timers: vec![
                mk("t1", "輪迴碑石", 9 * 60_000 + 50_000, 0),
                mk("t2", "燃燒戒指", 15 * 60_000 + 5_000, 0),
                mk("t3", "靈魂亞努斯", 2 * 60_000, 50_000),
            ],
        }
    }
}

/* ---------------- 執行狀態（不保存） ---------------- */

#[derive(Debug, Clone, Copy, Default)]
enum Phase {
    #[default]
    Idle,
    Running { start: Instant, total: u64 },
    Done,
    Over { since: Instant },
}

#[derive(Debug, Default)]
struct Rt {
    phase: Phase,
    last_restart: Option<Instant>,
    round: u32,
    last_notify: Option<Instant>,
}

#[derive(Serialize, Clone)]
pub struct Snap {
    id: String,
    name: String,
    /// 總開關與自身都開啟
    active: bool,
    phase: &'static str,
    remain_ms: u64,
    over_ms: u64,
    total_ms: u64,
    cooldown_left_ms: u64,
    round: u32,
}

#[derive(Serialize, Clone)]
pub struct OverlayState {
    bg: f64,
    locked: bool,
}

#[derive(Serialize, Clone)]
pub struct TickPayload {
    fg: fg::FgInfo,
    listening: bool,
    last_key: String,
    master_on: bool,
    overlay: OverlayState,
    timers: Vec<Snap>,
}

struct Engine {
    file: TimersFile,
    rt: HashMap<String, Rt>,
    path: PathBuf,
    dirty: bool,
}

impl Engine {
    fn active(&self, cfg: &TimerCfg) -> bool {
        self.file.master_on && cfg.enabled
    }
}

static ENGINE: OnceLock<Mutex<Engine>> = OnceLock::new();
static GAME_ONLY: AtomicBool = AtomicBool::new(true);
static GAME_FG: AtomicBool = AtomicBool::new(false);

fn eng() -> MutexGuard<'static, Engine> {
    ENGINE.get().expect("計時器尚未初始化").lock().unwrap()
}

/// 計時器需不需要全域按鍵監聽：總開關開著，而且有啟用的「按鍵觸發」計時器（不管在哪一頁、有沒有正在計時都要聽，按鍵才能重新計時）
pub fn needs_hook() -> bool {
    let Some(lock) = ENGINE.get() else { return false };
    let Ok(e) = lock.try_lock() else { return true };
    e.file.master_on && e.file.timers.iter().any(|t| t.enabled && t.mode == "key" && !t.keys.is_empty())
}

fn load(path: &Path) -> TimersFile {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

fn save(path: &Path, file: &TimersFile) {
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let tmp = path.with_extension("json.tmp");
    if let Ok(json) = serde_json::to_string_pretty(file) {
        if std::fs::write(&tmp, json).is_ok() {
            let _ = std::fs::rename(&tmp, path);
        }
    }
}

/* ---------------- 計時邏輯 ---------------- */

fn ms(d: Duration) -> u64 {
    d.as_millis() as u64
}

/// 重新計時。指派按鍵模式且開啟超逾回補時，扣掉已超時的時間。
fn restart(cfg: &TimerCfg, rt: &mut Rt, now: Instant) {
    let dur = cfg.duration_ms.max(1000);
    let total = match rt.phase {
        Phase::Over { since } if cfg.mode == "key" && cfg.overflow_comp => {
            let over = ms(now.saturating_duration_since(since));
            if over < dur { dur - over } else { dur }
        }
        _ => dur,
    };
    rt.phase = Phase::Running { start: now, total };
    rt.last_restart = Some(now);
    rt.round = 1;
    rt.last_notify = None;
}

fn in_cooldown(cfg: &TimerCfg, rt: &Rt, now: Instant) -> bool {
    match rt.last_restart {
        Some(t) => ms(now.saturating_duration_since(t)) < cfg.cooldown_ms,
        None => false,
    }
}

/// 全域按鍵 / 滑鼠事件
pub fn on_input(key: &str, mods: u8) {
    let Some(lock) = ENGINE.get() else { return };
    let info = fg::current();
    // 在本程式視窗內操作不觸發；開啟「只在新楓之谷」時其他程式也不觸發
    if info.is_self || (GAME_ONLY.load(Ordering::Relaxed) && !info.is_game) {
        return;
    }
    let now = Instant::now();
    let mut e = lock.lock().unwrap();
    if !e.file.master_on {
        return;
    }
    let Engine { file, rt, .. } = &mut *e;
    for cfg in &file.timers {
        if !cfg.enabled || cfg.mode != "key" || !cfg.keys.iter().any(|b| input::matches(b, key, mods)) {
            continue;
        }
        let r = rt.entry(cfg.id.clone()).or_default();
        if in_cooldown(cfg, r, now) {
            continue;
        }
        restart(cfg, r, now);
    }
}

impl Engine {
    /// 推進計時，回傳需要通知的計時器名稱
    fn tick(&mut self, now: Instant) -> Vec<String> {
        let mut out = Vec::new();
        let master = self.file.master_on;
        let long = self.file.prefs.notify == "long";
        let Engine { file, rt, .. } = self;
        for cfg in &file.timers {
            let r = rt.entry(cfg.id.clone()).or_default();
            if !(master && cfg.enabled) {
                *r = Rt::default();
                continue;
            }
            match r.phase {
                Phase::Running { start, total } => {
                    let end = start + Duration::from_millis(total);
                    if now >= end {
                        out.push(cfg.name.clone());
                        r.last_notify = Some(now);
                        r.phase = match cfg.mode.as_str() {
                            "normal" => Phase::Done,
                            "count" => {
                                if cfg.repeat == 0 || r.round < cfg.repeat {
                                    r.round += 1;
                                    Phase::Running { start: end, total: cfg.duration_ms.max(1000) }
                                } else {
                                    Phase::Done
                                }
                            }
                            _ => Phase::Over { since: end },
                        };
                    }
                }
                Phase::Done | Phase::Over { .. } => {
                    // 長通知：持續提醒直到重新計時；沒有指派按鍵就只通知一聲
                    let due = r.last_notify.map_or(true, |t| now.saturating_duration_since(t) >= LONG_NOTIFY_INTERVAL);
                    if long && cfg.mode == "key" && !cfg.keys.is_empty() && due {
                        out.push(cfg.name.clone());
                        r.last_notify = Some(now);
                    }
                }
                Phase::Idle => {}
            }
        }
        out
    }

    fn snapshot(&self, now: Instant) -> Vec<Snap> {
        self.file
            .timers
            .iter()
            .map(|cfg| {
                let r = self.rt.get(&cfg.id);
                let phase = r.map(|r| r.phase).unwrap_or_default();
                let cd = r
                    .and_then(|r| r.last_restart)
                    .map(|t| cfg.cooldown_ms.saturating_sub(ms(now.saturating_duration_since(t))))
                    .unwrap_or(0);
                let (pname, remain, over, total) = match phase {
                    Phase::Idle => ("idle", cfg.duration_ms, 0, cfg.duration_ms),
                    Phase::Running { start, total } => {
                        ("running", total.saturating_sub(ms(now.saturating_duration_since(start))), 0, total)
                    }
                    Phase::Done => ("done", 0, 0, cfg.duration_ms),
                    Phase::Over { since } => ("over", 0, ms(now.saturating_duration_since(since)), cfg.duration_ms),
                };
                Snap {
                    id: cfg.id.clone(),
                    name: cfg.name.clone(),
                    active: self.active(cfg),
                    phase: pname,
                    remain_ms: remain,
                    over_ms: over,
                    total_ms: total,
                    cooldown_left_ms: if matches!(phase, Phase::Running { .. } | Phase::Over { .. }) { cd } else { 0 },
                    round: r.map(|r| r.round).unwrap_or(0),
                }
            })
            .collect()
    }
}

/* ---------------- 浮動視窗（全部計時器共用一個） ---------------- */

#[derive(Clone, PartialEq, Default)]
struct OvApplied {
    visible: bool,
    locked: bool,
}

/// 顯示（不搶走遊戲的焦點）或隱藏浮動視窗。
///
/// 顯示時直接呼叫 Win32 ShowWindow，Tauri 內部不知道視窗已顯示；
/// 所以隱藏也必須直接呼叫 ShowWindow，否則 Tauri 會以為本來就隱藏而略過。
fn set_overlay_visible(w: &WebviewWindow, visible: bool) {
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

/// 浮動視窗的 HWND（第一次查到就記住，之後每 0.1 秒核對時不用再問主執行緒）
static OVERLAY_HWND: std::sync::atomic::AtomicIsize = std::sync::atomic::AtomicIsize::new(0);

/// 浮動視窗實際上有沒有顯示（Windows 才查得到；查不到時回傳 None）
fn overlay_shown(w: &WebviewWindow) -> Option<bool> {
    #[cfg(windows)]
    {
        use windows_sys::Win32::UI::WindowsAndMessaging::{IsWindow, IsWindowVisible};
        let mut h = OVERLAY_HWND.load(Ordering::Relaxed);
        if h == 0 || unsafe { IsWindow(h as _) } == 0 {
            h = w.hwnd().map(|x| x.0 as isize).unwrap_or(0);
            OVERLAY_HWND.store(h, Ordering::Relaxed);
        }
        if h != 0 {
            return Some(unsafe { IsWindowVisible(h as _) } != 0);
        }
    }
    let _ = w;
    None
}

/// 無框視窗其實還帶著系統標題列與邊框的樣式，拖曳、切換鎖定或被系統重畫時，
/// Windows 會把舊式的標題列與邊框畫在浮動視窗上。這裡直接攔下「畫標題列／邊框」的訊息，整個視窗都當作內容區
#[cfg(windows)]
unsafe extern "system" fn overlay_subclass(
    hwnd: windows_sys::Win32::Foundation::HWND,
    msg: u32,
    wp: windows_sys::Win32::Foundation::WPARAM,
    lp: windows_sys::Win32::Foundation::LPARAM,
    _id: usize,
    _data: usize,
) -> windows_sys::Win32::Foundation::LRESULT {
    use windows_sys::Win32::UI::Shell::DefSubclassProc;
    use windows_sys::Win32::UI::WindowsAndMessaging::{WM_NCACTIVATE, WM_NCCALCSIZE, WM_NCPAINT};
    match msg {
        // 不畫標題列與邊框（0xAE、0xAF 是佈景主題直接畫標題列與邊框用的訊息）
        WM_NCPAINT | 0x00AE | 0x00AF => 0,
        // 啟用狀態改變時不重畫標題列（回傳 TRUE 讓系統照常切換）
        WM_NCACTIVATE => 1,
        // 整個視窗都是內容區，沒有標題列與邊框的空間
        WM_NCCALCSIZE if wp != 0 => 0,
        _ => unsafe { DefSubclassProc(hwnd, msg, wp, lp) },
    }
}

/// 在主執行緒（視窗所屬的執行緒）裝上上面的攔截（通知視窗也用）
pub(crate) fn install_overlay_subclass(app: &AppHandle, w: &WebviewWindow) {
    #[cfg(windows)]
    if let Ok(h) = w.hwnd() {
        let h = h.0 as isize;
        let _ = app.run_on_main_thread(move || unsafe {
            windows_sys::Win32::UI::Shell::SetWindowSubclass(h as _, Some(overlay_subclass), 0x4D42_4F56, 0);
        });
    }
    #[cfg(not(windows))]
    let _ = (app, w);
}

fn create_overlay(app: &AppHandle, o: &Overlay) -> Result<WebviewWindow, String> {
    let w = WebviewWindowBuilder::new(app, OVERLAY_LABEL, WebviewUrl::App("overlay.html".into()))
        .title("計時器浮動視窗")
        .decorations(false)
        .transparent(true)
        .shadow(false)
        .always_on_top(true)
        .skip_taskbar(true)
        .resizable(true)
        .focused(false)
        // 點擊、拖曳都不會搶走遊戲的焦點；也不會因為被啟用而畫出系統標題列
        .focusable(false)
        .visible(false)
        .min_inner_size(140.0, 50.0)
        .build()
        .map_err(|e| format!("浮動視窗建立失敗：{e}"))?;
    install_overlay_subclass(app, &w);
    let _ = w.set_size(PhysicalSize::new(o.w.max(140), o.h.max(50)));
    let _ = w.set_position(PhysicalPosition::new(o.x, o.y));
    Ok(w)
}

/// 浮動視窗跑到所有螢幕外面（例如換過螢幕、螢幕解析度變了）時，移回主螢幕左上附近
fn ensure_on_screen(win: &WebviewWindow) {
    let (Ok(pos), Ok(size)) = (win.outer_position(), win.outer_size()) else { return };
    let Ok(monitors) = win.available_monitors() else { return };
    if monitors.is_empty() {
        return;
    }
    // 至少有 60×30 像素在某個螢幕裡才算看得到
    let seen = monitors.iter().any(|m| {
        let (mp, ms) = (m.position(), m.size());
        let x1 = pos.x.max(mp.x);
        let y1 = pos.y.max(mp.y);
        let x2 = (pos.x + size.width as i32).min(mp.x + ms.width as i32);
        let y2 = (pos.y + size.height as i32).min(mp.y + ms.height as i32);
        x2 - x1 >= 60 && y2 - y1 >= 30
    });
    if seen {
        return;
    }
    let base = win.primary_monitor().ok().flatten().map(|m| *m.position()).unwrap_or(PhysicalPosition::new(0, 0));
    let (x, y) = (base.x + 120, base.y + 160);
    let _ = win.set_position(PhysicalPosition::new(x, y));
    let mut e = eng();
    e.file.prefs.overlay.x = x;
    e.file.prefs.overlay.y = y;
    e.dirty = true;
}

/// 建立失敗時的錯誤（同一個錯誤只通知一次，5 秒內不重試）
static OVERLAY_ERR: Mutex<Option<(String, Instant)>> = Mutex::new(None);

fn sync_overlay(app: &AppHandle, applied: &mut Option<OvApplied>, want: OvApplied, o: &Overlay) {
    let win = match app.get_webview_window(OVERLAY_LABEL) {
        Some(w) => w,
        None if want.visible => {
            {
                let last = OVERLAY_ERR.lock().unwrap_or_else(|e| e.into_inner());
                if last.as_ref().is_some_and(|(_, t)| t.elapsed() < Duration::from_secs(5)) {
                    return;
                }
            }
            match create_overlay(app, o) {
                Ok(w) => {
                    *OVERLAY_ERR.lock().unwrap_or_else(|e| e.into_inner()) = None;
                    *applied = None;
                    w
                }
                Err(msg) => {
                    let mut last = OVERLAY_ERR.lock().unwrap_or_else(|e| e.into_inner());
                    if last.as_ref().map(|(m, _)| m != &msg).unwrap_or(true) {
                        let _ = app.emit("overlay-error", msg.clone());
                    }
                    *last = Some((msg, Instant::now()));
                    return;
                }
            }
        }
        None => return,
    };
    let prev = applied.clone();
    let was_visible = prev.as_ref().map(|p| p.visible);
    if prev.as_ref().map(|p| p.locked) != Some(want.locked) {
        let _ = win.set_ignore_cursor_events(want.locked);
    }
    // 切換鎖定（滑鼠穿透）時 Tauri 會重設視窗樣式，順便把視窗藏起來（它不知道我們自己顯示過）；
    // 所以每次都核對實際有沒有顯示，不一樣就補回來
    let mismatch = match overlay_shown(&win) {
        Some(shown) => shown != want.visible,
        None => was_visible != Some(want.visible),
    };
    if mismatch {
        if want.visible && was_visible != Some(true) {
            ensure_on_screen(&win);
        }
        set_overlay_visible(&win, want.visible);
    }
    *applied = Some(want);
}

/// 浮動視窗被拖曳或縮放時記住位置大小
pub fn overlay_moved(x: i32, y: i32) {
    if x <= -10000 || y <= -10000 {
        return; // 最小化等特殊座標
    }
    if let Some(lock) = ENGINE.get() {
        let mut e = lock.lock().unwrap();
        let o = &mut e.file.prefs.overlay;
        if (o.x, o.y) != (x, y) {
            o.x = x;
            o.y = y;
            e.dirty = true;
        }
    }
}

pub fn overlay_resized(w: u32, h: u32) {
    if w == 0 || h == 0 {
        return;
    }
    if let Some(lock) = ENGINE.get() {
        let mut e = lock.lock().unwrap();
        let o = &mut e.file.prefs.overlay;
        if (o.w, o.h) != (w, h) {
            o.w = w;
            o.h = h;
            e.dirty = true;
        }
    }
}

/* ---------------- 背景執行緒 ---------------- */

pub fn init(app: &AppHandle, path: PathBuf) {
    let mut file = load(&path);
    // 每次開啟程式，浮動視窗的顯示開關預設為關閉
    file.prefs.overlay.show = false;
    GAME_ONLY.store(file.game_only, Ordering::SeqCst);
    let _ = ENGINE.set(Mutex::new(Engine { file, rt: HashMap::new(), path, dirty: false }));

    let app = app.clone();
    std::thread::spawn(move || {
        let mut applied: Option<OvApplied> = None;
        let mut last_save = Instant::now();
        let mut wait = TICK;
        loop {
            std::thread::sleep(wait);
            crate::input::refresh();
            let now = Instant::now();
            let info = fg::current();
            // 自己的浮動視窗在前景（例如正在拖曳）時，維持原本的遊戲判定
            if !info.is_self || info.is_main {
                GAME_FG.store(info.is_game, Ordering::SeqCst);
            }
            let game_fg = GAME_FG.load(Ordering::SeqCst);
            let game_only = GAME_ONLY.load(Ordering::SeqCst);

            // 鎖內只做計算；視窗操作與音效都在解鎖後進行，避免與主執行緒互相等待
            let (names, snaps, prefs, master, want, to_save, any_running, any_enabled) = {
                let mut e = eng();
                let names = e.tick(now);
                let snaps = e.snapshot(now);
                let prefs = e.file.prefs.clone();
                let master = e.file.master_on;
                // 有計時器正在倒數或超時才顯示浮動視窗
                let any_running = snaps.iter().any(|s| s.active && matches!(s.phase, "running" | "over"));
                let o = &prefs.overlay;
                // 只在新楓之谷時：遊戲或本程式在前景才顯示，其他程式在前景就隱藏
                let in_place = !game_only || game_fg || info.is_self;
                let want = OvApplied { visible: o.show && master && any_running && in_place, locked: o.locked };
                let to_save = if e.dirty && now.duration_since(last_save) >= Duration::from_millis(800) {
                    e.dirty = false;
                    last_save = now;
                    Some((e.path.clone(), e.file.clone()))
                } else {
                    None
                };
                let any_enabled = e.file.timers.iter().any(|t| t.enabled);
                (names, snaps, prefs, master, want, to_save, any_running, any_enabled)
            };

            // 時間到只播音效，不跳通知視窗
            for name in names {
                audio::play(&prefs.sound, &prefs.sound_file, &name, prefs.volume);
            }
            sync_overlay(&app, &mut applied, want, &prefs.overlay);
            let listening = master && !info.is_self && (!game_only || info.is_game);
            // 有計時器在跑：每 0.1 秒更新；沒有在跑而且主視窗看不到（縮在右下角或最小化）時不更新畫面
            if any_running || fg::main_shown() {
                let _ = app.emit(
                    "timer-tick",
                    TickPayload {
                        fg: info,
                        listening,
                        last_key: input::last_key(),
                        master_on: master,
                        overlay: OverlayState { bg: prefs.overlay.bg.clamp(0.0, 1.0), locked: prefs.overlay.locked },
                        timers: snaps,
                    },
                );
            }
            if let Some((path, file)) = to_save {
                save(&path, &file);
            }
            // 沒有計時器在倒數，而且總開關關著或沒有啟用的計時器：放慢檢查（主視窗打開著時維持原本的流暢度）
            let idle = !any_running && (!master || !any_enabled);
            wait = if idle && !fg::main_shown() { IDLE_TICK } else { TICK };
        }
    });
}

/* ---------------- 指令 ---------------- */

#[tauri::command]
pub fn get_timers() -> TimersFile {
    eng().file.clone()
}

/// 更新單一計時器設定
#[tauri::command]
pub fn update_timer(timer: TimerCfg) {
    let mut e = eng();
    let Engine { file, rt, .. } = &mut *e;
    if let Some(t) = file.timers.iter_mut().find(|t| t.id == timer.id) {
        let dur_changed = t.duration_ms != timer.duration_ms;
        *t = TimerCfg { builtin: t.builtin, ..timer };
        // 關掉開關：立刻停止計時（浮動視窗也不再顯示）
        if !t.enabled {
            rt.remove(&t.id);
        }
        // 倒數中修改時間：已經過的時間保留，以新時間繼續倒數
        if dur_changed {
            if let Some(r) = rt.get_mut(&t.id) {
                if let Phase::Running { start, .. } = r.phase {
                    r.phase = Phase::Running { start, total: t.duration_ms.max(1000) };
                }
            }
        }
    }
    e.dirty = true;
}

/// 更新全局設定（浮動視窗的位置大小以後端為準）
#[tauri::command]
pub fn update_prefs(prefs: Prefs) {
    let mut e = eng();
    let o = &e.file.prefs.overlay;
    let geom = (o.x, o.y, o.w, o.h);
    let saved = (o.saved_show, o.saved_locked);
    e.file.prefs = Prefs { volume: prefs.volume.min(200), ..prefs };
    let o = &mut e.file.prefs.overlay;
    (o.x, o.y, o.w, o.h) = geom;
    (o.saved_show, o.saved_locked) = saved;
    o.bg = o.bg.clamp(0.0, 1.0);
    e.dirty = true;
}

#[tauri::command]
pub fn add_timer() -> TimerCfg {
    let mut e = eng();
    let id = format!("u{}", e.file.next_id);
    e.file.next_id += 1;
    let t = TimerCfg { id, ..TimerCfg::default() };
    e.file.timers.push(t.clone());
    e.dirty = true;
    t
}

#[tauri::command]
pub fn delete_timer(id: String) {
    let mut e = eng();
    e.file.timers.retain(|t| t.id != id);
    e.rt.remove(&id);
    e.dirty = true;
}

/// 「開始」按鈕：不受冷卻限制
#[tauri::command]
pub fn timer_start(id: String) {
    let now = Instant::now();
    let mut e = eng();
    let Engine { file, rt, .. } = &mut *e;
    if let Some(cfg) = file.timers.iter_mut().find(|t| t.id == id) {
        cfg.enabled = true;
        file.master_on = true;
        restart(cfg, rt.entry(id).or_default(), now);
    }
    e.dirty = true;
}

/// 滾輪調整正在倒數的計時器：只改這一次剩下的時間，設定的時間不變
#[tauri::command]
pub fn timer_adjust(id: String, delta_ms: i64) {
    let now = Instant::now();
    let mut e = eng();
    if let Some(r) = e.rt.get_mut(&id) {
        if let Phase::Running { start, total } = r.phase {
            r.phase = adjust_running(start, total, delta_ms, now);
        }
    }
}

fn adjust_running(start: Instant, total: u64, delta_ms: i64, now: Instant) -> Phase {
    let elapsed = ms(now.saturating_duration_since(start));
    let remain = total.saturating_sub(elapsed) as i64;
    let next = (remain + delta_ms).clamp(1000, (999 * 60 + 59) * 1000) as u64;
    Phase::Running { start, total: elapsed + next }
}

#[tauri::command]
pub fn timer_stop(id: String) {
    let mut e = eng();
    if let Some(r) = e.rt.get_mut(&id) {
        *r = Rt::default();
    }
}

/// 總開關：只影響原本有開啟的計時器（各自的啟用狀態不變），並連動浮動視窗的顯示開關
#[tauri::command]
pub fn set_master(on: bool) {
    let mut e = eng();
    e.file.master_on = on;
    let o = &mut e.file.prefs.overlay;
    if on {
        // 全部開啟：浮動視窗的顯示與鎖定恢復成關閉前的樣子
        o.show = o.saved_show.unwrap_or(true);
        o.locked = o.saved_locked.unwrap_or(o.locked);
    } else {
        o.saved_show = Some(o.show);
        o.saved_locked = Some(o.locked);
        o.show = false;
        o.locked = false;
    }
    if !on {
        e.rt.clear();
    }
    e.dirty = true;
}

#[tauri::command]
pub fn set_game_only(on: bool) {
    GAME_ONLY.store(on, Ordering::SeqCst);
    let mut e = eng();
    e.file.game_only = on;
    e.dirty = true;
}

/// 把浮動視窗移回主螢幕左上附近
#[tauri::command]
pub fn reset_overlay(app: AppHandle) {
    let (x, y) = (120, 160);
    {
        let mut e = eng();
        e.file.prefs.overlay.x = x;
        e.file.prefs.overlay.y = y;
        e.dirty = true;
    }
    if let Some(w) = app.get_webview_window(OVERLAY_LABEL) {
        let _ = w.set_position(PhysicalPosition::new(x, y));
    }
}

/// 試聽：用目前的音效與音量設定
#[tauri::command]
pub fn preview_sound(prefs: Prefs, name: String) {
    audio::play(&prefs.sound, &prefs.sound_file, &name, prefs.volume);
}

#[tauri::command]
pub async fn pick_sound_file() -> Option<String> {
    rfd::FileDialog::new()
        .set_title("選擇音效檔")
        .add_filter("音效檔", &["wav", "mp3", "ogg", "flac"])
        .pick_file()
        .map(|p| p.to_string_lossy().into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg(mode: &str, comp: bool) -> TimerCfg {
        TimerCfg { duration_ms: 10_000, mode: mode.into(), overflow_comp: comp, ..TimerCfg::default() }
    }

    #[test]
    fn overflow_compensation() {
        let t0 = Instant::now();
        let c = cfg("key", true);
        let mut r = Rt { phase: Phase::Over { since: t0 }, ..Rt::default() };
        restart(&c, &mut r, t0 + Duration::from_secs(3));
        assert!(matches!(r.phase, Phase::Running { total: 7_000, .. }));

        // 超時 ≥ 原本時間 → 用完整時間
        let mut r = Rt { phase: Phase::Over { since: t0 }, ..Rt::default() };
        restart(&c, &mut r, t0 + Duration::from_secs(12));
        assert!(matches!(r.phase, Phase::Running { total: 10_000, .. }));

        // 關閉回補 → 完整時間
        let mut r = Rt { phase: Phase::Over { since: t0 }, ..Rt::default() };
        restart(&cfg("key", false), &mut r, t0 + Duration::from_secs(3));
        assert!(matches!(r.phase, Phase::Running { total: 10_000, .. }));
    }

    #[test]
    fn cooldown() {
        let t0 = Instant::now();
        let c = TimerCfg { cooldown_ms: 50_000, ..cfg("key", false) };
        let r = Rt { last_restart: Some(t0), ..Rt::default() };
        assert!(in_cooldown(&c, &r, t0 + Duration::from_secs(10)));
        assert!(!in_cooldown(&c, &r, t0 + Duration::from_secs(51)));
    }

    fn engine(mode: &str, repeat: u32, t0: Instant) -> Engine {
        let c = TimerCfg { id: "a".into(), repeat, ..cfg(mode, false) };
        let mut rt = HashMap::new();
        let mut r = Rt::default();
        restart(&c, &mut r, t0);
        rt.insert("a".to_string(), r);
        Engine { file: TimersFile { timers: vec![c], ..TimersFile::default() }, rt, path: PathBuf::new(), dirty: false }
    }

    #[test]
    fn modes_at_timeout() {
        let t0 = Instant::now();
        let later = t0 + Duration::from_millis(10_050);

        let mut e = engine("normal", 0, t0);
        assert_eq!(e.tick(later).len(), 1);
        assert!(matches!(e.rt["a"].phase, Phase::Done));

        let mut e = engine("key", 0, t0);
        e.tick(later);
        assert!(matches!(e.rt["a"].phase, Phase::Over { .. }));

        let mut e = engine("count", 2, t0);
        e.tick(later);
        assert!(matches!(e.rt["a"].phase, Phase::Running { .. }));
        assert_eq!(e.rt["a"].round, 2);
        e.tick(t0 + Duration::from_millis(20_100));
        assert!(matches!(e.rt["a"].phase, Phase::Done));

        let mut e = engine("count", 0, t0); // 無限
        e.tick(later);
        e.tick(t0 + Duration::from_millis(20_100));
        assert!(matches!(e.rt["a"].phase, Phase::Running { .. }));
    }

    #[test]
    fn master_switch_pauses_without_touching_enabled() {
        let t0 = Instant::now();
        let mut e = engine("key", 0, t0);
        e.file.master_on = false;
        e.tick(t0 + Duration::from_secs(1));
        assert!(matches!(e.rt["a"].phase, Phase::Idle));
        assert!(e.file.timers[0].enabled);
    }

    #[test]
    fn adjust_only_changes_current_run() {
        let t0 = Instant::now();
        let now = t0 + Duration::from_millis(30_000);
        // 設定 5 分鐘、已經過 30 秒 → 剩 4:30；+1 分鐘 → 剩 5:30
        let Phase::Running { total, .. } = adjust_running(t0, 300_000, 60_000, now) else { panic!() };
        assert_eq!(total - 30_000, 330_000);
        // -1 秒
        let Phase::Running { total, .. } = adjust_running(t0, 300_000, -1000, now) else { panic!() };
        assert_eq!(total - 30_000, 269_000);
        // 不會少於 1 秒
        let Phase::Running { total, .. } = adjust_running(t0, 300_000, -600_000, now) else { panic!() };
        assert_eq!(total - 30_000, 1000);
    }

    #[test]
    fn old_file_format_still_loads() {
        // 舊版每個計時器各自有音效與浮動視窗設定，新版應能忽略並讀入
        let old = r#"{"game_only":false,"next_id":5,"timers":[{"id":"t1","name":"輪迴碑石","duration_ms":590000,"sound":"beep1","overlay":{"show":true}}]}"#;
        let f: TimersFile = serde_json::from_str(old).unwrap();
        assert!(!f.game_only);
        assert!(f.master_on);
        assert_eq!(f.timers[0].duration_ms, 590_000);
        assert_eq!(f.prefs.volume, 100);
    }
}
