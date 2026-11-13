//! 瀏覽器自己的頁面（新分頁、歷史記錄、下載、書籤、擴充功能、設定）用的指令。
//!
//! 這些頁面是 web-ui.html，開在分頁裡，網址列顯示成 chrome://…。
//! 外部網頁本來就不能呼叫程式的任何指令（Tauri 會拒絕）；這裡再確認一次呼叫的分頁目前顯示的是程式自己的頁面。

use super::*;

/// 呼叫的人是工具列，或正在顯示內部頁面的分頁。回傳 (視窗編號, 分頁編號)
fn guard(webview: &Webview) -> Result<(String, Option<String>), String> {
    if let Some(id) = id_of(webview) {
        return Ok((id, None));
    }
    let (wid, tid) = page_of(webview).ok_or("不是瀏覽器視窗")?;
    let ok = webview.url().ok().is_some_and(|u| webcore::local_to_virtual(&u).is_some());
    if !ok {
        return Err("這個頁面不能使用".into());
    }
    Ok((wid, Some(tid)))
}

/* ---------------- 新分頁 ---------------- */

#[derive(Serialize)]
pub struct Shortcut {
    url: String,
    title: String,
    icon: String,
}

#[derive(Serialize)]
pub struct NewTab {
    engine: &'static str,
    incognito: bool,
    shortcuts: Vec<Shortcut>,
}

#[tauri::command]
pub async fn web_ui_newtab(webview: Webview) -> Result<NewTab, String> {
    let (wid, _) = guard(&webview)?;
    let incognito = is_incognito(&wid);
    let shortcuts = if incognito {
        Vec::new()
    } else {
        hist_read(|h| {
            h.top_sites(10)
                .into_iter()
                .map(|(url, title, _)| Shortcut { icon: h.icon_for(&url), title: if title.trim().is_empty() { webcore::display_url(&url) } else { title }, url })
                .collect()
        })
        .unwrap_or_default()
    };
    Ok(NewTab { engine: engine().name, incognito, shortcuts })
}

/// 內部頁面裡點連結：在這個分頁開，或開新分頁
#[tauri::command]
pub async fn web_ui_open(app: AppHandle, webview: Webview, url: String, new_tab: bool) -> Result<(), String> {
    let (wid, tid) = guard(&webview)?;
    // 擴充功能自己的頁面（選項）：要是清單裡真的有的才開
    let target = if url.starts_with("chrome-extension://") {
        if !ext_list().iter().any(|e| e.options == url || e.popup == url) {
            return Ok(());
        }
        url
    } else {
        match webcore::resolve_input(&url, engine()) {
            Some((t, _)) => t,
            None => return Ok(()),
        }
    };
    let tid = tid.or_else(|| get(&wid).map(|w| w.active)).ok_or("找不到分頁")?;
    if new_tab {
        add_tab(&app, &wid, target, false, Some(&tid))?;
    } else {
        navigate_tab(&app, &wid, &tid, &target);
    }
    Ok(())
}

/* ---------------- 歷史記錄 ---------------- */

#[derive(Serialize)]
pub struct HistRow {
    u: String,
    t: String,
    at: u64,
    icon: String,
    host: String,
}

/// 新 → 舊。q = 搜尋；before = 只要比這個時間舊的（往下捲時接著載入）
#[tauri::command]
pub async fn web_ui_history(webview: Webview, q: String, before: Option<u64>, limit: Option<usize>) -> Result<Vec<HistRow>, String> {
    guard(&webview)?;
    let limit = limit.unwrap_or(150).clamp(1, 500);
    Ok(hist_read(|h| {
        h.search(&q, before, limit)
            .into_iter()
            .map(|v| HistRow { icon: h.icon_for(&v.u), host: webcore::host_of(&v.u), u: v.u, t: v.t, at: v.at })
            .collect()
    })
    .unwrap_or_default())
}

