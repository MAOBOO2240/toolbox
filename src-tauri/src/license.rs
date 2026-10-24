//! 授權：每台電腦有一組獨一無二的序號，使用者把序號給作者，作者用私鑰簽出專屬這個序號的授權碼。
//!
//! - 序號 = 這台電腦的 MachineGuid 與電腦名稱做雜湊，顯示成 MB-XXXX-XXXX-XXXX-XXXX
//! - 授權碼 = 用 Ed25519 私鑰對序號簽名，程式用內建的公鑰驗證；沒有網路也能驗
//! - 私鑰只在作者電腦上（app 資料夾的 license-admin.key），程式碼公開也沒關係
//! - 「工具箱」分類的工具要授權才能開；楓之谷工具不用
//! - 公會自動授權：使用者輸入角色名稱，Worker 查公會，符合就用 Worker 自己的金鑰簽授權碼回來（格式跟手簽的一樣）
use ed25519_dalek::{Signature, Signer, SigningKey, Verifier, VerifyingKey};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};

/// 作者的公鑰（十六進位，32 bytes）。作者在「授權管理」產生金鑰後把公鑰貼到這裡；空字串 = 還沒設定（所有授權碼都不會通過）
const PUBKEY_HEX: &str = "8f20ec48c8263df403ac0a5393159aae7dd09fcd645d629f9e0c449cb42d82a3";
/// Worker 公會自動授權用的公鑰（儀表板「產生金鑰」後顯示的那串）。空字串 = 自動授權還沒開
const PUBKEY2_HEX: &str = "4eb9e3b1ce1e2ef896ec5c9d8e2503d58fc27e4f31459703caffe60b6acee412";

static DIR: OnceLock<PathBuf> = OnceLock::new();
static CACHE: Mutex<Option<Info>> = Mutex::new(None);

pub fn init(dir: PathBuf) {
    let _ = DIR.set(dir);
}

fn dir() -> PathBuf {
    DIR.get().cloned().unwrap_or_default()
}

fn license_path() -> PathBuf {
    dir().join("license.json")
}

fn admin_key_path() -> PathBuf {
    dir().join("license-admin.key")
}

#[derive(Default, Serialize, Deserialize)]
#[serde(default)]
struct Saved {
    code: String,
    /// 綁定的角色（左下角那隻；公會成員綁定時順便自動授權）
    name: String,
    world: String,
    guild: String,
    /// 授權碼是靠公會自動授權拿到的（解除綁定時要一起收回）
    auto: bool,
}

fn read_saved() -> Saved {
    std::fs::read_to_string(license_path()).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default()
}

fn write_saved(s: &Saved) -> Result<(), String> {
    let _ = std::fs::create_dir_all(dir());
    std::fs::write(license_path(), serde_json::to_string(s).unwrap_or_default()).map_err(|e| e.to_string())?;
    *CACHE.lock().unwrap_or_else(|e| e.into_inner()) = None;
    Ok(())
}

/// 自動授權綁定的角色名稱（回報使用狀況用）；沒有就是空字串
pub fn bound_name() -> String {
    read_saved().name
}

/// 作者在儀表板移除了這台電腦的授權：刪掉本機存的授權碼。回傳有沒有真的刪掉東西（作者模式不受影響）
pub fn revoke_local() -> bool {
    if read_saved().code.is_empty() {
        return false;
    }
    let _ = std::fs::remove_file(license_path());
    *CACHE.lock().unwrap_or_else(|e| e.into_inner()) = None;
    true
}

/// Worker 回的狀態：有沒有被移除授權、這台目前綁的角色
#[derive(Deserialize, Default)]
#[serde(default)]
pub struct Remote {
    pub revoked: bool,
    pub bound: String,
}

