//! 使用者腳本（篡改猴／Tampermonkey 的 .user.js）：瀏覽器視窗自己執行，不用裝篡改猴。
//!
//! - 腳本放在 web-scripts/ 資料夾（匯入 .user.js 或篡改猴匯出的 zip）
//! - @match、@include、@exclude 決定在哪些網頁執行；@run-at document-start 的在網頁一開始就跑，其他在載入完成後跑
//! - @require、@resource 在匯入時先下載好，跟著腳本一起注入
//! - 提供常用的 GM_* 功能（GM_getValue／GM_setValue 存在該網站的 localStorage、GM_addStyle、GM_xmlhttpRequest 用 fetch…）
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

static DIR: OnceLock<PathBuf> = OnceLock::new();
static CACHE: Mutex<Option<Vec<Script>>> = Mutex::new(None);

pub fn init(dir: PathBuf) {
    let _ = DIR.set(dir.join("web-scripts"));
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

/// 腳本的 metadata
#[derive(Debug, Clone, Default, Serialize)]
pub struct Meta {
    pub name: String,
    pub version: String,
    pub description: String,
    pub matches: Vec<String>,
    pub includes: Vec<String>,
    pub excludes: Vec<String>,
    /// "document-start"、"document-end"、"document-idle"
    pub run_at: String,
    pub requires: Vec<String>,
    /// (名稱, 網址)
    pub resources: Vec<(String, String)>,
    pub grants: Vec<String>,
    pub noframes: bool,
}

/// 解析 ==UserScript== 區塊
pub fn parse_meta(src: &str) -> Meta {
    let mut m = Meta::default();
    let Some(start) = src.find("==UserScript==") else { return m };
    let end = src[start..].find("==/UserScript==").map(|e| start + e).unwrap_or(src.len());
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
            "match" => m.matches.push(val.to_string()),
            "include" => m.includes.push(val.to_string()),
            "exclude" => m.excludes.push(val.to_string()),
            "run-at" => m.run_at = val.to_string(),
            "require" => m.requires.push(val.to_string()),
            "resource" => {
                if let Some(i) = val.find(char::is_whitespace) {
                    m.resources.push((val[..i].to_string(), val[i..].trim().to_string()));
                }
            }
            "grant" => m.grants.push(val.to_string()),
            "noframes" => m.noframes = true,
            _ => {}
        }
    }
    if m.name.is_empty() {
        m.name = "未命名腳本".into();
    }
    m
}

/// Chrome 的 match pattern（*://*.gamer.com.tw/*）→ 正規表示式
pub fn match_to_regex(p: &str) -> Option<String> {
    let p = p.trim();
    if p == "<all_urls>" {
        return Some("^(https?|file|ftp)://.*$".into());
    }
    let (scheme, rest) = p.split_once("://")?;
    let (host, path) = match rest.find('/') {
        Some(i) => (&rest[..i], &rest[i..]),
        None => (rest, "/"),
    };
    let scheme_re = if scheme == "*" { "https?".to_string() } else { regex::escape(scheme) };
    let host_re = if host == "*" {
        "[^/]*".to_string()
    } else if let Some(h) = host.strip_prefix("*.") {
        format!("([^/]*\\.)?{}", regex::escape(h))
    } else {
        regex::escape(host)
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

fn any_match(res: &[String], url: &str) -> bool {
    res.iter().any(|r| regex::Regex::new(r).map(|re| re.is_match(url)).unwrap_or(false))
}

/// 腳本要不要在這個網址執行
pub fn applies(m: &Meta, url: &str) -> bool {
    let inc: Vec<String> = m.matches.iter().filter_map(|p| match_to_regex(p)).chain(m.includes.iter().filter_map(|p| include_to_regex(p))).collect();
    if inc.is_empty() || !any_match(&inc, url) {
        return false;
    }
    let exc: Vec<String> = m.excludes.iter().filter_map(|p| include_to_regex(p).or_else(|| match_to_regex(p))).collect();
    !any_match(&exc, url)
}

#[derive(Debug, Clone, Serialize)]
pub struct Script {
    /// 檔名（識別用）
    pub file: String,
    pub meta: Meta,
    pub off: bool,
    /// 匯入時沒下載到的 @require／@resource
    pub missing: Vec<String>,
    #[serde(skip)]
    pub code: String,
    #[serde(skip)]
    pub requires: Vec<String>,
    #[serde(skip)]
    pub resources: BTreeMap<String, String>,
}

fn dep_path(url: &str) -> PathBuf {
    dir().join("deps").join(format!("{}.txt", crate::sha256::hex(url.as_bytes())))
}

/// 下載 @require／@resource（已經有就不再下載）
fn fetch_dep(url: &str) -> Result<(), String> {
    let p = dep_path(url);
    if p.is_file() {
        return Ok(());
    }
    let body = ureq::AgentBuilder::new()
        .timeout(std::time::Duration::from_secs(60))
        .build()
        .get(url)
        .call()
        .map_err(|e| e.to_string())?
        .into_string()
        .map_err(|e| e.to_string())?;
    let _ = std::fs::create_dir_all(p.parent().unwrap_or(Path::new(".")));
    std::fs::write(&p, body).map_err(|e| e.to_string())
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
            match std::fs::read_to_string(dep_path(u)) {
                Ok(t) => requires.push(t),
                Err(_) => missing.push(u.clone()),
            }
        }
        let mut resources = BTreeMap::new();
        for (n, u) in &meta.resources {
            match std::fs::read_to_string(dep_path(u)) {
                Ok(t) => {
                    resources.insert(n.clone(), t);
                }
                Err(_) => missing.push(u.clone()),
            }
        }
        out.push(Script { off: st.off.contains(&name), file: name, meta, missing, code, requires, resources });
    }
    out.sort_by(|a, b| a.meta.name.to_lowercase().cmp(&b.meta.name.to_lowercase()));
    out
}

