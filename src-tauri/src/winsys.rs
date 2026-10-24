//! 直接呼叫 Windows 的功能：登錄檔、開網址或設定頁。
//! 不另外開 reg.exe、rundll32.exe 這類系統程式（沒有數位簽章的程式在背景開系統程式，容易被防毒誤判）。
#![allow(dead_code)]

#[cfg(windows)]
fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

/// 目前使用者（HKCU）的登錄檔
#[cfg(windows)]
pub mod reg {
    use super::wide;
    use windows_sys::Win32::Foundation::{ERROR_FILE_NOT_FOUND, ERROR_SUCCESS};
    use windows_sys::Win32::System::Registry::{
        RegCloseKey, RegCreateKeyExW, RegDeleteValueW, RegGetValueW, RegOpenKeyExW, RegSetValueExW, HKEY, HKEY_CURRENT_USER, KEY_SET_VALUE,
        REG_OPTION_NON_VOLATILE, REG_SZ, RRF_RT_REG_SZ,
    };

    /// 讀字串值（沒有時為 None）
    pub fn get(sub: &str, name: &str) -> Option<String> {
        let (sub, name) = (wide(sub), wide(name));
        unsafe {
            let mut len: u32 = 0;
            if RegGetValueW(HKEY_CURRENT_USER, sub.as_ptr(), name.as_ptr(), RRF_RT_REG_SZ, std::ptr::null_mut(), std::ptr::null_mut(), &mut len) != ERROR_SUCCESS {
                return None;
            }
            let mut buf = vec![0u16; len as usize / 2 + 1];
            let mut len = (buf.len() * 2) as u32;
            if RegGetValueW(HKEY_CURRENT_USER, sub.as_ptr(), name.as_ptr(), RRF_RT_REG_SZ, std::ptr::null_mut(), buf.as_mut_ptr().cast(), &mut len) != ERROR_SUCCESS {
                return None;
            }
            let text = &buf[..(len as usize / 2).min(buf.len())];
            let end = text.iter().position(|&c| c == 0).unwrap_or(text.len());
            Some(String::from_utf16_lossy(&text[..end]))
        }
    }

    /// 讀 HKLM 的字串值（沒有時為 None）
    pub fn get_lm(sub: &str, name: &str) -> Option<String> {
        use windows_sys::Win32::System::Registry::HKEY_LOCAL_MACHINE;
        let (sub, name) = (wide(sub), wide(name));
        unsafe {
            let mut buf = [0u16; 256];
            let mut len = (buf.len() * 2) as u32;
            if RegGetValueW(HKEY_LOCAL_MACHINE, sub.as_ptr(), name.as_ptr(), RRF_RT_REG_SZ, std::ptr::null_mut(), buf.as_mut_ptr().cast(), &mut len) != ERROR_SUCCESS {
                return None;
            }
            let text = &buf[..(len as usize / 2).min(buf.len())];
            let end = text.iter().position(|&c| c == 0).unwrap_or(text.len());
            Some(String::from_utf16_lossy(&text[..end]))
        }
    }

    /// 寫字串值（機碼不存在時建立）
    pub fn set(sub: &str, name: &str, value: &str) -> bool {
        let sub = wide(sub);
        let mut key: HKEY = std::ptr::null_mut();
        unsafe {
            let r = RegCreateKeyExW(
                HKEY_CURRENT_USER,
                sub.as_ptr(),
                0,
                std::ptr::null(),
                REG_OPTION_NON_VOLATILE,
                KEY_SET_VALUE,
                std::ptr::null(),
                &mut key,
                std::ptr::null_mut(),
            );
            if r != ERROR_SUCCESS {
                return false;
            }
            let (name, data) = (wide(name), wide(value));
            let ok = RegSetValueExW(key, name.as_ptr(), 0, REG_SZ, data.as_ptr().cast(), (data.len() * 2) as u32) == ERROR_SUCCESS;
            RegCloseKey(key);
            ok
        }
    }

    /// 和現在的值不一樣才寫入
    pub fn ensure(sub: &str, name: &str, value: &str) -> bool {
        get(sub, name).as_deref() == Some(value) || set(sub, name, value)
    }

    /// 刪除值（本來就沒有也算成功）
    pub fn delete(sub: &str, name: &str) -> bool {
        let sub = wide(sub);
        let mut key: HKEY = std::ptr::null_mut();
        unsafe {
            match RegOpenKeyExW(HKEY_CURRENT_USER, sub.as_ptr(), 0, KEY_SET_VALUE, &mut key) {
                ERROR_SUCCESS => {}
                ERROR_FILE_NOT_FOUND => return true,
                _ => return false,
            }
            let name = wide(name);
            let r = RegDeleteValueW(key, name.as_ptr());
            RegCloseKey(key);
            r == ERROR_SUCCESS || r == ERROR_FILE_NOT_FOUND
        }
    }
}

/// 用預設的程式開啟網址或 Windows 設定頁（ms-settings:…），和一般程式開網頁的方式一樣
#[cfg(windows)]
pub fn open(target: &str) -> Result<(), String> {
    use windows_sys::Win32::UI::Shell::ShellExecuteW;
    use windows_sys::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
    let (op, file) = (wide("open"), wide(target));
    let r = unsafe { ShellExecuteW(std::ptr::null_mut(), op.as_ptr(), file.as_ptr(), std::ptr::null(), std::ptr::null(), SW_SHOWNORMAL) };
    // 回傳值大於 32 代表成功
    if r as isize > 32 {
        Ok(())
    } else {
        Err(format!("無法開啟（錯誤代碼 {}）", r as isize))
    }
}

#[cfg(not(windows))]
pub fn open(_target: &str) -> Result<(), String> {
    Err("只支援 Windows".into())
}
