//! 瀏覽器視窗跟 WebView2 直接溝通的部分（只在 Windows 編譯，不碰 Tauri）：
//! 鍵盤快捷鍵、上一頁／下一頁能不能按、網址變動、聲音、縮放、網頁右鍵選單、下載、清除瀏覽資料…
//!
//! 這裡的函式都要在主執行緒（視窗所在的執行緒）呼叫；事件也是在主執行緒通知，
//! 收到事件的一方不可以在裡面等主執行緒做事（要另外丟給背景執行緒處理）。

use crate::webev::{CtxTarget, DlEv, Ev, NativeItem, Opts, Sink};
use std::cell::RefCell;
use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};
use webview2_com::Microsoft::Web::WebView2::Win32::*;
use webview2_com::{
    take_pwstr, AcceleratorKeyPressedEventHandler, AddScriptToExecuteOnDocumentCreatedCompletedHandler, BytesReceivedChangedEventHandler,
    CallDevToolsProtocolMethodCompletedHandler, ClearBrowsingDataCompletedHandler, ContainsFullScreenElementChangedEventHandler,
    ContextMenuRequestedEventHandler, DevToolsProtocolEventReceivedEventHandler, DownloadStartingEventHandler, FaviconChangedEventHandler,
    FindStartCompletedHandler, FrameChildFrameCreatedEventHandler, FrameCreatedEventHandler, FrameDestroyedEventHandler,
    FrameWebMessageReceivedEventHandler, GetCookiesCompletedHandler, HistoryChangedEventHandler, IsDocumentPlayingAudioChangedEventHandler,
    IsMutedChangedEventHandler, NavigationStartingEventHandler, NewWindowRequestedEventHandler, ShowSaveAsUICompletedHandler,
    SourceChangedEventHandler, StateChangedEventHandler, WindowCloseRequestedEventHandler,
    ZoomFactorChangedEventHandler,
};
use windows_core::{Interface, BOOL, HSTRING, PWSTR};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{GetKeyState, VK_CONTROL, VK_ESCAPE, VK_MENU, VK_SHIFT};

/// 一個網頁（或工具列）的 WebView2
pub struct Wv {
    pub controller: ICoreWebView2Controller,
    pub core: ICoreWebView2,
}

unsafe fn text(f: impl FnOnce(*mut PWSTR) -> windows_core::Result<()>) -> String {
    let mut p = PWSTR::null();
    if f(&mut p).is_ok() && !p.is_null() {
        take_pwstr(p)
    } else {
        String::new()
    }
}

unsafe fn flag(f: impl FnOnce(*mut BOOL) -> windows_core::Result<()>) -> bool {
    let mut b = BOOL(0);
    f(&mut b).is_ok() && b.as_bool()
}

fn key_down(vk: u16) -> bool {
    unsafe { GetKeyState(vk as i32) < 0 }
}

/* ---------------- 網頁右鍵選單 ---------------- */

struct CtxPending {
    seq: u64,
    args: ICoreWebView2ContextMenuRequestedEventArgs,
    deferral: ICoreWebView2Deferral,
}

thread_local! {
    /// 正在等使用者選的右鍵選單（一次只會有一個）
    static CTX: RefCell<Option<CtxPending>> = const { RefCell::new(None) };
    /// 下載中的檔案（可以暫停、繼續、取消）
    static DLS: RefCell<HashMap<u64, ICoreWebView2DownloadOperation>> = RefCell::new(HashMap::new());
    /// 網頁要求開新視窗（window.open、開新分頁的連結）時 WebView2 給的資料，先收著。
    /// 之後把新開的分頁交還給它（見 NewWin::adopt），開它的網頁才拿得到這個分頁
    static NEWWIN: RefCell<VecDeque<NewWin>> = const { RefCell::new(VecDeque::new()) };
    /// 每個網頁裡的內嵌框架（要回訊息給框架裡的使用者腳本時用）。鍵 = (網頁, 框架編號)
    static FRAMES: RefCell<HashMap<(usize, u32), ICoreWebView2Frame2>> = RefCell::new(HashMap::new());
    /// 每個網頁目前登記的「帶最新設定值啟動使用者腳本」的小程式（換新的時候要先拿掉舊的）。值 = (第幾次, 登記編號)
    static GM_BOOT: RefCell<HashMap<usize, (u64, String)>> = RefCell::new(HashMap::new());
}

/// 一次「開新視窗」的要求
pub struct NewWin {
    /// 要開的網址
    uri: String,
    args: ICoreWebView2NewWindowRequestedEventArgs,
    /// 是程式自己的頁面（設定、新分頁…）要開的
    from_own: bool,
    /// 提出要求的那個框架的網址（擴充功能的背景程式要開分頁時沒有框架，是空的）
    source: String,
}

// 只在主執行緒建立、使用（為了能放進要求 Send 的閉包裡；閉包是在主執行緒當場執行的）
unsafe impl Send for NewWin {}

impl NewWin {
    /// 是程式自己的頁面要開的：不可以讓開出來的網頁跟它連著
    pub fn from_own(&self) -> bool {
        self.from_own
    }

    /// 提出要求的那個框架的網址（沒有就是空的）
    pub fn source(&self) -> &str {
        &self.source
    }

    /// 把新開的網頁交給這個要求：WebView2 會把它當成 window.open 開出來的視窗——
    /// 開它的網頁拿得到它（回傳值不是 null）、它拿得到開它的網頁（window.opener）、
    /// 要送出的表單資料也會一起帶過去。新網頁要還沒載入過別的網址
    pub fn adopt(&self, w: &Wv) -> bool {
        unsafe { self.args.SetNewWindow(&w.core).is_ok() }
    }
}

