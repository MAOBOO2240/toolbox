//! 使用者腳本（篡改猴／Tampermonkey 的 .user.js）：瀏覽器視窗自己執行，不用靠篡改猴。
//! （WebView2 沒有「允許使用者腳本」這個開關，篡改猴在這個瀏覽器裡裝得起來但沒辦法執行腳本）
//!
//! - 腳本放在 web-scripts/ 資料夾（匯入 .user.js、篡改猴匯出的 zip，或開啟 .user.js 的網址安裝）
//! - @match、@include、@exclude 決定在哪些網頁執行；@run-at 決定什麼時候跑；內嵌框架也會跑（除非 @noframes）
//! - @require、@resource 在安裝時先下載好，跟著腳本一起注入
//! - 設定值（GM_getValue／GM_setValue）每支腳本一份、整個瀏覽器共用，存在 web-scripts/values/
//! - GM_xmlhttpRequest 由程式代為連線（不受網頁的跨網站限制），@connect 有寫的照著限制
//!
//! 這個檔案只有邏輯（不碰視窗）：產生要注入網頁的程式、解讀腳本傳來的訊息、設定值、連線、安裝與更新。
use base64::Engine as _;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex, OnceLock};
use std::time::{Duration, Instant};
use url::Url;

static DIR: OnceLock<PathBuf> = OnceLock::new();
static CACHE: Mutex<Option<Arc<Vec<Script>>>> = Mutex::new(None);
/// 腳本清單變過幾次（安裝、刪除、開關、更新都算）
static GEN: AtomicU64 = AtomicU64::new(1);
/// 腳本清單變了要通知誰（瀏覽器視窗：已經開著的分頁要換成新的清單）
static ON_CHANGE: OnceLock<Box<dyn Fn() + Send + Sync>> = OnceLock::new();

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

fn now_ms() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

fn rand_hex() -> String {
    format!("{:016x}{:016x}", rand::random::<u64>(), rand::random::<u64>())
}

/// 網頁裡的腳本傳訊息給程式時要附的權杖（每次執行都不一樣；網頁自己的程式拿不到）
pub fn token() -> &'static str {
    static V: OnceLock<String> = OnceLock::new();
    V.get_or_init(rand_hex)
}

/// 程式傳給網頁的訊息上的記號（跟上面那個不同：就算被看到也不能拿來冒充腳本）
pub fn down() -> &'static str {
    static V: OnceLock<String> = OnceLock::new();
    V.get_or_init(rand_hex)
}

/// 注入的幾段程式之間交接用的名稱（每次執行都不一樣）
pub fn slot() -> &'static str {
    static V: OnceLock<String> = OnceLock::new();
    V.get_or_init(|| format!("_{:016x}", rand::random::<u64>()))
}

pub fn init(dir: PathBuf) {
    let _ = DIR.set(dir.join("web-scripts"));
}

/// 腳本清單變了要呼叫的函式（只能設定一次）
pub fn on_change(f: impl Fn() + Send + Sync + 'static) {
    let _ = ON_CHANGE.set(Box::new(f));
}

fn dir() -> PathBuf {
    DIR.get().cloned().unwrap_or_else(|| PathBuf::from("web-scripts"))
}

fn state_path() -> PathBuf {
    dir().join("scripts.json")
}

#[derive(Default, Serialize, Deserialize)]
#[serde(default)]
struct State {
    /// 停用的腳本（檔名）
    off: Vec<String>,
    /// 從哪個網址安裝的（檔名 → 網址；腳本自己沒寫更新網址時用這個檢查更新）
    src: BTreeMap<String, String>,
    /// 不要自動更新
    no_update: bool,
    /// 上次檢查更新的時間（毫秒）
    checked: u64,
}

fn state() -> State {
    std::fs::read_to_string(state_path()).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default()
}

fn save_state(st: &State) {
    let _ = std::fs::create_dir_all(dir());
    if let Ok(t) = serde_json::to_string_pretty(st) {
        let _ = std::fs::write(state_path(), t);
    }
}

/* ---------------- 腳本的資訊（==UserScript== 區塊） ---------------- */

#[derive(Debug, Clone, Default, Serialize)]
pub struct Meta {
    pub name: String,
    pub version: String,
    pub description: String,
    pub namespace: String,
    pub author: String,
    pub homepage: String,
    pub icon: String,
    pub matches: Vec<String>,
    pub includes: Vec<String>,
    pub excludes: Vec<String>,
    pub exclude_matches: Vec<String>,
    /// "document-start"、"document-body"、"document-end"、"document-idle"
    pub run_at: String,
    pub requires: Vec<String>,
    /// (名稱, 網址)
    pub resources: Vec<(String, String)>,
    pub grants: Vec<String>,
    /// GM_xmlhttpRequest 可以連的網域（@connect）
    pub connects: Vec<String>,
    pub noframes: bool,
    pub update_url: String,
    pub download_url: String,
}

/// 這個語系的值要不要用（@name:zh-TW 這種）：只認繁體中文
fn locale_ok(l: &str) -> bool {
    matches!(l.to_ascii_lowercase().as_str(), "zh-tw" | "zh-hant" | "zh_tw" | "zh")
}

/// 解析 ==UserScript== 區塊
pub fn parse_meta(src: &str) -> Meta {
    let mut m = Meta::default();
    let Some(start) = src.find("==UserScript==") else { return m };
    let end = src[start..].find("==/UserScript==").map(|e| start + e).unwrap_or(src.len());
    let (mut name_l, mut desc_l) = (String::new(), String::new());
    for line in src[start..end].lines() {
        let line = line.trim().trim_start_matches("//").trim();
        let Some(rest) = line.strip_prefix('@') else { continue };
        let (key, val) = match rest.find(char::is_whitespace) {
            Some(i) => (&rest[..i], rest[i..].trim()),
            None => (rest, ""),
        };
        match key {
            "name" => m.name = val.to_string(),
            "version" => m.version = val.to_string(),
            "description" => m.description = val.to_string(),
            "namespace" => m.namespace = val.to_string(),
            "author" => m.author = val.to_string(),
            "homepage" | "homepageURL" | "website" | "source" => {
                if m.homepage.is_empty() {
                    m.homepage = val.to_string()
                }
            }
            "icon" | "iconURL" | "defaulticon" | "icon64" | "icon64URL" => {
                if m.icon.is_empty() {
                    m.icon = val.to_string()
                }
            }
            "match" => m.matches.push(val.to_string()),
            "include" => m.includes.push(val.to_string()),
            "exclude" => m.excludes.push(val.to_string()),
            "exclude-match" => m.exclude_matches.push(val.to_string()),
            "run-at" => m.run_at = val.to_string(),
            "require" => m.requires.push(val.to_string()),
            "resource" => {
                if let Some(i) = val.find(char::is_whitespace) {
                    m.resources.push((val[..i].to_string(), val[i..].trim().to_string()));
                }
            }
            "grant" => m.grants.push(val.to_string()),
            "connect" => m.connects.push(val.to_string()),
            "noframes" => m.noframes = true,
            "updateURL" => m.update_url = val.to_string(),
            "downloadURL" => m.download_url = val.to_string(),
            k => {
                if let Some(l) = k.strip_prefix("name:") {
                    if locale_ok(l) {
                        name_l = val.to_string();
                    }
                } else if let Some(l) = k.strip_prefix("description:") {
                    if locale_ok(l) {
                        desc_l = val.to_string();
                    }
                }
            }
        }
    }
    if !name_l.is_empty() {
        m.name = name_l;
    }
    if !desc_l.is_empty() {
        m.description = desc_l;
    }
    if m.name.is_empty() {
        m.name = "未命名腳本".into();
    }
    m
}

/// Chrome 的 match pattern（*://*.gamer.com.tw/*）→ 正規表示式
pub fn match_to_regex(p: &str) -> Option<String> {
    let p = p.trim();
    if p == "<all_urls>" || p == "*" {
        return Some("^(https?|file|ftp)://.*$".into());
    }
    let (scheme, rest) = p.split_once("://")?;
    let (host, path) = match rest.find('/') {
        Some(i) => (&rest[..i], &rest[i..]),
        None => (rest, "/"),
    };
    // 很多腳本會寫 http*://（Chrome 不認，篡改猴認）
    let scheme_re = if scheme == "*" { "https?".to_string() } else { regex::escape(scheme).replace("\\*", "[a-z]*") };
    let host_re = if host == "*" {
        "[^/]*".to_string()
    } else if let Some(h) = host.strip_prefix("*.") {
        format!("([^/]*\\.)?{}", regex::escape(h).replace("\\*", "[^/]*"))
    } else {
        regex::escape(host).replace("\\*", "[^/]*")
    };
    let path_re = regex::escape(path).replace("\\*", ".*");
    Some(format!("^{scheme_re}://{host_re}{path_re}$"))
}

/// @include／@exclude：/regex/ 或 glob（* 代表任意字元）
pub fn include_to_regex(p: &str) -> Option<String> {
    let p = p.trim();
    if p.len() >= 2 && p.starts_with('/') && p.ends_with('/') {
        return Some(p[1..p.len() - 1].to_string());
    }
    Some(format!("^{}$", regex::escape(p).replace("\\*", ".*")))
}

/// 要執行的網址（正規表示式）
fn inc_res(m: &Meta) -> Vec<String> {
    m.matches.iter().filter_map(|p| match_to_regex(p)).chain(m.includes.iter().filter_map(|p| include_to_regex(p))).collect()
}

/// 不要執行的網址
fn exc_res(m: &Meta) -> Vec<String> {
    m.excludes.iter().filter_map(|p| include_to_regex(p)).chain(m.exclude_matches.iter().filter_map(|p| match_to_regex(p))).collect()
}

/// 比對網址時不看 # 後面
fn strip_hash(url: &str) -> &str {
    url.split('#').next().unwrap_or(url)
}

