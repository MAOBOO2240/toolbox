//! 使用者腳本跟瀏覽器視窗接起來的部分：把腳本登記到每個分頁、處理網頁裡的腳本傳來的訊息
//! （設定值、選單、連線、開分頁…）、工具列「使用者腳本」選單要顯示的內容。
//! 腳本本身的邏輯（解析、產生要注入的程式、連線）在 userscripts.rs。

use super::*;
use crate::userscripts::{self as us, Msg};

/// 一份文件（最上層的網頁或內嵌框架）裡正在跑的腳本
struct Doc {
    ids: Vec<String>,
}

/// 腳本登記的選單項目（GM_registerMenuCommand）
struct MenuCmd {
    sid: String,
    id: String,
    caption: String,
    title: String,
    close: bool,
}

#[derive(Default)]
struct UsTab {
    /// 建立分頁時登記的腳本清單是第幾版（0 = 沒登記）
    built: u64,
    /// 之後另外登記（清單變過）的是第幾版（0 = 沒有）
    listed: u64,
    /// 另外登記的「帶設定值啟動」那一段，帶的設定值是第幾版（0 = 沒有）
    boot: u64,
    /// 框架編號（0 = 最上層）→ 那份文件
    docs: HashMap<u32, Doc>,
    menus: Vec<MenuCmd>,
}

type TabKey = (String, String);
static TABS: Mutex<Option<HashMap<TabKey, UsTab>>> = Mutex::new(None);
/// 進行中的連線：(視窗, 分頁, 框架, 連線編號) → 要它停下來的旗標
static XHRS: Mutex<Option<HashMap<(String, String, u32, u64), Arc<AtomicBool>>>> = Mutex::new(None);
static SYNC: OnceLock<Mutex<std::sync::mpsc::Sender<()>>> = OnceLock::new();

fn tabs<T>(f: impl FnOnce(&mut HashMap<TabKey, UsTab>) -> T) -> T {
    f(lock(&TABS).get_or_insert_with(HashMap::new))
}

fn xhrs<T>(f: impl FnOnce(&mut HashMap<(String, String, u32, u64), Arc<AtomicBool>>) -> T) -> T {
    f(lock(&XHRS).get_or_insert_with(HashMap::new))
}

/// 啟動：腳本清單或設定值變了，由一個背景執行緒把開著的分頁換成新的（連續變動合成一次）
pub(super) fn init(app: &AppHandle) {
    let (tx, rx) = std::sync::mpsc::channel::<()>();
    let _ = SYNC.set(Mutex::new(tx));
    let a = app.clone();
    std::thread::spawn(move || {
        let mut seen = 0;
        // 上次把各分頁預先帶著的設定值換新是什麼時候；later = 還有一次換新延後了
        let mut last_vals: Option<Instant> = None;
        let mut later = false;
        loop {
            // 有延後的就定時醒來看；沒有就等到有人通知
            let woke = if later { rx.recv_timeout(Duration::from_secs(2)).is_ok() } else { rx.recv().is_ok() || return };
            if woke {
                std::thread::sleep(Duration::from_millis(250));
                while rx.try_recv().is_ok() {}
            }
            // 設定值很多時（腳本拿它當快取），每變一次就把整包重新登記到每個分頁太浪費：資料越多隔越久才換。
            // 這段空檔載入的網頁會拿到比較舊的一份，啟動時再補上之後變過的（見 on_msg 的 Hi）
            let gap = Duration::from_secs((us::values_bytes() / (64 * 1024)).min(60) as u64);
            let vals_due = last_vals.is_none_or(|t| t.elapsed() >= gap);
            sync(&a, vals_due);
            if vals_due {
                last_vals = Some(Instant::now());
            }
            later = !vals_due;
            us::flush(false);
            // 腳本清單變了：工具列的「使用者腳本」按鈕可能要出現或消失
            let g = us::gen();
            if g != seen {
                seen = g;
                emit_state_all(&a);
            }
        }
    });
    us::on_change(request_sync);
    us::start_background();
}

