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

// 編譯時由 build.rs 產生：NK_SEED、NK_LEN、NK_A、NK_B（GitHub 祕密 NEXON_API_KEY 有設定時才有內容）
include!(concat!(env!("OUT_DIR"), "/nexon_key.rs"));

/// GitHub 祕密裡設定的金鑰（沒設定時是空字串）
#[inline(never)]
fn secret_key() -> String {
    let mut enc = vec![0u8; NK_LEN];
    for (j, b) in NK_A.iter().rev().enumerate() {
        enc[j * 2] = *b;
    }
    for (j, b) in NK_B.iter().rev().enumerate() {
        enc[j * 2 + 1] = *b;
    }
    let mut x = NK_SEED;
    let mut out = Vec::with_capacity(NK_LEN);
    for (i, b) in enc.iter().enumerate() {
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        let k = (x >> ((i % 7) * 8)) as u8;
        out.push(b ^ k ^ (i as u8).wrapping_mul(0x3D));
    }
    String::from_utf8(out).unwrap_or_default()
}

/// 使用的金鑰：GitHub 祕密有設定就用那個（換金鑰只要改祕密），否則用程式碼裡原本的。
/// 執行時才解出金鑰，用完即丟，不保存在任何全域變數
fn api_key() -> String {
    let s = secret_key();
    if !s.trim().is_empty() {
        return s;
    }
    builtin_key()
}

fn builtin_key() -> String {
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
    /// 經驗追蹤：OCID → (查詢時間, 最新資料)
    track_now: HashMap<String, (u64, Snap)>,
    /// 經驗追蹤：OCID → 日期 → 那天的資料（None = 那天查不到，例如角色還沒建立）
    track_hist: HashMap<String, HashMap<String, Option<Snap>>>,
    /// 經驗追蹤：OCID → (日期, 當天第一次查到的資料)；昨天的資料還沒出來時當作今天的起點
    track_first: HashMap<String, (String, Snap)>,
}

/// 經驗追蹤用的角色資料
#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Snap {
    level: u32,
    exp: i64,
    rate: f64,
    class: String,
    image: String,
    world: String,
}

fn snap_of(v: &Value) -> Option<Snap> {
    let level = v["character_level"].as_u64().or_else(|| v["character_level"].as_str()?.parse().ok())? as u32;
    if level == 0 {
        return None;
    }
    let exp = v["character_exp"].as_i64().or_else(|| v["character_exp"].as_str()?.parse().ok()).unwrap_or(0);
    let rate = v["character_exp_rate"]
        .as_str()
        .and_then(|s| s.trim().trim_end_matches('%').parse().ok())
        .or_else(|| v["character_exp_rate"].as_f64())
        .unwrap_or(0.0);
    let text = |k: &str| v[k].as_str().unwrap_or("").to_string();
    Some(Snap { level, exp, rate, class: text("character_class"), image: text("character_image"), world: text("world_name") })
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(default)]
struct CachedChar {
    fetched_at: u64,
    basic: Value,
    beauty: Value,
    cash: Value,
    /// 一般裝備（只有沒有點裝披風時才查，用來顯示披風）
    gear: Value,
}

impl Default for CachedChar {
    fn default() -> Self {
        Self { fetched_at: 0, basic: Value::Null, beauty: Value::Null, cash: Value::Null, gear: Value::Null }
    }
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
    date_tw(0)
}

/// 台灣時間幾天前的日期字串（YYYY-MM-DD）
fn date_tw(days_ago: u64) -> String {
    let days = (now() + 8 * 3600) / 86400 - days_ago;
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
    /// OPENAPI00009：Nexon 那邊的資料還在準備（例如凌晨還查不到昨天的資料）
    Preparing,
    Other(String),
}

fn get(path: &str, query: &[(&str, &str)]) -> Result<Value, ApiErr> {
    // 每秒最多 5 次：兩次呼叫之間至少隔 0.22 秒
    {
        static LAST: Mutex<Option<std::time::Instant>> = Mutex::new(None);
        let mut last = LAST.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(t) = *last {
            let gap = Duration::from_millis(220);
            let since = t.elapsed();
            if since < gap {
                std::thread::sleep(gap - since);
            }
        }
        *last = Some(std::time::Instant::now());
    }
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
            if code == 400 && name == "OPENAPI00004" {
                return Err(ApiErr::NotFound);
            }
            if name == "OPENAPI00009" {
                return Err(ApiErr::Preparing);
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
        Err(ApiErr::Preparing) => Err("Nexon 的資料還在準備中，請稍後再試".into()),
        Err(ApiErr::Other(m)) => Err(m),
    }
}

/// 點裝裡有沒有披風（有的話就不用再查一般裝備）
fn has_cash_cape(cash: &Value) -> bool {
    let preset = cash["preset_no"].as_i64().unwrap_or(0);
    let lists = [format!("cash_item_equipment_preset_{preset}"), "cash_item_equipment_base".to_string()];
    lists.iter().any(|k| {
        cash[k.as_str()]
            .as_array()
            .map(|a| a.iter().any(|x| is_cape(x["cash_item_equipment_slot"].as_str().unwrap_or(""))))
            .unwrap_or(false)
    })
}