/// 腳本要不要在這個網址執行。網頁那邊用 JavaScript 的正規表示式比對；這裡是程式這邊再確認一次用的，
/// 所以遇到這邊看不懂的寫法（JavaScript 才有的語法）：要執行的當成符合、不要執行的當成不符合
pub fn applies(m: &Meta, url: &str) -> bool {
    let url = strip_hash(url);
    let hit = |list: &[String], unknown: bool| list.iter().any(|r| regex::Regex::new(r).map(|re| re.is_match(url)).unwrap_or(unknown));
    let inc = inc_res(m);
    !inc.is_empty() && hit(&inc, true) && !hit(&exc_res(m), false)
}

/// 腳本附帶的資源（@resource）
#[derive(Debug, Clone)]
pub struct Res {
    pub b64: String,
    pub mime: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct Script {
    /// 檔名（識別用）
    pub file: String,
    /// 網頁裡用的編號（從檔名算出來，不會變）
    pub id: String,
    pub meta: Meta,
    pub off: bool,
    /// 安裝時沒下載到的 @require／@resource
    pub missing: Vec<String>,
    #[serde(skip)]
    pub code: String,
    #[serde(skip)]
    pub requires: Vec<String>,
    #[serde(skip)]
    pub resources: BTreeMap<String, Res>,
}

impl Script {
    /// 會不會執行（開著，而且需要的檔案都有）
    pub fn active(&self) -> bool {
        !self.off && self.missing.is_empty()
    }
}

fn id_of(file: &str) -> String {
    format!("s{}", &crate::sha256::hex(file.as_bytes())[..12])
}

fn dep_base(url: &str) -> PathBuf {
    dir().join("deps").join(crate::sha256::hex(url.as_bytes()))
}

/// 讀下載好的 @require／@resource：（內容, 類型）。舊版存成 .txt 的也認
fn read_dep(url: &str) -> Option<(Vec<u8>, String)> {
    let base = dep_base(url);
    if let Ok(b) = std::fs::read(base.with_extension("bin")) {
        let t = std::fs::read_to_string(base.with_extension("type")).unwrap_or_default();
        return Some((b, t.trim().to_string()));
    }
    std::fs::read(base.with_extension("txt")).ok().map(|b| (b, "text/plain; charset=utf-8".to_string()))
}

const UA: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/130.0.0.0 Safari/537.36";

/// 下載一個網址：（內容, 類型）。max = 最多幾個位元組
fn http_get(url: &str, secs: u64, max: usize) -> Result<(Vec<u8>, String), String> {
    let u = Url::parse(url).map_err(|_| "網址不對".to_string())?;
    if !matches!(u.scheme(), "http" | "https") {
        return Err("只能下載 http、https 的網址".into());
    }
    let resp = ureq::AgentBuilder::new()
        .timeout(Duration::from_secs(secs))
        .user_agent(UA)
        .build()
        .get(u.as_str())
        .set("Cache-Control", "no-cache")
        .call()
        .map_err(|e| match e {
            ureq::Error::Status(c, _) => format!("HTTP {c}"),
            e => e.to_string(),
        })?;
    let ctype = resp.header("content-type").unwrap_or("").to_string();
    let mut body = Vec::new();
    resp.into_reader().take(max as u64 + 1).read_to_end(&mut body).map_err(|e| e.to_string())?;
    if body.len() > max {
        return Err("檔案太大".into());
    }
    Ok((body, ctype))
}

/// 下載 @require／@resource（已經有就不再下載）
fn fetch_dep(url: &str) -> Result<(), String> {
    if read_dep(url).is_some() {
        return Ok(());
    }
    let (body, ctype) = http_get(url, 60, 64 << 20)?;
    let base = dep_base(url);
    let _ = std::fs::create_dir_all(base.parent().unwrap_or(Path::new(".")));
    std::fs::write(base.with_extension("bin"), body).map_err(|e| e.to_string())?;
    let _ = std::fs::write(base.with_extension("type"), ctype);
    Ok(())
}

fn mime_by_ext(url: &str) -> &'static str {
    let path = url.split(['?', '#']).next().unwrap_or(url).to_ascii_lowercase();
    match path.rsplit('.').next().unwrap_or("") {
        "css" => "text/css",
        "js" => "text/javascript",
        "json" => "application/json",
        "html" | "htm" => "text/html",
        "svg" => "image/svg+xml",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "woff" => "font/woff",
        "woff2" => "font/woff2",
        "txt" => "text/plain",
        _ => "application/octet-stream",
    }
}

fn load_all() -> Vec<Script> {
    let st = state();
    let mut out = Vec::new();
    let Ok(rd) = std::fs::read_dir(dir()) else { return out };
    for e in rd.flatten() {
        let name = e.file_name().to_string_lossy().to_string();
        if !name.ends_with(".user.js") {
            continue;
        }
        let Ok(code) = std::fs::read_to_string(e.path()) else { continue };
        let meta = parse_meta(&code);
        let mut missing = Vec::new();
        let mut requires = Vec::new();
        for u in &meta.requires {
            match read_dep(u) {
                Some((b, _)) => requires.push(String::from_utf8_lossy(&b).into_owned()),
                None => missing.push(u.clone()),
            }
        }
        let mut resources = BTreeMap::new();
        for (n, u) in &meta.resources {
            match read_dep(u) {
                Some((b, t)) => {
                    let mime = if t.is_empty() { mime_by_ext(u).to_string() } else { t };
                    resources.insert(n.clone(), Res { b64: base64::engine::general_purpose::STANDARD.encode(b), mime });
                }
                None => missing.push(u.clone()),
            }
        }
        out.push(Script { off: st.off.contains(&name), id: id_of(&name), file: name, meta, missing, code, requires, resources });
    }
    out.sort_by(|a, b| a.meta.name.to_lowercase().cmp(&b.meta.name.to_lowercase()));
    out
}

pub fn list() -> Arc<Vec<Script>> {
    let mut c = lock(&CACHE);
    if c.is_none() {
        *c = Some(Arc::new(load_all()));
    }
    c.clone().unwrap_or_default()
}

/// 清單要重新讀（不通知）
fn drop_cache() {
    *lock(&CACHE) = None;
    GEN.fetch_add(1, Ordering::SeqCst);
}

/// 清單變了：重新讀，並通知瀏覽器視窗
fn invalidate() {
    drop_cache();
    if let Some(f) = ON_CHANGE.get() {
        f();
    }
}

/// 腳本清單是第幾版（變一次加一）
pub fn gen() -> u64 {
    GEN.load(Ordering::SeqCst)
}

/// 有沒有會執行的腳本
pub fn any_active() -> bool {
    list().iter().any(|s| s.active())
}

/// 有沒有裝腳本（工具列要不要顯示「使用者腳本」按鈕）
pub fn any() -> bool {
    !list().is_empty()
}

/// 用網頁裡的編號找腳本（要是會執行的）
pub fn by_id(id: &str) -> Option<Script> {
    list().iter().find(|s| s.id == id && s.active()).cloned()
}

/// 這支腳本會不會在這個網址執行（top = 最上層的網頁；內嵌框架要看 @noframes）
pub fn runs_on(s: &Script, url: &str, top: bool) -> bool {
    (top || !s.meta.noframes) && applies(&s.meta, url)
}

/// 下載腳本需要的外部檔案（安裝時呼叫）
fn fetch_deps(meta: &Meta) {
    for u in meta.requires.iter().chain(meta.resources.iter().map(|(_, u)| u)) {
        let _ = fetch_dep(u);
    }
}

fn safe_name(s: &str) -> String {
    let t: String = s.chars().map(|c| if c.is_alphanumeric() || c == ' ' || c == '-' || c == '_' || c == '.' || (c as u32) > 0x7f { c } else { '_' }).collect();
    let t = t.trim().trim_end_matches('.').to_string();
    if t.is_empty() { "script".into() } else { t }
}

/// 把一個 .user.js（內容）存進腳本資料夾；同名的會被換成新的。回傳檔名（之後要呼叫 invalidate）
fn write_source(src: &str) -> Result<String, String> {
    if !src.contains("==UserScript==") {
        return Err("這不是使用者腳本（找不到 ==UserScript== 區塊）".into());
    }
    let meta = parse_meta(src);
    if meta.matches.is_empty() && meta.includes.is_empty() {
        return Err(format!("「{}」沒有 @match 或 @include，不知道要在哪些網頁執行", meta.name));
    }
    let _ = std::fs::create_dir_all(dir());
    let file = format!("{}.user.js", safe_name(&meta.name));
    std::fs::write(dir().join(&file), src).map_err(|e| e.to_string())?;
    fetch_deps(&meta);
    Ok(file)
}

/// 匯入檔案：.user.js 直接加；.zip（篡改猴匯出的備份）取出裡面的 .user.js 與停用狀態
pub fn import_file(path: &Path) -> Result<(usize, Vec<String>), String> {
    let mut n = 0;
    let mut errors = Vec::new();
    let lower = path.to_string_lossy().to_lowercase();
    if lower.ends_with(".zip") {
        let f = std::fs::File::open(path).map_err(|e| e.to_string())?;
        let mut zip = zip::ZipArchive::new(f).map_err(|e| format!("壓縮檔錯誤：{e}"))?;
        let mut off_names: Vec<String> = Vec::new();
        let mut added: Vec<(String, String)> = Vec::new();
        for i in 0..zip.len() {
            let Ok(mut e) = zip.by_index(i) else { continue };
            let name = e.name().to_string();
            let mut body = String::new();
            if e.read_to_string(&mut body).is_err() {
                continue;
            }
            if name.ends_with(".user.js") {
                match write_source(&body) {
                    Ok(file) => {
                        n += 1;
                        added.push((name.trim_end_matches(".user.js").to_string(), file));
                    }
                    Err(err) => errors.push(err),
                }
            } else if name.ends_with(".options.json") {
                // 篡改猴備份裡的設定：enabled=false 的腳本保持停用
                if let Ok(v) = serde_json::from_str::<serde_json::Value>(&body) {
                    if v["settings"]["enabled"] == false || v["enabled"] == false {
                        off_names.push(name.trim_end_matches(".options.json").to_string());
                    }
                }
            }
        }
        if !off_names.is_empty() {
            let mut st = state();
            for (base, file) in &added {
                if off_names.iter().any(|o| o == base) && !st.off.contains(file) {
                    st.off.push(file.clone());
                }
            }
            save_state(&st);
        }
    } else {
        let src = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
        write_source(&src)?;
        n = 1;
    }
    invalidate();
    Ok((n, errors))
}