/// 刪掉選取的記錄（網址 + 時間）
#[tauri::command]
pub async fn web_ui_history_delete(webview: Webview, items: Vec<(String, u64)>) -> Result<usize, String> {
    guard(&webview)?;
    Ok(hist_edit(|h| h.remove(&items)).unwrap_or(0))
}

/* ---------------- 下載 ---------------- */

#[derive(Serialize)]
pub struct DlRow {
    id: u64,
    url: String,
    path: String,
    name: String,
    total: i64,
    received: i64,
    state: String,
    at: u64,
    /// 檔案已經被刪除或移走
    gone: bool,
}

#[tauri::command]
pub async fn web_ui_downloads(webview: Webview, q: String) -> Result<Vec<DlRow>, String> {
    guard(&webview)?;
    let list = side_edit(|s| s.dl.search(&q)).unwrap_or_default();
    Ok(list
        .into_iter()
        .map(|d| DlRow {
            gone: d.state == "done" && !d.path.is_empty() && !Path::new(&d.path).exists(),
            name: webcore::file_name(&d.path).to_string(),
            id: d.id,
            url: d.url,
            path: d.path,
            total: d.total,
            received: d.received,
            state: d.state,
            at: d.at,
        })
        .collect())
}

/// 在檔案總管裡顯示這個檔案
fn show_in_folder(path: &str) {
    #[cfg(windows)]
    {
        let _ = std::process::Command::new("explorer.exe").arg(format!("/select,{path}")).spawn();
    }
    #[cfg(not(windows))]
    let _ = path;
}

/// 下載頁的操作：open（開啟檔案）、show（在資料夾中顯示）、remove（從清單移除）、cancel、pause、resume、retry、clear（清除全部）
#[tauri::command]
pub async fn web_ui_download(app: AppHandle, webview: Webview, id: u64, act: String) -> Result<(), String> {
    let (wid, tid) = guard(&webview)?;
    let found = side_edit(|s| {
        let d = s.dl.list.iter().find(|d| d.id == id).cloned();
        let key = s.dl_keys.iter().find(|(_, v)| **v == id).map(|(k, _)| *k);
        (d, key)
    });
    let (d, key) = found.unwrap_or((None, None));
    match act.as_str() {
        "clear" => {
            dl_edit(|d| d.clear_since(0));
        }
        "open" | "show" => {
            let d = d.ok_or("找不到這筆下載")?;
            if d.state != "done" || !Path::new(&d.path).exists() {
                return Err("檔案已經不在了".into());
            }
            if act == "show" {
                show_in_folder(&d.path);
            } else {
                crate::winsys::open(&d.path)?;
            }
        }
        "remove" => {
            dl_edit(|d| d.remove(id));
        }
        "cancel" => {
            if let Some(key) = key {
                let _ = app.run_on_main_thread(move || webnative::dl_cancel(key));
            }
        }
        "pause" | "resume" => {
            if let Some(key) = key {
                let pause = act == "pause";
                let _ = app.run_on_main_thread(move || webnative::dl_pause(key, pause));
            }
        }
        "retry" => {
            let d = d.ok_or("找不到這筆下載")?;
            if parse_web(&d.url).is_some() {
                let after = tid.or_else(|| get(&wid).map(|w| w.active));
                add_tab(&app, &wid, d.url, false, after.as_deref())?;
            }
        }
        _ => {}
    }
    emit_state_all(&app);
    Ok(())
}

/* ---------------- 書籤管理員 ---------------- */

#[tauri::command]
pub async fn web_ui_bookmarks(webview: Webview) -> Result<BmPayload, String> {
    guard(&webview)?;
    Ok(bm_payload())
}