pub(super) fn request_sync() {
    if let Some(tx) = SYNC.get() {
        let _ = lock(tx).send(());
    }
}

/// 建立分頁的網頁時：把執行環境、每支腳本、建立當下的設定值登記進去（第一份文件一定用得到）。
/// 無痕式視窗、檢視原始碼的分頁不跑腳本
pub(super) fn register(mut b: WebviewBuilder<tauri::Wry>, wid: &str, tid: &str, url: &Url, incognito: bool) -> WebviewBuilder<tauri::Wry> {
    let on = !incognito && url.scheme() != "view-source" && us::any_active();
    let gen = if on { us::gen() } else { 0 };
    tabs(|m| {
        m.insert((wid.to_string(), tid.to_string()), UsTab { built: gen, ..UsTab::default() });
    });
    if on {
        b = b.initialization_script_for_all_frames(us::shim_script());
        for d in us::def_scripts().iter() {
            b = b.initialization_script_for_all_frames(d.clone());
        }
        b = b.initialization_script_for_all_frames(us::boot_script(true));
    }
    b
}

/// 分頁的網頁建立好了：再登記「帶最新設定值啟動」的那一段（之後設定值變了會換新的）
pub(super) fn booted(wv: &Webview, wid: &str, tid: &str) {
    let ver = us::values_ver();
    let on = tabs(|m| match m.get_mut(&(wid.to_string(), tid.to_string())) {
        Some(t) if t.built != 0 => {
            t.boot = ver;
            true
        }
        _ => false,
    });
    if on {
        let boot = us::boot_script(false);
        with_wv(wv, move |w| webnative::gm_set_slot(w, 1, vec![boot]));
    }
}

/// 開著的分頁換成最新的腳本清單、最新的設定值（之後載入的網頁才會用到）。
/// vals_due = 這次要不要連「只有設定值變了」的分頁也換（腳本清單變了的一定換）
fn sync(app: &AppHandle, vals_due: bool) {
    let (gen, ver, active) = (us::gen(), us::values_ver(), us::any_active());
    // 這幾段可能很大：有分頁需要才算
    let mut defs: Option<Arc<Vec<String>>> = None;
    let mut boot: Option<String> = None;
    for wid in open_ids() {
        if is_incognito(&wid) {
            continue;
        }
        let Some(w) = get(&wid) else { continue };
        for t in &w.tabs {
            let Some(page) = app.get_webview(&page_label(&wid, &t.id)) else { continue };
            let (need0, need1, built) = tabs(|m| {
                let e = m.entry((wid.clone(), t.id.clone())).or_default();
                let cur = if e.listed != 0 { e.listed } else { e.built };
                // 這個分頁登記過執行環境（之後就算腳本全關了，也要讓它知道清單空了）
                let had = e.built != 0 || e.listed != 0;
                let need0 = cur != gen && (active || had);
                let need1 = (need0 || (vals_due && e.boot != ver)) && (active || had);
                if need0 {
                    e.listed = gen;
                }
                if need1 {
                    e.boot = ver;
                }
                (need0, need1, e.built != 0)
            });
            if !need0 && !need1 {
                continue;
            }
            // 建立分頁時登記過的拿不掉：先宣告作廢，再交新的清單；沒登記過的要連執行環境一起給
            let list: Vec<String> = match (need0, built, active) {
                (false, _, _) | (true, false, false) => Vec::new(),
                (true, true, false) => vec![us::reset_script()],
                (true, built, true) => {
                    let mut v = vec![if built { us::reset_script() } else { us::shim_script() }];
                    v.extend(defs.get_or_insert_with(us::def_scripts).iter().cloned());
                    v
                }
            };
            let go: Vec<String> = if built || active { vec![boot.get_or_insert_with(|| us::boot_script(false)).clone()] } else { Vec::new() };
            with_wv(&page, move |w| {
                if need0 {
                    webnative::gm_set_slot(w, 0, list);
                }
                // 清單換了之後，啟動的那一段也要重新登記（要排在最後面）
                if need1 {
                    webnative::gm_set_slot(w, 1, go);
                }
            });
        }
    }
}

