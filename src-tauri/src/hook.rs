//! 全域 Ctrl+V 偵測（Windows 低階鍵盤掛鉤）。
//!
//! 掛鉤只做最輕量的判斷，並透過 channel 通知背景執行緒；
//! 實際的剪貼簿比對與切換在背景執行緒進行，不會拖慢系統輸入。

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::sync::OnceLock;

/// 是否啟用自動切換（掛鉤內只讀這個旗標，不鎖任何東西）
pub static ACTIVE: AtomicBool = AtomicBool::new(false);

static PASTE_TX: OnceLock<Sender<()>> = OnceLock::new();

#[cfg(windows)]
mod imp {
    use super::*;
    use std::ptr::null_mut;
    use windows_sys::Win32::Foundation::{LPARAM, LRESULT, WPARAM};
    use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows_sys::Win32::System::Threading::GetCurrentProcessId;
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{GetAsyncKeyState, VK_CONTROL};
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        CallNextHookEx, DispatchMessageW, GetForegroundWindow, GetMessageW,
        GetWindowThreadProcessId, SetWindowsHookExW, TranslateMessage, KBDLLHOOKSTRUCT, MSG,
        WH_KEYBOARD_LL, WM_KEYDOWN, WM_KEYUP, WM_SYSKEYDOWN, WM_SYSKEYUP,
    };

    const VK_V: u32 = 0x56;
    /// 按住 V 不放時系統會重複送出 keydown，用這個旗標只算一次
    static V_HELD: AtomicBool = AtomicBool::new(false);
    static OWN_PID: OnceLock<u32> = OnceLock::new();

    fn foreground_is_self() -> bool {
        unsafe {
            let hwnd = GetForegroundWindow();
            if hwnd.is_null() {
                return false;
            }
            let mut pid: u32 = 0;
            GetWindowThreadProcessId(hwnd, &mut pid);
            Some(&pid) == OWN_PID.get()
        }
    }

    unsafe extern "system" fn proc(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
        if code >= 0 {
            let kb = &*(lparam as *const KBDLLHOOKSTRUCT);
            if kb.vkCode == VK_V {
                let msg = wparam as u32;
                if msg == WM_KEYDOWN || msg == WM_SYSKEYDOWN {
                    let first_press = !V_HELD.swap(true, Ordering::SeqCst);
                    let ctrl_down = (GetAsyncKeyState(VK_CONTROL as i32) as u16 & 0x8000) != 0;
                    if first_press
                        && ctrl_down
                        && ACTIVE.load(Ordering::Relaxed)
                        && !foreground_is_self()
                    {
                        if let Some(tx) = PASTE_TX.get() {
                            let _ = tx.send(());
                        }
                    }
                } else if msg == WM_KEYUP || msg == WM_SYSKEYUP {
                    V_HELD.store(false, Ordering::SeqCst);
                }
            }
        }
        CallNextHookEx(null_mut(), code, wparam, lparam)
    }

    pub fn start(tx: Sender<()>) {
        let _ = PASTE_TX.set(tx);
        let _ = OWN_PID.set(unsafe { GetCurrentProcessId() });
        std::thread::spawn(|| unsafe {
            let hmod = GetModuleHandleW(null_mut());
            let hook = SetWindowsHookExW(WH_KEYBOARD_LL, Some(proc), hmod, 0);
            if hook.is_null() {
                eprintln!("無法安裝鍵盤掛鉤");
                return;
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
mod imp {
    use super::*;
    /// 非 Windows 平台不支援全域監聽，只保留手動切換。
    pub fn start(tx: Sender<()>) {
        let _ = PASTE_TX.set(tx);
    }
}

pub use imp::start;