/// 拿出剛才收著的「開新視窗」要求。same(網址) = 是不是這一次的；排在它前面對不上的是過期的，丟掉
pub fn take_new_window(same: impl Fn(&str) -> bool) -> Option<NewWin> {
    // 過期的先收著，等放開清單之後才丟掉（丟掉時會通知 WebView2，不要在還拿著清單的時候做）
    let mut stale = Vec::new();
    let hit = NEWWIN.with(|q| {
        let mut q = q.borrow_mut();
        while let Some(req) = q.pop_front() {
            if same(&req.uri) {
                return Some(req);
            }
            stale.push(req);
        }
        None
    });
    drop(stale);
    hit
}

static DL_KEY: AtomicU64 = AtomicU64::new(0);
static CTX_SEQ: AtomicU64 = AtomicU64::new(0);

/// 拿出等待中的右鍵選單（seq = 0 不管是哪一個；不然要是同一個才拿）
fn ctx_take(seq: u64) -> Option<CtxPending> {
    CTX.with(|c| {
        let mut g = c.borrow_mut();
        if seq != 0 && g.as_ref().is_some_and(|p| p.seq != seq) {
            return None;
        }
        g.take()
    })
}

/// 右鍵選單結束：pick = 選了 WebView2 的哪個指令（沒選、選的是自己加的項目就 None）
pub fn ctx_finish(seq: u64, pick: Option<i32>) {
    if let Some(p) = ctx_take(seq) {
        unsafe {
            if let Some(id) = pick {
                let _ = p.args.SetSelectedCommandId(id);
            }
            let _ = p.deferral.Complete();
        }
    }
}

/// 自己的選單顯示不出來：交還給 WebView2 顯示原本的選單
pub fn ctx_default(seq: u64) {
    if let Some(p) = ctx_take(seq) {
        unsafe {
            let _ = p.args.SetHandled(false);
            let _ = p.deferral.Complete();
        }
    }
}

unsafe fn collect(list: &ICoreWebView2ContextMenuItemCollection, depth: u8) -> Vec<NativeItem> {
    let mut n = 0u32;
    if list.Count(&mut n).is_err() {
        return Vec::new();
    }
    let mut out = Vec::new();
    for i in 0..n.min(80) {
        let Ok(it) = list.GetValueAtIndex(i) else { continue };
        let mut id = 0i32;
        let _ = it.CommandId(&mut id);
        let mut kind = COREWEBVIEW2_CONTEXT_MENU_ITEM_KIND::default();
        let _ = it.Kind(&mut kind);
        let children = if kind == COREWEBVIEW2_CONTEXT_MENU_ITEM_KIND_SUBMENU && depth < 3 {
            it.Children().map(|c| collect(&c, depth + 1)).unwrap_or_default()
        } else {
            Vec::new()
        };
        out.push(NativeItem {
            id,
            name: text(|p| it.Name(p)),
            label: text(|p| it.Label(p)),
            kind: kind.0.clamp(0, 4) as u8,
            enabled: flag(|b| it.IsEnabled(b)),
            checked: flag(|b| it.IsChecked(b)),
            shortcut: text(|p| it.ShortcutKeyDescription(p)),
            children,
        });
    }
    out
}

unsafe fn read_target(t: &ICoreWebView2ContextMenuTarget) -> CtxTarget {
    let mut kind = COREWEBVIEW2_CONTEXT_MENU_TARGET_KIND::default();
    let _ = t.Kind(&mut kind);
    let short = |s: String, n: usize| if s.len() > n { String::new() } else { s };
    CtxTarget {
        kind: kind.0.clamp(0, 4) as u8,
        editable: flag(|b| t.IsEditable(b)),
        link: if flag(|b| t.HasLinkUri(b)) { short(text(|p| t.LinkUri(p)), 8000) } else { String::new() },
        src: if flag(|b| t.HasSourceUri(b)) { short(text(|p| t.SourceUri(p)), 8000) } else { String::new() },
        selection: if flag(|b| t.HasSelection(b)) { text(|p| t.SelectionText(p)).chars().take(2000).collect() } else { String::new() },
        page: text(|p| t.PageUri(p)),
    }
}

/* ---------------- 掛上事件 ---------------- */

/// 把瀏覽器需要的事件都掛到一個網頁（或工具列）上。任何一項失敗（WebView2 太舊）都只是少那個功能
pub fn attach(w: &Wv, opts: Opts, sink: Sink) {
    unsafe { attach_(&w.controller, &w.core, opts, sink) }
}