/// 欄位名稱是不是披風（台版「披風」、韓版「망토」、英文 Cape）
pub fn is_cape(slot: &str) -> bool {
    slot.contains("披風") || slot.contains("망토") || slot.to_ascii_lowercase().contains("cape")
}

fn fetch_char(ocid: &str) -> Result<CachedChar, ApiErr> {
    let q = [("ocid", ocid)];
    let basic = get("/character/basic", &q)?;
    let beauty = get("/character/beauty-equipment", &q)?;
    let cash = get("/character/cashitem-equipment", &q)?;
    // 沒有點裝披風時才多查一般裝備，省下一次呼叫
    let gear = if has_cash_cape(&cash) {
        Value::Null
    } else {
        get("/character/item-equipment", &q).unwrap_or(Value::Null)
    };
    Ok(CachedChar { fetched_at: now(), basic, beauty, cash, gear })
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
    gear: Value,
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
            // 舊版存的快取沒有一般裝備（披風畫不出來）：當作過期，重新查一次
            let has_gear = !c.gear.is_null() || has_cash_cape(&c.cash);
            if has_gear && now().saturating_sub(c.fetched_at) < DATA_TTL_SECS {
                return Ok(Lookup {
                    name, ocid: ocid.clone(), fetched_at: c.fetched_at, from_cache: true,
                    basic: c.basic.clone(), beauty: c.beauty.clone(), cash: c.cash.clone(), gear: c.gear.clone(),
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
                ApiErr::Preparing => "Nexon 的資料還在準備中，請稍後再試".into(),
                ApiErr::Other(m) => m,
            })?
        }
        Err(ApiErr::Preparing) => return Err("Nexon 的資料還在準備中，請稍後再試".into()),
        Err(ApiErr::Other(m)) => return Err(m),
    };

    let mut s = store.lock().unwrap();
    s.cache.ocid.insert(name.clone(), ocid.clone());
    s.cache.data.insert(ocid.clone(), data.clone());
    save(&s);
    Ok(Lookup {
        name, ocid, fetched_at: data.fetched_at, from_cache: false,
        basic: data.basic, beauty: data.beauty, cash: data.cash, gear: data.gear,
        calls_today: s.cache.calls,
    })
}

#[tauri::command]
pub async fn maple_lookup(name: String, force: bool) -> Result<Lookup, String> {
    tauri::async_runtime::spawn_blocking(move || lookup_blocking(name, force))
        .await
        .map_err(|e| e.to_string())?
}

/* ---------------- 經驗追蹤 ---------------- */

const TRACK_TTL_SECS: u64 = 5 * 60;

#[derive(Serialize)]
pub struct TrackOut {
    name: String,
    /// 最新資料的查詢時間
    fetched_at: u64,
    now: Option<Snap>,
    /// 昨天的資料（今天練了多少的基準）
    yesterday: Option<Snap>,
    /// 今天第一次查到的資料（昨天的資料還沒出來時當作基準）
    first_today: Option<Snap>,
    /// 幾天前的資料（算 30 天平均成長速度）
    past: Option<Snap>,
    past_days: u32,
    /// 7 天前的資料（算 7 天平均成長速度）
    past7: Option<Snap>,
    past7_days: u32,
    error: Option<String>,
}

/// 最近幾天查不到時，30 分鐘內不再重查（只記在記憶體）
static TRACK_MISS: Mutex<Option<HashMap<String, u64>>> = Mutex::new(None);

/// 某天的資料：查過就記住（歷史資料不會變）。
/// 昨天、前天的資料凌晨可能還沒準備好，查不到時不記住，過一段時間再查
fn hist_snap(ocid: &str, days_ago: u64) -> Option<Snap> {
    let store = STORE.get()?;
    let date = date_tw(days_ago);
    if let Some(v) = store.lock().unwrap().cache.track_hist.get(ocid).and_then(|m| m.get(&date)) {
        // 舊版會把「最近幾天查不到」也記住，這種要重查
        if v.is_some() || days_ago > 2 {
            return v.clone();
        }
    }
    let miss_key = format!("{ocid}|{date}");
    {
        let miss = TRACK_MISS.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(t) = miss.as_ref().and_then(|m| m.get(&miss_key)) {
            if now().saturating_sub(*t) < 30 * 60 {
                return None;
            }
        }
    }
    let remember_miss = || {
        TRACK_MISS.lock().unwrap_or_else(|e| e.into_inner()).get_or_insert_with(HashMap::new).insert(miss_key.clone(), now());
    };
    let v = match get("/character/basic", &[("ocid", ocid), ("date", &date)]) {
        Ok(v) => snap_of(&v),
        Err(ApiErr::NotFound) if days_ago > 2 => None,
        // 最近的資料還沒準備好、連線失敗、次數用完：不記住，晚點再試
        Err(_) => {
            remember_miss();
            return None;
        }
    };
    if v.is_none() && days_ago <= 2 {
        remember_miss();
        return None;
    }
    store.lock().unwrap().cache.track_hist.entry(ocid.to_string()).or_default().insert(date, v.clone());
    v
}