/// 問 Worker 這台的狀態；連不上回 None
fn remote_status(serial: &str) -> Option<Remote> {
    let url = crate::stats::STATS_URL.trim_end_matches('/');
    if url.is_empty() {
        return None;
    }
    let body = serde_json::json!({ "serial": serial }).to_string();
    let r = ureq::AgentBuilder::new().timeout(std::time::Duration::from_secs(4)).build().post(&format!("{url}/check")).set("Content-Type", "application/json").send_string(&body).ok()?;
    serde_json::from_str(&r.into_string().ok()?).ok()
}

fn revoked_remotely(serial: &str) -> bool {
    remote_status(serial).map(|r| r.revoked).unwrap_or(false)
}

/// 套用 Worker 回的狀態：被移除授權 → 刪授權碼；綁定被解除 → 清掉角色（靠公會拿到的授權碼一起收回）。回傳有沒有改到東西
pub fn apply_remote(r: &Remote) -> bool {
    let s = read_saved();
    if r.revoked && !s.code.is_empty() {
        return revoke_local();
    }
    if !s.name.is_empty() && r.bound != s.name {
        if s.auto {
            return revoke_local();
        }
        let _ = write_saved(&Saved { code: s.code, ..Default::default() });
        return true;
    }
    false
}

/// 這台電腦的序號
pub fn serial() -> String {
    let raw = format!("MAOBOO|{}|{}", machine_guid(), computer_name().to_uppercase());
    let h = crate::sha256::hex(raw.as_bytes());
    // 取前 80 bits 轉成 16 個好讀的字（去掉 0 O 1 I）
    const ALPHABET: &[u8] = b"23456789ABCDEFGHJKLMNPQRSTUVWXYZ";
    let bytes = hex_bytes(&h[..20]);
    let mut acc: u128 = 0;
    for b in &bytes {
        acc = (acc << 8) | *b as u128;
    }
    let mut chars = Vec::new();
    for _ in 0..16 {
        chars.push(ALPHABET[(acc & 31) as usize] as char);
        acc >>= 5;
    }
    let s: String = chars.into_iter().collect();
    format!("MB-{}-{}-{}-{}", &s[0..4], &s[4..8], &s[8..12], &s[12..16])
}

fn hex_bytes(h: &str) -> Vec<u8> {
    (0..h.len() / 2).filter_map(|i| u8::from_str_radix(&h[i * 2..i * 2 + 2], 16).ok()).collect()
}

#[cfg(windows)]
fn machine_guid() -> String {
    use windows_sys::Win32::System::Registry::{RegGetValueW, HKEY_LOCAL_MACHINE, RRF_RT_REG_SZ, RRF_SUBKEY_WOW6464KEY};
    let sub: Vec<u16> = "SOFTWARE\\Microsoft\\Cryptography".encode_utf16().chain(std::iter::once(0)).collect();
    let name: Vec<u16> = "MachineGuid".encode_utf16().chain(std::iter::once(0)).collect();
    unsafe {
        let mut buf = [0u16; 128];
        let mut len = (buf.len() * 2) as u32;
        let flags = RRF_RT_REG_SZ | RRF_SUBKEY_WOW6464KEY;
        if RegGetValueW(HKEY_LOCAL_MACHINE, sub.as_ptr(), name.as_ptr(), flags, std::ptr::null_mut(), buf.as_mut_ptr().cast(), &mut len) == 0 {
            let n = (len as usize / 2).saturating_sub(1).min(buf.len());
            return String::from_utf16_lossy(&buf[..n]).trim().to_string();
        }
    }
    String::new()
}

#[cfg(not(windows))]
fn machine_guid() -> String {
    std::fs::read_to_string("/etc/machine-id").unwrap_or_default().trim().to_string()
}

fn computer_name() -> String {
    std::env::var("COMPUTERNAME").or_else(|_| std::env::var("HOSTNAME")).unwrap_or_default()
}

fn key_of(hex: &str) -> Option<VerifyingKey> {
    let b = hex_bytes(hex.trim());
    let arr: [u8; 32] = b.try_into().ok()?;
    VerifyingKey::from_bytes(&arr).ok()
}

