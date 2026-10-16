fn main() {
    // 版本判斷用的 commit 與編號（GitHub Actions 提供），變了就重新編譯
    println!("cargo:rerun-if-env-changed=GITHUB_SHA");
    println!("cargo:rerun-if-env-changed=GITHUB_RUN_NUMBER");
    // 發行版要求以系統管理員身分執行（讓全域按鍵監聽在遊戲中也有效）。
    // 測試與開發版不套用，避免 cargo test 需要提權。
    let release = std::env::var("PROFILE").map(|p| p == "release").unwrap_or(false);
    if release {
        let windows = tauri_build::WindowsAttributes::new().app_manifest(include_str!("app.manifest"));
        tauri_build::try_build(tauri_build::Attributes::new().windows_attributes(windows))
            .expect("tauri build 失敗");
    } else {
        tauri_build::build()
    }
}
