//! 全域鍵盤／滑鼠監聽（Windows 低階掛鉤）。
//!
//! 掛鉤回呼只把原始事件丟進 channel，所有判斷都在背景執行緒進行，
//! 不會拖慢系統輸入。用途：
//! - 序號工具：偵測 Ctrl+V 後切換下一組
//! - 計時器：指派按鍵（含組合鍵、滑鼠側鍵）觸發重新計時
//!
//! 按鍵指派的擷取在介面端進行（程式視窗本身有焦點），名稱格式與這裡一致。

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};
use tauri::AppHandle;

pub const CTRL: u8 = 1;
pub const ALT: u8 = 2;
pub const SHIFT: u8 = 4;
pub const WIN: u8 = 8;

/// 序號工具的自動切換是否啟用
pub static PASTE_ACTIVE: AtomicBool = AtomicBool::new(false);

enum Raw {
    Key { vk: u32, down: bool },
    Mouse(&'static str),
}

static RAW_TX: OnceLock<Sender<Raw>> = OnceLock::new();
static PASTE_TX: OnceLock<Sender<()>> = OnceLock::new();
/// 最後一次偵測到的按鍵（顯示在介面上，方便確認監聽是否正常）
static LAST_KEY: Mutex<String> = Mutex::new(String::new());

pub fn last_key() -> String {
    LAST_KEY.lock().unwrap().clone()
}

fn set_last(key: String) {
    *LAST_KEY.lock().unwrap() = key;
}

pub fn start(app: AppHandle, paste_tx: Sender<()>) {
    let _ = app;
    let _ = PASTE_TX.set(paste_tx);
    let (tx, rx) = mpsc::channel();
    let _ = RAW_TX.set(tx);
    std::thread::spawn(move || worker(rx));
    platform::install_hooks();
}

/// 左右 Ctrl/Shift/Alt/Win 合併成同一個鍵
fn normalize(vk: u32) -> u32 {
    match vk {
        0xA0 | 0xA1 => 0x10,
        0xA2 | 0xA3 => 0x11,
        0xA4 | 0xA5 => 0x12,
        0x5C => 0x5B,
        v => v,
    }
}

fn mod_bit(vk: u32) -> u8 {
    match vk {
        0x11 => CTRL,
        0x12 => ALT,
        0x10 => SHIFT,
        0x5B => WIN,
        _ => 0,
    }
}

pub fn key_name(vk: u32) -> String {
    let s = match vk {
        0x08 => "Backspace",
        0x09 => "Tab",
        0x0D => "Enter",
        0x10 => "Shift",
        0x11 => "Ctrl",
        0x12 => "Alt",
        0x13 => "Pause",
        0x14 => "CapsLock",
        0x1B => "Esc",
        0x20 => "Space",
        0x21 => "PageUp",
        0x22 => "PageDown",
        0x23 => "End",
        0x24 => "Home",
        0x25 => "Left",
        0x26 => "Up",
        0x27 => "Right",
        0x28 => "Down",
        0x2C => "PrintScreen",
        0x2D => "Insert",
        0x2E => "Delete",
        0x5B => "Win",
        0x6A => "Num*",
        0x6B => "NumAdd",
        0x6D => "Num-",
        0x6E => "Num.",
        0x6F => "Num/",
        0x90 => "NumLock",
        0x91 => "ScrollLock",
        0xBA => ";",
        0xBB => "=",
        0xBC => ",",
        0xBD => "-",
        0xBE => ".",
        0xBF => "/",
        0xC0 => "`",
        0xDB => "[",
        0xDC => "\\",
        0xDD => "]",
        0xDE => "'",
        0x30..=0x39 | 0x41..=0x5A => return char::from_u32(vk).unwrap().to_string(),
        0x60..=0x69 => return format!("Num{}", vk - 0x60),
        0x70..=0x87 => return format!("F{}", vk - 0x6F),
        _ => return format!("VK{vk:02X}"),
    };
    s.to_string()
}

/// 組合鍵字串，例如 "Ctrl+Shift+S"
pub fn combo(mods: u8, key: &str) -> String {
    let mut parts: Vec<&str> = Vec::new();
    if mods & CTRL != 0 {
        parts.push("Ctrl");
    }
    if mods & ALT != 0 {
        parts.push("Alt");
    }
    if mods & SHIFT != 0 {
        parts.push("Shift");
    }
    if mods & WIN != 0 {
        parts.push("Win");
    }
    parts.push(key);
    parts.join("+")
}

/// 解析組合鍵字串 → (修飾鍵, 主鍵)
pub fn parse(binding: &str) -> (u8, &str) {
    let mut parts: Vec<&str> = binding.split('+').collect();
    let key = parts.pop().unwrap_or("");
    let mut mods = 0;
    for p in parts {
        mods |= match p {
            "Ctrl" => CTRL,
            "Alt" => ALT,
            "Shift" => SHIFT,
            "Win" => WIN,
            _ => 0,
        };
    }
    (mods, key)
}

/// 按鍵是否符合指派：主鍵相同，且指派的修飾鍵都有按住（多按其他修飾鍵也算）
pub fn matches(binding: &str, key: &str, mods: u8) -> bool {
    let (bm, bk) = parse(binding);
    bk == key && (bm & !mods) == 0
}

fn mods_of(pressed: &HashMap<u32, Instant>, except: u32) -> u8 {
    pressed
        .keys()
        .filter(|&&vk| vk != except)
        .fold(0, |m, &vk| m | mod_bit(vk))
}

fn worker(rx: Receiver<Raw>) {
    // 目前按住的鍵與最後一次收到的時間（用來過濾長按的重複事件）
    let mut pressed: HashMap<u32, Instant> = HashMap::new();

    while let Ok(raw) = rx.recv() {
        let now = Instant::now();
        match raw {
            Raw::Key { vk, down } => {
                let vk = normalize(vk);
                if down {
                    if let Some(last) = pressed.get_mut(&vk) {
                        let repeat = now.duration_since(*last) < Duration::from_millis(800);
                        *last = now;
                        if repeat {
                            continue;
                        }
                        // 很久沒收到同鍵事件 → 視為漏掉了放開，當作新的按下
                    } else {
                        pressed.insert(vk, now);
                    }
                    on_press(vk, mods_of(&pressed, vk));
                } else {
                    pressed.remove(&vk);
                }
            }
            Raw::Mouse(name) => {
                let mods = mods_of(&pressed, 0);
                set_last(combo(mods, name));
                crate::timers::on_input(name, mods);
            }
        }
    }
}

fn on_press(vk: u32, mods: u8) {
    set_last(combo(mods, &key_name(vk)));
    // Ctrl+V：序號工具自動切換（在本程式視窗內貼上不算）
    if vk == 0x56 && mods & CTRL != 0 && PASTE_ACTIVE.load(Ordering::Relaxed) {
        if !crate::fg::current().is_self {
            if let Some(tx) = PASTE_TX.get() {
                let _ = tx.send(());
            }
        }
    }
    crate::timers::on_input(&key_name(vk), mods);
}

#[cfg(windows)]
mod platform {
    use super::{Raw, RAW_TX};
    use std::ptr::null_mut;
    use windows_sys::Win32::Foundation::{LPARAM, LRESULT, WPARAM};
    use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        CallNextHookEx, DispatchMessageW, GetMessageW, SetWindowsHookExW, TranslateMessage,
        KBDLLHOOKSTRUCT, MSG, MSLLHOOKSTRUCT, WH_KEYBOARD_LL, WH_MOUSE_LL, WM_KEYDOWN, WM_KEYUP,
        WM_MBUTTONDOWN, WM_SYSKEYDOWN, WM_SYSKEYUP, WM_XBUTTONDOWN,
    };

