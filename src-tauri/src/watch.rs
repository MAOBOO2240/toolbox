//! 斷線通知：背景監看新楓之谷。
//!
//! - 遊戲程式（執行檔名稱含 MapleStory）原本在執行、後來消失 → 「已關閉」
//! - 遊戲還在執行，但連到遊戲伺服器的連線中斷超過設定秒數：
//!   - 只剩登入伺服器的連線 → 「回到登入畫面」（被踢回登入畫面時遊戲不會關閉）
//!   - 完全沒有連線 → 「斷線」
//!   （換頻道、進出商城時連線會短暫切換，所以要持續一段時間才算）
//!
//! 登入伺服器：新楓之谷慣用 8484 埠；看到遊戲剛啟動時，也會把第一條連線當作登入伺服器。
//! 另外可以把通知同步送到 Discord（Webhook）。

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
    /// 通知方式：windows（Windows 通知）/ discord（Discord）/ both
    pub mode: String,
    /// 回到登入畫面時通知
    pub on_login: bool,
    /// 舊版的「同步送到 Discord」開關，只用來轉換舊設定
    pub discord: bool,
    pub discord_url: String,
    /// 要提及的 Discord 使用者 ID（只通知這個人）
    pub discord_user: String,
    /// 週三維護：這些整點（0／7／8）前後斷線不通知
    pub quiet: Vec<u8>,
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
            mode: "windows".into(),
            on_login: true,
            discord: false,
            discord_url: String::new(),
            discord_user: String::new(),
            quiet: Vec::new(),
        }
    }
}

fn play(cfg: &WatchCfg, speech: &str) {
    if cfg.sound {
        audio::play(&cfg.sound_name, &cfg.sound_file, speech, cfg.volume.min(200));
    }
}

fn is_webhook(u: &str) -> bool {
    ["https://discord.com/api/webhooks/", "https://discordapp.com/api/webhooks/", "https://ptb.discord.com/api/webhooks/", "https://canary.discord.com/api/webhooks/"]
        .iter()
        .any(|p| u.starts_with(p))
}

/// Discord 使用者 ID：17～20 位數字
fn user_id(s: &str) -> Option<&str> {
    let s = s.trim();
    (s.len() >= 15 && s.len() <= 21 && s.bytes().all(|b| b.is_ascii_digit())).then_some(s)
}

/// 用 Discord Webhook 發一則訊息；有填使用者 ID 就只提及那個人
fn discord_send(url: &str, user: &str, title: &str, body: &str) -> Result<(), String> {
    let url = url.trim();
    if !is_webhook(url) {
        return Err("這不是 Discord Webhook 網址".into());
    }
    if !user.trim().is_empty() && user_id(user).is_none() {
        return Err("Discord 使用者 ID 應該是一串數字".into());
    }
    let uid = user_id(user);
    let content = match uid {
        Some(id) => format!("<@{id}> **{title}**\n{body}"),
        None => format!("**{title}**\n{body}"),
    };
    let users: Vec<&str> = uid.into_iter().collect();
    let payload = serde_json::json!({
        "username": "工具箱",
        "content": content,
        "allowed_mentions": { "parse": [], "users": users },
    });
    ureq::post(url)
        .timeout(Duration::from_secs(15))
        .send_json(payload)
        .map(|_| ())
        .map_err(|e| match e {
            ureq::Error::Status(401 | 403 | 404, _) => "Webhook 無效或已被刪除".to_string(),
            ureq::Error::Status(429, _) => "Discord 暫時限制發送次數，稍後再試".to_string(),
            ureq::Error::Status(c, _) => format!("Discord 回應錯誤 {c}"),
            e => format!("連線失敗：{e}"),
        })
}

/// Windows 通知；失敗時改用程式右下角的小視窗
fn windows_notify(app: &AppHandle, title: &str, body: &str) {
    if toast::show(title, body).is_err() {
        notify::show(app, title, body, "warn");
    }
}