unsafe fn attach_(controller: &ICoreWebView2Controller, core: &ICoreWebView2, opts: Opts, sink: Sink) {
    let mut token = 0i64;

    // 設定
    if let Ok(s) = core.Settings() {
        if opts.page {
            // 滑鼠移到連結上時，左下角顯示網址；Ctrl + 滾輪可以縮放（跟 Chrome 一樣）
            let _ = s.SetIsStatusBarEnabled(true);
            let _ = s.SetIsZoomControlEnabled(true);
        }
        if let Ok(s3) = s.cast::<ICoreWebView2Settings3>() {
            // 工具列這種自己的頁面不要 F5 重新整理、Ctrl+P 列印這些
            let _ = s3.SetAreBrowserAcceleratorKeysEnabled(opts.page);
        }
        if opts.page {
            if let Ok(s4) = s.cast::<ICoreWebView2Settings4>() {
                let _ = s4.SetIsPasswordAutosaveEnabled(opts.save_passwords);
            }
            // 開發人員工具：一開始照分頁要載入的網址決定（之後每次換網址都會重設，見下面）
            let _ = s.SetAreDevToolsEnabled(opts.web_first);
        }
    }
    if opts.page && !opts.download_dir.is_empty() {
        download_dir_(core, &opts.download_dir);
    }

    // 快捷鍵
    {
        let (s, accel, no_repeat) = (sink.clone(), opts.accel, opts.no_repeat);
        let h = AcceleratorKeyPressedEventHandler::create(Box::new(move |_c, args| {
            let Some(args) = args else { return Ok(()) };
            let mut kind = COREWEBVIEW2_KEY_EVENT_KIND::default();
            args.KeyEventKind(&mut kind)?;
            if kind != COREWEBVIEW2_KEY_EVENT_KIND_KEY_DOWN && kind != COREWEBVIEW2_KEY_EVENT_KIND_SYSTEM_KEY_DOWN {
                return Ok(());
            }
            let mut vk = 0u32;
            args.VirtualKey(&mut vk)?;
            let (ctrl, shift, alt) = (key_down(VK_CONTROL), key_down(VK_SHIFT), key_down(VK_MENU));
            if vk == VK_ESCAPE as u32 && !ctrl && !shift && !alt {
                // 只通知（關掉開著的選單），按鍵照樣給網頁
                s(Ev::Key("esc"));
                return Ok(());
            }
            if let Some(cmd) = accel(vk, ctrl, shift, alt) {
                let mut st = COREWEBVIEW2_PHYSICAL_KEY_STATUS::default();
                let _ = args.PhysicalKeyStatus(&mut st);
                let _ = args.SetHandled(true);
                if !(st.WasKeyDown.as_bool() && no_repeat(cmd)) {
                    s(Ev::Key(cmd));
                }
            }
            Ok(())
        }));
        let _ = controller.add_AcceleratorKeyPressed(&h, &mut token);
    }

    if !opts.page {
        return;
    }

    // 上一頁／下一頁
    {
        let s = sink.clone();
        let h = HistoryChangedEventHandler::create(Box::new(move |core, _| {
            if let Some(core) = core {
                s(Ev::Nav { back: flag(|b| core.CanGoBack(b)), forward: flag(|b| core.CanGoForward(b)) });
            }
            Ok(())
        }));
        let _ = core.add_HistoryChanged(&h, &mut token);
    }

    // 網址
    {
        let s = sink.clone();
        let h = SourceChangedEventHandler::create(Box::new(move |core, _| {
            if let Some(core) = core {
                let u = text(|p| core.Source(p));
                if !u.is_empty() {
                    s(Ev::Source(u));
                }
            }
            Ok(())
        }));
        let _ = core.add_SourceChanged(&h, &mut token);
    }

    // 內嵌框架不可以載入程式自己的頁面
    {
        let own = opts.own_page;
        let h = NavigationStartingEventHandler::create(Box::new(move |_c, args| {
            if let Some(args) = args {
                if own(&text(|p| args.Uri(p))) {
                    let _ = args.SetCancel(true);
                }
            }
            Ok(())
        }));
        let _ = core.add_FrameNavigationStarting(&h, &mut token);
    }

    // 開發人員工具：只有一般網頁可以用右鍵的「檢查」；程式自己的頁面關掉
    // （設定在下一次載入網頁時生效，所以每次要前往新的網址時照那個網址設定）
    {
        let own = opts.own_page;
        let h = NavigationStartingEventHandler::create(Box::new(move |c, args| {
            if let (Some(c), Some(args)) = (c, args) {
                if let Ok(s) = c.Settings() {
                    let _ = s.SetAreDevToolsEnabled(!own(&text(|p| args.Uri(p))));
                }
            }
            Ok(())
        }));
        let _ = core.add_NavigationStarting(&h, &mut token);
    }

    // 網頁要開新視窗：把這次的要求收著（開成新分頁時要把分頁交還給它，見 take_new_window）
    {
        let own = opts.own_page;
        let h = NewWindowRequestedEventHandler::create(Box::new(move |c, args| {
            if let Some(args) = args {
                let uri = text(|p| args.Uri(p));
                // 看不出是哪個網頁要開的，就當成程式自己的頁面（不連）
                let from_own = c.as_ref().is_none_or(|c| {
                    let src = text(|p| c.Source(p));
                    src.is_empty() || own(&src)
                });
                let source = args
                    .cast::<ICoreWebView2NewWindowRequestedEventArgs3>()
                    .ok()
                    .and_then(|a| a.OriginalSourceFrameInfo().ok())
                    .map(|f| text(|p| f.Source(p)))
                    .unwrap_or_default();
                crate::dlog!(
                    "[newwin] uri={uri} source={source:?} user={} name={:?} page={:?}",
                    flag(|b| args.IsUserInitiated(b)),
                    args.cast::<ICoreWebView2NewWindowRequestedEventArgs2>().map(|a| text(|p| a.Name(p))).unwrap_or_default(),
                    c.as_ref().map(|c| text(|p| c.Source(p))).unwrap_or_default()
                );
                // 沒人來拿的不要一直留著（放開清單之後才丟掉）
                let old = NEWWIN.with(|q| {
                    let mut q = q.borrow_mut();
                    let mut old = Vec::new();
                    while q.len() >= 6 {
                        old.extend(q.pop_front());
                    }
                    q.push_back(NewWin { uri, args, from_own, source });
                    old
                });
                drop(old);
            }
            Ok(())
        }));
        let _ = core.add_NewWindowRequested(&h, &mut token);
    }

    if opts.page {
        gm_attach(core, &sink, &opts.gm_bind);
        targets_attach(core, &sink);
    }

    // 網站圖示
    if let Ok(c15) = core.cast::<ICoreWebView2_15>() {
        let s = sink.clone();
        let h = FaviconChangedEventHandler::create(Box::new(move |core, _| {
            if let Some(c) = core.and_then(|c| c.cast::<ICoreWebView2_15>().ok()) {
                let u = text(|p| c.FaviconUri(p));
                if !u.is_empty() {
                    s(Ev::Favicon(u));
                }
            }
            Ok(())
        }));
        let _ = c15.add_FaviconChanged(&h, &mut token);
    }

    // 聲音
    if let Ok(c8) = core.cast::<ICoreWebView2_8>() {
        let audio = |core: Option<ICoreWebView2>| {
            core.and_then(|c| c.cast::<ICoreWebView2_8>().ok())
                .map(|c| Ev::Audio { playing: flag(|b| c.IsDocumentPlayingAudio(b)), muted: flag(|b| c.IsMuted(b)) })
        };
        let s = sink.clone();
        let h = IsDocumentPlayingAudioChangedEventHandler::create(Box::new(move |core, _| {
            if let Some(ev) = audio(core) {
                s(ev);
            }
            Ok(())
        }));
        let _ = c8.add_IsDocumentPlayingAudioChanged(&h, &mut token);
        let s = sink.clone();
        let h = IsMutedChangedEventHandler::create(Box::new(move |core, _| {
            if let Some(ev) = audio(core) {
                s(ev);
            }
            Ok(())
        }));
        let _ = c8.add_IsMutedChanged(&h, &mut token);
    }

    // 縮放（Ctrl + 滾輪）
    {
        let s = sink.clone();
        let h = ZoomFactorChangedEventHandler::create(Box::new(move |c, _| {
            if let Some(c) = c {
                let mut z = 1.0f64;
                if c.ZoomFactor(&mut z).is_ok() {
                    s(Ev::Zoom(z));
                }
            }
            Ok(())
        }));
        let _ = controller.add_ZoomFactorChanged(&h, &mut token);
    }

    // 影片全螢幕
    {
        let s = sink.clone();
        let h = ContainsFullScreenElementChangedEventHandler::create(Box::new(move |core, _| {
            if let Some(core) = core {
                s(Ev::Fullscreen(flag(|b| core.ContainsFullScreenElement(b))));
            }
            Ok(())
        }));
        let _ = core.add_ContainsFullScreenElementChanged(&h, &mut token);
    }

    // window.close()
    {
        let s = sink.clone();
        let h = WindowCloseRequestedEventHandler::create(Box::new(move |_c, _| {
            s(Ev::CloseRequested);
            Ok(())
        }));
        let _ = core.add_WindowCloseRequested(&h, &mut token);
    }

    // 右鍵選單
    if let Ok(c11) = core.cast::<ICoreWebView2_11>() {
        let s = sink.clone();
        let h = ContextMenuRequestedEventHandler::create(Box::new(move |c, args| {
            let Some(args) = args else { return Ok(()) };
            // 上一個還沒結束的先結束
            ctx_finish(0, None);
            let mut items = collect(&args.MenuItems()?, 0);
            // 「上一頁／下一頁」能不能按：以這個網頁實際能不能返回、前進為準（跟工具列的箭頭一致）。
            // WebView2 給的選單項目有時在沒有下一頁時也標成可以按
            if let Some(core) = &c {
                let (back, forward) = (flag(|b| core.CanGoBack(b)), flag(|b| core.CanGoForward(b)));
                for it in items.iter_mut() {
                    match it.name.as_str() {
                        "back" => it.enabled = back,
                        "forward" => it.enabled = forward,
                        _ => {}
                    }
                }
            }
            let target = read_target(&args.ContextMenuTarget()?);
            if items.is_empty() {
                return Ok(());
            }
            let deferral = args.GetDeferral()?;
            args.SetHandled(true)?;
            let seq = CTX_SEQ.fetch_add(1, Ordering::SeqCst) + 1;
            CTX.with(|c| *c.borrow_mut() = Some(CtxPending { seq, args, deferral }));
            s(Ev::Context { seq, target, items });
            Ok(())
        }));
        let _ = c11.add_ContextMenuRequested(&h, &mut token);
    }

    // 下載
    if let Ok(c4) = core.cast::<ICoreWebView2_4>() {
        let s = sink.clone();
        let h = DownloadStartingEventHandler::create(Box::new(move |_c, args| {
            let Some(args) = args else { return Ok(()) };
            let op = args.DownloadOperation()?;
            let key = DL_KEY.fetch_add(1, Ordering::SeqCst) + 1;
            let mut total = -1i64;
            let _ = op.TotalBytesToReceive(&mut total);
            s(Ev::Download(DlEv::Start { key, url: text(|p| op.Uri(p)), path: text(|p| args.ResultFilePath(p)), total }));
            DLS.with(|d| d.borrow_mut().insert(key, op.clone()));
            let mut tok = 0i64;
            // 進度：最多每 0.3 秒通知一次
            let s2 = s.clone();
            let mut last = Instant::now() - Duration::from_secs(1);
            let hb = BytesReceivedChangedEventHandler::create(Box::new(move |op, _| {
                if let Some(op) = op {
                    if last.elapsed() >= Duration::from_millis(300) {
                        last = Instant::now();
                        let (mut r, mut t) = (0i64, -1i64);
                        let _ = op.BytesReceived(&mut r);
                        let _ = op.TotalBytesToReceive(&mut t);
                        s2(Ev::Download(DlEv::Progress { key, received: r, total: t }));
                    }
                }
                Ok(())
            }));
            let _ = op.add_BytesReceivedChanged(&hb, &mut tok);
            let s3 = s.clone();
            let hs = StateChangedEventHandler::create(Box::new(move |op, _| {
                let Some(op) = op else { return Ok(()) };
                let mut st = COREWEBVIEW2_DOWNLOAD_STATE::default();
                let _ = op.State(&mut st);
                if st == COREWEBVIEW2_DOWNLOAD_STATE_IN_PROGRESS {
                    return Ok(());
                }
                let state = if st == COREWEBVIEW2_DOWNLOAD_STATE_COMPLETED {
                    "done"
                } else {
                    let mut why = COREWEBVIEW2_DOWNLOAD_INTERRUPT_REASON::default();
                    let _ = op.InterruptReason(&mut why);
                    if why == COREWEBVIEW2_DOWNLOAD_INTERRUPT_REASON_USER_CANCELED || why == COREWEBVIEW2_DOWNLOAD_INTERRUPT_REASON_USER_SHUTDOWN {
                        "canceled"
                    } else {
                        // 暫停也會通知「中斷」，但還可以繼續：不算結束
                        if flag(|b| op.CanResume(b)) {
                            return Ok(());
                        }
                        "failed"
                    }
                };
                let mut r = 0i64;
                let _ = op.BytesReceived(&mut r);
                DLS.with(|d| d.borrow_mut().remove(&key));
                s3(Ev::Download(DlEv::Done { key, state, path: text(|p| op.ResultFilePath(p)), received: r }));
                Ok(())
            }));
            let _ = op.add_StateChanged(&hs, &mut tok);
            Ok(())
        }));
        let _ = c4.add_DownloadStarting(&h, &mut token);
    }
}