fn pubkey() -> Option<VerifyingKey> {
    key_of(PUBKEY_HEX)
}

/// 能驗授權碼的公鑰：作者手簽的 + Worker 自動授權的
fn pubkeys() -> Vec<VerifyingKey> {
    [PUBKEY_HEX, PUBKEY2_HEX].iter().filter_map(|h| key_of(h)).collect()
}

/// 授權碼：MB1- 加上簽名的 base64url
fn encode_code(sig: &Signature) -> String {
    use base64::Engine;
    format!("MB1-{}", base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(sig.to_bytes()))
}

fn decode_code(code: &str) -> Option<Signature> {
    use base64::Engine;
    let body = code.trim().strip_prefix("MB1-")?;
    let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(body.trim()).ok()?;
    let arr: [u8; 64] = bytes.try_into().ok()?;
    Some(Signature::from_bytes(&arr))
}

/// 授權碼對這個序號有沒有效
pub fn verify_for(serial: &str, code: &str) -> bool {
    let Some(sig) = decode_code(code) else { return false };
    pubkeys().iter().any(|pk| pk.verify(serial.as_bytes(), &sig).is_ok())
}

#[derive(Clone, Serialize)]
pub struct Info {
    pub serial: String,
    pub ok: bool,
    /// 作者模式（有私鑰檔）
    pub admin: bool,
    /// 公鑰還沒放進程式（開發中的版本）
    pub no_pubkey: bool,
    /// 綁定的角色（空 = 還沒綁）
    pub name: String,
    pub world: String,
    pub guild: String,
    /// 這個版本有沒有開綁定角色／公會自動授權
    pub auto_available: bool,
}

fn compute() -> Info {
    let serial = serial();
    let saved = read_saved();
    let no_pubkey = pubkey().is_none();
    // 作者模式：私鑰跟程式內建的公鑰相符才算（隨便放一個私鑰檔不會變成作者）；還沒內建公鑰的版本有私鑰就算，方便作者第一次產生金鑰
    let admin = match admin_key() {
        Some(k) => no_pubkey || crate::sha256::hex_of(&k.verifying_key().to_bytes()) == PUBKEY_HEX.trim().to_lowercase(),
        None => false,
    };
    // 還沒內建公鑰的版本：所有工具都能用（過渡期）
    let ok = no_pubkey || admin || verify_for(&serial, &saved.code);
    let auto_available = key_of(PUBKEY2_HEX).is_some() && !crate::stats::STATS_URL.trim().is_empty();
    Info { ok, serial, admin, no_pubkey, name: saved.name, world: saved.world, guild: saved.guild, auto_available }
}

pub fn info() -> Info {
    let mut c = CACHE.lock().unwrap_or_else(|e| e.into_inner());
    if c.is_none() {
        *c = Some(compute());
    }
    c.clone().unwrap()
}

pub fn licensed() -> bool {
    info().ok
}

#[tauri::command]
pub fn license_info() -> Info {
    info()
}

/// 輸入授權碼
#[tauri::command]
pub fn license_activate(code: String) -> Result<Info, String> {
    let code = code.trim().to_string();
    let serial = serial();
    if !verify_for(&serial, &code) {
        return Err("授權碼不正確，或不是這台電腦的（請確認序號有給對）".into());
    }
    if revoked_remotely(&serial) {
        return Err("這台電腦的授權已被作者移除".into());
    }
    write_saved(&Saved { code, ..Default::default() })?;
    Ok(info())
}

