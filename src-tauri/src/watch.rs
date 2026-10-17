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
    /// 通知方式：app（程式右下角小視窗）/ windows（Windows 通知）/ both
    pub mode: String,
    /// 回到登入畫面時通知
    pub on_login: bool,
    /// 同步送到 Discord
    pub discord: bool,
    pub discord_url: String,
    /// Discord 訊息加上 @everyone（手機一定會推播）
    pub discord_everyone: bool,
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
            on_login: true,
            discord: false,
            discord_url: String::new(),
            discord_everyone: true,
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

/// 用 Discord Webhook 發一則訊息
fn discord_send(url: &str, everyone: bool, title: &str, body: &str) -> Result<(), String> {
    let url = url.trim();
    if !is_webhook(url) {
        return Err("這不是 Discord Webhook 網址".into());
    }
    let content = format!("{}**{}**\n{}", if everyone { "@everyone " } else { "" }, title, body);
    let mentions: Vec<&str> = if everyone { vec!["everyone"] } else { Vec::new() };
    let payload = serde_json::json!({
        "username": "工具箱",
        "content": content,
        "allowed_mentions": { "parse": mentions },
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

/// 依設定的通知方式發出通知；Windows 通知失敗時改用程式自己的通知；有設定 Discord 時同步送出
fn deliver(app: &AppHandle, cfg: &WatchCfg, title: &str, body: &str) {
    if cfg.discord && !cfg.discord_url.trim().is_empty() {
        let (url, everyone, t, b) = (cfg.discord_url.clone(), cfg.discord_everyone, title.to_string(), body.to_string());
        std::thread::spawn(move || {
            if let Err(e) = discord_send(&url, everyone, &t, &b) {
                let mut g = inner();
                g.status.events.insert(0, WatchEvent { at: now_ms(), text: format!("Discord 通知失敗：{e}") });
                g.status.events.truncate(30);
            }
        });
    }
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
    /// "off"（遊戲沒開）、"online"、"waiting"（還沒進入遊戲）、"lost"（連線中斷計時中）、
    /// "down"（已判定斷線）、"login"（已判定回到登入畫面）
    pub state: String,
    /// 目前連著登入伺服器（在登入畫面）
    pub at_login: bool,
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
        let mut first = true;
        let mut was_running = false;
        // 這次遊戲是不是在監看中啟動的（才能把第一條連線當成登入伺服器）
        let mut saw_start = false;
        let mut login_port: Option<u16> = None;
        let mut had_game = false;
        let mut lost_since: Option<Instant> = None;
        let mut fired = false;
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
                    alert(&app, "新楓之谷已關閉", "遊戲程式已結束。");
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
                let since = *lost_since.get_or_insert_with(Instant::now);
                let secs = cfg.threshold.max(5);
                if since.elapsed() >= Duration::from_secs(secs) {
                    if !fired {
                        if at_login {
                            if cfg.on_login {
                                alert(&app, "新楓之谷回到登入畫面", &format!("已經 {secs} 秒沒有連到遊戲伺服器。"));
                            }
                        } else if cfg.on_disconnect {
                            alert(&app, "新楓之谷斷線了", &format!("連線已中斷 {secs} 秒。"));
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

/// 測試通知：用目前的通知方式與音效（有開 Discord 也會送一則）
#[tauri::command]
pub async fn watch_test(app: AppHandle) {
    let cfg = inner().cfg.clone();
    play(&cfg, "測試通知");
    deliver(&app, &cfg, "測試通知", "斷線、回到登入畫面或遊戲關閉時，會像這樣通知你。");
}

/// 只測試 Discord，回傳成功或錯誤訊息
#[tauri::command]
pub async fn watch_discord_test(url: String, everyone: bool) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || discord_send(&url, everyone, "工具箱測試通知", "收到這則訊息，代表斷線通知會送到這個頻道。"))
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
    fn webhook_url() {
        assert!(is_webhook("https://discord.com/api/webhooks/1/abc"));
        assert!(!is_webhook("https://example.com/api/webhooks/1/abc"));
    }
}