pub fn set_off(file: &str, off: bool) {
    let mut st = state();
    st.off.retain(|f| f != file);
    if off {
        st.off.push(file.to_string());
    }
    save_state(&st);
    invalidate();
}

pub fn remove(file: &str) {
    // 檔名只能是腳本資料夾裡的檔案
    if file.contains(['/', '\\']) || !file.ends_with(".user.js") {
        return;
    }
    let _ = std::fs::remove_file(dir().join(file));
    let _ = std::fs::remove_file(values_path(file));
    {
        let mut g = lock(&VALS);
        if let Some(v) = g.as_mut() {
            v.map.remove(file);
            v.dirty.remove(file);
            v.ver += 1;
            v.log.clear();
        }
    }
    let mut st = state();
    st.off.retain(|f| f != file);
    st.src.remove(file);
    save_state(&st);
    invalidate();
}

pub fn open_dir() {
    let _ = std::fs::create_dir_all(dir());
    let _ = crate::winsys::open(&dir().to_string_lossy());
}

/* ---------------- 注入網頁的程式 ---------------- */

/// 交給腳本的東西（順序要跟 us_shim.js 的 args 一樣）
const ARGS: &str = "GM, GM_info, GM_getValue, GM_setValue, GM_deleteValue, GM_listValues, GM_getValues, GM_setValues, GM_deleteValues, \
GM_addValueChangeListener, GM_removeValueChangeListener, GM_addStyle, GM_addElement, GM_getResourceText, GM_getResourceURL, \
GM_registerMenuCommand, GM_unregisterMenuCommand, GM_xmlhttpRequest, GM_download, GM_openInTab, GM_notification, GM_setClipboard, GM_log, \
GM_getTab, GM_saveTab, GM_getTabs, unsafeWindow";

/// 第一段：執行環境（每份文件都跑；腳本本身由後面幾段交進來）
pub fn shim_script() -> String {
    let host = Url::parse(crate::webcore::ui_base()).ok().and_then(|u| u.host_str().map(str::to_string)).unwrap_or_default();
    include_str!("us_shim.js").replace("__SLOT__", slot()).replace("__APP_HOST__", &host).replace("__TOKEN__", token()).replace("__DOWN__", down())
}

fn run_at(m: &Meta) -> &'static str {
    match m.run_at.as_str() {
        "document-start" => "start",
        "document-body" => "body",
        "document-end" => "end",
        _ => "idle",
    }
}

/// 一支腳本交進執行環境的那一段（每支腳本各自一段：其中一支寫錯不會連累別支）
fn def_script(s: &Script) -> String {
    let m = &s.meta;
    let res: BTreeMap<&String, serde_json::Value> = s.resources.iter().map(|(n, r)| (n, serde_json::json!({ "b": r.b64, "t": r.mime }))).collect();
    let d = serde_json::json!({
        "id": s.id, "name": m.name, "version": m.version, "desc": m.description, "ns": m.namespace, "author": m.author, "home": m.homepage,
        // 圖示可能是很長的 data: 網址，太長的不帶
        "icon": if m.icon.len() <= 4096 { m.icon.as_str() } else { "" },
        "matches": m.matches, "includes": m.includes, "excludes": m.excludes, "grants": m.grants, "conn": m.connects,
        "upd": m.update_url, "dl": m.download_url, "at": run_at(m), "frames": !m.noframes,
        "inc": inc_res(m), "exc": exc_res(m), "res": res,
    });
    // 腳本本身包成箭頭函式：網頁沒辦法從外面拿到交給腳本的那些功能（一般函式可以被 .caller.arguments 摸到）
    let reqs: String = s.requires.iter().map(|r| format!("{r}\n;\n")).collect();
    format!(
        "(function (g) {{ if (!g || !g.add) return; var d = {d}; d.fn = ({ARGS}) => {{\n{reqs}{code}\n}}; g.add({tok:?}, d); }})(window[{slot:?}]);",
        code = s.code,
        tok = token(),
        slot = slot()
    )
}

/// 所有會執行的腳本各自的那一段（清單沒變就用上次算好的）
pub fn def_scripts() -> Arc<Vec<String>> {
    static DEFS: Mutex<Option<(u64, Arc<Vec<String>>)>> = Mutex::new(None);
    let g = gen();
    if let Some((at, v)) = lock(&DEFS).as_ref() {
        if *at == g {
            return v.clone();
        }
    }
    let v: Arc<Vec<String>> = Arc::new(list().iter().filter(|s| s.active() && run_at_ok(&s.meta)).map(def_script).collect());
    *lock(&DEFS) = Some((g, v.clone()));
    v
}

/// @run-at context-menu 的腳本不會自動執行
fn run_at_ok(m: &Meta) -> bool {
    m.run_at != "context-menu"
}

/// 「前面交進來的腳本作廢」：分頁建立之後腳本清單變過時，放在新清單前面
pub fn reset_script() -> String {
    format!("(function (g) {{ if (g && g.reset) g.reset({:?}); }})(window[{:?}]);", token(), slot())
}

/// 帶設定值啟動腳本的那一段。baked = 建立分頁時先放一份備用的（不啟動）；否則是最新的一份（啟動）。
/// 設定值整包當成一段文字交進去（網頁裡真的有腳本要跑才會解開）：每份文件、每個內嵌框架都會執行這一段，
/// 設定值很多時（例如腳本拿它當快取），直接寫成物件會讓每個框架都白白解讀一次
pub fn boot_script(baked: bool) -> String {
    let (ver, vals) = values_snapshot();
    let f = if baked { "baked" } else { "go" };
    let text = serde_json::to_string(&vals).unwrap_or_else(|_| "\"{}\"".into());
    format!("(function (g) {{ if (g && g.{f}) g.{f}({:?}, {ver}, {text}); }})(window[{:?}]);", token(), slot())
}

/* ---------------- 設定值（GM_getValue／GM_setValue） ---------------- */

#[derive(Default)]
struct Vals {
    /// 檔名 → （鍵 → JSON 文字）
    map: HashMap<String, BTreeMap<String, String>>,
    dirty: HashSet<String>,
    /// 變過幾次
    ver: u64,
    changed: Option<Instant>,
    /// 最近變過的：（變完之後是第幾版, 檔名, 鍵）。網頁啟動時拿到的版本比較舊時，補這幾個就好，不用整份重傳
    log: std::collections::VecDeque<(u64, String, String)>,
}

/// 變動記錄最多留幾筆
const LOG_MAX: usize = 4000;

static VALS: Mutex<Option<Vals>> = Mutex::new(None);

fn values_path(file: &str) -> PathBuf {
    dir().join("values").join(format!("{file}.json"))
}

fn with_vals<T>(f: impl FnOnce(&mut Vals) -> T) -> T {
    let mut g = lock(&VALS);
    let v = g.get_or_insert_with(|| {
        let mut v = Vals { ver: 1, ..Vals::default() };
        if let Ok(rd) = std::fs::read_dir(dir().join("values")) {
            for e in rd.flatten() {
                let name = e.file_name().to_string_lossy().to_string();
                let Some(file) = name.strip_suffix(".json") else { continue };
                if let Some(m) = std::fs::read_to_string(e.path()).ok().and_then(|t| serde_json::from_str::<BTreeMap<String, String>>(&t).ok()) {
                    v.map.insert(file.to_string(), m);
                }
            }
        }
        v
    });
    f(v)
}

/// 目前的版本，以及所有會執行的腳本的設定值（{ 腳本編號: { 鍵: JSON 文字 } }）
fn values_snapshot() -> (u64, String) {
    let scripts = list();
    with_vals(|v| {
        let mut out = serde_json::Map::new();
        for s in scripts.iter().filter(|s| s.active()) {
            if let Some(m) = v.map.get(&s.file) {
                if !m.is_empty() {
                    out.insert(s.id.clone(), serde_json::to_value(m).unwrap_or_default());
                }
            }
        }
        (v.ver, serde_json::Value::Object(out).to_string())
    })
}

pub fn values_ver() -> u64 {
    with_vals(|v| v.ver)
}

/// 會執行的腳本的設定值總共多少位元組（決定多久更新一次各分頁預先帶著的那一份）
pub fn values_bytes() -> usize {
    let scripts = list();
    with_vals(|v| scripts.iter().filter(|s| s.active()).filter_map(|s| v.map.get(&s.file)).map(|m| m.iter().map(|(k, j)| k.len() + j.len() + 8).sum::<usize>()).sum())
}

/// 第 since 版之後，這幾支腳本變過的設定值（{ 腳本編號: { 鍵: JSON 文字，刪掉的是 null } }）。
/// None = 記錄沒有留那麼久（要整份重傳）
pub fn changes_since(since: u64, ids: &[String]) -> Option<serde_json::Value> {
    let scripts = list();
    with_vals(|v| {
        if since > v.ver {
            return None;
        }
        // 記錄要從 since 的下一版開始都有
        let first = v.log.front().map(|e| e.0).unwrap_or(v.ver + 1);
        if since + 1 < first || (v.log.is_empty() && since != v.ver) {
            return None;
        }
        let mut out = serde_json::Map::new();
        for (ver, file, key) in v.log.iter() {
            if *ver <= since {
                continue;
            }
            let Some(s) = scripts.iter().find(|s| &s.file == file && ids.contains(&s.id)) else { continue };
            let cur = v.map.get(file).and_then(|m| m.get(key)).map(|j| serde_json::Value::String(j.clone())).unwrap_or(serde_json::Value::Null);
            out.entry(s.id.clone()).or_insert_with(|| serde_json::Value::Object(Default::default())).as_object_mut()?.insert(key.clone(), cur);
        }
        Some(serde_json::Value::Object(out))
    })
}