pub fn list() -> Vec<Script> {
    let mut c = CACHE.lock().unwrap_or_else(|e| e.into_inner());
    if c.is_none() {
        *c = Some(load_all());
    }
    c.clone().unwrap_or_default()
}

fn invalidate() {
    *CACHE.lock().unwrap_or_else(|e| e.into_inner()) = None;
}

/// 下載腳本需要的外部檔案（匯入時呼叫）
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

/// 匯入一個 .user.js（內容）；回傳檔名
pub fn add_source(src: &str) -> Result<String, String> {
    let meta = parse_meta(src);
    if meta.matches.is_empty() && meta.includes.is_empty() {
        return Err(format!("「{}」沒有 @match 或 @include，不知道要在哪些網頁執行", meta.name));
    }
    let _ = std::fs::create_dir_all(dir());
    let file = format!("{}.user.js", safe_name(&meta.name));
    std::fs::write(dir().join(&file), src).map_err(|e| e.to_string())?;
    fetch_deps(&meta);
    invalidate();
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
            use std::io::Read;
            if e.read_to_string(&mut body).is_err() {
                continue;
            }
            if name.ends_with(".user.js") {
                match add_source(&body) {
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
            invalidate();
        }
    } else {
        let src = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
        add_source(&src)?;
        n = 1;
    }
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
    let _ = std::fs::remove_file(dir().join(file));
    set_off(file, false);
}

pub fn open_dir() {
    let _ = std::fs::create_dir_all(dir());
    let _ = crate::winsys::open(&dir().to_string_lossy());
}

