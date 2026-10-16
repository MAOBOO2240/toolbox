//! 新楓之谷 Open API（Rust 端呼叫，金鑰不會傳到介面或寫在檔案裡）。
//!
//! 節省每日呼叫次數：
//! - 角色名稱 → OCID 永久快取（OCID 失效時才重新查）
//! - 角色資料（基本、髮型臉型膚色、點裝）快取 1 小時，可手動強制更新
//! - 只呼叫需要的 3 個端點

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const BASE: &str = "https://open.api.nexon.com/maplestorytw/v1";
const DATA_TTL_SECS: u64 = 60 * 60;

/* ---------------- 金鑰（混淆儲存） ---------------- */

const SHARD_A: [u8; 51] = [0x98, 0x39, 0xBB, 0xBF, 0x26, 0x2D, 0x9F, 0xFD, 0xDA, 0x69, 0xF2, 0xC0, 0xDE, 0x27, 0xC6, 0x78, 0x79, 0xBF, 0xF3, 0x60, 0x4A, 0x86, 0xB0, 0x18, 0x78, 0xBD, 0x64, 0xE8, 0xAE, 0x50, 0xC9, 0xCB, 0x77, 0xE6, 0xFD, 0xDB, 0x2C, 0x70, 0x42, 0x77, 0xF0, 0x82, 0x67, 0x7E, 0x05, 0xBA, 0x82, 0x19, 0x5D, 0x55, 0xDC];
const SHARD_B: [u8; 50] = [0xD6, 0xEB, 0x3A, 0xC9, 0x32, 0x99, 0x10, 0x15, 0x43, 0x97, 0x3F, 0x73, 0xD4, 0x03, 0x51, 0x99, 0x4C, 0xFE, 0xFB, 0xA6, 0x2B, 0xD7, 0x8D, 0xB8, 0x07, 0xF0, 0x1B, 0x43, 0x7E, 0x29, 0xFF, 0xCF, 0xA2, 0xF3, 0x5C, 0xF7, 0xC8, 0xC5, 0x64, 0xB0, 0xD0, 0xDC, 0x5A, 0x51, 0x04, 0x32, 0x74, 0x07, 0xD4, 0x6E];

fn ks_next(x: &mut u64) -> u8 {
    *x ^= *x >> 12;
    *x ^= *x << 25;
    *x ^= *x >> 27;
    (x.wrapping_mul(0x2545_F491_4F6C_DD1D) >> 56) as u8
}

/// 執行時才解出金鑰，用完即丟，不保存在任何全域變數
fn api_key() -> String {
    let n = SHARD_A.len() + SHARD_B.len();
    let mut state: u64 = 0x5A17_C3E9_2B6D_8F41;
    let mut out = Vec::with_capacity(n);
    for i in 0..n {
        let e = if i % 2 == 0 { SHARD_A[i / 2] } else { SHARD_B[i / 2] };
        let k = ks_next(&mut state);
        out.push(e ^ k ^ ((i as u32 * 31 + 7) & 0xFF) as u8);
    }
    String::from_utf8(out).unwrap_or_default()
}

/* ---------------- 快取 ---------------- */

#[derive(Default, Serialize, Deserialize)]
#[serde(default)]
struct Cache {
    /// 角色名稱 → OCID
    ocid: HashMap<String, String>,
    /// OCID → 角色資料
    data: HashMap<String, CachedChar>,
    /// 本機今天已使用的呼叫次數（日期 YYYY-MM-DD）
    calls_day: String,
    calls: u32,
}

#[derive(Clone, Serialize, Deserialize)]
struct CachedChar {
    fetched_at: u64,
    basic: Value,
    beauty: Value,
    cash: Value,
}

struct Store {
    cache: Cache,
    path: PathBuf,
}

static STORE: OnceLock<Mutex<Store>> = OnceLock::new();