/// 依設定的通知方式發出通知：Windows 通知、Discord，或兩者。
/// Discord 沒設定好或送不出去時，改用 Windows 通知，避免漏掉。
fn deliver(app: &AppHandle, cfg: &WatchCfg, title: &str, body: &str) {
    let use_win = cfg.mode != "discord";
    let use_dc = cfg.mode == "discord" || cfg.mode == "both";
    if use_win {
        windows_notify(app, title, body);
    }
    if use_dc {
        if cfg.discord_url.trim().is_empty() {
            if !use_win {
                windows_notify(app, title, body);
            }
            return;
        }
        let (app, url, user, t, b) = (app.clone(), cfg.discord_url.clone(), cfg.discord_user.clone(), title.to_string(), body.to_string());
        std::thread::spawn(move || {
            if let Err(e) = discord_send(&url, &user, &t, &b) {
                {
                    let mut g = inner();
                    g.status.events.insert(0, WatchEvent { at: now_ms(), text: format!("Discord 通知失敗：{e}") });
                    g.status.events.truncate(30);
                }
                if !use_win {
                    windows_notify(&app, &t, &b);
                }
            }
        });
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
    /// "off"（遊戲沒開）、"online"、"waiting"（還沒進入遊戲）、"lost"（連線中斷計時中）、
    /// "down"（已判定斷線）、"login"（已判定回到登入畫面）
    pub state: String,
    /// 目前連著登入伺服器（在登入畫面）
    pub at_login: bool,
    /// 中斷計時中：再過幾秒會通知
    pub notify_in: u64,
    /// 中斷時你正在操作電腦（自己登出的可能性高，登入畫面要待滿 5 分鐘才通知）
    pub active_loss: bool,
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
    let mut cfg: WatchCfg = std::fs::read_to_string(path).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default();
    // 舊設定：「程式通知」改成 Windows 通知；開過「同步送到 Discord」的改成兩者
    if cfg.mode == "app" {
        cfg.mode = "windows".into();
    }
    if cfg.discord {
        if cfg.mode != "discord" {
            cfg.mode = "both".into();
        }
        cfg.discord = false;
    }
    if !matches!(cfg.mode.as_str(), "windows" | "discord" | "both") {
        cfg.mode = "windows".into();
    }
    cfg
}

/// 鍵盤滑鼠多久沒有動作（秒）
#[cfg(windows)]
fn idle_secs() -> u64 {
    use windows_sys::Win32::System::SystemInformation::GetTickCount;
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{GetLastInputInfo, LASTINPUTINFO};
    let mut li = LASTINPUTINFO { cbSize: std::mem::size_of::<LASTINPUTINFO>() as u32, dwTime: 0 };
    unsafe {
        if GetLastInputInfo(&mut li) == 0 {
            return 0;
        }
        (GetTickCount().wrapping_sub(li.dwTime) / 1000) as u64
    }
}

#[cfg(not(windows))]
fn idle_secs() -> u64 {
    0
}

/// 現在的星期（0＝週日）、時、分（本機時間）
#[cfg(windows)]
fn local_now() -> (u32, u32, u32) {
    use windows_sys::Win32::System::SystemInformation::GetLocalTime;
    let mut t: windows_sys::Win32::Foundation::SYSTEMTIME = unsafe { std::mem::zeroed() };
    unsafe { GetLocalTime(&mut t) };
    (t.wDayOfWeek as u32, t.wHour as u32, t.wMinute as u32)
}

#[cfg(not(windows))]
fn local_now() -> (u32, u32, u32) {
    (0, 0, 0)
}

/// 週三維護時間：所選整點的前 5 分鐘到後 30 分鐘
fn in_maintenance(quiet: &[u8], (dow, h, m): (u32, u32, u32)) -> bool {
    let week = 7 * 1440;
    let now = (dow * 1440 + h * 60 + m) as i64;
    quiet.iter().any(|&q| {
        let target = (3 * 1440 + q as u32 * 60) as i64;
        let d = (now - target).rem_euclid(week);
        // d 在 [0, 30] 或 [week-5, week) 之間
        d <= 30 || d >= week - 5
    })
}

/// 自己登出時，登入畫面要待多久才通知
const LOGIN_WAIT: u64 = 300;
/// 中斷前這麼多秒都沒有碰鍵盤滑鼠，就當作不是自己登出的（離開電腦時被送回登入畫面）
const AFK_SECS: u64 = 60;

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

/// 一條對外連線的遠端位址與埠
#[derive(Clone, Copy, PartialEq, Debug)]
#[allow(dead_code)]
struct Conn {
    ip: u32,
    port: u16,
}

/// 這些程式目前「已建立」的對外 TCP 連線（不含本機 127.x，也不含網頁用的 80／443 埠）
#[cfg(windows)]
fn connections(pids: &[u32]) -> Vec<Conn> {
    use windows_sys::Win32::NetworkManagement::IpHelper::GetExtendedTcpTable;
    const AF_INET: u32 = 2;
    const TCP_TABLE_OWNER_PID_ALL: i32 = 5;
    const ESTABLISHED: u32 = 5;
    let mut out = Vec::new();
    let mut size: u32 = 0;
    unsafe {
        GetExtendedTcpTable(std::ptr::null_mut(), &mut size, 0, AF_INET, TCP_TABLE_OWNER_PID_ALL as _, 0);
    }
    if size == 0 {
        return out;
    }
    let mut buf = vec![0u8; size as usize + 1024];
    let mut size2 = buf.len() as u32;
    let r = unsafe { GetExtendedTcpTable(buf.as_mut_ptr() as _, &mut size2, 0, AF_INET, TCP_TABLE_OWNER_PID_ALL as _, 0) };
    if r != 0 || buf.len() < 4 {
        return out;
    }
    let u32_at = |off: usize| u32::from_le_bytes(buf[off..off + 4].try_into().unwrap());
    let n = u32_at(0) as usize;
    // MIB_TCPROW_OWNER_PID：state, localAddr, localPort, remoteAddr, remotePort, owningPid（各 4 bytes）
    for i in 0..n {
        let off = 4 + i * 24;
        if off + 24 > buf.len() {
            break;
        }
        let state = u32_at(off);
        let remote = u32_at(off + 12);
        let port = port_of(u32_at(off + 16));
        let pid = u32_at(off + 20);
        let loopback = (remote & 0xFF) == 127 || remote == 0;
        if state == ESTABLISHED && !loopback && !matches!(port, 80 | 443) && pids.contains(&pid) {
            out.push(Conn { ip: remote, port });
        }
    }
    out
}

#[cfg(not(windows))]
fn connections(_pids: &[u32]) -> Vec<Conn> {
    Vec::new()
}

/// 連線表裡的埠是網路位元組順序，存在低 16 位元
fn port_of(raw: u32) -> u16 {
    (((raw & 0xFF) << 8) | ((raw >> 8) & 0xFF)) as u16
}

/// 新楓之谷登入伺服器慣用的埠
const LOGIN_PORT: u16 = 8484;

/* ---------------- 背景執行緒 ---------------- */

fn log(text: String) {
    let mut g = inner();
    g.status.events.insert(0, WatchEvent { at: now_ms(), text });
    g.status.events.truncate(30);
}

/// 發出通知；週三維護時間（maint）只記錄、不通知
fn alert(app: &AppHandle, title: &str, body: &str, maint: bool) {
    if maint {
        log(format!("{title}（週三維護時間，不通知）"));
        return;
    }
    log(format!("{title}：{body}"));
    let cfg = inner().cfg.clone();
    play(&cfg, title);
    deliver(app, &cfg, title, body);
}

pub fn init(app: &AppHandle, path: PathBuf) {
    let cfg = load(&path);
    let _ = INNER.set(Mutex::new(Inner { cfg, path, status: WatchStatus { state: "off".into(), ..Default::default() } }));
    let app = app.clone();
    std::thread::spawn(move || {
        let mut first = true;
        let mut was_running = false;
        // 這次遊戲是不是在監看中啟動的（才能把第一條連線當成登入伺服器）
        let mut saw_start = false;
        let mut login_port: Option<u16> = None;
        let mut had_game = false;
        let mut lost_since: Option<Instant> = None;
        // 中斷的那一刻是否正在操作電腦、是否在週三維護時間
        let mut active_loss = false;
        let mut maint_loss = false;
        let mut fired = false;
        let mut notify_in = 0u64;
        loop {
            std::thread::sleep(Duration::from_secs(2));
            let pids = game_pids();
            let running = !pids.is_empty();
            let conns = if running { connections(&pids) } else { Vec::new() };
            let cfg = inner().cfg.clone();

            if running && !was_running {
                saw_start = !first;
                login_port = None;
            }
            if conns.iter().any(|c| c.port == LOGIN_PORT) {
                login_port = Some(LOGIN_PORT);
            } else if login_port.is_none() && saw_start && !had_game {
                login_port = conns.first().map(|c| c.port);
            }
            let at_login = login_port.is_some_and(|p| conns.iter().any(|c| c.port == p));
            let game_conns = conns.iter().filter(|c| Some(c.port) != login_port).count() as u32;

            let state;
            if !running {
                if was_running && cfg.on_close {
                    let maint = in_maintenance(&cfg.quiet, local_now()) || (lost_since.is_some() && maint_loss);
                    alert(&app, "新楓之谷已關閉", "遊戲程式已結束。", maint);
                }
                had_game = false;
                lost_since = None;
                fired = false;
                state = "off";
            } else if game_conns > 0 {
                if fired {
                    let mut g = inner();
                    g.status.events.insert(0, WatchEvent { at: now_ms(), text: "已重新進入遊戲".into() });
                    g.status.events.truncate(30);
                }
                had_game = true;
                lost_since = None;
                fired = false;
                state = "online";
            } else if !had_game {
                state = "waiting"; // 登入畫面，或剛開遊戲還沒連上伺服器
            } else {
                if lost_since.is_none() {
                    lost_since = Some(Instant::now());
                    active_loss = idle_secs() < AFK_SECS;
                    maint_loss = in_maintenance(&cfg.quiet, local_now());
                }
                let since = lost_since.unwrap_or_else(Instant::now);
                let secs = cfg.threshold.max(5);
                // 回到登入畫面時，若中斷前你正在操作電腦（多半是自己登出），要在登入畫面待滿 5 分鐘才通知
                let wait = if at_login && active_loss { LOGIN_WAIT.max(secs) } else { secs };
                let elapsed = since.elapsed().as_secs();
                notify_in = wait.saturating_sub(elapsed);
                if elapsed >= wait {
                    if !fired {
                        if at_login {
                            if cfg.on_login {
                                let body = if active_loss { "已經在登入畫面待了 5 分鐘。".to_string() } else { format!("已經 {secs} 秒沒有連到遊戲伺服器。") };
                                alert(&app, "新楓之谷回到登入畫面", &body, maint_loss);
                            }
                        } else if cfg.on_disconnect {
                            alert(&app, "新楓之谷斷線了", &format!("連線已中斷 {secs} 秒。"), maint_loss);
                        }
                    }
                    fired = true;
                    state = if at_login { "login" } else { "down" };
                } else {
                    state = "lost";
                }
            }
            was_running = running;
            first = false;

            let status = {
                let mut g = inner();
                g.status.running = running;
                g.status.pid = pids.first().copied().unwrap_or(0);
                g.status.conns = game_conns;
                g.status.at_login = at_login;
                g.status.notify_in = if state == "lost" { notify_in } else { 0 };
                g.status.active_loss = active_loss;
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
    let mut quiet: Vec<u8> = cfg.quiet.iter().copied().filter(|h| *h < 24).collect();
    quiet.sort_unstable();
    quiet.dedup();
    g.cfg = WatchCfg { threshold: cfg.threshold.clamp(5, 600), volume: cfg.volume.min(200), quiet, ..cfg };
    if let Ok(s) = serde_json::to_string_pretty(&g.cfg) {
        let _ = std::fs::write(&g.path, s);
    }
}

#[tauri::command]
pub fn watch_clear_log() {
    inner().status.events.clear();
}

/// 測試通知：用目前的通知方式與音效（有開 Discord 也會送一則）
#[tauri::command]
pub async fn watch_test(app: AppHandle) {
    let cfg = inner().cfg.clone();
    play(&cfg, "測試通知");
    deliver(&app, &cfg, "測試通知", "斷線、回到登入畫面或遊戲關閉時，會像這樣通知你。");
}

/// 只測試 Discord，回傳成功或錯誤訊息
#[tauri::command]
pub async fn watch_discord_test(url: String, user: String) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || discord_send(&url, &user, "工具箱測試通知", "收到這則訊息，代表斷線通知會送到這裡。"))
        .await
        .map_err(|e| e.to_string())?
}

/// 試聽音效
#[tauri::command]
pub fn watch_preview(cfg: WatchCfg) {
    audio::play(&cfg.sound_name, &cfg.sound_file, "新楓之谷斷線了", cfg.volume.min(200));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn port_order() {
        // 8484 = 0x2124，網路順序存成 bytes [0x21, 0x24, 0, 0]
        assert_eq!(port_of(u32::from_le_bytes([0x21, 0x24, 0, 0])), 8484);
        assert_eq!(port_of(u32::from_le_bytes([0x01, 0xBB, 0, 0])), 443);
    }

    #[test]
    fn maintenance_window() {
        let q = [0u8, 7];
        assert!(in_maintenance(&q, (3, 0, 10))); // 週三 00:10
        assert!(in_maintenance(&q, (2, 23, 57))); // 週二 23:57（前 5 分鐘）
        assert!(in_maintenance(&q, (3, 7, 30)));
        assert!(!in_maintenance(&q, (3, 7, 31)));
        assert!(!in_maintenance(&q, (3, 8, 5))); // 沒選 08:00
        assert!(!in_maintenance(&q, (4, 0, 10))); // 週四
        assert!(!in_maintenance(&[], (3, 0, 0)));
    }

    #[test]
    fn discord_user_id() {
        assert_eq!(user_id(" 123456789012345678 "), Some("123456789012345678"));
        assert_eq!(user_id("abc"), None);
        assert_eq!(user_id("12345"), None);
    }

    #[test]
    fn webhook_url() {
        assert!(is_webhook("https://discord.com/api/webhooks/1/abc"));
        assert!(!is_webhook("https://example.com/api/webhooks/1/abc"));
    }
}