/// 改書籤的名稱與網址（資料夾只有名稱）
#[tauri::command]
pub fn web_bm_edit(app: AppHandle, webview: Webview, id: String, title: String, url: Option<String>) -> Result<(), String> {
    guard(&webview)?;
    let title = title.trim().to_string();
    let url = match url.as_deref().map(str::trim).filter(|u| !u.is_empty()) {
        Some(u) => Some(webcore::resolve_input(u, engine()).filter(|(_, search)| !search).map(|(u, _)| u).filter(|u| parse_web(u).is_some()).ok_or("網址不正確")?),
        None => None,
    };
    bm(|b| {
        if let Some(x) = bm_find_mut(&mut b.bar, &id).or_else(|| bm_find_mut(&mut b.other, &id)) {
            if !title.is_empty() && x.title != title {
                x.title = title;
                x.renamed = true;
            }
            if let (Some(u), false) = (url, x.url.is_empty()) {
                if x.url != u {
                    x.url = u;
                    // 改過網址就不再跟 Chrome 裡的那個書籤同步
                    x.src.clear();
                }
            }
        }
    });
    emit_bm(&app);
    Ok(())
}

/// 新增資料夾。parent：bar（書籤列）、other（所有書籤）或資料夾的編號
#[tauri::command]
pub fn web_bm_add_folder(app: AppHandle, webview: Webview, parent: String, title: String) -> Result<String, String> {
    guard(&webview)?;
    let title = title.trim().to_string();
    if title.is_empty() {
        return Err("請輸入資料夾名稱".into());
    }
    let id = bm(|b| {
        b.next_id += 1;
        let id = format!("b{}", b.next_id);
        let f = Bm { id: id.clone(), title, ..Bm::default() };
        match parent.as_str() {
            "bar" => b.bar.push(f),
            "other" => b.other.push(f),
            p => match bm_find_mut(&mut b.bar, p).or_else(|| bm_find_mut(&mut b.other, p)).filter(|x| x.url.is_empty()) {
                Some(x) => x.children.push(f),
                None => b.bar.push(f),
            },
        }
        id
    });
    emit_bm(&app);
    Ok(id)
}

/// 新增書籤。parent：bar、other 或資料夾的編號
#[tauri::command]
pub fn web_bm_add(app: AppHandle, webview: Webview, parent: String, title: String, url: String) -> Result<String, String> {
    guard(&webview)?;
    let url = webcore::resolve_input(&url, engine()).filter(|(_, search)| !search).map(|(u, _)| u).filter(|u| parse_web(u).is_some()).ok_or("網址不正確")?;
    let title = if title.trim().is_empty() { webcore::display_url(&url) } else { title.trim().to_string() };
    let id = bm(|b| {
        b.ignored.retain(|i| !same_url(i, &url));
        b.next_id += 1;
        let id = format!("b{}", b.next_id);
        let x = Bm { id: id.clone(), title, url, ..Bm::default() };
        match parent.as_str() {
            "bar" => b.bar.push(x),
            "other" => b.other.push(x),
            p => match bm_find_mut(&mut b.bar, p).or_else(|| bm_find_mut(&mut b.other, p)).filter(|f| f.url.is_empty()) {
                Some(f) => f.children.push(x),
                None => b.bar.push(x),
            },
        }
        id
    });
    emit_bm(&app);
    Ok(id)
}

fn contains(b: &Bm, id: &str) -> bool {
    b.id == id || b.children.iter().any(|c| contains(c, id))
}

/// 把書籤（或資料夾）搬到別的地方。parent：bar、other 或資料夾的編號；index = 放在第幾個（沒給就放最後）
#[tauri::command]
pub fn web_bm_place(app: AppHandle, webview: Webview, id: String, parent: String, index: Option<usize>) -> Result<(), String> {
    guard(&webview)?;
    let r = bm(|b| {
        // 不能搬進自己裡面
        if let Some(x) = bm_find_any(b, &id) {
            if contains(x, &parent) {
                return Err("不能把資料夾搬進自己裡面".to_string());
            }
        }
        let Some(x) = bm_remove(&mut b.bar, &id).or_else(|| bm_remove(&mut b.other, &id)) else { return Err("找不到這個書籤".to_string()) };
        let list = match parent.as_str() {
            "bar" => &mut b.bar,
            "other" => &mut b.other,
            p => {
                let in_bar = bm_find(&b.bar, p).is_some_and(|f| f.url.is_empty());
                let f = if in_bar { bm_find_mut(&mut b.bar, p) } else { bm_find_mut(&mut b.other, p).filter(|f| f.url.is_empty()) };
                match f {
                    Some(f) => &mut f.children,
                    None => &mut b.bar,
                }
            }
        };
        let at = index.unwrap_or(list.len()).min(list.len());
        list.insert(at, x);
        Ok(())
    });
    emit_bm(&app);
    r
}