/// 幾支腳本目前的設定值（網頁啟動時拿到的太舊，要整份對一次時用）
pub fn values_of(ids: &[String]) -> serde_json::Value {
    let scripts = list();
    with_vals(|v| {
        let mut out = serde_json::Map::new();
        for s in scripts.iter().filter(|s| ids.contains(&s.id)) {
            out.insert(s.id.clone(), serde_json::to_value(v.map.get(&s.file).cloned().unwrap_or_default()).unwrap_or_default());
        }
        serde_json::Value::Object(out)
    })
}

/// 改一個設定值（v = None 是刪掉）。回傳有沒有真的變
pub fn value_set(file: &str, k: &str, val: Option<&str>) -> bool {
    // 單一個值最多 32 MB（再大多半是腳本寫錯了）
    if val.is_some_and(|v| v.len() > 32 << 20) || k.len() > 4096 {
        return false;
    }
    with_vals(|v| {
        let m = v.map.entry(file.to_string()).or_default();
        let changed = match val {
            Some(j) => m.insert(k.to_string(), j.to_string()).as_deref() != Some(j),
            None => m.remove(k).is_some(),
        };
        if changed {
            v.dirty.insert(file.to_string());
            v.ver += 1;
            v.changed = Some(Instant::now());
            v.log.push_back((v.ver, file.to_string(), k.to_string()));
            if v.log.len() > LOG_MAX {
                v.log.pop_front();
            }
        }
        changed
    })
}

/// 把變過的設定值寫進檔案（force = 不等，馬上寫；平常等沒有再變動一下子才寫）
pub fn flush(force: bool) {
    let todo: Vec<(String, String)> = with_vals(|v| {
        if v.dirty.is_empty() || (!force && v.changed.is_some_and(|t| t.elapsed() < Duration::from_millis(600))) {
            return Vec::new();
        }
        let files: Vec<String> = v.dirty.drain().collect();
        files.into_iter().map(|f| (f.clone(), serde_json::to_string(v.map.get(&f).unwrap_or(&BTreeMap::new())).unwrap_or_else(|_| "{}".into()))).collect()
    });
    if todo.is_empty() {
        return;
    }
    let _ = std::fs::create_dir_all(dir().join("values"));
    for (file, json) in todo {
        let p = values_path(&file);
        let tmp = p.with_extension("tmp");
        if std::fs::write(&tmp, json).is_ok() {
            let _ = std::fs::rename(&tmp, &p);
        }
    }
}

/* ---------------- 網頁裡的腳本傳來的訊息 ---------------- */

fn yes() -> bool {
    true
}

#[derive(Debug, Clone, Deserialize)]
pub struct XhrReq {
    pub s: String,
    pub rid: u64,
    pub url: String,
    #[serde(default)]
    pub method: String,
    #[serde(default)]
    pub headers: BTreeMap<String, String>,
    /// base64
    #[serde(default)]
    pub body: Option<String>,
    /// 毫秒；0 = 不限
    #[serde(default)]
    pub timeout: f64,
    /// 不要帶 Cookie
    #[serde(default)]
    pub anon: bool,
    #[serde(default)]
    pub ua: String,
    /// follow（預設）/ error / manual
    #[serde(default)]
    pub redirect: String,
    #[serde(default)]
    pub nocache: bool,
    #[serde(default)]
    pub user: Option<String>,
    #[serde(default)]
    pub password: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "op", rename_all = "kebab-case")]
pub enum Msg {
    /// 一份文件啟動了：裡面跑了哪些腳本、它拿到的設定值是第幾版
    Hi {
        #[serde(default)]
        ids: Vec<String>,
        #[serde(default)]
        ver: u64,
    },
    Set {
        s: String,
        k: String,
        v: Option<String>,
    },
    Menu {
        s: String,
        id: String,
        caption: String,
        #[serde(default)]
        title: String,
        #[serde(default = "yes")]
        close: bool,
    },
    Unmenu {
        s: String,
        id: String,
    },
    Xhr(XhrReq),
    XhrAbort {
        rid: u64,
    },
    Tab {
        s: String,
        url: String,
        #[serde(default = "yes")]
        active: bool,
    },
    Notify {
        s: String,
        #[serde(default)]
        title: String,
        #[serde(default)]
        text: String,
    },
    Clip {
        s: String,
        text: String,
    },
}

/// 是不是使用者腳本傳來的訊息（看開頭就知道；其他網頁自己傳的不理）
pub fn msg_like(raw: &str) -> bool {
    raw.starts_with("{\"cmd\":\"mb\",")
}

/// 解讀網頁傳來的訊息：權杖不對、格式不對都當成沒這回事
pub fn parse_msg(raw: &str) -> Option<Msg> {
    #[derive(Deserialize)]
    struct Envelope {
        mb: String,
        d: String,
    }
    if !msg_like(raw) || raw.len() > 96 << 20 {
        return None;
    }
    let e: Envelope = serde_json::from_str(raw).ok()?;
    if e.mb != token() {
        return None;
    }
    serde_json::from_str(&e.d).ok()
}

/// 程式要傳給網頁的訊息（加上記號）
pub fn down_msg(mut v: serde_json::Value) -> String {
    if let Some(o) = v.as_object_mut() {
        o.insert("mb".into(), down().into());
    }
    v.to_string()
}

/* ---------------- GM_xmlhttpRequest：可以連哪裡、要不要帶 Cookie ---------------- */

/// 兩層的公共網域（example.com.tw 這種，網站是三層）。只列常見的；沒列到的當成兩層
const SUFFIX2: [&str; 44] = [
    "com.tw", "org.tw", "net.tw", "edu.tw", "gov.tw", "idv.tw", "game.tw", "club.tw", "co.uk", "org.uk", "ac.uk", "gov.uk", "me.uk", "co.jp", "ne.jp", "or.jp",
    "ac.jp", "go.jp", "com.cn", "net.cn", "org.cn", "edu.cn", "gov.cn", "com.hk", "org.hk", "edu.hk", "co.kr", "or.kr", "com.au", "net.au", "org.au", "com.sg",
    "com.my", "co.nz", "co.in", "com.br", "co.th", "github.io", "pages.dev", "workers.dev", "vercel.app", "netlify.app", "web.app", "blogspot.com",
];

/// 網站（可以註冊的那一層網域）：www.example.com.tw → example.com.tw。IP 位址就是它自己
pub fn site_of(host: &str) -> String {
    let h = host.trim_end_matches('.').to_ascii_lowercase();
    if h.parse::<std::net::IpAddr>().is_ok() || h.starts_with('[') {
        return h;
    }
    let labels: Vec<&str> = h.split('.').collect();
    let n = labels.len();
    if n <= 2 {
        return h;
    }
    let last2 = labels[n - 2..].join(".");
    let take = if SUFFIX2.contains(&last2.as_str()) { 3 } else { 2 };
    labels[n.saturating_sub(take)..].join(".")
}

#[derive(Debug, PartialEq, Clone, Copy)]
pub struct Access {
    /// 可以連
    pub ok: bool,
    /// 可以帶這個瀏覽器在那個網站的 Cookie
    pub cookies: bool,
}

/// 腳本在 page 這個網頁上，能不能連到 target。
/// - 跟網頁同一個網站：可以，也帶 Cookie（跟網頁自己連線一樣）
/// - @connect 有明確寫到的網域：可以，也帶 Cookie
/// - @connect *，或腳本完全沒寫 @connect：可以，但不帶 Cookie（不然任何網站的登入狀態都能被拿去用）
pub fn connect_access(m: &Meta, page: &str, target: &Url) -> Access {
    let no = Access { ok: false, cookies: false };
    if !matches!(target.scheme(), "http" | "https") {
        return no;
    }
    let Some(host) = target.host_str().map(|h| h.to_ascii_lowercase()) else { return no };
    let page_host = Url::parse(page).ok().and_then(|u| u.host_str().map(|h| h.to_ascii_lowercase())).unwrap_or_default();
    let same = !page_host.is_empty() && site_of(&page_host) == site_of(&host);
    if same {
        return Access { ok: true, cookies: true };
    }
    let mut star = m.connects.is_empty();
    for c in &m.connects {
        let c = c.trim().to_ascii_lowercase();
        match c.as_str() {
            "*" => star = true,
            "self" => {}
            "localhost" => {
                if host == "localhost" || host == "127.0.0.1" || host == "[::1]" {
                    return Access { ok: true, cookies: true };
                }
            }
            d => {
                // 有人會寫成網址或 *.example.com
                let d = d.trim_start_matches("*.").trim_start_matches("http://").trim_start_matches("https://");
                let d = d.split('/').next().unwrap_or(d);
                if !d.is_empty() && (host == d || host.ends_with(&format!(".{d}"))) {
                    return Access { ok: true, cookies: true };
                }
            }
        }
    }
    Access { ok: star, cookies: false }
}

/* ---------------- GM_xmlhttpRequest：連線 ---------------- */

/// 同時最多幾條連線（腳本一次丟幾十條時排隊）
const XHR_MAX: usize = 16;
static XHR_SLOTS: (Mutex<usize>, Condvar) = (Mutex::new(0), Condvar::new());

struct SlotGuard;

impl SlotGuard {
    fn take(abort: &AtomicBool) -> Option<SlotGuard> {
        let (m, cv) = &XHR_SLOTS;
        let mut n = lock(m);
        while *n >= XHR_MAX {
            if abort.load(Ordering::SeqCst) {
                return None;
            }
            n = cv.wait_timeout(n, Duration::from_millis(200)).map(|(g, _)| g).unwrap_or_else(|e| e.into_inner().0);
        }
        *n += 1;
        Some(SlotGuard)
    }
}