/* ---------------- 操作 ---------------- */

/// 網頁自己呼叫 window.close() 時通知（給登入用的小視窗：要把整個視窗關掉）
pub fn on_close_requested(w: &Wv, f: impl Fn() + 'static) {
    unsafe {
        let mut token = 0i64;
        let h = WindowCloseRequestedEventHandler::create(Box::new(move |_c, _| {
            f();
            Ok(())
        }));
        let _ = w.core.add_WindowCloseRequested(&h, &mut token);
    }
}

/// 打開開發人員工具（F12）。own_page(網址) = 程式自己的頁面：不可以開。回傳有沒有開
pub fn open_devtools(w: &Wv, own_page: fn(&str) -> bool) -> bool {
    unsafe {
        let src = text(|p| w.core.Source(p));
        if src.is_empty() || own_page(&src) {
            return false;
        }
        if let Ok(s) = w.core.Settings() {
            let _ = s.SetAreDevToolsEnabled(true);
        }
        w.core.OpenDevToolsWindow().is_ok()
    }
}

pub fn go_back(w: &Wv) {
    unsafe {
        let _ = w.core.GoBack();
    }
}

pub fn go_forward(w: &Wv) {
    unsafe {
        let _ = w.core.GoForward();
    }
}

pub fn stop(w: &Wv) {
    unsafe {
        let _ = w.core.Stop();
    }
}