    fn send(raw: Raw) {
        if let Some(tx) = RAW_TX.get() {
            let _ = tx.send(raw);
        }
    }

    unsafe extern "system" fn kb_proc(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
        if code >= 0 {
            let kb = &*(lparam as *const KBDLLHOOKSTRUCT);
            match wparam as u32 {
                WM_KEYDOWN | WM_SYSKEYDOWN => send(Raw::Key { vk: kb.vkCode, down: true }),
                WM_KEYUP | WM_SYSKEYUP => send(Raw::Key { vk: kb.vkCode, down: false }),
                _ => {}
            }
        }
        CallNextHookEx(null_mut(), code, wparam, lparam)
    }

    unsafe extern "system" fn mouse_proc(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
        if code >= 0 {
            match wparam as u32 {
                WM_XBUTTONDOWN => {
                    let ms = &*(lparam as *const MSLLHOOKSTRUCT);
                    let button = (ms.mouseData >> 16) & 0xFFFF;
                    send(Raw::Mouse(if button == 1 { "Mouse4" } else { "Mouse5" }));
                }
                WM_MBUTTONDOWN => send(Raw::Mouse("MouseMid")),
                _ => {}
            }
        }
        CallNextHookEx(null_mut(), code, wparam, lparam)
    }

    pub fn install_hooks() {
        std::thread::spawn(|| unsafe {
            let hmod = GetModuleHandleW(null_mut());
            let kb = SetWindowsHookExW(WH_KEYBOARD_LL, Some(kb_proc), hmod, 0);
            let ms = SetWindowsHookExW(WH_MOUSE_LL, Some(mouse_proc), hmod, 0);
            if kb.is_null() || ms.is_null() {
                eprintln!("無法安裝輸入掛鉤");
            }
            // 低階掛鉤需要所在執行緒持續處理訊息
            let mut msg: MSG = std::mem::zeroed();
            while GetMessageW(&mut msg, null_mut(), 0, 0) > 0 {
                TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        });
    }
}

#[cfg(not(windows))]
mod platform {
    pub fn install_hooks() {}
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn combo_and_match() {
        assert_eq!(combo(CTRL | SHIFT, "S"), "Ctrl+Shift+S");
        assert_eq!(key_name(0x53), "S");
        assert_eq!(key_name(0x25), "Left");
        assert_eq!(key_name(0x70), "F1");
        assert_eq!(key_name(0x61), "Num1");
        // 單鍵指派：按住其他修飾鍵也會觸發
        assert!(matches("S", "S", 0));
        assert!(matches("S", "S", SHIFT));
        // 組合鍵：修飾鍵必須按住
        assert!(matches("Ctrl+S", "S", CTRL));
        assert!(!matches("Ctrl+S", "S", 0));
        assert!(matches("Mouse4", "Mouse4", 0));
        assert!(!matches("Left", "Right", 0));
    }
}
