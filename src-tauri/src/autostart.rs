//! 開機自動啟動：用 Windows「工作排程器」在登入時啟動程式。
//! 程式需要系統管理員權限，Windows 不會從「啟動」（Run 機碼）提權開啟這種程式（會直接被略過），
//! 所以改用工作排程器的「以最高權限執行」。開機時帶 --autostart 參數啟動，只在右下角背景執行（斷線通知、計時器照常運作）。

pub const ARG: &str = "--autostart";
#[cfg(windows)]
const TASK: &str = "MAOBOO_Toolbox";
/// 舊版用的 Run 機碼（提權的程式不會從這裡啟動），開啟或關閉時一律清掉
#[cfg(windows)]
const OLD_KEY: &str = r"HKCU\Software\Microsoft\Windows\CurrentVersion\Run";
#[cfg(windows)]
const OLD_NAME: &str = "MAOBOO_Toolbox";

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;").replace('\'', "&apos;")
}

/// 工作排程器的工作內容：登入後 10 秒以最高權限啟動；不限制執行時間、用電池時也會啟動
fn task_xml(user: &str, exe: &str, dir: &str) -> String {
    let (user, exe, dir) = (xml_escape(user), xml_escape(exe), xml_escape(dir));
    format!(
        r#"<?xml version="1.0" encoding="UTF-16"?>
<Task version="1.2" xmlns="http://schemas.microsoft.com/windows/2004/02/mit/task">
  <RegistrationInfo>
    <Description>工具箱：登入時自動啟動（只在右下角背景執行）</Description>
  </RegistrationInfo>
  <Triggers>
    <LogonTrigger>
      <Enabled>true</Enabled>
      <UserId>{user}</UserId>
      <Delay>PT10S</Delay>
    </LogonTrigger>
  </Triggers>
  <Principals>
    <Principal id="Author">
      <UserId>{user}</UserId>
      <LogonType>InteractiveToken</LogonType>
      <RunLevel>HighestAvailable</RunLevel>
    </Principal>
  </Principals>
  <Settings>
    <MultipleInstancesPolicy>IgnoreNew</MultipleInstancesPolicy>
    <DisallowStartIfOnBatteries>false</DisallowStartIfOnBatteries>
    <StopIfGoingOnBatteries>false</StopIfGoingOnBatteries>
    <StartWhenAvailable>false</StartWhenAvailable>
    <RunOnlyIfNetworkAvailable>false</RunOnlyIfNetworkAvailable>
    <IdleSettings>
      <StopOnIdleEnd>false</StopOnIdleEnd>
      <RestartOnIdle>false</RestartOnIdle>
    </IdleSettings>
    <AllowStartOnDemand>true</AllowStartOnDemand>
    <Enabled>true</Enabled>
    <Hidden>false</Hidden>
    <RunOnlyIfIdle>false</RunOnlyIfIdle>
    <WakeToRun>false</WakeToRun>
    <ExecutionTimeLimit>PT0S</ExecutionTimeLimit>
    <Priority>4</Priority>
  </Settings>
  <Actions Context="Author">
    <Exec>
      <Command>"{exe}"</Command>
      <Arguments>{ARG}</Arguments>
      <WorkingDirectory>{dir}</WorkingDirectory>
    </Exec>
  </Actions>
</Task>
"#
    )
}

/// 呼叫系統程式（不顯示主控台視窗），回傳是否成功
#[cfg(windows)]
fn run(cmd: &str, args: &[&str]) -> bool {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    std::process::Command::new(cmd)
        .args(args)
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

#[cfg(windows)]
fn enabled() -> bool {
    run("schtasks", &["/Query", "/TN", TASK])
}

#[cfg(not(windows))]
fn enabled() -> bool {
    false
}

/// 舊版的 Run 機碼還在嗎（開過舊版的開機啟動）
#[cfg(windows)]
fn old_enabled() -> bool {
    run("reg", &["query", OLD_KEY, "/v", OLD_NAME])
}

#[cfg(windows)]
fn remove_old() {
    if old_enabled() {
        run("reg", &["delete", OLD_KEY, "/v", OLD_NAME, "/f"]);
    }
}

#[cfg(windows)]
fn write(on: bool) -> Result<(), String> {
    remove_old();
    if on {
        let exe = std::env::current_exe().map_err(|e| e.to_string())?;
        let dir = exe.parent().map(|p| p.display().to_string()).unwrap_or_default();
        let user = match (std::env::var("USERDOMAIN"), std::env::var("USERNAME")) {
            (Ok(d), Ok(u)) if !d.is_empty() => format!("{d}\\{u}"),
            (_, Ok(u)) => u,
            _ => return Err("無法取得目前的使用者名稱".into()),
        };
        // 工作排程器的 XML 要用 UTF-16 存檔
        let xml = task_xml(&user, &exe.display().to_string(), &dir);
        let mut bytes = vec![0xFF, 0xFE];
        for u in xml.encode_utf16() {
            bytes.extend_from_slice(&u.to_le_bytes());
        }
        let path = std::env::temp_dir().join(format!("maoboo_autostart_{}.xml", std::process::id()));
        std::fs::write(&path, bytes).map_err(|e| format!("無法寫入開機啟動設定：{e}"))?;
        let ok = run("schtasks", &["/Create", "/TN", TASK, "/XML", &path.display().to_string(), "/F"]);
        let _ = std::fs::remove_file(&path);
        if !ok {
            return Err("無法建立開機啟動的排程工作".into());
        }
    } else if enabled() && !run("schtasks", &["/Delete", "/TN", TASK, "/F"]) {
        return Err("無法移除開機啟動的排程工作".into());
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

/// 程式啟動時：已開啟就更新成目前程式的位置（程式搬到別的資料夾也能正常開機啟動）；
/// 開過舊版（Run 機碼）的開機啟動，改成工作排程器
pub fn refresh() {
    #[cfg(windows)]
    {
        if enabled() || old_enabled() {
            let _ = write(true);
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
    fn task_xml_escapes_and_runs_elevated() {
        let x = task_xml(r"PC\小明", r"C:\Games & Tools\MAOBOO.exe", r"C:\Games & Tools");
        assert!(x.contains(r"<UserId>PC\小明</UserId>"));
        assert!(x.contains(r#"<Command>"C:\Games &amp; Tools\MAOBOO.exe"</Command>"#));
        assert!(x.contains("<Arguments>--autostart</Arguments>"));
        assert!(x.contains("<RunLevel>HighestAvailable</RunLevel>"));
        assert!(x.contains("<ExecutionTimeLimit>PT0S</ExecutionTimeLimit>"));
        assert!(x.contains("<DisallowStartIfOnBatteries>false</DisallowStartIfOnBatteries>"));
    }
}