pub fn can_go(w: &Wv) -> (bool, bool) {
    unsafe { (flag(|b| w.core.CanGoBack(b)), flag(|b| w.core.CanGoForward(b))) }
}

pub fn set_muted(w: &Wv, muted: bool) {
    unsafe {
        if let Ok(c) = w.core.cast::<ICoreWebView2_8>() {
            let _ = c.SetIsMuted(muted);
        }
    }
}

pub fn set_zoom(w: &Wv, z: f64) {
    unsafe {
        let mut cur = 1.0f64;
        if w.controller.ZoomFactor(&mut cur).is_ok() && (cur - z).abs() < 0.001 {
            return;
        }
        let _ = w.controller.SetZoomFactor(z);
    }
}

pub fn set_password_save(w: &Wv, on: bool) {
    unsafe {
        if let Ok(s4) = w.core.Settings().and_then(|s| s.cast::<ICoreWebView2Settings4>()) {
            let _ = s4.SetIsPasswordAutosaveEnabled(on);
        }
    }
}

unsafe fn download_dir_(core: &ICoreWebView2, dir: &str) {
    if dir.trim().is_empty() {
        return;
    }
    if let Ok(p) = core.cast::<ICoreWebView2_13>().and_then(|c| c.Profile()) {
        let _ = p.SetDefaultDownloadFolderPath(&HSTRING::from(dir));
    }
}

/// 下載要存的資料夾（整個瀏覽器設定檔共用）
pub fn set_download_dir(w: &Wv, dir: &str) {
    unsafe { download_dir_(&w.core, dir) }
}

/// 瀏覽器的工作管理員
pub fn task_manager(w: &Wv) {
    unsafe {
        if let Ok(c) = w.core.cast::<ICoreWebView2_6>() {
            let _ = c.OpenTaskManagerWindow();
        }
    }
}

/// 另存網頁
pub fn save_as(w: &Wv) {
    unsafe {
        if let Ok(c) = w.core.cast::<ICoreWebView2_25>() {
            let h = ShowSaveAsUICompletedHandler::create(Box::new(|_, _| Ok(())));
            let _ = c.ShowSaveAsUI(&h);
        }
    }
}

/// 下載清單的小視窗：開著就關掉，關著就打開
pub fn toggle_downloads(w: &Wv) {
    unsafe {
        if let Ok(c) = w.core.cast::<ICoreWebView2_9>() {
            if flag(|b| c.IsDefaultDownloadDialogOpen(b)) {
                let _ = c.CloseDefaultDownloadDialog();
            } else {
                let _ = c.OpenDefaultDownloadDialog();
            }
        }
    }
}