impl Drop for SlotGuard {
    fn drop(&mut self) {
        let (m, cv) = &XHR_SLOTS;
        let mut n = lock(m);
        *n = n.saturating_sub(1);
        cv.notify_one();
    }
}

fn xhr_ev(rid: u64, t: &str, mut extra: serde_json::Value) -> String {
    if let Some(o) = extra.as_object_mut() {
        o.insert("ev".into(), "xhr".into());
        o.insert("rid".into(), rid.into());
        o.insert("t".into(), t.into());
    }
    down_msg(extra)
}

/// 這些標頭由連線本身決定，腳本給的不用
fn hop_header(name: &str) -> bool {
    matches!(name.to_ascii_lowercase().as_str(), "host" | "content-length" | "connection" | "transfer-encoding" | "accept-encoding" | "x-mb-cookie")
}

/// 代替腳本連線。cookies = 取得這個瀏覽器在某個網址的 Cookie（"a=1; b=2"）；emit = 把一則訊息傳回網頁。
/// 在背景執行緒呼叫（會等到連線結束）
pub fn run_xhr(req: XhrReq, meta: &Meta, page: &str, cookies: &dyn Fn(&Url) -> Option<String>, abort: &AtomicBool, emit: &dyn Fn(String)) {
    let rid = req.rid;
    let fail = |kind: &str, msg: &str| emit(xhr_ev(rid, "err", serde_json::json!({ "kind": kind, "msg": msg })));
    let Some(_slot) = SlotGuard::take(abort) else { return };
    let Ok(mut url) = Url::parse(&req.url) else { return fail("error", "網址不對") };
    let mut method = if req.method.is_empty() { "GET".to_string() } else { req.method.to_ascii_uppercase() };
    let mut body: Option<Vec<u8>> = match &req.body {
        Some(b) => match base64::engine::general_purpose::STANDARD.decode(b) {
            Ok(v) => Some(v),
            Err(_) => return fail("error", "內容格式不對"),
        },
        None => None,
    };
    let deadline = (req.timeout > 0.0).then(|| Instant::now() + Duration::from_millis(req.timeout.min(3_600_000.0) as u64));
    let script_cookie = req.headers.iter().find(|(k, _)| k.eq_ignore_ascii_case("x-mb-cookie")).map(|(_, v)| v.clone());
    for _hop in 0..=10 {
        if abort.load(Ordering::SeqCst) {
            return;
        }
        let acc = connect_access(meta, page, &url);
        if !acc.ok {
            return fail("error", "腳本沒有允許連到這個網域（@connect）");
        }
        let mut ab = ureq::AgentBuilder::new().redirects(0).timeout_connect(Duration::from_secs(30)).timeout_read(Duration::from_secs(120)).timeout_write(Duration::from_secs(120));
        if let Some(d) = deadline {
            let left = d.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return fail("timeout", "");
            }
            ab = ab.timeout(left);
        }
        let mut r = ab.build().request(&method, url.as_str());
        let has = |n: &str| req.headers.keys().any(|k| k.eq_ignore_ascii_case(n));
        for (k, v) in &req.headers {
            if !hop_header(k) && !k.trim().is_empty() {
                r = r.set(k, v);
            }
        }
        if !has("user-agent") {
            r = r.set("User-Agent", if req.ua.is_empty() { UA } else { &req.ua });
        }
        if !has("accept") {
            r = r.set("Accept", "*/*");
        }
        if req.nocache && !has("cache-control") {
            r = r.set("Cache-Control", "no-cache");
        }
        if !has("cookie") {
            let mut jar = if !req.anon && acc.cookies { cookies(&url).unwrap_or_default() } else { String::new() };
            if let Some(c) = &script_cookie {
                if !jar.is_empty() {
                    jar.push_str("; ");
                }
                jar.push_str(c);
            }
            if !jar.is_empty() {
                r = r.set("Cookie", &jar);
            }
        }
        if let (Some(u), false) = (&req.user, has("authorization")) {
            let raw = format!("{u}:{}", req.password.clone().unwrap_or_default());
            r = r.set("Authorization", &format!("Basic {}", base64::engine::general_purpose::STANDARD.encode(raw)));
        }
        let res = match &body {
            Some(b) => r.send_bytes(b),
            None => r.call(),
        };
        let resp = match res {
            Ok(r) | Err(ureq::Error::Status(_, r)) => r,
            Err(ureq::Error::Transport(e)) => {
                let text = e.to_string();
                let timed = deadline.is_some_and(|d| Instant::now() >= d) || text.contains("timed out") || text.contains("timeout");
                return fail(if timed { "timeout" } else { "error" }, &text);
            }
        };
        let status = resp.status();
        // 轉址：自己跟（每一站都要重新確認能不能連）
        if matches!(status, 301 | 302 | 303 | 307 | 308) && req.redirect != "manual" {
            if let Some(next) = resp.header("location").and_then(|l| url.join(l).ok()) {
                if req.redirect == "error" {
                    return fail("error", "遇到轉址");
                }
                if status == 303 || (matches!(status, 301 | 302) && method == "POST") {
                    method = "GET".into();
                    body = None;
                }
                url = next;
                continue;
            }
        }
        let mut headers = String::new();
        let mut encoded = false;
        for name in resp.headers_names() {
            if name.eq_ignore_ascii_case("content-encoding") {
                encoded = true;
            }
            for v in resp.all(&name) {
                headers.push_str(&format!("{name}: {v}\r\n"));
            }
        }
        let total: u64 = if encoded { 0 } else { resp.header("content-length").and_then(|v| v.parse().ok()).unwrap_or(0) };
        emit(xhr_ev(rid, "head", serde_json::json!({ "status": status, "statusText": resp.status_text(), "headers": headers, "url": resp.get_url(), "total": total })));
        let mut rd = resp.into_reader();
        // base64 之後大約 256 KB 一則
        let mut buf = vec![0u8; 192 * 1024];
        loop {
            if abort.load(Ordering::SeqCst) {
                return;
            }
            match rd.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => emit(xhr_ev(rid, "data", serde_json::json!({ "b": base64::engine::general_purpose::STANDARD.encode(&buf[..n]) }))),
                Err(e) => {
                    let timed = e.kind() == std::io::ErrorKind::TimedOut || e.kind() == std::io::ErrorKind::WouldBlock || deadline.is_some_and(|d| Instant::now() >= d);
                    return fail(if timed { "timeout" } else { "error" }, &e.to_string());
                }
            }
        }
        return emit(xhr_ev(rid, "end", serde_json::json!({})));
    }
    fail("error", "轉址太多次")
}

/* ---------------- 從網址安裝、更新 ---------------- */

/// 是不是使用者腳本的網址（.user.js 結尾）
pub fn is_script_url(u: &Url) -> bool {
    matches!(u.scheme(), "http" | "https") && u.path().to_ascii_lowercase().ends_with(".user.js")
}

/// 版本比大小（1.10 比 1.9 新）
pub fn version_cmp(a: &str, b: &str) -> std::cmp::Ordering {
    fn parts(v: &str) -> Vec<(u64, String)> {
        v.trim()
            .split(['.', '-', '_', '+'])
            .map(|p| {
                let digits: String = p.chars().take_while(|c| c.is_ascii_digit()).collect();
                (digits.parse().unwrap_or(0), p[digits.len()..].to_string())
            })
            .collect()
    }
    let (pa, pb) = (parts(a), parts(b));
    for i in 0..pa.len().max(pb.len()) {
        let x = pa.get(i).cloned().unwrap_or_default();
        let y = pb.get(i).cloned().unwrap_or_default();
        match x.0.cmp(&y.0).then_with(|| x.1.cmp(&y.1)) {
            std::cmp::Ordering::Equal => {}
            o => return o,
        }
    }
    std::cmp::Ordering::Equal
}

/// 剛下載、等使用者按「安裝」的腳本（網址 → 內容）
static PENDING: Mutex<Vec<(String, String)>> = Mutex::new(Vec::new());

fn fetch_script(url: &str) -> Result<String, String> {
    let (body, _) = http_get(url, 45, 24 << 20)?;
    let text = String::from_utf8_lossy(&body).into_owned();
    if !text.contains("==UserScript==") {
        return Err("這個網址不是使用者腳本（找不到 ==UserScript== 區塊）".into());
    }
    Ok(text)
}

#[derive(Serialize)]
pub struct Preview {
    pub url: String,
    pub meta: Meta,
    /// 位元組
    pub size: usize,
    /// 已經裝了同名的：目前的版本（沒裝是空的）
    pub installed: String,
    /// 已經裝了同名的
    pub exists: bool,
}

/// 下載腳本給使用者看（還沒安裝）
pub fn preview(url: &str) -> Result<Preview, String> {
    let text = fetch_script(url)?;
    let meta = parse_meta(&text);
    if meta.matches.is_empty() && meta.includes.is_empty() {
        return Err(format!("「{}」沒有 @match 或 @include，不知道要在哪些網頁執行", meta.name));
    }
    let file = format!("{}.user.js", safe_name(&meta.name));
    let old = list().iter().find(|s| s.file == file).map(|s| s.meta.version.clone());
    let size = text.len();
    {
        let mut p = lock(&PENDING);
        p.retain(|(u, _)| u != url);
        p.push((url.to_string(), text));
        if p.len() > 4 {
            p.remove(0);
        }
    }
    Ok(Preview { url: url.to_string(), meta, size, exists: old.is_some(), installed: old.unwrap_or_default() })
}