/// GM_* 功能（每個腳本各自一份；設定值存在該網站的 localStorage）
const GM_JS: &str = r#"
const __name = __NAME__;
const __res = __RES__;
const unsafeWindow = window;
const __k = (k) => "gm:" + __name + ":" + k;
const GM_getValue = (k, d) => { try { const v = localStorage.getItem(__k(k)); return v === null ? d : JSON.parse(v); } catch (_) { return d; } };
const GM_setValue = (k, v) => { try { localStorage.setItem(__k(k), JSON.stringify(v)); } catch (_) {} };
const GM_deleteValue = (k) => { try { localStorage.removeItem(__k(k)); } catch (_) {} };
const GM_listValues = () => { const out = []; try { for (let i = 0; i < localStorage.length; i++) { const key = localStorage.key(i); if (key && key.startsWith(__k(""))) out.push(key.slice(__k("").length)); } } catch (_) {} return out; };
const GM_addStyle = (css) => { const s = document.createElement("style"); s.textContent = css; (document.head || document.documentElement).appendChild(s); return s; };
const GM_addElement = (a, b, c) => { const parent = typeof a === "string" ? (document.head || document.documentElement) : a; const tag = typeof a === "string" ? a : b; const attrs = (typeof a === "string" ? b : c) || {}; const el = document.createElement(tag); for (const [k, v] of Object.entries(attrs)) { if (k === "textContent") el.textContent = v; else el.setAttribute(k, v); } parent.appendChild(el); return el; };
const GM_getResourceText = (n) => __res[n] || "";
const GM_getResourceURL = (n) => "data:text/plain;charset=utf-8," + encodeURIComponent(__res[n] || "");
const GM_openInTab = (u) => window.open(u, "_blank");
const GM_registerMenuCommand = () => 0;
const GM_unregisterMenuCommand = () => {};
const GM_notification = (a) => { try { const t = typeof a === "string" ? a : (a && (a.text || a.title)) || ""; if (t) console.log("[腳本通知] " + t); } catch (_) {} };
const GM_setClipboard = (t) => { try { navigator.clipboard.writeText(String(t)); } catch (_) {} };
const GM_log = (...a) => console.log(...a);
const GM_info = { script: { name: __name, version: __VERSION__ }, scriptHandler: "MAOBOO", version: "1.0" };
const GM_xmlhttpRequest = (d) => {
  const ctl = new AbortController();
  const h = new Headers(d.headers || {});
  fetch(d.url, { method: d.method || "GET", headers: h, body: d.data, credentials: d.anonymous ? "omit" : "include", signal: ctl.signal })
    .then(async (r) => {
      const rt = d.responseType || "";
      const body = rt === "json" ? await r.json().catch(() => null) : rt === "blob" ? await r.blob() : rt === "arraybuffer" ? await r.arrayBuffer() : await r.text();
      const hd = []; r.headers.forEach((v, k) => hd.push(k + ": " + v));
      const res = { status: r.status, statusText: r.statusText, readyState: 4, responseHeaders: hd.join("\r\n"), response: body, responseText: typeof body === "string" ? body : "", finalUrl: r.url };
      if (d.onload) d.onload(res);
    })
    .catch((e) => { if (d.onerror) d.onerror({ error: String(e), status: 0, readyState: 4 }); });
  return { abort: () => ctl.abort() };
};
const GM = { getValue: async (k, d) => GM_getValue(k, d), setValue: async (k, v) => GM_setValue(k, v), deleteValue: async (k) => GM_deleteValue(k), listValues: async () => GM_listValues(), addStyle: GM_addStyle, addElement: GM_addElement, getResourceText: async (n) => GM_getResourceText(n), getResourceUrl: async (n) => GM_getResourceURL(n), openInTab: GM_openInTab, registerMenuCommand: GM_registerMenuCommand, notification: GM_notification, setClipboard: GM_setClipboard, xmlHttpRequest: GM_xmlhttpRequest, info: GM_info, log: GM_log };
"#;

fn wrap(s: &Script) -> String {
    let res = serde_json::to_string(&s.resources).unwrap_or_else(|_| "{}".into());
    let gm = GM_JS
        .replace("__NAME__", &serde_json::to_string(&s.meta.name).unwrap_or_default())
        .replace("__VERSION__", &serde_json::to_string(&s.meta.version).unwrap_or_default())
        .replace("__RES__", &res);
    let reqs = s.requires.join("\n;\n");
    format!("(() => {{\n{gm}\n{reqs}\n;\ntry {{\n{}\n}} catch (e) {{ console.error(\"[使用者腳本] \" + __name, e); }}\n}})();\n", s.code)
}

