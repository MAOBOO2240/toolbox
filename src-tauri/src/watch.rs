//! 斷線通知：背景監看新楓之谷。
//!
//! - 遊戲程式（執行檔名稱含 MapleStory）原本在執行、後來消失 → 「遊戲已關閉」
//! - 遊戲還在執行，但它對外的 TCP 連線全部中斷超過設定秒數 → 「可能斷線」
//!   （換頻道、進出商城時連線會短暫切換，所以要持續一段時間才算）

use crate::{audio, notify, toast};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tauri::{AppHandle, Emitter};

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(default)]
pub struct WatchCfg {
    /// 遊戲關閉時通知
    pub on_close: bool,
    /// 斷線時通知
    pub on_disconnect: bool,
    /// 連線中斷幾秒才算斷線
    pub threshold: u64,
    /// 通知時播放音效
    pub sound: bool,
    /// 音效：beep1 / beep2 / beep3 / tts / file
    pub sound_name: String,
    pub sound_file: String,
    /// 音量 0–200
    pub volume: u32,
    /// 通知方式：app（程式右下角小視窗）/ windows（Windows 通知）/ both
    pub mode: String,
}

impl Default for WatchCfg {
    fn default() -> Self {
        Self {
            on_close: true,
            on_disconnect: true,
            threshold: 20,
            sound: true,
            sound_name: "beep2".into(),
            sound_file: String::new(),
            volume: 100,
            mode: "both".into(),
        }
    }
}

fn play(cfg: &WatchCfg, speech: &str) {
    if cfg.sound {
        audio::play(&cfg.sound_name, &cfg.sound_file, speech, cfg.volume.min(200));
    }
}

/// 依設定的通知方式發出通知；Windows 通知失敗時改用程式自己的通知
fn deliver(app: &AppHandle, cfg: &WatchCfg, title: &str, body: &str) {
    let win = cfg.mode == "windows" || cfg.mode == "both";
    let mut app_popup = cfg.mode != "windows";
    if win && toast::show(title, body).is_err() {
        app_popup = true;
    }
    if app_popup {
        notify::show(app, title, body, "warn");
    }
}

#[derive(Serialize, Clone, Default)]
pub struct WatchEvent {
    pub at: u64,
    pub text: String,
}

#[derive(Serialize, Clone, Default)]
pub struct WatchStatus {
    pub running: bool,
    pub pid: u32,
    pub conns: u32,
    /// "off"（遊戲沒開）、"online"、"waiting"（剛開還沒連線）、"lost"（連線中斷計時中）、"down"（已判定斷線）
    pub state: String,
    pub lost_secs: u64,
    pub events: Vec<WatchEvent>,
}

struct Inner {
    cfg: WatchCfg,
    path: PathBuf,
    status: WatchStatus,
}

static INNER: OnceLock<Mutex<Inner>> = OnceLock::new();

fn inner() -> std::sync::MutexGuard<'static, Inner> {
    INNER.get().expect("watch 尚未初始化").lock().unwrap_or_else(|e| e.into_inner())
}

fn now_ms() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

fn load(path: &Path) -> WatchCfg {
    std::fs::read_to_string(path).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default()
}

/* ---------------- Windows：找遊戲程式、數連線 ---------------- */

/// 執行檔名稱含 MapleStory 的程式編號
#[cfg(windows)]
fn game_pids() -> Vec<u32> {
    use windows_sys::Win32::Foundation::{CloseHandle, INVALID_HANDLE_VALUE};
    use windows_sys::Win32::System::Diagnostics::ToolHelp::{CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W, TH32CS_SNAPPROCESS};
    let mut out = Vec::new();
    unsafe {
        let snap = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
        if snap == INVALID_HANDLE_VALUE || snap.is_null() {
            return out;
        }
        let mut e: PROCESSENTRY32W = std::mem::zeroed();
        e.dwSize = std::mem::size_of::<PROCESSENTRY32W>() as u32;
        let mut ok = Process32FirstW(snap, &mut e) != 0;
        while ok {
            let len = e.szExeFile.iter().position(|&c| c == 0).unwrap_or(e.szExeFile.len());
            let name = String::from_utf16_lossy(&e.szExeFile[..len]).to_lowercase();
            if name.contains("maplestory") {
                out.push(e.th32ProcessID);
            }
            ok = Process32NextW(snap, &mut e) != 0;
        }
        CloseHandle(snap);
    }
    out
}

#[cfg(not(windows))]
fn game_pids() -> Vec<u32> {
    Vec::new()
}