/* ---------------- 設定 ---------------- */

#[derive(Serialize)]
pub struct EngineOut {
    id: &'static str,
    name: &'static str,
}

#[derive(Serialize)]
pub struct SettingsOut {
    settings: webcore::Settings,
    engines: Vec<EngineOut>,
    zooms: Vec<u32>,
    chrome: ChromeStatus,
    /// 程式與瀏覽器核心的版本
    app: String,
    engine_version: String,
}

/// 瀏覽器核心（WebView2）的版本：開第一個網頁時記下來
static CORE_VERSION: Mutex<String> = Mutex::new(String::new());

#[tauri::command]
pub async fn web_ui_settings(app: AppHandle, webview: Webview) -> Result<SettingsOut, String> {
    guard(&webview)?;
    if lock(&CORE_VERSION).is_empty() {
        let (tx, rx) = std::sync::mpsc::channel::<String>();
        with_wv(&webview, move |w| {
            let _ = tx.send(webnative::version(w));
        });
        if let Ok(v) = rx.recv_timeout(Duration::from_millis(800)) {
            *lock(&CORE_VERSION) = v;
        }
    }
    Ok(SettingsOut {
        settings: settings(),
        engines: webcore::ENGINES.iter().map(|e| EngineOut { id: e.id, name: e.name }).collect(),
        zooms: webcore::ZOOMS.iter().map(|z| webcore::zoom_pct(*z)).collect(),
        chrome: web_chrome_status(),
        app: app.package_info().version.to_string(),
        engine_version: lock(&CORE_VERSION).clone(),
    })
}

/// 套用會影響每個網頁的設定（密碼、下載位置）
fn apply_to_pages(app: &AppHandle) {
    let set = settings();
    let open: Vec<WebWin> = st().file.wins.iter().filter(|w| w.open).cloned().collect();
    for w in open {
        for t in &w.tabs {
            if let Some(p) = app.get_webview(&page_label(&w.id, &t.id)) {
                let (save, dir) = (set.save_passwords && !w.incognito, set.download_dir.clone());
                with_wv(&p, move |wv| {
                    webnative::set_password_save(wv, save);
                    webnative::set_download_dir(wv, &dir);
                });
            }
        }
    }
}

/// 改一項設定
#[tauri::command]
pub async fn web_ui_set(app: AppHandle, webview: Webview, key: String, value: Value) -> Result<webcore::Settings, String> {
    guard(&webview)?;
    let flag = || value.as_bool().ok_or("設定值不正確");
    {
        let mut s = st();
        let set = &mut s.file.settings;
        match key.as_str() {
            "engine" => {
                let id = value.as_str().unwrap_or("");
                if !webcore::ENGINES.iter().any(|e| e.id == id) {
                    return Err("沒有這個搜尋引擎".into());
                }
                set.engine = id.to_string();
            }
            "suggest" => set.suggest = flag()?,
            "home_button" => set.home_button = flag()?,
            "home_url" => {
                let u = value.as_str().unwrap_or("").trim().to_string();
                if !u.is_empty() && !webcore::resolve_input(&u, webcore::engine(&set.engine)).is_some_and(|(_, search)| !search) {
                    return Err("網址不正確".into());
                }
                set.home_url = u;
            }
            "show_bm" => set.show_bm = flag()?,
            "save_passwords" => set.save_passwords = flag()?,
            "default_zoom" => {
                let z = value.as_f64().unwrap_or(1.0);
                if !(0.25..=5.0).contains(&z) {
                    return Err("縮放不正確".into());
                }
                set.default_zoom = z;
            }
            "download_dir" => {
                let d = value.as_str().unwrap_or("").trim().to_string();
                if !d.is_empty() && !Path::new(&d).is_dir() {
                    return Err("找不到這個資料夾".into());
                }
                set.download_dir = d;
            }
            _ => return Err("沒有這項設定".into()),
        }
        s.dirty = true;
    }
    match key.as_str() {
        "show_bm" => {
            for id in open_ids() {
                relayout(&app, &id);
            }
        }
        "save_passwords" | "download_dir" => apply_to_pages(&app),
        _ => {}
    }
    emit_state_all(&app);
    Ok(settings())
}

