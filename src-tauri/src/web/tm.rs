//! 篡改猴（Tampermonkey）裡的腳本：讀過來，由瀏覽器自己的執行環境代為執行。
//!
//! 篡改猴要靠瀏覽器的「允許使用者腳本」開關才能把腳本放進網頁，那個開關在瀏覽器核心自己的擴充功能管理頁
//! （edge://extensions）裡，而 WebView2 沒有那一頁（網址開不起來），開關所在的設定檔又有防竄改保護——
//! 所以在這個瀏覽器裡，篡改猴裝得起來、介面也都能用，就是沒辦法執行腳本。
//!
//! 這裡的做法：在瀏覽器視窗裡放一個藏起來的小網頁，網址是篡改猴套件裡的 manifest.json。那只是一份文字檔，
//! 不會執行篡改猴的任何程式，但在那份文件裡可以用擴充功能自己的存放區（chrome.storage.local）。
//! - 從那裡把使用者的腳本讀出來（tm_read.js），交給 userscripts.rs 存一份、照平常的方式執行
//! - 存放區一有變動（使用者在篡改猴裡新增、編輯、開關、刪除腳本），那份文件會改標題通知（tm_watch.js），這裡再讀一次
//! 只讀不寫：不會去改篡改猴的任何資料。

use super::*;
use crate::userscripts as scripts;
use std::sync::atomic::AtomicU64;

/// 文件標題開頭是這個 = 篡改猴的存放區變了（後面接第幾次）
const MARK: &str = "\u{2063}mbtm:";

struct St {
    /// 藏起來的那個網頁：（掛在哪個瀏覽器視窗下面, 網頁的名稱）
    host: Option<(String, String)>,
    /// 正在讀（什麼時候開始的）
    reading: Option<Instant>,
    /// 正在讀的時候又有變動：讀完再讀一次
    again: bool,
    /// 讀不到之後重新載入了幾次（篡改猴可能還沒裝進瀏覽器）
    retries: u32,
    /// 上一次有讀到
    ok: bool,
    /// 排了第幾次「等一下讀」（之後又排了新的，舊的就不用做）
    seq: u64,
    /// 讀完了幾次（等結果的人看這個）
    done: u64,
}

static ST: Mutex<St> = Mutex::new(St { host: None, reading: None, again: false, retries: 0, ok: false, seq: 0, done: 0 });
/// 第幾個讀取用的網頁（每次建立用不同的名稱）
static MADE: AtomicU64 = AtomicU64::new(0);

/// 這個瀏覽器裡開著的篡改猴（擴充功能編號）
fn ext_id() -> Option<String> {
    if EXT_BROKEN.load(Ordering::SeqCst) {
        return None;
    }
    let st = ext_state();
    scripts::TM_IDS.iter().find(|id| st.exts.get(**id).is_some_and(|m| !m.off && !m.dir.is_empty())).map(|id| id.to_string())
}

fn helper_url(ext: &str) -> Option<Url> {
    Url::parse(&format!("chrome-extension://{ext}/manifest.json")).ok()
}

/// 是不是那個藏起來的網頁的網址（瀏覽器通知「多了一個頁面」時，要認得它是程式自己開的）
pub(super) fn is_helper_url(url: &str) -> bool {
    scripts::TM_IDS.iter().any(|id| url.strip_prefix("chrome-extension://").and_then(|r| r.strip_prefix(id)) == Some("/manifest.json"))
}

fn helper(app: &AppHandle) -> Option<Webview> {
    let label = lock(&ST).host.as_ref().map(|(_, l)| l.clone())?;
    app.get_webview(&label)
}

/// 擴充功能清單可能變了（啟動、跟 Chrome 同步、開關擴充功能）：篡改猴在不在跟著更新。
/// 只更新「在不在」，不建立讀取用的網頁
pub(super) fn refresh() {
    scripts::tm_set(ext_id());
}

/// 擴充功能清單變了，而且可能有視窗開著：篡改猴變成開著的就準備好讀取用的網頁，變成沒有了就收掉
pub(super) fn refresh_live(app: &AppHandle) {
    let before = scripts::tm_ext();
    refresh();
    let now = scripts::tm_ext();
    if now == before {
        return;
    }
    if now.is_some() {
        if let Some(wid) = open_ids().into_iter().next() {
            kick(app, &wid, 1500, true);
        }
    } else {
        drop_helper(app);
    }
}

fn drop_helper(app: &AppHandle) {
    let old = {
        let mut st = lock(&ST);
        st.reading = None;
        st.ok = false;
        st.host.take()
    };
    if let Some((_, label)) = old {
        if let Some(wv) = app.get_webview(&label) {
            let _ = wv.close();
        }
    }
}