/// 開工具前即時問一次 Worker：有沒有被作者移除授權、綁定還在不在（作者模式只看綁定）
#[tauri::command]
pub fn license_check() -> Info {
    let i = info();
    let s = read_saved();
    if !s.code.is_empty() || !s.name.is_empty() {
        if let Some(r) = remote_status(&i.serial) {
            let r = if i.admin { Remote { revoked: false, ..r } } else { r };
            apply_remote(&r);
        }
    }
    info()
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct AutoResp {
    ok: bool,
    msg: String,
    code: String,
    signed: bool,
    name: String,
    world: String,
    guild: String,
}

/// 綁定角色：把序號和角色名稱交給 Worker。已授權的只綁定；還沒授權的公會符合就順便拿回授權碼
#[tauri::command]
pub fn license_bind(name: String) -> Result<Info, String> {
    let name = name.trim().to_string();
    if name.is_empty() {
        return Err("請輸入角色名稱".into());
    }
    let cur = info();
    if !cur.auto_available {
        return Err("這個版本還沒開放綁定角色".into());
    }
    let serial = serial();
    let body = serde_json::json!({ "serial": serial, "name": name, "licensed": cur.ok });
    let resp = ureq::AgentBuilder::new()
        .timeout(std::time::Duration::from_secs(25))
        .build()
        .post(&format!("{}/auto", crate::stats::STATS_URL.trim_end_matches('/')))
        .set("User-Agent", "MAOBOO-toolbox")
        .set("Content-Type", "application/json")
        .send_string(&body.to_string());
    let text = match resp {
        Ok(r) | Err(ureq::Error::Status(_, r)) => r.into_string().map_err(|e| format!("回應讀不到：{e}"))?,
        Err(e) => return Err(format!("無法連線到授權伺服器：{e}")),
    };
    let r: AutoResp = serde_json::from_str(&text).map_err(|_| "授權伺服器回應看不懂".to_string())?;
    if !r.ok {
        return Err(if r.msg.is_empty() { "綁定失敗".into() } else { r.msg });
    }
    let old = read_saved();
    if r.signed {
        if !verify_for(&serial, &r.code) {
            return Err("伺服器回的授權碼驗證不過（程式內建的公鑰可能跟伺服器不符）".into());
        }
        write_saved(&Saved { code: r.code, name: r.name, world: r.world, guild: r.guild, auto: true })?;
    } else {
        write_saved(&Saved { code: old.code, name: r.name, world: r.world, guild: r.guild, auto: old.auto })?;
    }
    Ok(info())
}

/* ---------------- 作者用 ---------------- */

fn admin_key() -> Option<SigningKey> {
    let t = std::fs::read_to_string(admin_key_path()).ok()?;
    let b = hex_bytes(t.trim());
    let arr: [u8; 32] = b.try_into().ok()?;
    Some(SigningKey::from_bytes(&arr))
}

#[derive(Serialize)]
pub struct AdminInfo {
    pub has_key: bool,
    pub pubkey: String,
    /// 程式內建的公鑰跟私鑰配不配（不配的話簽出來的授權碼使用者那邊不會過）
    pub matches_builtin: bool,
    pub key_path: String,
}

#[tauri::command]
pub fn license_admin_info() -> AdminInfo {
    let k = admin_key();
    let pk = k.as_ref().map(|k| crate::sha256::hex_of(&k.verifying_key().to_bytes())).unwrap_or_default();
    AdminInfo { has_key: k.is_some(), matches_builtin: !pk.is_empty() && pk == PUBKEY_HEX.trim().to_lowercase(), pubkey: pk, key_path: admin_key_path().to_string_lossy().into_owned() }
}

/// 產生新的金鑰（已經有就不覆蓋）
#[tauri::command]
pub fn license_admin_keygen() -> Result<AdminInfo, String> {
    if admin_key_path().is_file() {
        return Err("已經有私鑰了，不會覆蓋（要重做請先把私鑰檔移走）".into());
    }
    let mut rng = rand::rngs::OsRng;
    let sk = SigningKey::generate(&mut rng);
    let _ = std::fs::create_dir_all(dir());
    std::fs::write(admin_key_path(), crate::sha256::hex_of(&sk.to_bytes())).map_err(|e| e.to_string())?;
    *CACHE.lock().unwrap_or_else(|e| e.into_inner()) = None;
    Ok(license_admin_info())
}

/// 對某個序號簽出授權碼
#[tauri::command]
pub fn license_admin_sign(serial: String) -> Result<String, String> {
    let sk = admin_key().ok_or("沒有私鑰")?;
    let s = serial.trim().to_uppercase();
    if !s.starts_with("MB-") || s.len() != 22 {
        return Err("序號格式不對，應該像 MB-XXXX-XXXX-XXXX-XXXX".into());
    }
    Ok(encode_code(&sk.sign(s.as_bytes())))
}

/// 後台一次性密碼：用作者私鑰簽「maboo-admin|目前分鐘數」，Worker 用內建的作者公鑰驗、用過就作廢
#[tauri::command]
pub fn license_admin_otp() -> Result<String, String> {
    let sk = admin_key().ok_or("沒有私鑰")?;
    let ts = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() / 60).unwrap_or(0);
    let sig = sk.sign(format!("maboo-admin|{ts}").as_bytes());
    use base64::Engine;
    Ok(format!("MA1-{}-{}", base36(ts), base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(sig.to_bytes())))
}