/// 網頁一開始就要跑的腳本（document-start）：注入每個網頁，由 JS 自己比對網址
pub fn start_script() -> String {
    let all = list();
    let mut parts = Vec::new();
    for s in all.iter().filter(|s| !s.off && s.meta.run_at == "document-start" && s.missing.is_empty()) {
        let inc: Vec<String> = s.meta.matches.iter().filter_map(|p| match_to_regex(p)).chain(s.meta.includes.iter().filter_map(|p| include_to_regex(p))).collect();
        let exc: Vec<String> = s.meta.excludes.iter().filter_map(|p| include_to_regex(p).or_else(|| match_to_regex(p))).collect();
        let body = wrap(s);
        parts.push(format!(
            "if (__us_ok({inc}, {exc}, {frames})) {{ {body} }}",
            inc = serde_json::to_string(&inc).unwrap_or_default(),
            exc = serde_json::to_string(&exc).unwrap_or_default(),
            frames = !s.meta.noframes,
            body = body
        ));
    }
    if parts.is_empty() {
        return String::new();
    }
    format!(
        "(() => {{ const u = location.href; const top = window.top === window; const __us_ok = (inc, exc, frames) => {{ if (!top && !frames) return false; const m = (l) => l.some((r) => {{ try {{ return new RegExp(r).test(u); }} catch (_) {{ return false; }} }}); return m(inc) && !m(exc); }};\n{}\n}})();",
        parts.join("\n")
    )
}

/// 網頁載入完成後要跑的腳本（document-end／idle，或沒寫 @run-at）
pub fn scripts_for(url: &str) -> Vec<String> {
    list()
        .iter()
        .filter(|s| !s.off && s.meta.run_at != "document-start" && s.missing.is_empty() && applies(&s.meta, url))
        .map(wrap)
        .collect()
}

#[derive(Serialize)]
pub struct ScriptsOut {
    pub list: Vec<Script>,
    pub dir: String,
}

#[tauri::command]
pub fn web_scripts_list() -> ScriptsOut {
    ScriptsOut { list: list(), dir: dir().to_string_lossy().into_owned() }
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
    Ok(msg)
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

/// 重新讀取資料夾（使用者自己放檔案進去後）
#[tauri::command]
pub fn web_scripts_reload() -> ScriptsOut {
    invalidate();
    for s in list() {
        if !s.missing.is_empty() {
            fetch_deps(&s.meta);
        }
    }
    invalidate();
    web_scripts_list()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn meta_and_match() {
        let src = "// ==UserScript==\n// @name  測試\n// @match *://*.gamer.com.tw/*\n// @match https://ani.gamer.com.tw/animeVideo.php*\n// @exclude https://forum.gamer.com.tw/*\n// @run-at document-start\n// @require https://x/y.js\n// @resource css https://x/a.css\n// @grant GM_setValue\n// ==/UserScript==\nconsole.log(1)";
        let m = parse_meta(src);
        assert_eq!(m.name, "測試");
        assert_eq!(m.run_at, "document-start");
        assert_eq!(m.requires, vec!["https://x/y.js"]);
        assert_eq!(m.resources, vec![("css".to_string(), "https://x/a.css".to_string())]);
        assert!(applies(&m, "https://ani.gamer.com.tw/animeVideo.php?sn=1"));
        assert!(applies(&m, "http://home.gamer.com.tw/"));
        assert!(applies(&m, "https://gamer.com.tw/x"));
        assert!(!applies(&m, "https://forum.gamer.com.tw/C.php"));
        assert!(!applies(&m, "https://www.youtube.com/"));
    }

    #[test]
    fn include_patterns() {
        let m = parse_meta("// ==UserScript==\n// @include /^https?:\\/\\/m\\..*/\n// @include *://x.com/*\n// ==/UserScript==");
        assert!(applies(&m, "https://m.youtube.com/watch"));
        assert!(applies(&m, "https://x.com/home"));
        assert!(!applies(&m, "https://www.youtube.com/"));
        let all = parse_meta("// ==UserScript==\n// @match *://*/*\n// ==/UserScript==");
        assert!(applies(&all, "https://any.site/a/b?c"));
        assert!(!applies(&all, "chrome-extension://abc/popup.html"));
    }

    #[test]
    fn wrapped_code_is_valid_shape() {
        let s = Script { file: "a.user.js".into(), meta: parse_meta("// ==UserScript==\n// @name a\n// @match *://*/*\n// ==/UserScript==\nconsole.log(1)"), off: false, missing: vec![], code: "console.log(1)".into(), requires: vec!["var q = 1;".into()], resources: Default::default() };
        let w = wrap(&s);
        assert!(w.contains("var q = 1;") && w.contains("console.log(1)") && w.contains("GM_setValue"));
    }
}