/// 過一下子確認讀取用的網頁在：沒有就建立（建立好會自己讀一次）；已經有了、但上次沒讀到，就重新載入再讀一次。
/// force = 已經有了、上次也有讀到，還是重新載入（擴充功能剛裝進瀏覽器：篡改猴可能換了新版，原本那份文件已經不能用）
pub(super) fn kick(app: &AppHandle, wid: &str, ms: u64, force: bool) {
    if scripts::tm_ext().is_none() {
        return;
    }
    watchdog(app);
    let (app, wid) = (app.clone(), wid.to_string());
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(ms));
        if helper(&app).is_none() {
            lock(&ST).retries = 0;
            make_soon(&app, &wid, 0, 0);
            return;
        }
        // 正在讀、或已經排了重試的，不用另外重新載入
        let idle = {
            let st = lock(&ST);
            !st.ok && st.reading.is_none() && st.retries == 0
        };
        if force || idle {
            lock(&ST).retries = 0;
            reload(&app);
        }
    });
}

/// 保險：每隔幾分鐘自己再讀一次（存放區變動的通知萬一漏了也跟得上）；讀取用的網頁不在了、壞了就重新準備
fn watchdog(app: &AppHandle) {
    static STARTED: AtomicBool = AtomicBool::new(false);
    if STARTED.swap(true, Ordering::SeqCst) {
        return;
    }
    let app = app.clone();
    std::thread::spawn(move || {
        let mut n = 0u32;
        loop {
            std::thread::sleep(Duration::from_secs(180));
            n = n.wrapping_add(1);
            let Some(wid) = open_ids().into_iter().next() else { continue };
            if scripts::tm_ext().is_none() {
                continue;
            }
            if helper(&app).is_none() {
                lock(&ST).retries = 0;
                make_soon(&app, &wid, 0, 0);
                continue;
            }
            let (ok, busy, retries) = {
                let st = lock(&ST);
                (st.ok, st.reading.is_some_and(|t| t.elapsed() < Duration::from_secs(45)), st.retries)
            };
            if busy {
                continue;
            }
            if ok {
                read(&app);
            } else if retries == 0 || (retries > 8 && n % 5 == 0) {
                // 一直沒讀到、也沒有排重試（試過幾次放棄了的：每十五分鐘再試一輪）
                lock(&ST).retries = 0;
                reload(&app);
            }
        }
    });
}

fn make_soon(app: &AppHandle, wid: &str, ms: u64, tries: u32) {
    let (app, wid) = (app.clone(), wid.to_string());
    std::thread::spawn(move || {
        if ms > 0 {
            std::thread::sleep(Duration::from_millis(ms));
        }
        let (a, w) = (app.clone(), wid.clone());
        let _ = app.run_on_main_thread(move || make(&a, &w, tries));
    });
}

/// （主執行緒）建立讀取用的網頁：放在看不到的地方、藏起來、不拿鍵盤；只能待在那一個網址
fn make(app: &AppHandle, wid: &str, tries: u32) {
    let Some(ext) = scripts::tm_ext() else { return };
    if !get(wid).is_some_and(|w| w.open) {
        return;
    }
    {
        // 已經有了就不用再建立（記著有、實際上已經不在了的：重新建立）
        let mut st = lock(&ST);
        if st.host.as_ref().is_some_and(|(_, l)| app.get_webview(l).is_some()) {
            return;
        }
        st.host = None;
    }
    let Some(win) = app.get_window(&win_label(wid)) else { return };
    let Some(url) = helper_url(&ext) else { return };
    // 正在建立別的網頁：等一下再來
    let Some(creating) = try_creating() else {
        if tries < 80 {
            make_soon(app, wid, 100, tries + 1);
        }
        return;
    };
    let label = format!("webtm-{}", MADE.fetch_add(1, Ordering::SeqCst) + 1);
    let only = url.clone();
    let (a1, a2, a3) = (app.clone(), app.clone(), app.clone());
    let w3 = wid.to_string();
    let b = WebviewBuilder::new(&label, WebviewUrl::External(url))
        .data_directory(profile_dir())
        .browser_extensions_enabled(true)
        .focused(false)
        .disable_drag_drop_handler()
        .on_navigation(move |u| *u == only)
        // 這份文件自己不會開新視窗。會通知到這裡的只有一種情況：擴充功能的背景程式要開分頁，而瀏覽器元件剛好
        // 把這件事通知給這個網頁——照擴充功能的小視窗那樣開成分頁（一般網頁，或開著的擴充功能自己的頁面）
        .on_new_window(move |u, _features| {
            let ok = !webcore::is_app_target(&u) && (web_url(&u) || (u.scheme() == "chrome-extension" && u.host_str().is_some_and(|h| ext_state().exts.get(h).is_some_and(|m| !m.off))));
            crate::dlog!("[tm] 開新分頁的要求通知到讀取用的網頁：{u} → {}", if ok { "開成分頁" } else { "不開" });
            if ok {
                let wid = if get(&w3).is_some_and(|w| w.open) { Some(w3.clone()) } else { open_ids().into_iter().next() };
                if let Some(wid) = wid {
                    open_ext_tab(&a3, &wid, None, &u);
                }
            }
            NewWindowResponse::Deny
        })
        .on_page_load(move |_wv, p| {
            if matches!(p.event(), PageLoadEvent::Finished) {
                loaded(&a1);
            }
        })
        .on_document_title_changed(move |_wv, title| {
            if title.starts_with(MARK) {
                read_soon(&a2, 150);
            }
        });
    let made = win.add_child(b, PhysicalPosition::new(-96i32, -96i32), PhysicalSize::new(32u32, 32u32));
    drop(creating);
    let made = match made {
        Ok(w) => w,
        Err(e) => {
            crate::dlog!("[tm] 讀取用的網頁建立失敗：{e}");
            return;
        }
    };
    let _ = made.hide();
    // 建立的這段時間視窗關了：不要了
    if !get(wid).is_some_and(|w| w.open) {
        let _ = made.close();
        return;
    }
    crate::dlog!("[tm] {wid}：建立了讀取用的網頁 {label}");
    lock(&ST).host = Some((wid.to_string(), label));
    // 它在瀏覽器裡的編號記成「程式自己開的」（不然會被當成擴充功能自己開的視窗，改開成分頁）
    with_wv(&made, |w| {
        webnative::cdp_call(
            w,
            "Target.getTargetInfo",
            "{}",
            Box::new(|r| {
                if let Some(id) = r.ok().and_then(|json| target_field(&json, "targetId")) {
                    let mut ours = lock(&OUR_TARGETS);
                    if !ours.contains(&id) {
                        ours.push(id);
                    }
                }
            }),
        )
    });
}

