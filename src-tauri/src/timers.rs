//! 計時器核心：倒數、冷卻、超時、重複、通知，以及每個計時器的浮動視窗。
//!
//! 計時在 Rust 背景執行緒進行（每 100ms 一次），主視窗縮小或在背景時也不會停。

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

/* ---------------- 設定（會保存） ---------------- */

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Overlay {
    pub show: bool,
    pub locked: bool,
    /// 0.2 ~ 1.0
    pub opacity: f64,
    pub x: i32,
    pub y: i32,
    pub w: u32,
    pub h: u32,
}

impl Default for Overlay {
    fn default() -> Self {
        Self { show: true, locked: false, opacity: 0.9, x: 120, y: 160, w: 300, h: 110 }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct TimerCfg {
    pub id: String,
    pub name: String,
    /// 預設項目（不可刪除）
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
    /// "short" | "long"
    pub notify: String,
    /// "beep1" | "beep2" | "beep3" | "tts" | "file"
    pub sound: String,
    pub sound_file: String,
    pub enabled: bool,
    pub overlay: Overlay,
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
            notify: "short".into(),
            sound: "tts".into(),
            sound_file: String::new(),
            enabled: true,
            overlay: Overlay::default(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct TimersFile {
    /// 只在新楓之谷為前景時監聽按鍵、顯示浮動視窗
    pub game_only: bool,
    pub next_id: u64,
    pub timers: Vec<TimerCfg>,
}

impl Default for TimersFile {
    fn default() -> Self {
        let mk = |id: &str, name: &str, ms: u64, cd: u64, sound: &str, i: i32| TimerCfg {
            id: id.into(),
            name: name.into(),
            builtin: true,
            duration_ms: ms,
            cooldown_ms: cd,
            sound: sound.into(),
            overlay: Overlay { y: 160 + i * 130, ..Overlay::default() },
            ..TimerCfg::default()
        };
        Self {
            game_only: true,
            next_id: 4,
            timers: vec![
                mk("t1", "輪迴碑石", 9 * 60_000 + 50_000, 0, "beep1", 0),
                mk("t2", "燃燒戒指", 15 * 60_000 + 5_000, 0, "beep2", 1),
                mk("t3", "靈魂亞努斯", 2 * 60_000, 50_000, "beep3", 2),
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
    enabled: bool,
    opacity: f64,
    locked: bool,
    phase: &'static str,
    remain_ms: u64,
    over_ms: u64,
    total_ms: u64,
    cooldown_left_ms: u64,
    round: u32,
}

#[derive(Serialize, Clone)]
pub struct TickPayload {
    fg: fg::FgInfo,
    listening: bool,
    timers: Vec<Snap>,
}

struct Engine {
    file: TimersFile,
    rt: HashMap<String, Rt>,
    path: PathBuf,
    dirty: bool,
}

static ENGINE: OnceLock<Mutex<Engine>> = OnceLock::new();
static GAME_ONLY: AtomicBool = AtomicBool::new(true);
static GAME_FG: AtomicBool = AtomicBool::new(false);

fn eng() -> MutexGuard<'static, Engine> {
    ENGINE.get().expect("計時器尚未初始化").lock().unwrap()
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
    // 在本程式主視窗內打字不觸發；開啟「只在新楓之谷」時其他程式也不觸發
    if info.is_main || (GAME_ONLY.load(Ordering::Relaxed) && !info.is_game) {
        return;
    }
    let now = Instant::now();
    let mut e = lock.lock().unwrap();
    let Engine { file, rt, .. } = &mut *e;
    for cfg in &file.timers {
        if !cfg.enabled || !cfg.keys.iter().any(|b| input::matches(b, key, mods)) {
            continue;
        }
        let r = rt.entry(cfg.id.clone()).or_default();
        if in_cooldown(cfg, r, now) {
            continue;
        }
        restart(cfg, r, now);
    }
}

struct Notice {
    sound: String,
    file: String,
    name: String,
}

impl Engine {
    fn tick(&mut self, now: Instant) -> Vec<Notice> {
        let mut out = Vec::new();
        let Engine { file, rt, .. } = self;
        for cfg in &file.timers {
            let r = rt.entry(cfg.id.clone()).or_default();
            if !cfg.enabled {
                r.phase = Phase::Idle;
                continue;
            }
            let notice = || Notice { sound: cfg.sound.clone(), file: cfg.sound_file.clone(), name: cfg.name.clone() };
            match r.phase {
                Phase::Running { start, total } => {
                    let end = start + Duration::from_millis(total);
                    if now >= end {
                        out.push(notice());
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
                    // 長通知：持續提醒直到重新計時；沒有指派按鍵就只通知一次
                    let due = r.last_notify.map_or(true, |t| now.saturating_duration_since(t) >= LONG_NOTIFY_INTERVAL);
                    if cfg.notify == "long" && !cfg.keys.is_empty() && due {
                        out.push(notice());
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
                let (name, remain, over, total) = match phase {
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
                    enabled: cfg.enabled,
                    opacity: cfg.overlay.opacity.clamp(0.2, 1.0),
                    locked: cfg.overlay.locked,
                    phase: name,
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

/* ---------------- 浮動視窗 ---------------- */

#[derive(Clone, PartialEq)]
struct OvState {
    visible: bool,
    locked: bool,
}

fn overlay_label(id: &str) -> String {
    format!("ov-{id}")
}

/// 顯示但不搶走遊戲的焦點
fn show_no_activate(w: &WebviewWindow) {
    #[cfg(windows)]
    {
        use windows_sys::Win32::UI::WindowsAndMessaging::{ShowWindow, SW_SHOWNOACTIVATE};
        if let Ok(h) = w.hwnd() {
            unsafe {
                ShowWindow(h.0 as _, SW_SHOWNOACTIVATE);
            }
            return;
        }
    }
    let _ = w.show();
}

fn create_overlay(app: &AppHandle, cfg: &TimerCfg) -> Option<WebviewWindow> {
    let w = WebviewWindowBuilder::new(app, overlay_label(&cfg.id), WebviewUrl::App("overlay.html".into()))
        .title(format!("{} 浮動視窗", cfg.name))
        .decorations(false)
        .transparent(true)
        .shadow(false)
        .always_on_top(true)
        .skip_taskbar(true)
        .resizable(true)
        .focused(false)
        .visible(false)
        .min_inner_size(120.0, 50.0)
        .build()
        .ok()?;
    let o = &cfg.overlay;
    let _ = w.set_size(PhysicalSize::new(o.w.max(120), o.h.max(50)));
    let _ = w.set_position(PhysicalPosition::new(o.x, o.y));
    Some(w)
}

fn sync_overlays(app: &AppHandle, applied: &mut HashMap<String, OvState>, desired: Vec<(TimerCfg, OvState)>) {
    let keep: Vec<String> = desired.iter().map(|(c, _)| c.id.clone()).collect();
    // 已刪除的計時器 → 關閉視窗
    applied.retain(|id, _| {
        if keep.contains(id) {
            true
        } else {
            if let Some(w) = app.get_webview_window(&overlay_label(id)) {
                let _ = w.destroy();
            }
            false
        }
    });
    for (cfg, want) in desired {
        let label = overlay_label(&cfg.id);
        let existing = app.get_webview_window(&label);
        let win = match existing {
            Some(w) => w,
            None if want.visible => match create_overlay(app, &cfg) {
                Some(w) => {
                    applied.remove(&cfg.id);
                    w
                }
                None => continue,
            },
            None => continue,
        };
        let prev = applied.get(&cfg.id).cloned();
        if prev.as_ref().map(|p| p.locked) != Some(want.locked) {
            let _ = win.set_ignore_cursor_events(want.locked);
        }
        if prev.as_ref().map(|p| p.visible) != Some(want.visible) {
            if want.visible {
                show_no_activate(&win);
            } else {
                let _ = win.hide();
            }
        }
        applied.insert(cfg.id.clone(), want);
    }
}

/// 浮動視窗被拖曳或縮放時記住位置大小
pub fn overlay_moved(id: &str, x: i32, y: i32) {
    if let Some(lock) = ENGINE.get() {
        let mut e = lock.lock().unwrap();
        if let Some(t) = e.file.timers.iter_mut().find(|t| t.id == id) {
            if (t.overlay.x, t.overlay.y) != (x, y) && x > -10000 && y > -10000 {
                t.overlay.x = x;
                t.overlay.y = y;
                e.dirty = true;
            }
        }
    }
}

pub fn overlay_resized(id: &str, w: u32, h: u32) {
    if w == 0 || h == 0 {
        return;
    }
    if let Some(lock) = ENGINE.get() {
        let mut e = lock.lock().unwrap();
        if let Some(t) = e.file.timers.iter_mut().find(|t| t.id == id) {
            if (t.overlay.w, t.overlay.h) != (w, h) {
                t.overlay.w = w;
                t.overlay.h = h;
                e.dirty = true;
            }
        }
    }
}

/* ---------------- 背景執行緒 ---------------- */

pub fn init(app: &AppHandle, path: PathBuf) {
    let file = load(&path);
    GAME_ONLY.store(file.game_only, Ordering::SeqCst);
    let _ = ENGINE.set(Mutex::new(Engine { file, rt: HashMap::new(), path, dirty: false }));

    let app = app.clone();
    std::thread::spawn(move || {
        let mut applied: HashMap<String, OvState> = HashMap::new();
        let mut last_save = Instant::now();
        loop {
            std::thread::sleep(TICK);
            let now = Instant::now();
            let info = fg::current();
            // 自己的浮動視窗在前景（例如正在拖曳）時，維持原本的遊戲判定
            if !info.is_self {
                GAME_FG.store(info.is_game, Ordering::SeqCst);
            }
            let game_fg = GAME_FG.load(Ordering::SeqCst);
            let game_only = GAME_ONLY.load(Ordering::SeqCst);

            // 鎖內只做計算，視窗操作與音效都在解鎖後進行，避免與主執行緒互相等待
            let (notices, snaps, desired, to_save) = {
                let mut e = eng();
                let notices = e.tick(now);
                let snaps = e.snapshot(now);
                let desired: Vec<(TimerCfg, OvState)> = e
                    .file
                    .timers
                    .iter()
                    .map(|c| {
                        // 未鎖定時一律顯示，方便調整位置；鎖定後才依遊戲前景顯示
                        let visible = c.enabled && c.overlay.show && (!game_only || game_fg || !c.overlay.locked);
                        (c.clone(), OvState { visible, locked: c.overlay.locked })
                    })
                    .collect();
                let to_save = if e.dirty && now.duration_since(last_save) >= Duration::from_secs(1) {
                    e.dirty = false;
                    last_save = now;
                    Some((e.path.clone(), e.file.clone()))
                } else {
                    None
                };
                (notices, snaps, desired, to_save)
            };

            for n in notices {
                audio::play(&n.sound, &n.file, &n.name);
            }
            sync_overlays(&app, &mut applied, desired);
            let listening = !info.is_main && (!game_only || info.is_game);
            let _ = app.emit("timer-tick", TickPayload { fg: info, listening, timers: snaps });
            if let Some((path, file)) = to_save {
                save(&path, &file);
            }
        }
    });
}

/* ---------------- 指令 ---------------- */

fn mark_dirty(e: &mut Engine) {
    e.dirty = true;
}

#[tauri::command]
pub fn get_timers() -> TimersFile {
    eng().file.clone()
}

/// 更新單一計時器設定（浮動視窗的位置大小以後端為準）
#[tauri::command]
pub fn update_timer(timer: TimerCfg) {
    let mut e = eng();
    let Engine { file, rt, .. } = &mut *e;
    if let Some(t) = file.timers.iter_mut().find(|t| t.id == timer.id) {
        let geom = (t.overlay.x, t.overlay.y, t.overlay.w, t.overlay.h);
        let dur_changed = t.duration_ms != timer.duration_ms;
        *t = TimerCfg { builtin: t.builtin, ..timer };
        (t.overlay.x, t.overlay.y, t.overlay.w, t.overlay.h) = geom;
        // 倒數中修改時間：已經過的時間保留，以新時間繼續倒數
        if dur_changed {
            if let Some(r) = rt.get_mut(&t.id) {
                if let Phase::Running { start, .. } = r.phase {
                    r.phase = Phase::Running { start, total: t.duration_ms.max(1000) };
                }
            }
        }
    }
    mark_dirty(&mut e);
}

#[tauri::command]
pub fn add_timer() -> TimerCfg {
    let mut e = eng();
    let id = format!("u{}", e.file.next_id);
    e.file.next_id += 1;
    let n = e.file.timers.len() as i32;
    let t = TimerCfg {
        id,
        overlay: Overlay { x: 160 + (n % 4) * 40, y: 160 + (n % 4) * 40, ..Overlay::default() },
        ..TimerCfg::default()
    };
    e.file.timers.push(t.clone());
    mark_dirty(&mut e);
    t
}

#[tauri::command]
pub fn delete_timer(id: String) {
    let mut e = eng();
    e.file.timers.retain(|t| t.id != id || t.builtin);
    e.rt.remove(&id);
    mark_dirty(&mut e);
}

/// 「開始」按鈕：不受冷卻限制
#[tauri::command]
pub fn timer_start(id: String) {
    let now = Instant::now();
    let mut e = eng();
    let Engine { file, rt, .. } = &mut *e;
    if let Some(cfg) = file.timers.iter_mut().find(|t| t.id == id) {
        if !cfg.enabled {
            cfg.enabled = true;
        }
        restart(cfg, rt.entry(id).or_default(), now);
    }
    mark_dirty(&mut e);
}

#[tauri::command]
pub fn timer_stop(id: String) {
    let mut e = eng();
    if let Some(r) = e.rt.get_mut(&id) {
        *r = Rt::default();
    }
}

/// 全部開啟／關閉：同時切換啟用狀態與浮動視窗
#[tauri::command]
pub fn set_all(on: bool) -> TimersFile {
    let mut e = eng();
    for t in e.file.timers.iter_mut() {
        t.enabled = on;
        t.overlay.show = on;
    }
    if !on {
        e.rt.clear();
    }
    mark_dirty(&mut e);
    e.file.clone()
}

#[tauri::command]
pub fn set_game_only(on: bool) {
    GAME_ONLY.store(on, Ordering::SeqCst);
    let mut e = eng();
    e.file.game_only = on;
    mark_dirty(&mut e);
}

/// 把浮動視窗移回主螢幕左上附近
#[tauri::command]
pub fn reset_overlay(app: AppHandle, id: String) {
    let (x, y) = (120, 160);
    {
        let mut e = eng();
        if let Some(t) = e.file.timers.iter_mut().find(|t| t.id == id) {
            t.overlay.x = x;
            t.overlay.y = y;
        }
        mark_dirty(&mut e);
    }
    if let Some(w) = app.get_webview_window(&overlay_label(&id)) {
        let _ = w.set_position(PhysicalPosition::new(x, y));
    }
}

#[tauri::command]
pub fn preview_sound(timer: TimerCfg) {
    audio::play(&timer.sound, &timer.sound_file, &timer.name);
}

#[tauri::command]
pub async fn pick_sound_file() -> Option<String> {
    rfd::FileDialog::new()
        .set_title("選擇音效檔")
        .add_filter("音效檔", &["wav", "mp3", "ogg", "flac"])
        .pick_file()
        .map(|p| p.to_string_lossy().into_owned())
}

#[tauri::command]
pub fn start_capture() {
    input::start_capture();
}

#[tauri::command]
pub fn cancel_capture() {
    input::cancel_capture();
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

    #[test]
    fn modes_at_timeout() {
        let t0 = Instant::now();
        let mk = |mode: &str, repeat: u32| {
            let c = TimerCfg { id: "a".into(), repeat, ..cfg(mode, false) };
            let mut rt = HashMap::new();
            let mut r = Rt::default();
            restart(&c, &mut r, t0);
            rt.insert("a".to_string(), r);
            Engine { file: TimersFile { timers: vec![c], ..TimersFile::default() }, rt, path: PathBuf::new(), dirty: false }
        };
        let later = t0 + Duration::from_millis(10_050);

        let mut e = mk("normal", 0);
        assert_eq!(e.tick(later).len(), 1);
        assert!(matches!(e.rt["a"].phase, Phase::Done));

        let mut e = mk("key", 0);
        e.tick(later);
        assert!(matches!(e.rt["a"].phase, Phase::Over { .. }));

        let mut e = mk("count", 2);
        e.tick(later);
        assert!(matches!(e.rt["a"].phase, Phase::Running { .. }));
        assert_eq!(e.rt["a"].round, 2);
        e.tick(t0 + Duration::from_millis(20_100));
        assert!(matches!(e.rt["a"].phase, Phase::Done));

        let mut e = mk("count", 0); // 無限
        e.tick(later);
        e.tick(t0 + Duration::from_millis(20_100));
        assert!(matches!(e.rt["a"].phase, Phase::Running { .. }));
    }
}
