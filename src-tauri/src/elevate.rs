//! 以系統管理員身分執行（讓全域按鍵監聽在遊戲中也有效）。
//!
//! 程式本身用一般權限啟動（這樣 Windows 的「啟動」清單才會在登入時開它），
//! 沒有管理員權限時馬上用「以系統管理員身分執行」重新開一次自己（參數照舊），原本這個就結束。
//! 使用者在權限確認視窗按「否」時就不開，和以前直接要求管理員權限的效果一樣。
#![allow(dead_code)]

/// Windows 命令列的參數加引號（有空白、引號時）
pub fn quote(arg: &str) -> String {
    if !arg.is_empty() && !arg.contains([' ', '\t', '"']) {
        return arg.to_string();
    }
    let mut out = String::from("\"");
    let mut slashes = 0usize;
    for c in arg.chars() {
        match c {
            '\\' => slashes += 1,
            '"' => {
                out.extend(std::iter::repeat('\\').take(slashes * 2 + 1));
                out.push('"');
                slashes = 0;
            }
            _ => {
                out.extend(std::iter::repeat('\\').take(slashes));
                out.push(c);
                slashes = 0;
            }
        }
    }
    out.extend(std::iter::repeat('\\').take(slashes * 2));
    out.push('"');
    out
}

/// 目前是不是以系統管理員身分執行
#[cfg(windows)]
pub fn is_elevated() -> bool {
    unsafe { windows_sys::Win32::UI::Shell::IsUserAnAdmin() != 0 }
}

/// 沒有管理員權限時，用「以系統管理員身分執行」重新開一次自己。回傳 true = 這個要結束
#[cfg(windows)]
pub fn relaunch_if_needed() -> bool {
    use windows_sys::Win32::UI::Shell::{ShellExecuteExW, SEE_MASK_NOASYNC, SHELLEXECUTEINFOW};
    use windows_sys::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
    if is_elevated() {
        return false;
    }
    let Ok(exe) = std::env::current_exe() else { return false };
    let wide = |s: &str| s.encode_utf16().chain(std::iter::once(0)).collect::<Vec<u16>>();
    let args: Vec<String> = std::env::args_os().skip(1).map(|a| quote(&a.to_string_lossy())).collect();
    let verb = wide("runas");
    let file = wide(&exe.to_string_lossy());
    let params = wide(&args.join(" "));
    let dir = wide(&exe.parent().map(|p| p.to_string_lossy().into_owned()).unwrap_or_default());
    unsafe {
        let mut info: SHELLEXECUTEINFOW = std::mem::zeroed();
        info.cbSize = std::mem::size_of::<SHELLEXECUTEINFOW>() as u32;
        info.fMask = SEE_MASK_NOASYNC;
        info.lpVerb = verb.as_ptr();
        info.lpFile = file.as_ptr();
        info.lpParameters = params.as_ptr();
        info.lpDirectory = dir.as_ptr();
        info.nShow = SW_SHOWNORMAL;
        // 使用者按「否」時開不起來；不管怎樣，沒有權限的這個都結束
        ShellExecuteExW(&mut info);
    }
    true
}

#[cfg(test)]
mod tests {
    use super::quote;

    #[test]
    fn quotes_like_windows() {
        assert_eq!(quote("--autostart"), "--autostart");
        assert_eq!(quote("--after-update=123"), "--after-update=123");
        assert_eq!(quote(""), "\"\"");
        assert_eq!(quote(r"C:\My Games\a.txt"), r#""C:\My Games\a.txt""#);
        assert_eq!(quote(r"C:\My Dir\"), r#""C:\My Dir\\""#);
        assert_eq!(quote(r#"say "hi""#), r#""say \"hi\"""#);
    }
}