/// 安裝剛才看過的那一份（沒看過就重新下載）。回傳腳本名稱
pub fn install(url: &str) -> Result<String, String> {
    let cached = {
        let mut p = lock(&PENDING);
        p.iter().position(|(u, _)| u == url).map(|i| p.remove(i).1)
    };
    let text = match cached {
        Some(t) => t,
        None => fetch_script(url)?,
    };
    let name = parse_meta(&text).name;
    let file = write_source(&text)?;
    let mut st = state();
    st.src.insert(file, url.to_string());
    save_state(&st);
    invalidate();
    Ok(name)
}

/// 檢查並安裝腳本的新版（force = 不管上次什麼時候檢查過、也不管有沒有關掉自動更新）。回傳更新了哪些（名稱 版本）
pub fn check_updates(force: bool) -> Vec<String> {
    static BUSY: AtomicBool = AtomicBool::new(false);
    let st = state();
    if !force && (st.no_update || now_ms().saturating_sub(st.checked) < 20 * 3600 * 1000) {
        return Vec::new();
    }
    if BUSY.swap(true, Ordering::SeqCst) {
        return Vec::new();
    }
    let mut done = Vec::new();
    for s in list().iter() {
        let m = &s.meta;
        let from = [m.download_url.as_str(), m.update_url.as_str(), st.src.get(&s.file).map(String::as_str).unwrap_or("")].into_iter().find(|u| !u.is_empty());
        let Some(from) = from else { continue };
        // 有另外的「只有資訊」的網址就先看那個，確定有新版才下載整支
        if !m.update_url.is_empty() && m.update_url != from {
            match http_get(&m.update_url, 30, 2 << 20) {
                Ok((b, _)) => {
                    let v = parse_meta(&String::from_utf8_lossy(&b)).version;
                    if version_cmp(&v, &m.version) != std::cmp::Ordering::Greater {
                        continue;
                    }
                }
                Err(_) => continue,
            }
        }
        let Ok(text) = fetch_script(from) else { continue };
        let new = parse_meta(&text);
        if version_cmp(&new.version, &m.version) != std::cmp::Ordering::Greater || (new.matches.is_empty() && new.includes.is_empty()) {
            continue;
        }
        // 換掉原本那個檔案（就算新版改了名字，也還是同一支）
        if std::fs::write(dir().join(&s.file), &text).is_ok() {
            fetch_deps(&new);
            done.push(format!("{} {}", new.name, new.version));
        }
    }
    let mut st = state();
    st.checked = now_ms();
    save_state(&st);
    BUSY.store(false, Ordering::SeqCst);
    if !done.is_empty() {
        invalidate();
    }
    done
}

/// 背景：程式開著的時候定期檢查腳本更新、把設定值寫進檔案
pub fn start_background() {
    std::thread::spawn(|| {
        std::thread::sleep(Duration::from_secs(90));
        loop {
            if any() {
                check_updates(false);
            }
            std::thread::sleep(Duration::from_secs(3 * 3600));
        }
    });
}

/* ---------------- 給「擴充功能」頁面的指令 ---------------- */

#[derive(Serialize)]
pub struct ScriptsOut {
    pub list: Vec<Script>,
    pub dir: String,
    /// 自動更新開著
    pub auto_update: bool,
}

#[tauri::command]
pub fn web_scripts_list() -> ScriptsOut {
    ScriptsOut { list: list().as_ref().clone(), dir: dir().to_string_lossy().into_owned(), auto_update: !state().no_update }
}

#[tauri::command]
pub async fn web_scripts_import() -> Result<String, String> {
    let files = tauri::async_runtime::spawn_blocking(|| {
        rfd::FileDialog::new()
            .set_title("選擇腳本（.user.js）或篡改猴匯出的備份（.zip）")
            .add_filter("腳本或備份", &["js", "zip"])
            .pick_files()
    })
    .await
    .map_err(|e| e.to_string())?;
    let Some(files) = files else { return Ok(String::new()) };
    tauri::async_runtime::spawn_blocking(move || {
        let mut n = 0;
        let mut errors = Vec::new();
        for f in files {
            match import_file(&f) {
                Ok((k, errs)) => {
                    n += k;
                    errors.extend(errs);
                }
                Err(e) => errors.push(e),
            }
        }
        let mut msg = format!("已匯入 {n} 個腳本");
        if !errors.is_empty() {
            msg.push_str(&format!("；{} 個略過：{}", errors.len(), errors.join("、")));
        }
        msg
    })
    .await
    .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn web_scripts_toggle(file: String, on: bool) {
    set_off(&file, !on);
}

#[tauri::command]
pub fn web_scripts_delete(file: String) {
    remove(&file);
}

#[tauri::command]
pub fn web_scripts_open_dir() {
    open_dir();
}

/// 重新讀取資料夾（使用者自己放檔案進去後），缺的檔案再下載一次
#[tauri::command]
pub async fn web_scripts_reload() -> ScriptsOut {
    let _ = tauri::async_runtime::spawn_blocking(|| {
        drop_cache();
        for s in list().iter() {
            if !s.missing.is_empty() {
                fetch_deps(&s.meta);
            }
        }
        invalidate();
    })
    .await;
    web_scripts_list()
}

/// 下載一個網址上的腳本給使用者看（安裝頁）
#[tauri::command]
pub async fn web_scripts_preview(url: String) -> Result<Preview, String> {
    tauri::async_runtime::spawn_blocking(move || preview(&url)).await.map_err(|e| e.to_string())?
}

/// 安裝（或更新成）那個網址上的腳本
#[tauri::command]
pub async fn web_scripts_install(url: String) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || install(&url)).await.map_err(|e| e.to_string())?
}

/// 現在就檢查更新。回傳訊息
#[tauri::command]
pub async fn web_scripts_update() -> Result<String, String> {
    let done = tauri::async_runtime::spawn_blocking(|| check_updates(true)).await.map_err(|e| e.to_string())?;
    Ok(if done.is_empty() { "腳本都是最新的".to_string() } else { format!("已更新：{}", done.join("、")) })
}

