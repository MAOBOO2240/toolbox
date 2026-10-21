//! 序號擷取規則與狀態保存。

use regex::Regex;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

pub const MIN_LEN: usize = 15;
pub const MAX_LEN: usize = 25;

/// 從混雜文字中擷取序號。
///
/// 規則：
/// - 只取「完整的」英數字片段（前後不可緊接英數字），避免從更長的字串中切出一段。
/// - 長度 15–25 碼。
/// - 排除純數字（例如時間戳記）。
/// - 去除重複，保留第一次出現的順序。
pub fn extract(text: &str) -> Vec<String> {
    static RE: OnceLock<Regex> = OnceLock::new();
    let re = RE.get_or_init(|| Regex::new(r"[A-Za-z0-9]+").unwrap());

    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for m in re.find_iter(text) {
        let s = m.as_str();
        let len = s.len();
        if !(MIN_LEN..=MAX_LEN).contains(&len) {
            continue;
        }
        if s.bytes().all(|b| b.is_ascii_digit()) {
            continue;
        }
        if seen.insert(s.to_string()) {
            out.push(s.to_string());
        }
    }
    out
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// "system" | "light" | "dark"
    pub theme: String,
    /// "s" | "m" | "l"
    pub font: String,
    /// 使用者調整過的主視窗大小（畫面像素，跟螢幕等比例；舊版的實際像素大小不再使用）
    pub view_w: Option<u32>,
    pub view_h: Option<u32>,
    /// 按右上角關閉時縮小到右下角（系統匣），而不是結束程式
    pub close_tray: bool,
    /// 第一次按關閉時已經問過要直接關閉還是縮小（只問一次）
    pub close_asked: bool,
    /// 更新方式：false＝每次開啟程式檢查更新；true＝不自動檢查，想更新時自己按「檢查更新」
    pub upd_manual: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self { theme: "system".into(), font: "m".into(), view_w: None, view_h: None, close_tray: false, close_asked: false, upd_manual: false }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct AppState {
    /// 使用者貼上的原始文字
    pub raw: String,
    /// 擷取出的序號清單
    pub serials: Vec<String>,
    /// 目前序號的位置；等於 serials.len() 代表全部用完
    pub index: usize,
    /// 是否啟用自動切換
    pub active: bool,
    pub settings: Settings,
}

impl AppState {
    pub fn current(&self) -> Option<&str> {
        self.serials.get(self.index).map(|s| s.as_str())
    }

    pub fn load(path: &Path) -> Self {
        let mut st: AppState = std::fs::read_to_string(path)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default();
        if st.index > st.serials.len() {
            st.index = st.serials.len();
        }
        st
    }

    /// 先寫入暫存檔再改名，避免寫到一半當機造成檔案損毀。
    pub fn save(&self, path: &Path) {
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let tmp: PathBuf = path.with_extension("json.tmp");
        if let Ok(json) = serde_json::to_string_pretty(self) {
            if std::fs::write(&tmp, json).is_ok() {
                let _ = std::fs::rename(&tmp, path);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_mixed_text() {
        let text = "2026-10-16 08:15:00 道具：鑽石x100 序號 ABCDE12345FGHIJ6789 已發送\n\
                    時間戳 1760573700000123 不算\n\
                    重複 ABCDE12345FGHIJ6789\n\
                    太長 ABCDEFGHIJKLMNOPQRSTUVWXYZ0123 不算\n\
                    太短 ABC123 不算\n\
                    小寫 abcde12345fghij99";
        assert_eq!(extract(text), vec!["ABCDE12345FGHIJ6789", "abcde12345fghij99"]);
    }

    #[test]
    fn boundaries_with_cjk_and_symbols() {
        let text = "【獎勵】XyZ987654321abcde【結束】|QWERTYUIOP12345";
        assert_eq!(extract(text), vec!["XyZ987654321abcde", "QWERTYUIOP12345"]);
    }
}