fn base36(mut n: u64) -> String {
    const D: &[u8] = b"0123456789abcdefghijklmnopqrstuvwxyz";
    if n == 0 {
        return "0".into();
    }
    let mut out = Vec::new();
    while n > 0 {
        out.push(D[(n % 36) as usize]);
        n /= 36;
    }
    out.reverse();
    String::from_utf8(out).unwrap_or_default()
}

/// 用瀏覽器開後台
#[tauri::command]
pub fn license_admin_open() -> Result<(), String> {
    let url = crate::stats::STATS_URL.trim_end_matches('/');
    if url.is_empty() {
        return Err("還沒設定收集端網址".into());
    }
    crate::winsys::open(&format!("{url}/admin"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sign_and_verify() {
        let sk = SigningKey::from_bytes(&[7u8; 32]);
        let serial = "MB-2345-6789-ABCD-EFGH";
        let code = encode_code(&sk.sign(serial.as_bytes()));
        assert!(code.starts_with("MB1-"));
        let sig = decode_code(&code).unwrap();
        assert!(sk.verifying_key().verify(serial.as_bytes(), &sig).is_ok());
        assert!(sk.verifying_key().verify(b"MB-OTHER", &sig).is_err());
        assert!(decode_code("MB1-xx").is_none());
    }

    /// Worker（WebCrypto Ed25519）簽出來的授權碼，程式這邊要驗得過
    #[test]
    fn worker_signature_format() {
        let pk = key_of("009cae4485d2c01441154fdbe89761f60a287c7e24c1b87860277f70c07940c9").unwrap();
        let sig = decode_code("MB1-h3Oza4DkRK23Nd4hEimnCNEbOMLvPCsXYyaPg44zLKfaf84lRSiT3BNappu3SsSDriR949VcWpa55vlYHquMDQ").unwrap();
        assert!(pk.verify(b"MB-AAAA-BBBB-CCCC-DDDD", &sig).is_ok());
        assert!(pk.verify(b"MB-AAAA-BBBB-CCCC-EEEE", &sig).is_err());
    }

    #[test]
    fn base36_ok() {
        assert_eq!(base36(0), "0");
        assert_eq!(base36(35), "z");
        assert_eq!(base36(36), "10");
        assert_eq!(u64::from_str_radix(&base36(29_876_543), 36).unwrap(), 29_876_543);
    }

    #[test]
    fn serial_shape() {
        let s = serial();
        assert_eq!(s.len(), 22);
        assert!(s.starts_with("MB-"));
        assert!(!s.contains('0') && !s.contains('O') && !s.contains('1') && !s.contains('I'));
        assert_eq!(s, serial());
    }
}
