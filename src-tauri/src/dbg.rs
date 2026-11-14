//! 查問題用的訊息：平常什麼都不印；啟動時有設環境變數 MAOBOO_DEBUG 才印到 stderr
//! （發行版沒有主控台，要把 stderr 導到檔案才看得到）。只印擴充功能、使用者腳本相關的經過，不印網頁內容。

use std::sync::OnceLock;

pub fn on() -> bool {
    static ON: OnceLock<bool> = OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("MAOBOO_DEBUG").is_some())
}

#[macro_export]
macro_rules! dlog {
    ($($t:tt)*) => {
        if $crate::dbg::on() {
            eprintln!($($t)*);
        }
    };
}
