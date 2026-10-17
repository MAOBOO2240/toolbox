fn main() {
    // 版本判斷用的 commit 與編號（GitHub Actions 提供），變了就重新編譯
    println!("cargo:rerun-if-env-changed=GITHUB_SHA");
    println!("cargo:rerun-if-env-changed=GITHUB_RUN_NUMBER");
    embed_update_token();
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

/// 把 GitHub Actions 的祕密 UPDATE_TOKEN 混淆後編進程式（原文不會出現在 exe 或原始碼裡）。
/// 每次編譯用不同的種子產生金鑰流，再把結果打散存成兩段。
fn embed_update_token() {
    println!("cargo:rerun-if-env-changed=UPDATE_TOKEN");
    let token = std::env::var("UPDATE_TOKEN").unwrap_or_default();
    let token = token.trim().as_bytes();
    let seed: u64 = {
        let t = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos() as u64).unwrap_or(0x9E37_79B9);
        let p = std::process::id() as u64;
        (t ^ p.rotate_left(29) ^ 0xA076_1D64_78BD_642F) | 1
    };
    let mut x = seed;
    let mut enc: Vec<u8> = Vec::with_capacity(token.len());
    for (i, b) in token.iter().enumerate() {
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        let k = (x >> ((i % 7) * 8)) as u8;
        enc.push(b ^ k ^ (i as u8).wrapping_mul(0x5B));
    }
    // 奇偶位置分成兩段，倒過來存
    let even: Vec<u8> = enc.iter().step_by(2).rev().copied().collect();
    let odd: Vec<u8> = enc.iter().skip(1).step_by(2).rev().copied().collect();
    let out = std::path::PathBuf::from(std::env::var("OUT_DIR").unwrap()).join("update_token.rs");
    let code = format!(
        "const K_SEED: u64 = {seed:#x};\nconst K_LEN: usize = {};\nstatic K_A: [u8; {}] = {:?};\nstatic K_B: [u8; {}] = {:?};\n",
        enc.len(),
        even.len(),
        even,
        odd.len(),
        odd
    );
    std::fs::write(out, code).expect("無法寫入 update_token.rs");
}