/// 在讀取用的網頁裡執行一段程式（結果要是文字）。false = 沒有那個網頁
fn eval(app: &AppHandle, expr: String, done: impl FnOnce(Result<String, String>) + Send + 'static) -> bool {
    let Some(wv) = helper(app) else { return false };
    let params = serde_json::json!({ "expression": expr, "awaitPromise": true, "returnByValue": true }).to_string();
    with_wv(&wv, move |w| webnative::cdp_call(w, "Runtime.evaluate", &params, Box::new(done)));
    true
}

/// 讀取用的網頁載入好了（WebView2 的通知，在主執行緒）：裝上「存放區變了就通知」，再讀一次
fn loaded(app: &AppHandle) {
    let app = app.clone();
    std::thread::spawn(move || {
        eval(&app, scripts::tm_watch_js(MARK), |r| {
            crate::dlog!("[tm] 存放區變動的通知：{}", r.map(|j| j.chars().take(120).collect::<String>()).unwrap_or_else(|e| format!("裝不上 {e}")));
        });
        read(&app);
    });
}

/// 過一下子讀一次（連續的變動合成一次）
pub(super) fn read_soon(app: &AppHandle, ms: u64) {
    let seq = {
        let mut st = lock(&ST);
        st.seq += 1;
        st.seq
    };
    let app = app.clone();
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(ms));
        if lock(&ST).seq == seq {
            read(&app);
        }
    });
}

/// 把篡改猴裡的腳本讀一次（不可以在主執行緒呼叫：要讀檔案）
fn read(app: &AppHandle) {
    {
        let mut st = lock(&ST);
        // 正在讀：讀完再讀一次（卡太久的不算）
        if st.reading.is_some_and(|t| t.elapsed() < Duration::from_secs(45)) {
            st.again = true;
            return;
        }
        st.reading = Some(Instant::now());
        st.again = false;
    }
    let a = app.clone();
    let sent = eval(app, scripts::tm_read_js(), move |r| {
        // （主執行緒）存檔、下載腳本需要的檔案都丟給背景做
        std::thread::spawn(move || finish(&a, r));
    });
    if !sent {
        let mut st = lock(&ST);
        st.reading = None;
        st.done += 1;
    }
}

