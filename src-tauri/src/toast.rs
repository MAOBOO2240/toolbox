//! Windows 11 原生通知（右下角通知中心）。
//!
//! 免安裝的 exe 沒有註冊應用程式識別碼，Windows 不會顯示它的通知；
//! 所以啟動時先在 HKCU\Software\Classes\AppUserModelId 註冊一個（名稱、圖示），再用它發通知。

use std::path::PathBuf;
use std::sync::OnceLock;

pub const AUMID: &str = "MAOBOO.Toolbox";
static ICON: OnceLock<PathBuf> = OnceLock::new();

#[cfg(windows)]
fn hidden(cmd: &mut std::process::Command) -> &mut std::process::Command {
    use std::os::windows::process::CommandExt;
    cmd.creation_flags(0x0800_0000) // CREATE_NO_WINDOW
}

/// 註冊通知用的應用程式識別碼（名稱「工具箱」與圖示），每次啟動都檢查一次
pub fn init(data_dir: PathBuf) {
    let icon = data_dir.join("toast-icon.png");
    let _ = std::fs::create_dir_all(&data_dir);
    let _ = std::fs::write(&icon, include_bytes!("../icons/128x128.png"));
    let _ = ICON.set(icon.clone());
    // 直接寫登錄檔（不開 reg.exe）；內容沒變就不寫
    #[cfg(windows)]
    std::thread::spawn(move || {
        let key = format!(r"Software\Classes\AppUserModelId\{AUMID}");
        for (name, value) in [("DisplayName", "工具箱".to_string()), ("IconUri", icon.display().to_string())] {
            crate::winsys::reg::ensure(&key, name, &value);
        }
    });
}

fn esc(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

/// 發一則 Windows 通知（不帶聲音，聲音由程式自己播）
#[cfg(windows)]
pub fn show(title: &str, body: &str) -> Result<(), String> {
    use windows::core::HSTRING;
    use windows::Data::Xml::Dom::XmlDocument;
    use windows::UI::Notifications::{ToastNotification, ToastNotificationManager};
    let xml = format!(
        r#"<toast scenario="reminder"><visual><binding template="ToastGeneric"><text>{}</text><text>{}</text></binding></visual><audio silent="true"/><actions><action content="關閉" arguments="dismiss" activationType="system"/></actions></toast>"#,
        esc(title),
        esc(body)
    );
    let doc = XmlDocument::new().map_err(|e| e.to_string())?;
    doc.LoadXml(&HSTRING::from(xml)).map_err(|e| e.to_string())?;
    let toast = ToastNotification::CreateToastNotification(&doc).map_err(|e| e.to_string())?;
    let notifier = ToastNotificationManager::CreateToastNotifierWithId(&HSTRING::from(AUMID)).map_err(|e| e.to_string())?;
    notifier.Show(&toast).map_err(|e| e.to_string())
}

#[cfg(not(windows))]
pub fn show(_title: &str, _body: &str) -> Result<(), String> {
    Err("只支援 Windows".into())
}

/// Windows 是否允許這個程式發通知：enabled / app（關閉了這個程式）/ user（關閉了全部通知）/ policy / unknown
#[cfg(windows)]
pub fn status() -> String {
    use windows::core::HSTRING;
    use windows::UI::Notifications::{NotificationSetting, ToastNotificationManager};
    let Ok(n) = ToastNotificationManager::CreateToastNotifierWithId(&HSTRING::from(AUMID)) else { return "unknown".into() };
    match n.Setting() {
        Ok(v) if v == NotificationSetting::Enabled => "enabled",
        Ok(v) if v == NotificationSetting::DisabledForApplication => "app",
        Ok(v) if v == NotificationSetting::DisabledForUser => "user",
        Ok(v) if v == NotificationSetting::DisabledByGroupPolicy => "policy",
        _ => "unknown",
    }
    .into()
}

#[cfg(not(windows))]
pub fn status() -> String {
    "unknown".into()
}

#[tauri::command]
pub async fn toast_status() -> String {
    tauri::async_runtime::spawn_blocking(status).await.unwrap_or_else(|_| "unknown".into())
}

/// 打開 Windows 的「通知」設定頁
#[tauri::command]
pub fn open_notify_settings() {
    #[cfg(windows)]
    {
        // 和一般程式開設定頁的方式一樣；萬一開不起來才透過檔案總管開
        if crate::winsys::open("ms-settings:notifications").is_err() {
            let _ = hidden(std::process::Command::new("explorer.exe").arg("ms-settings:notifications")).spawn();
        }
    }
}