/// 在網頁中尋找（顯示尋找列）。回傳 false = 這個 WebView2 版本沒有這個功能
pub fn find(w: &Wv) -> bool {
    unsafe {
        let Ok(c28) = w.core.cast::<ICoreWebView2_28>() else { return false };
        let Ok(find) = c28.Find() else { return false };
        let Ok(env) = w.core.cast::<ICoreWebView2_2>().and_then(|c| c.Environment()) else { return false };
        let Ok(env15) = env.cast::<ICoreWebView2Environment15>() else { return false };
        let Ok(options) = env15.CreateFindOptions() else { return false };
        let _ = options.SetFindTerm(&HSTRING::from(""));
        let _ = options.SetSuppressDefaultFindDialog(false);
        let h = FindStartCompletedHandler::create(Box::new(|_| Ok(())));
        find.Start(&options, &h).is_ok()
    }
}

/// WebView2（瀏覽器核心）的版本
pub fn version(w: &Wv) -> String {
    unsafe {
        match w.core.cast::<ICoreWebView2_2>().and_then(|c| c.Environment()) {
            Ok(env) => text(|p| env.BrowserVersionString(p)),
            Err(_) => String::new(),
        }
    }
}

pub fn dl_cancel(key: u64) {
    if let Some(op) = DLS.with(|d| d.borrow().get(&key).cloned()) {
        unsafe {
            let _ = op.Cancel();
        }
    }
}

pub fn dl_pause(key: u64, pause: bool) {
    if let Some(op) = DLS.with(|d| d.borrow().get(&key).cloned()) {
        unsafe {
            let _ = if pause { op.Pause() } else { op.Resume() };
        }
    }
}

/// 清除瀏覽資料。kinds = 要清的種類（webev::data_kinds 的數值相加）；
/// since_ms = 只清這個時間（Unix 毫秒）之後的，None = 全部
pub fn clear_data(w: &Wv, kinds: i32, since_ms: Option<u64>, done: Box<dyn FnOnce(bool) + Send + 'static>) {
    unsafe {
        let Ok(p2) = w.core.cast::<ICoreWebView2_13>().and_then(|c| c.Profile()).and_then(|p| p.cast::<ICoreWebView2Profile2>()) else {
            done(false);
            return;
        };
        let kinds = COREWEBVIEW2_BROWSING_DATA_KINDS(kinds);
        let h = ClearBrowsingDataCompletedHandler::create(Box::new(move |r| {
            done(r.is_ok());
            Ok(())
        }));
        let r = match since_ms {
            // 時間是從 1970 年起的秒數
            Some(ms) => p2.ClearBrowsingDataInTimeRange(kinds, ms as f64 / 1000.0, 4_102_444_800.0, &h),
            None => p2.ClearBrowsingData(kinds, &h),
        };
        if let Err(e) = r {
            eprintln!("清除瀏覽資料失敗：{e}");
        }
    }
}

/// 這個分頁可以回去的網頁清單（上一頁按鈕按右鍵用）：回傳 DevTools 的 Page.getNavigationHistory 結果（JSON 文字，失敗是空字串）
pub fn nav_entries(w: &Wv, done: Box<dyn FnOnce(String) + Send + 'static>) {
    unsafe {
        let h = CallDevToolsProtocolMethodCompletedHandler::create(Box::new(move |r, json| {
            done(if r.is_ok() { json } else { String::new() });
            Ok(())
        }));
        let _ = w.core.CallDevToolsProtocolMethod(&HSTRING::from("Page.getNavigationHistory"), &HSTRING::from("{}"), &h);
    }
}

/// 跳到清單裡的某一頁
pub fn nav_to(w: &Wv, entry: i64) {
    unsafe {
        let h = CallDevToolsProtocolMethodCompletedHandler::create(Box::new(|_, _| Ok(())));
        let params = format!("{{\"entryId\":{entry}}}");
        let _ = w.core.CallDevToolsProtocolMethod(&HSTRING::from("Page.navigateToHistoryEntry"), &HSTRING::from(params), &h);
    }
}

/// 清掉一個網站的 Cookie 與儲存資料（origins = 要清的來源，例如 https://x.com）。
/// Cookie 用 WebView2 的 Cookie 管理：把「開這個網址時會送出的 cookie」（含 .x.com 這種整個網域的）全部刪掉；
/// 其他資料（localStorage、IndexedDB、快取、Service Worker）用 DevTools 的 Storage.clearDataForOrigin
pub fn clear_site(w: &Wv, origins: &[String]) {
    unsafe {
        let core = &w.core;
        if let Ok(cm) = core.cast::<ICoreWebView2_2>().and_then(|c| c.CookieManager()) {
            for o in origins {
                let cm2 = cm.clone();
                let handler = GetCookiesCompletedHandler::create(Box::new(move |_, list| {
                    if let Some(list) = list {
                        let mut n = 0u32;
                        let _ = list.Count(&mut n);
                        for i in 0..n {
                            if let Ok(c) = list.GetValueAtIndex(i) {
                                let _ = cm2.DeleteCookie(&c);
                            }
                        }
                    }
                    Ok(())
                }));
                if let Err(e) = cm.GetCookies(&HSTRING::from(format!("{o}/")), &handler) {
                    eprintln!("讀取 cookie 失敗：{e}");
                }
            }
        } else {
            eprintln!("這個 WebView2 版本沒有 Cookie 管理");
        }
        for o in origins {
            let params = format!("{{\"origin\":\"{}\",\"storageTypes\":\"all\"}}", o.replace(['"', '\\'], ""));
            let handler = CallDevToolsProtocolMethodCompletedHandler::create(Box::new(|_, _| Ok(())));
            if let Err(e) = core.CallDevToolsProtocolMethod(&HSTRING::from("Storage.clearDataForOrigin"), &HSTRING::from(params), &handler) {
                eprintln!("清除網站資料失敗：{e}");
            }
        }
    }
}

/* ---------------- 使用者腳本：跟網頁裡的腳本互傳訊息 ---------------- */