fn track_one(name: &str, force: bool) -> Result<TrackOut, String> {
    let name = name.trim();
    let store = STORE.get().ok_or("尚未初始化")?;
    let cached_ocid = store.lock().unwrap().cache.ocid.get(name).cloned();
    let mut ocid = match cached_ocid {
        Some(o) => o,
        None => fetch_ocid(name)?,
    };
    store.lock().unwrap().cache.ocid.insert(name.to_string(), ocid.clone());

    // 最新資料：5 分鐘內查過就不再查（按重新整理時 1 分鐘）
    let cached = store.lock().unwrap().cache.track_now.get(&ocid).cloned();
    let ttl = if force { 60 } else { TRACK_TTL_SECS };
    let (fetched_at, cur) = match cached {
        Some((t, c)) if now().saturating_sub(t) < ttl => (t, c),
        _ => {
            let r = match get("/character/basic", &[("ocid", &ocid)]) {
                Err(ApiErr::NotFound) => {
                    // OCID 失效：重查一次
                    ocid = fetch_ocid(name)?;
                    store.lock().unwrap().cache.ocid.insert(name.to_string(), ocid.clone());
                    get("/character/basic", &[("ocid", &ocid)])
                }
                r => r,
            };
            match r.ok().and_then(|v| snap_of(&v)) {
                Some(c) => {
                    store.lock().unwrap().cache.track_now.insert(ocid.clone(), (now(), c.clone()));
                    (now(), c)
                }
                None => match cached {
                    Some((t, c)) => (t, c),
                    None => return Err(format!("讀不到「{name}」的資料")),
                },
            }
        }
    };

    // 今天第一次查到的資料
    let today = today_tw();
    let first_today = {
        let mut s = store.lock().unwrap();
        let e = s.cache.track_first.entry(ocid.clone()).or_insert_with(|| (today.clone(), cur.clone()));
        if e.0 != today {
            *e = (today.clone(), cur.clone());
        }
        Some(e.1.clone())
    };
    let yesterday = hist_snap(&ocid, 1);
    // 平均成長：優先用 30 天（或 7 天）前；角色太新查不到就用比較近的日子，再不行用昨天
    let pick = |days: &[u64]| -> (Option<Snap>, u32) {
        for &d in days {
            if let Some(s) = hist_snap(&ocid, d) {
                return (Some(s), d as u32);
            }
        }
        match &yesterday {
            Some(y) => (Some(y.clone()), 1),
            None => (None, 0),
        }
    };
    let (past, past_days) = pick(&[30, 14, 7, 3]);
    let (past7, past7_days) = pick(&[7, 3]);
    Ok(TrackOut { name: name.to_string(), fetched_at, now: Some(cur), yesterday, first_today, past, past_days, past7, past7_days, error: None })
}

/// 經驗追蹤：每隻角色的最新等級與經驗、昨天、7 天前與 30 天前的資料
#[tauri::command]
pub async fn exp_track(names: Vec<String>, force: bool) -> Vec<TrackOut> {
    tauri::async_runtime::spawn_blocking(move || {
        let out: Vec<TrackOut> = names
            .iter()
            .filter(|n| !n.trim().is_empty())
            .map(|n| {
                track_one(n, force).unwrap_or_else(|e| TrackOut {
                    name: n.trim().to_string(),
                    fetched_at: 0,
                    now: None,
                    yesterday: None,
                    first_today: None,
                    past: None,
                    past_days: 0,
                    past7: None,
                    past7_days: 0,
                    error: Some(e),
                })
            })
            .collect();
        if let Some(store) = STORE.get() {
            let mut s = store.lock().unwrap();
            // 只留最近 40 天的歷史資料
            let oldest = date_tw(40);
            for m in s.cache.track_hist.values_mut() {
                m.retain(|d, _| *d >= oldest);
            }
            save(&s);
        }
        out
    })
    .await
    .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snapshot() {
        let v: Value = serde_json::json!({"character_level": 271, "character_exp": 123456789, "character_exp_rate": "12.345", "character_class": "主教"});
        let s = snap_of(&v).unwrap();
        assert_eq!((s.level, s.exp, s.rate), (271, 123456789, 12.345));
        assert!(snap_of(&serde_json::json!({"character_level": null})).is_none());
    }

    #[test]
    fn dates() {
        assert_eq!(date_tw(0), today_tw());
        assert!(date_tw(30) < date_tw(1));
        assert_eq!(date_tw(1).len(), 10);
    }

    #[test]
    fn key_decodes() {
        let k = builtin_key();
        assert!(k.starts_with("test_"));
        assert_eq!(k.len(), 101);
        // GitHub 祕密的金鑰：沒設定時是空的（用原本的），有設定時長度一致
        assert_eq!(secret_key().len(), NK_LEN);
        assert!(!api_key().is_empty());
    }
}