/// 選下載要存的資料夾
#[tauri::command]
pub async fn web_ui_pick_dir(webview: Webview) -> Result<Option<String>, String> {
    guard(&webview)?;
    let dir = tauri::async_runtime::spawn_blocking(|| rfd::FileDialog::new().set_title("選擇下載要存的資料夾").pick_folder()).await.map_err(|e| e.to_string())?;
    Ok(dir.map(|d| d.to_string_lossy().into_owned()))
}

/// 刪除瀏覽資料。range：hour / day / week / month / all；
/// kinds：history（瀏覽記錄）、downloads（下載記錄）、cookies（Cookie 與網站資料）、cache（快取）、passwords、autofill
#[tauri::command]
pub async fn web_ui_clear(app: AppHandle, webview: Webview, range: String, kinds: Vec<String>) -> Result<(), String> {
    guard(&webview)?;
    let now = webcore::now_ms();
    let span: Option<u64> = match range.as_str() {
        "hour" => Some(3_600_000),
        "day" => Some(86_400_000),
        "week" => Some(7 * 86_400_000),
        "month" => Some(28 * 86_400_000),
        "all" => None,
        _ => return Err("時間範圍不正確".into()),
    };
    let since = span.map(|s| now.saturating_sub(s));
    let has = |k: &str| kinds.iter().any(|x| x == k);
    let mut native = 0i32;
    if has("history") {
        hist_edit(|h| h.clear_since(since.unwrap_or(0)));
        {
            let mut s = st();
            s.file.closed.retain(|c| since.is_some_and(|t| c.at < t));
            s.dirty = true;
        }
        native |= webev::data_kinds::HISTORY;
    }
    if has("downloads") {
        dl_edit(|d| d.clear_since(since.unwrap_or(0)));
        native |= webev::data_kinds::DOWNLOADS;
    }
    if has("cookies") {
        native |= webev::data_kinds::SITE;
    }
    if has("cache") {
        native |= webev::data_kinds::CACHE;
    }
    if has("passwords") {
        native |= webev::data_kinds::PASSWORDS;
    }
    if has("autofill") {
        native |= webev::data_kinds::AUTOFILL;
    }
    flush_side(true);
    emit_state_all(&app);
    if native == 0 {
        return Ok(());
    }
    // 瀏覽器本身存的資料（整個設定檔共用，隨便找一個網頁來清）
    let (tx, rx) = std::sync::mpsc::channel::<bool>();
    with_wv(&webview, move |w| {
        webnative::clear_data(
            w,
            native,
            since,
            Box::new(move |ok| {
                let _ = tx.send(ok);
            }),
        );
    });
    match rx.recv_timeout(Duration::from_secs(20)) {
        Ok(true) => Ok(()),
        Ok(false) => Err("這台電腦的瀏覽器元件（WebView2）版本太舊，沒辦法清除這些資料".into()),
        // 沒等到結果：通常是還在清（資料很多），不當成失敗
        Err(_) => Ok(()),
    }
}