/// 分頁關掉了
pub(super) fn forget(app: &AppHandle, wid: &str, tid: &str) {
    tabs(|m| {
        m.remove(&(wid.to_string(), tid.to_string()));
    });
    abort_xhrs(wid, tid, None);
    if let Some(p) = app.get_webview(&page_label(wid, tid)) {
        with_wv(&p, |w| webnative::gm_forget(w));
    }
}

/// 停掉某個分頁（frame = 只停某一份文件）還在進行的連線
fn abort_xhrs(wid: &str, tid: &str, frame: Option<u32>) {
    xhrs(|m| {
        m.retain(|(w, t, f, _), flag| {
            let hit = w == wid && t == tid && frame.map_or(true, |x| x == *f);
            if hit {
                flag.store(true, Ordering::SeqCst);
            }
            !hit
        });
    });
}

/// 傳一則訊息給某個分頁裡的某份文件
fn post(app: &AppHandle, wid: &str, tid: &str, frame: u32, json: String) {
    if let Some(p) = app.get_webview(&page_label(wid, tid)) {
        with_wv(&p, move |w| {
            webnative::gm_post(w, frame, &json);
        });
    }
}

/// 設定值變了：告訴其他也在跑這支腳本的文件（別的分頁、別的框架）
fn broadcast(app: &AppHandle, sid: &str, json: &str, except: (&str, &str, u32)) {
    let targets: Vec<(String, String, u32)> = tabs(|m| {
        let mut out = Vec::new();
        for ((w, t), tab) in m.iter() {
            for (f, d) in &tab.docs {
                if d.ids.iter().any(|i| i == sid) && (w.as_str(), t.as_str(), *f) != except {
                    out.push((w.clone(), t.clone(), *f));
                }
            }
        }
        out
    });
    for (w, t, f) in targets {
        post(app, &w, &t, f, json.to_string());
    }
}

/// 10 秒內最多 max 次（腳本發通知、開分頁不要太頻繁）
fn rate_ok(recent: &Mutex<Vec<Instant>>, max: usize) -> bool {
    let mut r = lock(recent);
    r.retain(|t| t.elapsed() < Duration::from_secs(10));
    if r.len() >= max {
        return false;
    }
    r.push(Instant::now());
    true
}

fn notify_ok() -> bool {
    static RECENT: Mutex<Vec<Instant>> = Mutex::new(Vec::new());
    rate_ok(&RECENT, 4)
}

fn tab_ok() -> bool {
    static RECENT: Mutex<Vec<Instant>> = Mutex::new(Vec::new());
    rate_ok(&RECENT, 12)
}

