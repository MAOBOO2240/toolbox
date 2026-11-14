//! 瀏覽器視窗跟 WebView2 之間傳遞的資料（只用標準函式庫，Windows 與測試環境共用）。

use std::sync::Arc;

/// WebView2 提供的一個右鍵選單項目
#[derive(Debug, Clone, Default, PartialEq)]
pub struct NativeItem {
    pub id: i32,
    /// 不隨語言改變的名稱（back、copy、saveImageAs…）
    pub name: String,
    pub label: String,
    /// 0 一般、1 核取、2 單選、3 分隔線、4 子選單
    pub kind: u8,
    pub enabled: bool,
    pub checked: bool,
    pub shortcut: String,
    pub children: Vec<NativeItem>,
}

/// 右鍵點到的東西
#[derive(Debug, Clone, Default, PartialEq)]
pub struct CtxTarget {
    /// 0 網頁、1 圖片、2 選取的文字、3 音訊、4 影片
    pub kind: u8,
    pub editable: bool,
    pub link: String,
    pub src: String,
    pub selection: String,
    pub page: String,
}

/// 下載的進度
#[derive(Debug, Clone, PartialEq)]
pub enum DlEv {
    Start { key: u64, url: String, path: String, total: i64 },
    Progress { key: u64, received: i64, total: i64 },
    /// state：done / canceled / failed
    Done { key: u64, state: &'static str, path: String, received: i64 },
}

/// WebView2 通知的事情
#[derive(Debug, Clone, PartialEq)]
pub enum Ev {
    /// 按了瀏覽器的快捷鍵（指令名稱見 webcore::shortcut）；"esc" 只是通知，按鍵照樣交給網頁
    Key(&'static str),
    /// 上一頁／下一頁能不能按
    Nav { back: bool, forward: bool },
    /// 網址變了（包含不換頁就換網址的網站）
    Source(String),
    Favicon(String),
    Audio { playing: bool, muted: bool },
    Zoom(f64),
    /// 網頁（影片）進入／離開全螢幕
    Fullscreen(bool),
    /// 網頁按了右鍵：要自己顯示選單，結束時用同一個 seq 呼叫 ctx_finish
    Context { seq: u64, target: CtxTarget, items: Vec<NativeItem> },
    Download(DlEv),
    /// 網頁自己呼叫 window.close()
    CloseRequested,
    /// 網頁裡的使用者腳本傳來的訊息（JSON 文字）。frame = 0 是最上層的網頁，其他是內嵌框架的編號；
    /// source = 傳訊息的那份文件的網址
    Gm { frame: u32, source: String, json: String },
    /// 這個網頁自己在瀏覽器裡的資料（DevTools 的 Target.getTargetInfo 回傳的 JSON 文字）
    SelfTarget(String),
    /// 瀏覽器裡多了一個頁面、分頁或背景程式（DevTools 的 Target.targetCreated 的 JSON 文字）。
    /// 可能是程式開的，也可能是擴充功能叫瀏覽器自己開的視窗
    Target(String),
}

pub type Sink = Arc<dyn Fn(Ev) + Send + Sync>;

pub struct Opts {
    /// 一般網頁（false = 工具列這種程式自己的頁面：不要右鍵選單、下載、瀏覽器內建的快捷鍵）
    pub page: bool,
    /// 快捷鍵對照（按鍵代碼、Ctrl、Shift、Alt）→ 指令
    pub accel: fn(u32, bool, bool, bool) -> Option<&'static str>,
    /// 按住不放時不要重複的指令
    pub no_repeat: fn(&str) -> bool,
    /// 這個網址是不是程式自己的頁面（設定、新分頁…）。網頁裡的內嵌框架不可以載入、
    /// 開發人員工具也不可以開在上面
    pub own_page: fn(&str) -> bool,
    /// 這個分頁一開始要載入的是一般網頁（第一個網頁就可以用開發人員工具；之後每次換網址再照網址決定）
    pub web_first: bool,
    /// 詢問要不要儲存密碼
    pub save_passwords: bool,
    /// 下載要存的資料夾（空白 = 不改，用 Windows 的「下載」）
    pub download_dir: String,
}

/// 清除瀏覽資料的種類（WebView2 的 COREWEBVIEW2_BROWSING_DATA_KINDS）
pub mod data_kinds {
    /// Cookie 與網站存的所有資料（登入狀態）
    pub const SITE: i32 = 128;
    /// 快取的圖片與檔案
    pub const CACHE: i32 = 256;
    pub const DOWNLOADS: i32 = 512;
    pub const AUTOFILL: i32 = 1024;
    pub const PASSWORDS: i32 = 2048;
    pub const HISTORY: i32 = 4096;
}