/// 是不是使用者腳本傳來的訊息（一定是物件，而且有 "mb" 這個欄位；其他網頁自己傳的不理）
fn gm_like(json: &str) -> bool {
    json.starts_with('{') && json.contains("\"mb\"")
}

/// 接網頁裡的使用者腳本傳來的訊息。
/// - 內嵌框架：用 chrome.webview.postMessage（每個框架各自接）
/// - 最上層的網頁：不能用同一招——Tauri 自己也接了最上層的 chrome.webview 訊息，而且它遇到不是文字的訊息會回報失敗，
///   WebView2 就不再通知後面的人。所以改用 DevTools 的「綁定函式」：網頁呼叫那個函式，程式這邊會收到通知
///   （只加這個函式，不開 Runtime 的其他功能，網站看不出差別）
unsafe fn gm_attach(core: &ICoreWebView2, sink: &Sink, bind: &str) {
    let mut token = 0i64;
    if !bind.is_empty() {
        if let Ok(recv) = core.GetDevToolsProtocolEventReceiver(&HSTRING::from("Runtime.bindingCalled")) {
            let (sink, name) = (sink.clone(), format!("\"{bind}\""));
            let h = DevToolsProtocolEventReceivedEventHandler::create(Box::new(move |c, args| {
                if let Some(args) = args {
                    let json = text(|p| args.ParameterObjectAsJson(p));
                    if json.contains(&name) {
                        let source = c.as_ref().map(|c| text(|p| c.Source(p))).unwrap_or_default();
                        sink(Ev::Gm { frame: 0, source, json, wrapped: true });
                    }
                }
                Ok(())
            }));
            let _ = recv.add_DevToolsProtocolEventReceived(&h, &mut token);
            let h2 = CallDevToolsProtocolMethodCompletedHandler::create(Box::new(|r, json| {
                crate::dlog!("[cdp] Runtime.addBinding → {:?} {}", r.is_ok(), json.chars().take(160).collect::<String>());
                Ok(())
            }));
            let params = format!("{{\"name\":\"{bind}\"}}");
            let _ = core.CallDevToolsProtocolMethod(&HSTRING::from("Runtime.addBinding"), &HSTRING::from(params), &h2);
        }
    }
    if let Ok(c4) = core.cast::<ICoreWebView2_4>() {
        let key = core.as_raw() as usize;
        let sink = sink.clone();
        let h = FrameCreatedEventHandler::create(Box::new(move |_c, args| {
            if let Some(frame) = args.and_then(|a| a.Frame().ok()) {
                hook_frame(key, &frame, sink.clone(), 0);
            }
            Ok(())
        }));
        let _ = c4.add_FrameCreated(&h, &mut token);
    }
}

/// 一個內嵌框架：接它傳來的訊息、記住它（要回訊息）、它裡面再有框架也一樣處理
unsafe fn hook_frame(key: usize, frame: &ICoreWebView2Frame, sink: Sink, depth: u8) {
    let Ok(f2) = frame.cast::<ICoreWebView2Frame2>() else { return };
    let mut id = 0u32;
    if let Ok(f5) = frame.cast::<ICoreWebView2Frame5>() {
        let _ = f5.FrameId(&mut id);
    }
    if id == 0 {
        return;
    }
    FRAMES.with(|m| m.borrow_mut().insert((key, id), f2.clone()));
    let mut token = 0i64;
    {
        let sink = sink.clone();
        let h = FrameWebMessageReceivedEventHandler::create(Box::new(move |_f, args| {
            if let Some(args) = args {
                let json = text(|p| args.WebMessageAsJson(p));
                if gm_like(&json) {
                    sink(Ev::Gm { frame: id, source: text(|p| args.Source(p)), json, wrapped: false });
                }
            }
            Ok(())
        }));
        let _ = f2.add_WebMessageReceived(&h, &mut token);
    }
    {
        let h = FrameDestroyedEventHandler::create(Box::new(move |_f, _| {
            FRAMES.with(|m| m.borrow_mut().remove(&(key, id)));
            Ok(())
        }));
        let _ = frame.add_Destroyed(&h, &mut token);
    }
    if depth < 6 {
        if let Ok(f7) = frame.cast::<ICoreWebView2Frame7>() {
            let h = FrameChildFrameCreatedEventHandler::create(Box::new(move |_f, args| {
                if let Some(child) = args.and_then(|a| a.Frame().ok()) {
                    hook_frame(key, &child, sink.clone(), depth + 1);
                }
                Ok(())
            }));
            let _ = f7.add_FrameCreated(&h, &mut token);
        }
    }
}

/// 傳訊息給網頁裡的使用者腳本。frame = 0 是最上層的網頁，其他是內嵌框架。回傳有沒有送出去
pub fn gm_post(w: &Wv, frame: u32, json: &str) -> bool {
    unsafe {
        if frame == 0 {
            return w.core.PostWebMessageAsJson(&HSTRING::from(json)).is_ok();
        }
        let key = w.core.as_raw() as usize;
        let f = FRAMES.with(|m| m.borrow().get(&(key, frame)).cloned());
        f.is_some_and(|f| f.PostWebMessageAsJson(&HSTRING::from(json)).is_ok())
    }
}

/// 這個網頁目前有哪些內嵌框架（廣播設定值變動時用）
pub fn gm_frames(w: &Wv) -> Vec<u32> {
    let key = w.core.as_raw() as usize;
    FRAMES.with(|m| m.borrow().keys().filter(|(k, _)| *k == key).map(|(_, f)| *f).collect())
}

