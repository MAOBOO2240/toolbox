//! 前景視窗偵測：判斷目前是否為新楓之谷、或是本程式自己的視窗。

use serde::Serialize;
use std::sync::atomic::{AtomicIsize, Ordering};

#[derive(Debug, Clone, Default, Serialize, PartialEq)]
pub struct FgInfo {
    /// 前景程式的執行檔名稱
    pub exe: String,
    /// 是否判定為新楓之谷
    pub is_game: bool,
    /// 是否為本程式的任一視窗
    pub is_self: bool,
    /// 是否為本程式的主視窗
    pub is_main: bool,
}

static MAIN_HWND: AtomicIsize = AtomicIsize::new(0);

pub fn set_main_hwnd(h: isize) {
    MAIN_HWND.store(h, Ordering::SeqCst);
}

/// 主視窗看得到嗎（沒有縮到右下角、也沒有最小化）；還不知道主視窗時當作看得到
#[cfg(windows)]
pub fn main_shown() -> bool {
    use windows_sys::Win32::UI::WindowsAndMessaging::{IsIconic, IsWindowVisible};
    let h = MAIN_HWND.load(Ordering::SeqCst);
    if h == 0 {
        return true;
    }
    let hwnd = h as windows_sys::Win32::Foundation::HWND;
    unsafe { IsWindowVisible(hwnd) != 0 && IsIconic(hwnd) == 0 }
}

#[cfg(not(windows))]
pub fn main_shown() -> bool {
    true
}

/// 依執行檔名稱或視窗標題判斷是否為新楓之谷
pub fn looks_like_game(exe: &str, title: &str) -> bool {
    exe.to_lowercase().contains("maplestory")
        || title.to_lowercase().contains("maplestory")
        || title.contains("楓之谷")
}

#[cfg(windows)]
pub fn current() -> FgInfo {
    use std::sync::Mutex;
    use windows_sys::Win32::Foundation::CloseHandle;
    use windows_sys::Win32::System::Threading::{
        GetCurrentProcessId, OpenProcess, QueryFullProcessImageNameW,
        PROCESS_QUERY_LIMITED_INFORMATION,
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        GetForegroundWindow, GetWindowTextW, GetWindowThreadProcessId,
    };

    // 同一個程序的執行檔名稱不會變，快取起來
    static EXE_CACHE: Mutex<(u32, String)> = Mutex::new((0, String::new()));

    unsafe {
        let hwnd = GetForegroundWindow();
        if hwnd.is_null() {
            return FgInfo::default();
        }
        let mut pid: u32 = 0;
        GetWindowThreadProcessId(hwnd, &mut pid);
        let is_self = pid == GetCurrentProcessId();
        let is_main = is_self && hwnd as isize == MAIN_HWND.load(Ordering::SeqCst);

        let mut title_buf = [0u16; 512];
        let n = GetWindowTextW(hwnd, title_buf.as_mut_ptr(), title_buf.len() as i32);
        let title = String::from_utf16_lossy(&title_buf[..n.max(0) as usize]);

        let exe = {
            let mut cache = EXE_CACHE.lock().unwrap();
            if cache.0 != pid || cache.1.is_empty() {
                let mut name = String::new();
                let h = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
                if !h.is_null() {
                    let mut buf = [0u16; 1024];
                    let mut size = buf.len() as u32;
                    if QueryFullProcessImageNameW(h, 0, buf.as_mut_ptr(), &mut size) != 0 {
                        let full = String::from_utf16_lossy(&buf[..size as usize]);
                        name = full.rsplit(['\\', '/']).next().unwrap_or("").to_string();
                    }
                    CloseHandle(h);
                }
                *cache = (pid, name);
            }
            cache.1.clone()
        };

        let is_game = !is_self && looks_like_game(&exe, &title);
        FgInfo { exe, is_game, is_self, is_main }
    }
}

#[cfg(not(windows))]
pub fn current() -> FgInfo {
    FgInfo::default()
}