fn finish(app: &AppHandle, r: Result<String, String>) {
    let (mut again, mut failed) = (false, false);
    match r.and_then(|json| scripts::tm_parse(&json)) {
        Ok(list) => {
            let n = list.len();
            let done = scripts::tm_apply(list);
            crate::dlog!("[tm] 讀到 {n} 支腳本，現在有 {} 支，清單{}", done.count, if done.changed { "變了" } else { "沒變" });
            let mut st = lock(&ST);
            st.retries = 0;
            st.ok = true;
            drop(st);
            again = done.again;
        }
        Err(e) => {
            crate::dlog!("[tm] 讀不到篡改猴裡的腳本：{e}");
            lock(&ST).ok = false;
            failed = true;
        }
    }
    let again = {
        let mut st = lock(&ST);
        st.reading = None;
        st.done += 1;
        std::mem::take(&mut st.again) || again
    };
    if failed {
        // 篡改猴可能還沒裝進瀏覽器（第一次開、剛同步過來）：等一下重新載入那個網頁再試，試幾次就先算了
        let n = {
            let mut st = lock(&ST);
            st.retries += 1;
            st.retries
        };
        if n <= 8 {
            let app = app.clone();
            std::thread::spawn(move || {
                std::thread::sleep(Duration::from_secs((3 * n as u64).min(20)));
                reload(&app);
            });
        }
    } else if again {
        read_soon(app, 300);
    }
}

/// 重新載入讀取用的網頁（載入好會再讀一次）
fn reload(app: &AppHandle) {
    let Some(wv) = helper(app) else { return };
    let Some(url) = scripts::tm_ext().and_then(|e| helper_url(&e)) else { return };
    let _ = wv.navigate(url);
}

/// 現在就讀一次，等它讀完（最多等 secs 秒）。擴充功能頁面的「重新讀取」用
pub(super) fn read_wait(app: &AppHandle, secs: u64) {
    refresh();
    if scripts::tm_ext().is_none() {
        return;
    }
    let before = lock(&ST).done;
    if helper(app).is_some() {
        read(app);
    } else if let Some(wid) = open_ids().into_iter().next() {
        lock(&ST).retries = 0;
        make_soon(app, &wid, 0, 0);
    } else {
        return;
    }
    let end = Instant::now() + Duration::from_secs(secs);
    while Instant::now() < end && lock(&ST).done == before {
        std::thread::sleep(Duration::from_millis(100));
    }
}

/// 瀏覽器視窗要關了：讀取用的網頁掛在它下面的話，換到別的開著的視窗
pub(super) fn window_closing(app: &AppHandle, wid: &str) {
    let mine = lock(&ST).host.as_ref().is_some_and(|(w, _)| w == wid);
    if !mine {
        return;
    }
    drop_helper(app);
    if let Some(other) = open_ids().into_iter().find(|w| w != wid) {
        kick(app, &other, 1200, false);
    }
}

/// 查問題用：這個檔案裡的鎖現在有沒有被佔著
pub(super) fn lock_state() -> String {
    format!("篡改猴={}", if ST.try_lock().is_err() { "佔著" } else { "空" })
}

/* ---------------- 篡改猴自己的頁面上的說明 ---------------- */

/// 篡改猴的頁面（控制台、編輯器、工具列的小選單）最上面會一直提示「請開啟允許使用者腳本」，但這個瀏覽器沒有那個開關。
/// 在它的頁面最下面加一行說明：不用找那個開關，這裡的腳本由瀏覽器代為執行。
/// （只加一行字、可以按 × 收起來，不動篡改猴自己的內容；其他網頁、其他擴充功能的頁面不會有）
pub(super) fn note_script() -> String {
    let ids = serde_json::to_string(&scripts::TM_IDS).unwrap_or_else(|_| "[]".into());
    NOTE_JS.replace("__IDS__", &ids)
}

const NOTE_JS: &str = r#"(() => {
  try {
    if (location.protocol !== "chrome-extension:" || !__IDS__.includes(location.host) || window.top !== window) return;
    if (!/\.html?$/i.test(location.pathname)) return;
    const add = () => {
      if (!document.body || document.getElementById("mb-tm-note")) return;
      const bar = document.createElement("div");
      bar.id = "mb-tm-note";
      bar.style.cssText = "position:fixed;left:0;right:0;bottom:0;z-index:2147483647;display:flex;align-items:center;gap:10px;padding:7px 12px;background:#1f2937;color:#f3f4f6;font:13px/1.5 'Microsoft JhengHei UI','Microsoft JhengHei',system-ui,sans-serif;box-shadow:0 -1px 6px rgba(0,0,0,.35)";
      const text = document.createElement("span");
      text.style.cssText = "flex:1;min-width:0";
      text.textContent = "工具箱：這個瀏覽器沒有「允許使用者腳本（Allow User Scripts）」開關，上面的提示不用理會。這裡的腳本由瀏覽器代為執行，存檔後重新整理網頁就生效。";
      const x = document.createElement("span");
      x.textContent = "×";
      x.title = "收起來";
      x.style.cssText = "cursor:pointer;font-size:18px;line-height:1;padding:0 4px;opacity:.8";
      x.addEventListener("click", () => bar.remove());
      bar.append(text, x);
      document.body.append(bar);
    };
    if (document.readyState === "loading") document.addEventListener("DOMContentLoaded", add, { once: true });
    else add();
  } catch (e) {}
})();"#;