/// 換掉這個網頁「每份文件一開始就執行」的那一小段程式（帶最新的設定值啟動使用者腳本）。
/// 之後載入的文件（含內嵌框架）才會用到新的；空字串 = 只拿掉舊的
pub fn gm_set_boot(w: &Wv, script: &str) {
    static SEQ: AtomicU64 = AtomicU64::new(0);
    let key = w.core.as_raw() as usize;
    let seq = SEQ.fetch_add(1, Ordering::SeqCst) + 1;
    unsafe {
        // 先記「現在是第幾次」：比較晚登記好的舊版要自己拿掉
        let old = GM_BOOT.with(|m| m.borrow_mut().insert(key, (seq, String::new())));
        if let Some((_, id)) = old {
            if !id.is_empty() {
                let _ = w.core.RemoveScriptToExecuteOnDocumentCreated(&HSTRING::from(id));
            }
        }
        if script.is_empty() {
            return;
        }
        let core = w.core.clone();
        let h = AddScriptToExecuteOnDocumentCreatedCompletedHandler::create(Box::new(move |r, id| {
            if r.is_err() || id.is_empty() {
                return Ok(());
            }
            let keep = GM_BOOT.with(|m| {
                let mut m = m.borrow_mut();
                match m.get_mut(&key) {
                    Some((cur, slot)) if *cur == seq => {
                        *slot = id.clone();
                        true
                    }
                    _ => false,
                }
            });
            if !keep {
                // 登記好的時候已經有更新的版本了：這一份不要
                let _ = core.RemoveScriptToExecuteOnDocumentCreated(&HSTRING::from(id));
            }
            Ok(())
        }));
        let _ = w.core.AddScriptToExecuteOnDocumentCreated(&HSTRING::from(script), &h);
    }
}

/// 網頁關掉了：把記著的東西清掉
pub fn gm_forget(w: &Wv) {
    let key = w.core.as_raw() as usize;
    FRAMES.with(|m| m.borrow_mut().retain(|(k, _), _| *k != key));
    GM_BOOT.with(|m| m.borrow_mut().remove(&key));
}

/* ---------------- 瀏覽器裡有哪些頁面（認出不是程式開的視窗） ---------------- */

/// 記下這個網頁自己的編號，並在瀏覽器多了一個頁面時通知（內容是 JSON 文字，由收到的人解讀）
unsafe fn targets_attach(core: &ICoreWebView2, sink: &Sink) {
    {
        let sink = sink.clone();
        let h = CallDevToolsProtocolMethodCompletedHandler::create(Box::new(move |r, json| {
            if r.is_ok() {
                sink(Ev::SelfTarget(json));
            }
            Ok(())
        }));
        let _ = core.CallDevToolsProtocolMethod(&HSTRING::from("Target.getTargetInfo"), &HSTRING::from("{}"), &h);
    }
    // 每個分頁都會收到整個瀏覽器的通知，數量很多：這裡只留「擴充功能的頁面」（其他的不用處理）
    for ev in ["Target.targetCreated", "Target.targetInfoChanged"] {
        if let Ok(recv) = core.GetDevToolsProtocolEventReceiver(&HSTRING::from(ev)) {
            let sink = sink.clone();
            let h = DevToolsProtocolEventReceivedEventHandler::create(Box::new(move |_c, args| {
                if let Some(args) = args {
                    let json = text(|p| args.ParameterObjectAsJson(p));
                    if json.contains("\"type\":\"page\"") && json.contains("\"url\":\"chrome-extension://") {
                        sink(Ev::Target(json));
                    }
                }
                Ok(())
            }));
            let mut token = 0i64;
            let _ = recv.add_DevToolsProtocolEventReceived(&h, &mut token);
        }
    }
    let h2 = CallDevToolsProtocolMethodCompletedHandler::create(Box::new(|r, json| {
        if r.is_err() {
            crate::dlog!("[cdp] Target.setDiscoverTargets 失敗 {}", json.chars().take(200).collect::<String>());
        }
        Ok(())
    }));
    let _ = core.CallDevToolsProtocolMethod(&HSTRING::from("Target.setDiscoverTargets"), &HSTRING::from("{\"discover\":true}"), &h2);
}

/// 關掉瀏覽器裡的一個頁面（擴充功能叫瀏覽器自己開的視窗：改開成分頁之後把那個視窗關掉）
pub fn close_target(w: &Wv, id: &str) {
    unsafe {
        let params = format!("{{\"targetId\":\"{}\"}}", id.replace(['"', '\\'], ""));
        let h = CallDevToolsProtocolMethodCompletedHandler::create(Box::new(|r, json| {
            crate::dlog!("[cdp] Target.closeTarget → {:?} {}", r.is_ok(), json.chars().take(120).collect::<String>());
            Ok(())
        }));
        let _ = w.core.CallDevToolsProtocolMethod(&HSTRING::from("Target.closeTarget"), &HSTRING::from(params), &h);
    }
}

/* ---------------- 視窗 ---------------- */

/// 選單小視窗：Windows 11 的圓角與深色的細邊框（Windows 10 沒有這兩個設定，會直接略過）
pub fn round_corners(hwnd: isize) {
    use windows_sys::Win32::Graphics::Dwm::{DwmSetWindowAttribute, DWMWA_BORDER_COLOR, DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_ROUND};
    let pref: i32 = DWMWCP_ROUND;
    // 顏色是 0x00BBGGRR：#444746
    let border: u32 = 0x0046_4744;
    unsafe {
        DwmSetWindowAttribute(hwnd as _, DWMWA_WINDOW_CORNER_PREFERENCE as u32, &pref as *const i32 as *const _, 4);
        DwmSetWindowAttribute(hwnd as _, DWMWA_BORDER_COLOR as u32, &border as *const u32 as *const _, 4);
    }
}