pub fn init(path: PathBuf) {
    let cache = std::fs::read_to_string(&path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default();
    let _ = STORE.set(Mutex::new(Store { cache, path }));
}

fn now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

/// 台灣時間的日期字串（用來每天歸零呼叫次數）
fn today_tw() -> String {
    let days = (now() + 8 * 3600) / 86400;
    // 1970-01-01 起算的天數 → 年月日
    let (mut y, mut d) = (1970i64, days as i64);
    loop {
        let len = if (y % 4 == 0 && y % 100 != 0) || y % 400 == 0 { 366 } else { 365 };
        if d < len { break; }
        d -= len;
        y += 1;
    }
    let leap = (y % 4 == 0 && y % 100 != 0) || y % 400 == 0;
    let months = [31, if leap { 29 } else { 28 }, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];
    let mut m = 0;
    while d >= months[m] {
        d -= months[m];
        m += 1;
    }
    format!("{y:04}-{:02}-{:02}", m + 1, d + 1)
}

fn save(store: &Store) {
    if let Some(dir) = store.path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    if let Ok(json) = serde_json::to_string(&store.cache) {
        let tmp = store.path.with_extension("json.tmp");
        if std::fs::write(&tmp, json).is_ok() {
            let _ = std::fs::rename(&tmp, &store.path);
        }
    }
}

/* ---------------- 呼叫 ---------------- */

#[derive(Debug)]
enum ApiErr {
    NotFound,
    Other(String),
}

fn get(path: &str, query: &[(&str, &str)]) -> Result<Value, ApiErr> {
    {
        let mut s = STORE.get().unwrap().lock().unwrap();
        let today = today_tw();
        if s.cache.calls_day != today {
            s.cache.calls_day = today;
            s.cache.calls = 0;
        }
        s.cache.calls += 1;
    }
    let mut req = ureq::get(&format!("{BASE}{path}"))
        .timeout(Duration::from_secs(15))
        .set("x-nxopen-api-key", &api_key());
    for (k, v) in query {
        req = req.query(k, v);
    }
    match req.call() {
        Ok(resp) => resp.into_json::<Value>().map_err(|e| ApiErr::Other(e.to_string())),
        Err(ureq::Error::Status(code, resp)) => {
            let body: Value = resp.into_json().unwrap_or(Value::Null);
            let name = body["error"]["name"].as_str().unwrap_or("").to_string();
            // OPENAPI00004：參數錯誤（多半是角色名稱不存在）
            if code == 400 && (name == "OPENAPI00004" || name == "OPENAPI00009") {
                return Err(ApiErr::NotFound);
            }
            let msg = match name.as_str() {
                "OPENAPI00007" => "今天的 API 次數已用完，明天再試".to_string(),
                "OPENAPI00005" | "OPENAPI00006" => "API 金鑰無效或已過期".to_string(),
                "OPENAPI00010" | "OPENAPI00011" => "Nexon API 維護中，請稍後再試".to_string(),
                _ => format!("API 錯誤 {code} {name}"),
            };
            Err(ApiErr::Other(msg))
        }
        Err(e) => Err(ApiErr::Other(format!("無法連線到 Nexon API：{e}"))),
    }
}

fn fetch_ocid(name: &str) -> Result<String, String> {
    match get("/id", &[("character_name", name)]) {
        Ok(v) => v["ocid"].as_str().map(str::to_string).ok_or_else(|| "找不到 OCID".into()),
        Err(ApiErr::NotFound) => Err(format!("找不到角色「{name}」")),
        Err(ApiErr::Other(m)) => Err(m),
    }
}

fn fetch_char(ocid: &str) -> Result<CachedChar, ApiErr> {
    let q = [("ocid", ocid)];
    let basic = get("/character/basic", &q)?;
    let beauty = get("/character/beauty-equipment", &q)?;
    let cash = get("/character/cashitem-equipment", &q)?;
    Ok(CachedChar { fetched_at: now(), basic, beauty, cash })
}

#[derive(Serialize)]
pub struct Lookup {
    name: String,
    ocid: String,
    fetched_at: u64,
    from_cache: bool,
    basic: Value,
    beauty: Value,
    cash: Value,
    calls_today: u32,
}

fn lookup_blocking(name: String, force: bool) -> Result<Lookup, String> {
    let name = name.trim().to_string();
    if name.is_empty() {
        return Err("請輸入角色名稱".into());
    }
    let store = STORE.get().ok_or("尚未初始化")?;

    // 1. 快取命中
    let cached_ocid = store.lock().unwrap().cache.ocid.get(&name).cloned();
    if let (Some(ocid), false) = (&cached_ocid, force) {
        let s = store.lock().unwrap();
        if let Some(c) = s.cache.data.get(ocid) {
            if now().saturating_sub(c.fetched_at) < DATA_TTL_SECS {
                return Ok(Lookup {
                    name, ocid: ocid.clone(), fetched_at: c.fetched_at, from_cache: true,
                    basic: c.basic.clone(), beauty: c.beauty.clone(), cash: c.cash.clone(),
                    calls_today: if s.cache.calls_day == today_tw() { s.cache.calls } else { 0 },
                });
            }
        }
    }

    // 2. 取得 OCID（有快取就不查）
    let mut ocid = match cached_ocid {
        Some(o) => o,
        None => fetch_ocid(&name)?,
    };
    // 3. 取得角色資料；OCID 失效時重查一次
    let data = match fetch_char(&ocid) {
        Ok(d) => d,
        Err(ApiErr::NotFound) => {
            ocid = fetch_ocid(&name)?;
            fetch_char(&ocid).map_err(|e| match e {
                ApiErr::NotFound => format!("找不到角色「{name}」的資料"),
                ApiErr::Other(m) => m,
            })?
        }
        Err(ApiErr::Other(m)) => return Err(m),
    };

    let mut s = store.lock().unwrap();
    s.cache.ocid.insert(name.clone(), ocid.clone());
    s.cache.data.insert(ocid.clone(), data.clone());
    save(&s);
    Ok(Lookup {
        name, ocid, fetched_at: data.fetched_at, from_cache: false,
        basic: data.basic, beauty: data.beauty, cash: data.cash,
        calls_today: s.cache.calls,
    })
}

#[tauri::command]
pub async fn maple_lookup(name: String, force: bool) -> Result<Lookup, String> {
    tauri::async_runtime::spawn_blocking(move || lookup_blocking(name, force))
        .await
        .map_err(|e| e.to_string())?
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn key_decodes() {
        let k = api_key();
        assert!(k.starts_with("test_"));
        assert_eq!(k.len(), 101);
    }
}