/// 網頁裡的腳本傳來一則訊息。frame = 0 是最上層的網頁；source = 傳訊息的那份文件的網址（WebView2 給的，網頁假造不了）
pub(super) fn on_msg(app: &AppHandle, wid: &str, tid: &str, frame: u32, source: &str, raw: &str) {
    let Some(msg) = us::parse_msg(raw) else { return };
    if is_incognito(wid) {
        return;
    }
    let top = frame == 0;
    // 這份文件真的會跑這支腳本，才接受它用這支腳本的名義做事
    let script = |id: &str| us::by_id(id).filter(|s| us::runs_on(s, source, top));
    let key = (wid.to_string(), tid.to_string());
    match msg {
        Msg::Hi { ids, ver } => {
            let ids: Vec<String> = ids.into_iter().filter(|i| script(i).is_some()).collect();
            crate::dlog!("[us] {wid}-{tid} 框架 {frame} {source}：跑 {} 支腳本（設定值第 {ver} 版）", ids.len());
            // 這份文件是新的：之前那份文件留下的東西（選單、還在跑的連線）作廢
            abort_xhrs(wid, tid, if top { None } else { Some(frame) });
            tabs(|m| {
                let t = m.entry(key).or_default();
                if top {
                    t.docs.clear();
                    t.menus.clear();
                }
                if ids.is_empty() {
                    t.docs.remove(&frame);
                } else {
                    t.docs.insert(frame, Doc { ids: ids.clone() });
                }
            });
            // 它啟動時拿到的設定值比較舊（在兩次更新之間載入的）：補上之後變過的；差太多就整份對一次
            if !ids.is_empty() && ver != us::values_ver() {
                let msg = match us::changes_since(ver, &ids) {
                    Some(d) if d.as_object().is_some_and(|o| o.is_empty()) => None,
                    Some(d) => Some(serde_json::json!({ "ev": "delta", "vals": d })),
                    None => Some(serde_json::json!({ "ev": "sync", "vals": us::values_of(&ids) })),
                };
                if let Some(m) = msg {
                    post(app, wid, tid, frame, us::down_msg(m));
                }
            }
        }
        Msg::Set { s, k, v } => {
            let Some(sc) = script(&s) else { return };
            if us::value_set(&sc.file, &k, v.as_deref()) {
                let json = us::down_msg(serde_json::json!({ "ev": "val", "s": s, "k": k, "v": v }));
                broadcast(app, &s, &json, (wid, tid, frame));
                request_sync();
            }
        }
        Msg::Menu { s, id, caption, title, close } => {
            if !top || script(&s).is_none() {
                return;
            }
            tabs(|m| {
                let t = m.entry(key).or_default();
                let caption: String = caption.chars().take(200).collect();
                let title: String = title.chars().take(400).collect();
                match t.menus.iter().position(|c| c.sid == s && c.id == id) {
                    Some(i) => {
                        let c = &mut t.menus[i];
                        c.caption = caption;
                        c.title = title;
                        c.close = close;
                    }
                    None if t.menus.len() < 200 => t.menus.push(MenuCmd { sid: s, id, caption, title, close }),
                    None => {}
                }
            });
            pop::scripts_changed(app, wid, tid);
        }
        Msg::Unmenu { s, id } => {
            if !top {
                return;
            }
            tabs(|m| {
                if let Some(t) = m.get_mut(&key) {
                    t.menus.retain(|c| !(c.sid == s && c.id == id));
                }
            });
            pop::scripts_changed(app, wid, tid);
        }
        Msg::Xhr(req) => {
            let rid = req.rid;
            let Some(sc) = script(&req.s) else {
                post(app, wid, tid, frame, us::down_msg(serde_json::json!({ "ev": "xhr", "rid": rid, "t": "err", "kind": "error" })));
                return;
            };
            let flag = Arc::new(AtomicBool::new(false));
            let xkey = (wid.to_string(), tid.to_string(), frame, rid);
            xhrs(|m| {
                m.insert(xkey.clone(), flag.clone());
            });
            let (app, wid, tid, page) = (app.clone(), wid.to_string(), tid.to_string(), source.to_string());
            std::thread::spawn(move || {
                let label = page_label(&wid, &tid);
                let cookies = |u: &Url| -> Option<String> {
                    let p = app.get_webview(&label)?;
                    let (tx, rx) = std::sync::mpsc::channel::<String>();
                    let url = u.to_string();
                    with_wv(&p, move |w| {
                        webnative::cookies(
                            w,
                            &url,
                            Box::new(move |c| {
                                let _ = tx.send(c);
                            }),
                        )
                    });
                    rx.recv_timeout(Duration::from_secs(3)).ok().filter(|c| !c.is_empty())
                };
                let emit = |json: String| {
                    if !flag.load(Ordering::SeqCst) {
                        post(&app, &wid, &tid, frame, json);
                    }
                };
                us::run_xhr(req, &sc.meta, &page, &cookies, &flag, &emit);
                xhrs(|m| {
                    m.remove(&xkey);
                });
            });
        }
        Msg::XhrAbort { rid } => {
            if let Some(f) = xhrs(|m| m.remove(&(wid.to_string(), tid.to_string(), frame, rid))) {
                f.store(true, Ordering::SeqCst);
            }
        }
        Msg::Tab { s, url, active } => {
            if script(&s).is_none() || !tab_ok() {
                return;
            }
            let Some(u) = parse_web(&url) else { return };
            // 一般網頁才開（腳本的網址會變成安裝頁，見 add_tab）
            if let Ok(new) = add_tab(app, wid, u.to_string(), false, Some(tid)) {
                if active {
                    set_opener(wid, &new, tid);
                    let _ = select_tab(app, wid, &new);
                }
            }
        }
        Msg::Notify { s, title, text } => {
            let Some(sc) = script(&s) else { return };
            if notify_ok() {
                let title: String = if title.trim().is_empty() { sc.meta.name.clone() } else { title.chars().take(120).collect() };
                let text: String = text.chars().take(400).collect();
                let _ = crate::toast::show(&title, &text);
            }
        }
        Msg::Clip { s, text } => {
            if script(&s).is_some() && text.len() <= 4 << 20 {
                copy_text(&text);
            }
        }
    }
}