/// 這些程式目前「已建立」的對外 TCP 連線數（不含本機 127.x）
#[cfg(windows)]
fn established(pids: &[u32]) -> u32 {
    use windows_sys::Win32::NetworkManagement::IpHelper::GetExtendedTcpTable;
    const AF_INET: u32 = 2;
    const TCP_TABLE_OWNER_PID_ALL: i32 = 5;
    const ESTABLISHED: u32 = 5;
    let mut size: u32 = 0;
    unsafe {
        GetExtendedTcpTable(std::ptr::null_mut(), &mut size, 0, AF_INET, TCP_TABLE_OWNER_PID_ALL as _, 0);
    }
    if size == 0 {
        return 0;
    }
    let mut buf = vec![0u8; size as usize + 1024];
    let mut size2 = buf.len() as u32;
    let r = unsafe { GetExtendedTcpTable(buf.as_mut_ptr() as _, &mut size2, 0, AF_INET, TCP_TABLE_OWNER_PID_ALL as _, 0) };
    if r != 0 || buf.len() < 4 {
        return 0;
    }
    let u32_at = |off: usize| u32::from_le_bytes(buf[off..off + 4].try_into().unwrap());
    let n = u32_at(0) as usize;
    let mut count = 0;
    // MIB_TCPROW_OWNER_PID：state, localAddr, localPort, remoteAddr, remotePort, owningPid（各 4 bytes）
    for i in 0..n {
        let off = 4 + i * 24;
        if off + 24 > buf.len() {
            break;
        }
        let state = u32_at(off);
        let remote = u32_at(off + 12);
        let pid = u32_at(off + 20);
        let loopback = (remote & 0xFF) == 127 || remote == 0;
        if state == ESTABLISHED && !loopback && pids.contains(&pid) {
            count += 1;
        }
    }
    count
}

#[cfg(not(windows))]
fn established(_pids: &[u32]) -> u32 {
    0
}

/* ---------------- 背景執行緒 ---------------- */

fn alert(app: &AppHandle, title: &str, body: &str) {
    let cfg = {
        let mut g = inner();
        g.status.events.insert(0, WatchEvent { at: now_ms(), text: format!("{title}：{body}") });
        g.status.events.truncate(30);
        g.cfg.clone()
    };
    play(&cfg, title);
    deliver(app, &cfg, title, body);
}

pub fn init(app: &AppHandle, path: PathBuf) {
    let cfg = load(&path);
    let _ = INNER.set(Mutex::new(Inner { cfg, path, status: WatchStatus { state: "off".into(), ..Default::default() } }));
    let app = app.clone();
    std::thread::spawn(move || {
        let mut was_running = false;
        let mut had_conn = false;
        let mut lost_since: Option<Instant> = None;
        let mut fired = false;
        loop {
            std::thread::sleep(Duration::from_secs(2));
            let pids = game_pids();
            let running = !pids.is_empty();
            let conns = if running { established(&pids) } else { 0 };
            let cfg = inner().cfg.clone();

            let state;
            if !running {
                if was_running && cfg.on_close {
                    alert(&app, "新楓之谷已關閉", if fired { "遊戲在斷線後關閉了。" } else { "遊戲程式已經結束，可能是斷線或當機。" });
                }
                had_conn = false;
                lost_since = None;
                fired = false;
                state = "off";
            } else if conns > 0 {
                if fired {
                    let mut g = inner();
                    g.status.events.insert(0, WatchEvent { at: now_ms(), text: "已重新連線".into() });
                    g.status.events.truncate(30);
                }
                had_conn = true;
                lost_since = None;
                fired = false;
                state = "online";
            } else if !had_conn {
                state = "waiting"; // 剛開遊戲還沒連上伺服器
            } else {
                let since = *lost_since.get_or_insert_with(Instant::now);
                if since.elapsed() >= Duration::from_secs(cfg.threshold.max(5)) {
                    if !fired && cfg.on_disconnect {
                        alert(&app, "新楓之谷可能斷線了", &format!("遊戲的連線已中斷超過 {} 秒。", cfg.threshold.max(5)));
                    }
                    fired = true;
                    state = "down";
                } else {
                    state = "lost";
                }
            }
            was_running = running;

            let status = {
                let mut g = inner();
                g.status.running = running;
                g.status.pid = pids.first().copied().unwrap_or(0);
                g.status.conns = conns;
                g.status.state = state.into();
                g.status.lost_secs = lost_since.map(|s| s.elapsed().as_secs()).unwrap_or(0);
                g.status.clone()
            };
            let _ = app.emit("watch-status", status);
        }
    });
}

#[tauri::command]
pub fn watch_get() -> (WatchCfg, WatchStatus) {
    let g = inner();
    (g.cfg.clone(), g.status.clone())
}

#[tauri::command]
pub fn watch_set(cfg: WatchCfg) {
    let mut g = inner();
    g.cfg = WatchCfg { threshold: cfg.threshold.clamp(5, 600), volume: cfg.volume.min(200), ..cfg };
    if let Ok(s) = serde_json::to_string_pretty(&g.cfg) {
        let _ = std::fs::write(&g.path, s);
    }
}

#[tauri::command]
pub fn watch_clear_log() {
    inner().status.events.clear();
}

/// 測試通知：用目前的通知方式與音效
#[tauri::command]
pub async fn watch_test(app: AppHandle) {
    let cfg = inner().cfg.clone();
    play(&cfg, "測試通知");
    deliver(&app, &cfg, "測試通知", "斷線或遊戲關閉時，會像這樣通知你。");
}

/// 試聽音效
#[tauri::command]
pub fn watch_preview(cfg: WatchCfg) {
    audio::play(&cfg.sound_name, &cfg.sound_file, "新楓之谷斷線了", cfg.volume.min(200));
}
