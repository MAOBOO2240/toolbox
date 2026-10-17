//! 開機自動啟動：在「目前使用者」的 Run 機碼寫入程式路徑。
//! 開機時帶 --autostart 參數啟動，只在右下角背景執行（斷線通知、計時器照常運作）。

pub const ARG: &str = "--autostart";
#[cfg(windows)]
const KEY: &str = r"HKCU\Software\Microsoft\Windows\CurrentVersion\Run";
#[cfg(windows)]
const NAME: &str = "MAOBOO_Toolbox";

/// 呼叫系統的 reg.exe（不顯示主控台視窗），成功時回傳輸出
#[cfg(windows)]
fn reg(args: &[&str]) -> Option<String> {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let out = std::process::Command::new("reg").args(args).creation_flags(CREATE_NO_WINDOW).output().ok()?;
    if out.status.success() {
        Some(String::from_utf8_lossy(&out.stdout).into_owned())
    } else {
        None
    }
}

#[cfg(windows)]
fn enabled() -> bool {
    reg(&["query", KEY, "/v", NAME]).is_some()
}

#[cfg(not(windows))]
fn enabled() -> bool {
    false
}

#[cfg(windows)]
fn write(on: bool) -> Result<(), String> {
    if on {
        let exe = std::env::current_exe().map_err(|e| e.to_string())?;
        let data = format!("\"{}\" {ARG}", exe.display());
        reg(&["add", KEY, "/v", NAME, "/t", "REG_SZ", "/d", &data, "/f"]).ok_or("無法寫入開機啟動設定")?;
    } else if enabled() {
        reg(&["delete", KEY, "/v", NAME, "/f"]).ok_or("無法移除開機啟動設定")?;
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

/// 已開啟時更新成目前程式的位置（程式搬到別的資料夾也能正常開機啟動）
pub fn refresh() {
    if enabled() {
        let _ = write(true);
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