/* ---------------- 工具列的「使用者腳本」選單 ---------------- */

pub(super) struct Cmd {
    pub sid: String,
    pub id: String,
    pub caption: String,
    pub title: String,
    pub close: bool,
}

/// 選單裡的一支腳本
pub(super) struct Row {
    pub file: String,
    pub name: String,
    pub off: bool,
    /// 正在這個分頁裡跑
    pub running: bool,
    /// 需要的檔案沒下載到
    pub broken: bool,
    pub cmds: Vec<Cmd>,
}

/// 這個分頁的網址會用到哪些腳本（開著的、關著的都列，才能在這裡開關），以及它們登記的選單項目
pub(super) fn rows(wid: &str, tid: &str) -> Vec<Row> {
    let url = tab_of(wid, tid).map(|t| t.url).unwrap_or_default();
    let web = parse_web(&url).is_some();
    let key = (wid.to_string(), tid.to_string());
    let (running, cmds): (HashSet<String>, Vec<Cmd>) = tabs(|m| match m.get(&key) {
        Some(t) => (
            t.docs.values().flat_map(|d| d.ids.iter().cloned()).collect(),
            t.menus.iter().map(|c| Cmd { sid: c.sid.clone(), id: c.id.clone(), caption: c.caption.clone(), title: c.title.clone(), close: c.close }).collect(),
        ),
        None => (HashSet::new(), Vec::new()),
    });
    let mut cmds = cmds;
    let mut out = Vec::new();
    for s in us::list().iter() {
        let run = running.contains(&s.id);
        if !(run || (web && us::applies(&s.meta, &url))) {
            continue;
        }
        let mine: Vec<Cmd> = {
            let (a, b): (Vec<Cmd>, Vec<Cmd>) = std::mem::take(&mut cmds).into_iter().partition(|c| c.sid == s.id);
            cmds = b;
            a
        };
        out.push(Row { file: s.file.clone(), name: s.meta.name.clone(), off: s.off, running: run, broken: !s.missing.is_empty(), cmds: if s.off { Vec::new() } else { mine } });
    }
    out
}

/// 有沒有裝腳本（工具列要不要顯示按鈕）
pub(super) fn any() -> bool {
    us::any()
}

/// 點了腳本登記的選單項目：叫網頁裡的那支腳本做事
pub(super) fn click(app: &AppHandle, wid: &str, tid: &str, sid: &str, id: &str) {
    post(app, wid, tid, 0, us::down_msg(serde_json::json!({ "ev": "menu", "s": sid, "id": id })));
}

/* ---------------- 安裝頁 ---------------- */

/// 安裝腳本的頁面（程式自己的頁面）的網址
pub(super) fn install_url(target: &str) -> String {
    let q: String = url::form_urlencoded::byte_serialize(target.as_bytes()).collect();
    format!("chrome://extensions/install?url={q}")
}

/// 要開的網址是使用者腳本（.user.js）：改開安裝頁
pub(super) fn redirect(target: &str) -> Option<String> {
    parse_web(target).filter(us::is_script_url).map(|u| install_url(u.as_str()))
}
