//! 開機自動啟動：寫在 Windows 的「啟動」清單（目前使用者的 Run 機碼），登入後以一般權限開啟，
//! 程式再自己要求系統管理員權限（elevate.rs，會跳出權限確認視窗，按「是」才會開）。
//! 開機時帶 --autostart 參數啟動，只在右下角背景執行（斷線通知、計時器照常運作）。
//!
//! 舊版用工作排程器建立「登入時以最高權限執行」的工作，會被 Windows Defender 誤判成惡意程式；
//! 現在不再使用工作排程器，舊版留下的排程工作會在程式開啟時移除，並改成「啟動」清單。

pub const ARG: &str = "--autostart";
#[cfg(windows)]
const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
#[cfg(windows)]
const NAME: &str = "MAOBOO_Toolbox";
/// 舊版建立的排程工作名稱
#[cfg(windows)]
const OLD_TASK: &str = "MAOBOO_Toolbox";

/// 「啟動」清單裡的命令列
#[cfg_attr(not(windows), allow(dead_code))]
fn command_line(exe: &str) -> String {
    format!("\"{exe}\" {ARG}")
}

#[cfg(windows)]
use crate::winsys::reg;

#[cfg(windows)]
fn current_command() -> Result<String, String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    Ok(command_line(&exe.display().to_string()))
}

#[cfg(windows)]
fn enabled() -> bool {
    reg::get(RUN_KEY, NAME).is_some()
}

#[cfg(not(windows))]
fn enabled() -> bool {
    false
}

/// 舊版建立的排程工作還在嗎
#[cfg(windows)]
fn old_task_exists() -> bool {
    let root = std::env::var("SystemRoot").unwrap_or_else(|_| r"C:\Windows".into());
    std::path::Path::new(&root).join("System32").join("Tasks").join(OLD_TASK).exists()
}

/// 移除舊版的排程工作（只有還在時才動作）
#[cfg(windows)]
fn remove_old_task() {
    if !old_task_exists() {
        return;
    }
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let _ = std::process::Command::new("schtasks").args(["/Delete", "/TN", OLD_TASK, "/F"]).creation_flags(CREATE_NO_WINDOW).output();
}

#[cfg(windows)]
fn write(on: bool) -> Result<(), String> {
    remove_old_task();
    if on {
        if !reg::set(RUN_KEY, NAME, &current_command()?) {
            return Err("無法加入 Windows 的啟動清單".into());
        }
    } else if !reg::delete(RUN_KEY, NAME) {
        return Err("無法從 Windows 的啟動清單移除".into());
    }
    Ok(())
}

#[cfg(not(windows))]
fn write(_on: bool) -> Result<(), String> {
    Err("只支援 Windows".into())
}

/// 這次是不是開機自動啟動的（是的話不顯示主視窗）
pub fn launched_at_boot() -> bool {
    std::env::args().any(|a| a == ARG)
}

/// 程式啟動時：舊版的排程工作移除並改成「啟動」清單；程式搬到別的資料夾時更新位置。
/// 內容沒變就不寫入
pub fn refresh() {
    #[cfg(windows)]
    {
        let had_task = old_task_exists();
        if had_task {
            remove_old_task();
        }
        let Ok(want) = current_command() else { return };
        match reg::get(RUN_KEY, NAME) {
            Some(v) if v != want => {
                reg::set(RUN_KEY, NAME, &want);
            }
            None if had_task => {
                reg::set(RUN_KEY, NAME, &want);
            }
            _ => {}
        }
    }
}

#[tauri::command]
pub async fn autostart_get() -> bool {
    tauri::async_runtime::spawn_blocking(enabled).await.unwrap_or(false)
}

#[tauri::command]
pub async fn autostart_set(on: bool) -> Result<bool, String> {
    tauri::async_runtime::spawn_blocking(move || {
        write(on)?;
        Ok(enabled())
    })
    .await
    .map_err(|e| e.to_string())?
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn startup_command_quotes_path() {
        assert_eq!(command_line(r"C:\Games & Tools\MAOBOO.exe"), r#""C:\Games & Tools\MAOBOO.exe" --autostart"#);
    }
}