#[tauri::command]
pub fn web_scripts_auto(on: bool) {
    let mut st = state();
    st.no_update = !on;
    save_state(&st);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn meta_and_match() {
        let src = "// ==UserScript==\n// @name  測試\n// @match *://*.gamer.com.tw/*\n// @match https://ani.gamer.com.tw/animeVideo.php*\n// @exclude https://forum.gamer.com.tw/*\n// @run-at document-start\n// @require https://x/y.js\n// @resource css https://x/a.css\n// @grant GM_setValue\n// @connect api.x.com\n// @connect *\n// @updateURL https://u/x.meta.js\n// @downloadURL https://u/x.user.js\n// ==/UserScript==\nconsole.log(1)";
        let m = parse_meta(src);
        assert_eq!(m.name, "測試");
        assert_eq!(m.run_at, "document-start");
        assert_eq!(m.requires, vec!["https://x/y.js"]);
        assert_eq!(m.resources, vec![("css".to_string(), "https://x/a.css".to_string())]);
        assert_eq!(m.connects, vec!["api.x.com", "*"]);
        assert_eq!((m.update_url.as_str(), m.download_url.as_str()), ("https://u/x.meta.js", "https://u/x.user.js"));
        assert!(applies(&m, "https://ani.gamer.com.tw/animeVideo.php?sn=1"));
        assert!(applies(&m, "http://home.gamer.com.tw/"));
        assert!(applies(&m, "https://gamer.com.tw/x"));
        assert!(!applies(&m, "https://forum.gamer.com.tw/C.php"));
        assert!(!applies(&m, "https://www.youtube.com/"));
    }

    #[test]
    fn include_patterns() {
        let m = parse_meta("// ==UserScript==\n// @include /^https?:\\/\\/m\\..*/\n// @include *://x.com/*\n// @exclude-match *://x.com/settings/*\n// ==/UserScript==");
        assert!(applies(&m, "https://m.youtube.com/watch"));
        assert!(applies(&m, "https://x.com/home"));
        assert!(applies(&m, "https://x.com/home#top"));
        assert!(!applies(&m, "https://x.com/settings/account"));
        assert!(!applies(&m, "https://www.youtube.com/"));
        let all = parse_meta("// ==UserScript==\n// @match *://*/*\n// ==/UserScript==");
        assert!(applies(&all, "https://any.site/a/b?c"));
        assert!(!applies(&all, "chrome-extension://abc/popup.html"));
        // 很多腳本寫的 http*://
        let star = parse_meta("// ==UserScript==\n// @match http*://*.example.com/*\n// ==/UserScript==");
        assert!(applies(&star, "https://a.example.com/x"));
        assert!(applies(&star, "http://example.com/"));
        assert!(!applies(&star, "ftp://example.com/"));
        // 這邊看不懂的正規表示式（JavaScript 才有的寫法）：要執行的當成符合
        let js = parse_meta("// ==UserScript==\n// @include /^https:\\/\\/(?<!x)a\\.com\\//\n// ==/UserScript==");
        assert!(applies(&js, "https://a.com/"));
    }

    #[test]
    fn localized_name() {
        let m = parse_meta("// ==UserScript==\n// @name Foo\n// @name:zh-TW 中文名\n// @name:ja 日本語\n// @match *://*/*\n// ==/UserScript==");
        assert_eq!(m.name, "中文名");
    }

    #[test]
    fn sites() {
        assert_eq!(site_of("www.example.com"), "example.com");
        assert_eq!(site_of("a.b.example.com.tw"), "example.com.tw");
        assert_eq!(site_of("example.com.tw"), "example.com.tw");
        assert_eq!(site_of("foo.github.io"), "foo.github.io");
        assert_eq!(site_of("127.0.0.1"), "127.0.0.1");
        assert_eq!(site_of("localhost"), "localhost");
    }

    #[test]
    fn connect_rules() {
        let u = |s: &str| Url::parse(s).unwrap();
        let m = parse_meta("// ==UserScript==\n// @match *://*/*\n// @connect api.example.com\n// @connect *\n// ==/UserScript==");
        // 同一個網站：帶 Cookie
        assert_eq!(connect_access(&m, "https://www.site.com/a", &u("https://img.site.com/x.png")), Access { ok: true, cookies: true });
        // 明確寫到的網域（含子網域）：帶 Cookie
        assert_eq!(connect_access(&m, "https://www.site.com/a", &u("https://v2.api.example.com/x")), Access { ok: true, cookies: true });
        // 只靠 * 才能連的：不帶 Cookie
        assert_eq!(connect_access(&m, "https://www.site.com/a", &u("https://mail.google.com/")), Access { ok: true, cookies: false });
        // 不同網站只是結尾一樣，不算同一個網站
        assert_eq!(connect_access(&m, "https://evil.com.tw/", &u("https://bank.com.tw/")), Access { ok: true, cookies: false });
        // 沒有 * 的：沒寫到的不能連
        let strict = parse_meta("// ==UserScript==\n// @match *://*/*\n// @connect api.example.com\n// ==/UserScript==");
        assert_eq!(connect_access(&strict, "https://www.site.com/a", &u("https://other.com/")), Access { ok: false, cookies: false });
        assert_eq!(connect_access(&strict, "https://www.site.com/a", &u("https://notapi.example.com.evil.com/")), Access { ok: false, cookies: false });
        // 完全沒寫 @connect：可以連，但不帶 Cookie
        let none = parse_meta("// ==UserScript==\n// @match *://*/*\n// ==/UserScript==");
        assert_eq!(connect_access(&none, "https://www.site.com/a", &u("https://other.com/")), Access { ok: true, cookies: false });
        // 不是 http 的不行
        assert!(!connect_access(&m, "https://www.site.com/a", &u("file:///c:/x")).ok);
    }

    #[test]
    fn versions() {
        use std::cmp::Ordering::*;
        assert_eq!(version_cmp("1.10", "1.9"), Greater);
        assert_eq!(version_cmp("1.6.1", "1.6.1"), Equal);
        assert_eq!(version_cmp("1.6", "1.6.0"), Equal);
        assert_eq!(version_cmp("2.0", "10.0"), Less);
        assert_eq!(version_cmp("1.0.1b", "1.0.1a"), Greater);
        assert_eq!(version_cmp("2026.11.14", "2026.2.1"), Greater);
    }

    #[test]
    fn messages() {
        let env = |d: &str, tok: &str| format!("{{\"cmd\":\"mb\",\"callback\":0,\"error\":0,\"payload\":null,\"__TAURI_INVOKE_KEY__\":\"\",\"mb\":{},\"d\":{}}}", serde_json::to_string(tok).unwrap(), serde_json::to_string(d).unwrap());
        let m = parse_msg(&env(r#"{"op":"set","s":"s1","k":"a","v":"1"}"#, token()));
        assert!(matches!(m, Some(Msg::Set { ref s, ref k, v: Some(ref v) }) if s == "s1" && k == "a" && v == "1"));
        assert!(matches!(parse_msg(&env(r#"{"op":"set","s":"s1","k":"a","v":null}"#, token())), Some(Msg::Set { v: None, .. })));
        assert!(matches!(parse_msg(&env(r#"{"op":"hi","ids":["s1"],"ver":7}"#, token())), Some(Msg::Hi { ver: 7, .. })));
        assert!(matches!(parse_msg(&env(r#"{"op":"xhr-abort","rid":3}"#, token())), Some(Msg::XhrAbort { rid: 3 })));
        let x = parse_msg(&env(r#"{"op":"xhr","s":"s1","rid":2,"url":"https://a/","method":"POST","headers":{"A":"b"},"body":"eA==","timeout":5000,"anon":false,"ua":"x","redirect":"","nocache":false,"user":null,"password":null}"#, token()));
        assert!(matches!(x, Some(Msg::Xhr(ref r)) if r.rid == 2 && r.method == "POST" && r.headers.get("A").map(String::as_str) == Some("b")));
        // 權杖不對、不是這種訊息：不理
        assert!(parse_msg(&env(r#"{"op":"hi"}"#, "wrong")).is_none());
        assert!(parse_msg(r#"{"mb":"x","d":"{}"}"#).is_none());
        assert!(parse_msg("hello").is_none());
    }

    /// 測試用的小伺服器：回傳收到的要求（方法、路徑、標頭、內容），幾個特別的路徑有特別的行為
    fn test_server() -> u16 {
        use std::io::{BufRead, BufReader, Write};
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = l.local_addr().unwrap().port();
        std::thread::spawn(move || {
            for s in l.incoming().flatten() {
                std::thread::spawn(move || {
                    let mut rd = BufReader::new(s.try_clone().unwrap());
                    let mut first = String::new();
                    if rd.read_line(&mut first).is_err() {
                        return;
                    }
                    let mut headers = Vec::new();
                    let mut len = 0usize;
                    loop {
                        let mut h = String::new();
                        if rd.read_line(&mut h).is_err() || h.trim().is_empty() {
                            break;
                        }
                        if let Some(v) = h.to_ascii_lowercase().strip_prefix("content-length:") {
                            len = v.trim().parse().unwrap_or(0);
                        }
                        headers.push(h.trim().to_string());
                    }
                    let mut body = vec![0u8; len];
                    let _ = rd.read_exact(&mut body);
                    let mut s = s;
                    let path = first.split_whitespace().nth(1).unwrap_or("/").to_string();
                    let method = first.split_whitespace().next().unwrap_or("").to_string();
                    if path == "/redir" {
                        let _ = s.write_all(b"HTTP/1.1 302 Found\r\nLocation: /echo?after=1\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
                    } else if path == "/away" {
                        let _ = s.write_all(b"HTTP/1.1 302 Found\r\nLocation: http://localhost:1/x\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
                    } else if path == "/slow" {
                        std::thread::sleep(Duration::from_millis(1500));
                        let _ = s.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok");
                    } else if path.starts_with("/s.user.js") {
                        let out = "// ==UserScript==\n// @name Vals\n// @version 2.0\n// @match *://*/*\n// @grant GM_setValue\n// ==/UserScript==\n2;";
                        let _ = s.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: text/javascript\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{out}", out.len()).as_bytes());
                    } else if path == "/404" {
                        let _ = s.write_all(b"HTTP/1.1 404 Not Found\r\nContent-Type: text/plain\r\nContent-Length: 4\r\nConnection: close\r\n\r\nnope");
                    } else if path == "/stream" {
                        let _ = s.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n");
                        for part in ["data: 1\n", "data: 2\n", "data: 3\n"] {
                            let _ = s.write_all(format!("{:x}\r\n{part}\r\n", part.len()).as_bytes());
                            let _ = s.flush();
                            std::thread::sleep(Duration::from_millis(120));
                        }
                        let _ = s.write_all(b"0\r\n\r\n");
                    } else {
                        let out = serde_json::json!({ "method": method, "path": path, "headers": headers, "body": String::from_utf8_lossy(&body) }).to_string();
                        let _ = s.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nX-Test: yes\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{out}", out.len()).as_bytes());
                    }
                });
            }
        });
        port
    }

    /// 跑一次連線，回傳收到的每一則訊息（已經解開）
    fn xhr(meta: &Meta, page: &str, req: serde_json::Value, jar: Option<&str>) -> Vec<serde_json::Value> {
        let req: XhrReq = serde_json::from_value(req).unwrap();
        let out = Mutex::new(Vec::new());
        let abort = AtomicBool::new(false);
        let cookies = |_u: &Url| jar.map(str::to_string);
        let emit = |j: String| out.lock().unwrap().push(serde_json::from_str::<serde_json::Value>(&j).unwrap());
        run_xhr(req, meta, page, &cookies, &abort, &emit);
        out.into_inner().unwrap()
    }

    fn body_of(evs: &[serde_json::Value]) -> String {
        let mut b = Vec::new();
        for e in evs.iter().filter(|e| e["t"] == "data") {
            b.extend(base64::engine::general_purpose::STANDARD.decode(e["b"].as_str().unwrap()).unwrap());
        }
        String::from_utf8_lossy(&b).into_owned()
    }

    #[test]
    fn xhr_requests() {
        let port = test_server();
        let base = format!("http://127.0.0.1:{port}");
        let any = parse_meta("// ==UserScript==\n// @match *://*/*\n// @connect *\n// ==/UserScript==");
        let listed = parse_meta("// ==UserScript==\n// @match *://*/*\n// @connect 127.0.0.1\n// ==/UserScript==");
        let strict = parse_meta("// ==UserScript==\n// @match *://*/*\n// @connect example.com\n// ==/UserScript==");
        let page = "https://www.site.com/a";

        // 一般的要求：方法、標頭、內容都帶到；回應分成 head、data、end
        let evs = xhr(&any, page, serde_json::json!({ "s": "s1", "rid": 7, "url": format!("{base}/echo?x=1"), "method": "POST",
            "headers": { "Content-Type": "application/json", "Referer": "https://ref.test/", "X-Api-Key": "k" }, "body": base64::engine::general_purpose::STANDARD.encode("{\"q\":\"中文\"}"),
            "ua": "UA-Test/1.0" }), Some("sid=secret"));
        assert_eq!(evs.first().map(|e| (e["t"].as_str(), e["rid"].as_u64(), e["status"].as_u64(), e["mb"].as_str() == Some(down()))), Some((Some("head"), Some(7), Some(200), true)), "{evs:?}");
        assert_eq!(evs.last().map(|e| e["t"].as_str()), Some(Some("end")));
        assert!(evs[0]["headers"].as_str().unwrap().to_ascii_lowercase().contains("x-test: yes"));
        let got: serde_json::Value = serde_json::from_str(&body_of(&evs)).unwrap();
        let hs: Vec<String> = got["headers"].as_array().unwrap().iter().map(|h| h.as_str().unwrap().to_ascii_lowercase()).collect();
        assert_eq!((got["method"].as_str(), got["path"].as_str(), got["body"].as_str()), (Some("POST"), Some("/echo?x=1"), Some("{\"q\":\"中文\"}")));
        assert!(hs.contains(&"referer: https://ref.test/".to_string()) && hs.contains(&"x-api-key: k".to_string()) && hs.contains(&"user-agent: ua-test/1.0".to_string()), "{hs:?}");
        // 只靠 @connect * 連的網站：不帶這個瀏覽器的 Cookie
        assert!(!hs.iter().any(|h| h.starts_with("cookie:")), "{hs:?}");

        // @connect 明確寫到的網域：帶 Cookie；腳本說不要帶（anonymous）就不帶
        let evs = xhr(&listed, page, serde_json::json!({ "s": "s1", "rid": 1, "url": format!("{base}/echo") }), Some("sid=secret"));
        assert!(body_of(&evs).to_ascii_lowercase().contains("cookie: sid=secret"), "{}", body_of(&evs));
        let evs = xhr(&listed, page, serde_json::json!({ "s": "s1", "rid": 1, "url": format!("{base}/echo"), "anon": true }), Some("sid=secret"));
        assert!(!body_of(&evs).to_ascii_lowercase().contains("cookie:"));

        // 沒有允許的網域：不連
        let evs = xhr(&strict, page, serde_json::json!({ "s": "s1", "rid": 2, "url": format!("{base}/echo") }), None);
        assert_eq!(evs.len(), 1);
        assert_eq!((evs[0]["t"].as_str(), evs[0]["kind"].as_str()), (Some("err"), Some("error")));

        // 轉址：自己跟過去，最後的網址在 head 裡
        let evs = xhr(&any, page, serde_json::json!({ "s": "s1", "rid": 3, "url": format!("{base}/redir") }), None);
        assert_eq!(evs[0]["status"].as_u64(), Some(200));
        assert!(evs[0]["url"].as_str().unwrap().ends_with("/echo?after=1"), "{evs:?}");
        // 轉到沒有允許的網域：停下來（@connect 只寫了 127.0.0.1，轉去 localhost 不行）
        let evs = xhr(&listed, page, serde_json::json!({ "s": "s1", "rid": 4, "url": format!("{base}/away") }), None);
        assert_eq!(evs.last().map(|e| (e["t"].as_str(), e["kind"].as_str())), Some((Some("err"), Some("error"))), "{evs:?}");
        // redirect: manual → 把轉址的回應原樣交回去
        let evs = xhr(&any, page, serde_json::json!({ "s": "s1", "rid": 5, "url": format!("{base}/redir"), "redirect": "manual" }), None);
        assert_eq!(evs[0]["status"].as_u64(), Some(302));

        // 404 也是正常回應（腳本自己看狀態）
        let evs = xhr(&any, page, serde_json::json!({ "s": "s1", "rid": 6, "url": format!("{base}/404") }), None);
        assert_eq!((evs[0]["status"].as_u64(), body_of(&evs).as_str(), evs.last().unwrap()["t"].as_str()), (Some(404), "nope", Some("end")));

        // 逾時
        let evs = xhr(&any, page, serde_json::json!({ "s": "s1", "rid": 8, "url": format!("{base}/slow"), "timeout": 300.0 }), None);
        assert_eq!(evs.last().map(|e| (e["t"].as_str(), e["kind"].as_str())), Some((Some("err"), Some("timeout"))), "{evs:?}");

        // 串流：伺服器一段一段送，這邊也一段一段交回去（不是等全部收完）
        let evs = xhr(&any, page, serde_json::json!({ "s": "s1", "rid": 9, "url": format!("{base}/stream") }), None);
        assert_eq!(body_of(&evs), "data: 1\ndata: 2\ndata: 3\n");
        assert!(evs.iter().filter(|e| e["t"] == "data").count() >= 2, "{evs:?}");

        // 連不上
        let evs = xhr(&any, page, serde_json::json!({ "s": "s1", "rid": 10, "url": "http://127.0.0.1:1/" }), None);
        assert_eq!(evs.last().map(|e| (e["t"].as_str(), e["kind"].as_str())), Some((Some("err"), Some("error"))), "{evs:?}");
    }

    /// 安裝、設定值、開關、更新、移除（這幾件事都要用到腳本資料夾，放在同一個測試裡依序做）
    #[test]
    fn install_values_update() {
        let tmp = std::env::temp_dir().join(format!("mb-us-test-{}", rand::random::<u32>()));
        init(tmp.clone());
        let port = test_server();
        let src = format!("// ==UserScript==\n// @name Vals\n// @version 1.0\n// @match *://*/*\n// @grant GM_setValue\n// @downloadURL http://127.0.0.1:{port}/s.user.js\n// ==/UserScript==\n1;");
        assert!(write_source("console.log(1)").is_err());
        assert!(write_source("// ==UserScript==\n// @name NoMatch\n// ==/UserScript==").is_err());
        let file = write_source(&src).unwrap();
        drop_cache();
        let sc = list().iter().find(|s| s.file == file).cloned().unwrap();
        assert!(sc.active() && by_id(&sc.id).is_some() && any() && any_active());
        assert!(runs_on(&sc, "https://a.com/x", true) && runs_on(&sc, "https://a.com/x", false) && !runs_on(&sc, "about:blank", true));

        // 設定值：變了才算、存進檔案、帶進啟動的那一段
        let v0 = values_ver();
        assert!(value_set(&file, "a", Some("1")));
        assert!(!value_set(&file, "a", Some("1")));
        assert!(values_ver() > v0);
        flush(true);
        let saved: BTreeMap<String, String> = serde_json::from_str(&std::fs::read_to_string(values_path(&file)).unwrap()).unwrap();
        assert_eq!(saved.get("a").map(String::as_str), Some("1"));
        let (_, snap) = values_snapshot();
        assert!(snap.contains(&sc.id) && snap.contains("\"a\":\"1\""), "{snap}");
        assert!(boot_script(false).contains(".go(") && boot_script(true).contains(".baked("));
        assert_eq!(values_of(&[sc.id.clone()])[&sc.id], serde_json::json!({ "a": "1" }));
        assert!(values_bytes() > 0);
        // 之後變過的：只補那幾個（刪掉的是 null）；版本一樣就沒有東西要補；太舊的（記錄沒留）要整份重傳
        let v1 = values_ver();
        assert_eq!(changes_since(v1, &[sc.id.clone()]), Some(serde_json::json!({})));
        assert!(value_set(&file, "b", Some("\"x\"")));
        assert!(value_set(&file, "c", Some("3")));
        assert!(value_set(&file, "c", None));
        assert_eq!(changes_since(v1, &[sc.id.clone()]), Some(serde_json::json!({ sc.id.clone(): { "b": "\"x\"", "c": null } })));
        assert_eq!(changes_since(v1, &["sother".to_string()]), Some(serde_json::json!({})));
        assert_eq!(changes_since(v1 + 1, &[sc.id.clone()]), Some(serde_json::json!({ sc.id.clone(): { "c": null } })));
        assert_eq!(changes_since(0, &[sc.id.clone()]), None);
        assert_eq!(changes_since(values_ver() + 5, &[sc.id.clone()]), None);
        assert!(value_set(&file, "b", None));

        // 關掉就不執行，設定值也不帶
        let g0 = gen();
        set_off(&file, true);
        assert!(gen() > g0 && by_id(&sc.id).is_none() && !any_active() && any());
        assert_eq!(values_snapshot().1, "{}");
        set_off(&file, false);
        assert!(any_active() && def_scripts().len() == 1);

        // 更新：伺服器上是 2.0 → 換成新的；設定值還在；再檢查一次就沒有東西要更新
        let done = check_updates(true);
        assert_eq!(done, vec!["Vals 2.0".to_string()]);
        let now = list().iter().find(|s| s.file == file).cloned().unwrap();
        assert_eq!((now.meta.version.as_str(), now.code.trim_end().ends_with("2;")), ("2.0", true));
        assert_eq!(values_of(&[sc.id.clone()])[&sc.id], serde_json::json!({ "a": "1" }));
        assert!(check_updates(true).is_empty());

        // 從網址安裝：先看、再裝（同名的換掉）
        let url = format!("http://127.0.0.1:{port}/s.user.js?v=3");
        let p = preview(&url).unwrap();
        assert_eq!((p.meta.name.as_str(), p.exists, p.installed.as_str()), ("Vals", true, "2.0"));
        assert_eq!(install(&url).unwrap(), "Vals");
        assert!(preview(&format!("http://127.0.0.1:{port}/echo")).is_err());
        assert!(is_script_url(&Url::parse(&url).unwrap()) && !is_script_url(&Url::parse("https://a.com/x.js").unwrap()) && !is_script_url(&Url::parse("file:///c:/a.user.js").unwrap()));

        // 移除：腳本、設定值都刪掉
        assert!(value_set(&file, "a", None));
        remove(&file);
        assert!(list().is_empty() && !values_path(&file).exists());
        let _ = std::fs::remove_dir_all(tmp);
    }

    #[test]
    fn injected_scripts_shape() {
        let s = Script {
            file: "a.user.js".into(),
            id: id_of("a.user.js"),
            meta: parse_meta("// ==UserScript==\n// @name a\n// @match *://*/*\n// @grant GM_getValue\n// ==/UserScript==\nconsole.log(1)"),
            off: false,
            missing: vec![],
            code: "console.log(1) // 結尾是註解".into(),
            requires: vec!["var q = 1;".into()],
            resources: Default::default(),
        };
        let d = def_script(&s);
        assert!(d.contains("var q = 1;") && d.contains("console.log(1)") && d.contains("GM_xmlhttpRequest") && d.contains(token()) && d.contains(slot()));
        // 腳本結尾是註解也不會把後面吃掉
        assert!(d.contains("// 結尾是註解\n};"));
        let shim = shim_script();
        assert!(!shim.contains("__SLOT__") && !shim.contains("__TOKEN__") && !shim.contains("__DOWN__") && !shim.contains("__APP_HOST__"));
        assert!(reset_script().contains("g.reset("));
    }
}
