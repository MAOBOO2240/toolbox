//! 小視窗：選單（可以一層一層展開）、網址列的建議清單、提示框。
//!
//! 每一層是一個置頂、不搶焦點的獨立視窗（可以超出瀏覽器視窗的範圍），內容由 web-menu.html 顯示：
//! 這裡決定內容與每一項要做什麼 → 小視窗量好自己需要多大（web_pop_size）→ 這裡算位置並顯示。
//! 滑鼠移到有下一層的項目（web_pop_hover）就開下一層；點了某一項（web_pop_pick）就執行。

use super::*;
use crate::webcore::{place_rect, CtxAct, CtxEntry, Place, Sug};
use crate::webev::{CtxTarget, NativeItem};
use std::sync::atomic::AtomicU64;

/// 最多幾層
pub const LEVELS: usize = 5;
/// 相鄰兩層重疊幾個像素（邏輯像素）
const OVERLAP: f64 = 3.0;

fn label(i: usize) -> String {
    if i == 0 {
        "webmenu".to_string()
    } else {
        format!("webmenu{i}")
    }
}

/// 呼叫指令的小視窗是第幾層
fn level_of(webview: &Webview) -> Option<usize> {
    let l = webview.label();
    if l == "webmenu" {
        return Some(0);
    }
    l.strip_prefix("webmenu")?.parse::<usize>().ok().filter(|n| *n > 0 && *n < LEVELS)
}

/* ---------------- 內容 ---------------- */

#[derive(Serialize, Clone, Default)]
pub struct Item {
    /// item（一般）/ sep（分隔線）/ head（標題）/ empty（沒有東西）/ info（說明文字）/
    /// zoom（縮放那一列）/ sug（網址列的建議）/ tab（分頁）/ ext（擴充功能）
    kind: &'static str,
    title: String,
    /// 圖示：內建圖示的名稱，或圖片網址（http、data:）
    icon: String,
    /// 次要文字（網址、說明）
    sub: String,
    /// 右邊的快捷鍵文字
    key: String,
    /// 不能按
    off: bool,
    /// 打勾
    check: bool,
    danger: bool,
    /// 有下一層
    more: bool,
    /// 數字（縮放的百分比）
    n: i64,
    /// 其他標記（建議的種類、擴充功能是否固定…）
    tag: String,
}

impl Item {
    fn new(title: impl Into<String>, icon: &str) -> Self {
        Item { kind: "item", title: title.into(), icon: icon.to_string(), ..Item::default() }
    }
    fn of(kind: &'static str, title: impl Into<String>) -> Self {
        Item { kind, title: title.into(), ..Item::default() }
    }
    fn key(mut self, k: &str) -> Self {
        self.key = k.to_string();
        self
    }
    fn sub(mut self, s: impl Into<String>) -> Self {
        self.sub = s.into();
        self
    }
    fn more(mut self) -> Self {
        self.more = true;
        self
    }
    fn check(mut self, on: bool) -> Self {
        self.check = on;
        self
    }
    fn off(mut self, off: bool) -> Self {
        self.off = off;
        self
    }
    fn danger(mut self) -> Self {
        self.danger = true;
        self
    }
    fn tag(mut self, t: impl Into<String>) -> Self {
        self.tag = t.into();
        self
    }
}

/// 下一層是什麼
#[derive(Clone)]
enum Sub {
    History,
    Bookmarks,
    BmFolder(String),
    BmOther,
    Extensions,
    Save,
    Tools,
    /// 網頁右鍵選單裡的子選單
    Entries(Vec<CtxEntry>),
}

/// 點了某一項要做的事
#[derive(Clone)]
enum Act {
    Nothing,
    /// 瀏覽器指令（針對打開選單時的分頁）
    Cmd(&'static str),
    /// 針對某個分頁的指令
    TabCmd(String, &'static str),
    Sub(Sub),
    /// 書籤（左鍵開啟、中鍵開新分頁、右鍵選單）
    Link(String),
    /// 開新分頁（true = 馬上切過去）
    NewTab(String, bool),
    /// 開新視窗（true = 無痕式）
    NewWindow(String, bool),
    BmRename(String),
    BmDelete(String),
    BmOpenAll(String),
    Ext(String),
    ExtPage(String),
    ExtToggle(String, bool),
    ExtPin(String),
    ExtSync,
    /// WebView2 右鍵選單的指令
    Native(i32),
    /// 重新開啟最近關閉的分頁（從新到舊第幾個）
    Reopen(usize),
    SelectTab(String),
    /// 上一頁／下一頁清單裡的某一頁
    NavEntry(i64),
    /// 換成從這個 Chrome 使用者同步
    Profile(String),
    SyncNow,
    /// 網址列的建議
    Sug(String),
    /// 縮放那一列（按鈕在 part 裡）
    Zoom,
}

#[derive(Default)]
struct Menu {
    items: Vec<Item>,
    acts: Vec<Act>,
}

impl Menu {
    fn add(&mut self, item: Item, act: Act) {
        self.items.push(item);
        self.acts.push(act);
    }
    fn sep(&mut self) {
        // 不要連續兩條、也不要放在最前面
        if self.items.last().is_some_and(|i| i.kind != "sep") {
            self.add(Item::of("sep", ""), Act::Nothing);
        }
    }
    fn head(&mut self, title: impl Into<String>) {
        self.add(Item::of("head", title), Act::Nothing);
    }
    fn empty(&mut self, title: impl Into<String>) {
        self.add(Item::of("empty", title), Act::Nothing);
    }
    fn trim(mut self) -> Self {
        while self.items.last().is_some_and(|i| i.kind == "sep") {
            self.items.pop();
            self.acts.pop();
        }
        self
    }
}

struct Level {
    gen: u64,
    /// menu / omni / bubble
    style: &'static str,
    /// 固定寬度（邏輯像素；0 = 照內容）
    width: f64,
    items: Vec<Item>,
    acts: Vec<Act>,
    place: Place,
    rect: (i32, i32, i32, i32),
    visible: bool,
    /// 是上一層的哪一項開出來的
    from: Option<usize>,
    /// 目前選到的項目（網址列的建議用方向鍵選）
    sel: i32,
}

struct Pop {
    owner: String,
    tid: String,
    /// 是哪個按鈕開的（同一個按鈕再按一次 = 關掉）
    key: String,
    levels: Vec<Level>,
    sf: f64,
    /// 點在這個範圍裡不算點到外面（網址列的建議：網址列本身）
    keep: Option<(i32, i32, i32, i32)>,
    /// 網頁右鍵選單：WebView2 正在等結果（編號）
    ctx: Option<u64>,
    shown_at: Instant,
}

static POP: Mutex<Option<Pop>> = Mutex::new(None);
/// 最近一次因為點到外面而關掉的（同一個按鈕再點一下＝關掉，不要又打開）
static CLOSED: Mutex<Option<(String, Instant)>> = Mutex::new(None);
static HWNDS: [AtomicIsize; LEVELS] = [const { AtomicIsize::new(0) }; LEVELS];
static GEN: AtomicU64 = AtomicU64::new(0);
static MAKE: Mutex<()> = Mutex::new(());

fn next_gen() -> u64 {
    GEN.fetch_add(1, Ordering::SeqCst) + 1
}

#[derive(Serialize, Clone)]
pub struct Model {
    gen: u64,
    level: usize,
    style: &'static str,
    width: f64,
    sel: i32,
    items: Vec<Item>,
}

/* ---------------- 視窗 ---------------- */

fn ensure_window(app: &AppHandle, i: usize) -> Result<(), String> {
    let name = label(i);
    if app.get_webview_window(&name).is_some() {
        return Ok(());
    }
    let _g = lock(&MAKE);
    if app.get_webview_window(&name).is_some() {
        return Ok(());
    }
    let w = WebviewWindowBuilder::new(app, &name, WebviewUrl::App("web-menu.html".into()))
        .title("瀏覽器選單")
        .decorations(false)
        .always_on_top(true)
        .skip_taskbar(true)
        .resizable(false)
        .maximizable(false)
        .minimizable(false)
        .focused(false)
        .focusable(false)
        .visible(false)
        .shadow(true)
        .background_color(Color(31, 31, 31, 255))
        .inner_size(280.0, 120.0)
        .build()
        .map_err(|e| format!("選單建立失敗：{e}"))?;
    #[cfg(windows)]
    if let Ok(h) = w.hwnd() {
        let h = h.0 as isize;
        HWNDS[i].store(h, Ordering::SeqCst);
        let _ = app.run_on_main_thread(move || {
            win32::subclass_menu(h);
            webnative::round_corners(h);
        });
    }
    #[cfg(not(windows))]
    let _ = w;
    Ok(())
}

/// 選單用的小視窗先建好（第一次按選單才不用等）
pub fn warm_up(app: &AppHandle) {
    for i in 0..2 {
        let _ = ensure_window(app, i);
    }
}

fn show_window(app: &AppHandle, i: usize, rect: (i32, i32, i32, i32)) {
    let (x, y, w, h) = (rect.0, rect.1, rect.2 - rect.0, rect.3 - rect.1);
    #[cfg(windows)]
    {
        let hwnd = HWNDS[i].load(Ordering::SeqCst);
        if hwnd != 0 {
            win32::show_at(hwnd, x, y, w, h);
            let _ = app;
            return;
        }
    }
    if let Some(win) = app.get_webview_window(&label(i)) {
        let _ = win.set_position(PhysicalPosition::new(x, y));
        let _ = win.set_size(PhysicalSize::new(w.max(1) as u32, h.max(1) as u32));
        let _ = win.show();
    }
}

fn hide_window(app: &AppHandle, i: usize) {
    #[cfg(windows)]
    {
        let hwnd = HWNDS[i].load(Ordering::SeqCst);
        if hwnd != 0 {
            win32::show(hwnd, false);
            let _ = app;
            return;
        }
    }
    if let Some(win) = app.get_webview_window(&label(i)) {
        let _ = win.hide();
    }
}

fn send(app: &AppHandle, i: usize) {
    let model = lock(&POP).as_ref().and_then(|p| {
        p.levels.get(i).map(|l| Model { gen: l.gen, level: i, style: l.style, width: l.width, sel: l.sel, items: l.items.clone() })
    });
    let Some(model) = model else { return };
    if ensure_window(app, i).is_err() {
        return;
    }
    let _ = app.emit_to(label(i).as_str(), "web-pop", model);
}

fn finish_ctx(app: &AppHandle, seq: u64, pick: Option<i32>) {
    let _ = app.run_on_main_thread(move || webnative::ctx_finish(seq, pick));
}

struct Spec {
    key: String,
    style: &'static str,
    width: f64,
    menu: Menu,
    place: Place,
    keep: Option<(i32, i32, i32, i32)>,
    ctx: Option<u64>,
}

/// 關掉目前開著的（不通知工具列「選單關了」以外的事）。回傳 (是誰的, 哪個按鈕)
fn take_current(app: &AppHandle, finish: bool) -> Option<(String, String)> {
    let old = lock(&POP).take()?;
    for i in 0..old.levels.len().min(LEVELS) {
        hide_window(app, i);
    }
    if finish {
        if let Some(seq) = old.ctx {
            finish_ctx(app, seq, None);
        }
    }
    Some((old.owner, old.key))
}

fn open(app: &AppHandle, owner: &str, tid: &str, spec: Spec) {
    // 新的也是網頁右鍵選單時，舊的那個 WebView2 已經自己結束了
    if let Some((old_owner, _)) = take_current(app, spec.ctx.is_none()) {
        let _ = app.emit_to(bar_label(&old_owner).as_str(), "web-menu-closed", old_owner.clone());
    }
    let sf = app.get_window(&win_label(owner)).and_then(|w| w.scale_factor().ok()).unwrap_or(1.0).max(0.5);
    let menu = spec.menu.trim();
    let level = Level {
        gen: next_gen(),
        style: spec.style,
        width: spec.width,
        items: menu.items,
        acts: menu.acts,
        place: spec.place,
        rect: (0, 0, 0, 0),
        visible: false,
        from: None,
        sel: -1,
    };
    *lock(&POP) = Some(Pop {
        owner: owner.to_string(),
        tid: tid.to_string(),
        key: spec.key,
        levels: vec![level],
        sf,
        keep: spec.keep,
        ctx: spec.ctx,
        shown_at: Instant::now(),
    });
    send(app, 0);
}

/// 全部關掉。outside = 因為點到外面才關的
pub fn hide_all(app: &AppHandle, outside: bool) {
    let Some((owner, key)) = take_current(app, true) else { return };
    if outside {
        *lock(&CLOSED) = Some((key, Instant::now()));
    }
    let _ = app.emit_to(bar_label(&owner).as_str(), "web-menu-closed", owner.clone());
}

/// 這個視窗的選單開著就關掉
pub fn hide_of(app: &AppHandle, wid: &str) {
    if open_for(wid) {
        hide_all(app, false);
    }
}

/// 這個視窗有選單開著
pub fn open_for(wid: &str) -> bool {
    lock(&POP).as_ref().is_some_and(|p| p.owner == wid)
}

/// 關掉第 keep 層之後的
fn close_deeper(app: &AppHandle, keep: usize) {
    let n = {
        let mut g = lock(&POP);
        let Some(p) = g.as_mut() else { return };
        let n = p.levels.len();
        p.levels.truncate(keep + 1);
        n
    };
    for i in (keep + 1)..n.min(LEVELS) {
        hide_window(app, i);
    }
}

/// 在第 from 層後面再開一層
fn push_level(app: &AppHandle, from: usize, item: Option<usize>, menu: Menu, place: Place) {
    // 已經是最後一層：換掉最後一層
    let at = (from + 1).min(LEVELS - 1);
    close_deeper(app, at - 1);
    let menu = menu.trim();
    {
        let mut g = lock(&POP);
        let Some(p) = g.as_mut() else { return };
        if p.levels.len() != at {
            return;
        }
        p.levels.push(Level {
            gen: next_gen(),
            style: "menu",
            width: 0.0,
            items: menu.items,
            acts: menu.acts,
            place,
            rect: (0, 0, 0, 0),
            visible: false,
            from: item,
            sel: -1,
        });
    }
    send(app, at);
}

/// 換掉某一層的內容（位置不變）
fn replace_level(app: &AppHandle, level: usize, menu: Menu) {
    let menu = menu.trim();
    {
        let mut g = lock(&POP);
        let Some(l) = g.as_mut().and_then(|p| p.levels.get_mut(level)) else { return };
        l.items = menu.items;
        l.acts = menu.acts;
        l.gen = next_gen();
    }
    send(app, level);
}

fn work_area(app: &AppHandle, owner: &str, x: i32, y: i32) -> Option<(i32, i32, i32, i32)> {
    let win = app.get_window(&win_label(owner))?;
    let m = win.monitor_from_point(x as f64 + 1.0, y as f64 + 1.0).ok().flatten().or_else(|| win.current_monitor().ok().flatten())?;
    let wa = m.work_area();
    Some((wa.position.x, wa.position.y, wa.position.x + wa.size.width as i32, wa.position.y + wa.size.height as i32))
}

/* ---------------- 選單的內容 ---------------- */

/// 書籤的圖示：逛過記下來的；沒有就試網站的 favicon.ico（載入失敗時選單會改用地球圖示）
fn bm_icon(url: &str, icons: &HashMap<String, String>) -> String {
    let Some(host) = host_of(url) else { return "globe".to_string() };
    icons.get(&host).filter(|i| !i.is_empty()).cloned().unwrap_or_else(|| format!("https://{host}/favicon.ico"))
}

fn bm_menu(list: &[Bm], icons: &HashMap<String, String>) -> Menu {
    let mut m = Menu::default();
    if list.is_empty() {
        m.empty("（空白）");
        return m;
    }
    for b in list {
        if b.url.is_empty() {
            m.add(Item::new(b.title.clone(), "folder").more(), Act::Sub(Sub::BmFolder(b.id.clone())));
        } else {
            let title = if b.title.trim().is_empty() { webcore::display_url(&b.url) } else { b.title.clone() };
            m.add(Item::new(title, &bm_icon(&b.url, icons)).sub(b.url.clone()).tag("bm"), Act::Link(b.id.clone()));
        }
    }
    m
}

/// 書籤的右鍵選單。shown = 書籤列目前有沒有顯示
/// （要由呼叫的人先查好傳進來：這個函式會在鎖著書籤資料時被呼叫，裡面不可以再讀設定）
fn bm_ctx(b: &Bm, shown: bool) -> Menu {
    let mut m = Menu::default();
    let id = b.id.clone();
    if b.url.is_empty() {
        let n = count_links(&b.children);
        m.add(Item::new(format!("全部開啟（{n} 個）"), "tab").off(n == 0), Act::BmOpenAll(id.clone()));
        m.sep();
        m.add(Item::new("重新命名…", "edit"), Act::BmRename(id.clone()));
        m.add(Item::new(format!("刪除（含 {n} 個書籤）"), "trash").danger(), Act::BmDelete(id));
    } else {
        m.add(Item::new("在新分頁中開啟", "tab"), Act::NewTab(b.url.clone(), true));
        m.add(Item::new("在新視窗中開啟", "window"), Act::NewWindow(b.url.clone(), false));
        m.add(Item::new("在無痕式視窗中開啟", "incognito"), Act::NewWindow(b.url.clone(), true));
        m.sep();
        m.add(Item::new("重新命名…", "edit"), Act::BmRename(id.clone()));
        m.add(Item::new("刪除", "trash").danger(), Act::BmDelete(id));
    }
    m.sep();
    m.add(Item::new("顯示書籤列", "").check(shown).key("Ctrl+Shift+B"), Act::Cmd("bm.bar"));
    m.add(Item::new("書籤管理員", "").key("Ctrl+Shift+O"), Act::Cmd("bm.manager"));
    m
}

/// 某個書籤（或資料夾）的右鍵選單
fn bm_ctx_of(id: &str) -> Option<Menu> {
    let shown = show_bm();
    bm_read(|b| bm_find_any(b, id).map(|x| bm_ctx(x, shown)))
}

/// 書籤列空白處的右鍵選單
fn bmbar_menu() -> Menu {
    let mut m = Menu::default();
    m.add(Item::new("顯示書籤列", "").check(show_bm()).key("Ctrl+Shift+B"), Act::Cmd("bm.bar"));
    m.add(Item::new("書籤管理員", "").key("Ctrl+Shift+O"), Act::Cmd("bm.manager"));
    m
}

fn closed_items(m: &mut Menu, limit: usize) {
    let closed = st().file.closed.clone();
    for (i, c) in closed.iter().rev().take(limit).enumerate() {
        let title = if c.title.trim().is_empty() { webcore::display_url(&c.url) } else { c.title.clone() };
        let icon = if c.icon.is_empty() { "globe".to_string() } else { c.icon.clone() };
        let mut it = Item::new(title, &icon).sub(c.url.clone());
        if i == 0 {
            it = it.key("Ctrl+Shift+T");
        }
        m.add(it, Act::Reopen(i));
    }
}

fn main_menu(wid: &str, tid: &str) -> Menu {
    let incognito = is_incognito(wid);
    let pct = webcore::zoom_pct(tab_zoom(wid, tid)) as i64;
    let full = rt(|m| m.get(wid).is_some_and(|r| r.fs));
    let mut m = Menu::default();
    m.add(Item::new("新分頁", "tab").key("Ctrl+T"), Act::Cmd("tab.new"));
    m.add(Item::new("新視窗", "window").key("Ctrl+N"), Act::Cmd("win.new"));
    m.add(Item::new("新增無痕式視窗", "incognito").key("Ctrl+Shift+N"), Act::Cmd("win.incognito"));
    m.sep();
    if !incognito {
        m.add(Item::new("歷史記錄", "history").more(), Act::Sub(Sub::History));
    }
    m.add(Item::new("下載", "download").key("Ctrl+J"), Act::Cmd("page.downloads"));
    m.add(Item::new("書籤和清單", "star").more(), Act::Sub(Sub::Bookmarks));
    m.add(Item::new("擴充功能", "puzzle").more(), Act::Sub(Sub::Extensions));
    m.add(Item::new("刪除瀏覽資料…", "trash").key("Ctrl+Shift+Del"), Act::Cmd("page.cleardata"));
    m.sep();
    m.add(Item { kind: "zoom", title: "縮放".into(), icon: "zoom".into(), n: pct, check: full, ..Item::default() }, Act::Zoom);
    m.sep();
    m.add(Item::new("列印…", "print").key("Ctrl+P"), Act::Cmd("print"));
    m.add(Item::new("尋找…", "find").key("Ctrl+F"), Act::Cmd("find"));
    m.add(Item::new("儲存及分享", "share").more(), Act::Sub(Sub::Save));
    m.add(Item::new("更多工具", "tools").more(), Act::Sub(Sub::Tools));
    m.sep();
    m.add(Item::new("設定", "gear"), Act::Cmd("page.settings"));
    m.add(Item::new("結束", "exit"), Act::Cmd("exit"));
    m
}

fn sub_menu(wid: &str, tid: &str, sub: &Sub) -> Menu {
    let mut m = Menu::default();
    match sub {
        Sub::History => {
            m.add(Item::new("歷史記錄", "history").key("Ctrl+H"), Act::Cmd("page.history"));
            m.sep();
            m.head("最近關閉的分頁");
            let before = m.items.len();
            closed_items(&mut m, 8);
            if m.items.len() == before {
                m.empty("沒有最近關閉的分頁");
            }
        }
        Sub::Bookmarks => {
            let web = tab_of(wid, tid).is_some_and(|t| parse_web(&t.url).is_some());
            m.add(Item::new("將這個分頁加入書籤…", "star").key("Ctrl+D").off(!web), Act::Cmd("bm.add"));
            m.add(Item::new("將所有分頁加入書籤…", ""), Act::Cmd("bm.all_tabs"));
            m.sep();
            m.add(Item::new("顯示書籤列", "").check(show_bm()).key("Ctrl+Shift+B"), Act::Cmd("bm.bar"));
            m.add(Item::new("書籤管理員", "").key("Ctrl+Shift+O"), Act::Cmd("bm.manager"));
            m.sep();
            let (bar, other, icons) = bm_read(|b| (b.bar.clone(), !b.other.is_empty(), b.icons.clone()));
            let list = bm_menu(&bar, &icons);
            if !bar.is_empty() {
                for (item, act) in list.items.into_iter().zip(list.acts) {
                    m.add(item, act);
                }
            }
            if other {
                m.sep();
                m.add(Item::new("所有書籤", "folder").more(), Act::Sub(Sub::BmOther));
            }
        }
        Sub::BmFolder(id) => {
            m = bm_read(|b| bm_find_any(b, id).map(|f| bm_menu(&f.children, &b.icons))).unwrap_or_default();
        }
        Sub::BmOther => {
            m = bm_read(|b| bm_menu(&b.other, &b.icons));
        }
        Sub::Extensions => {
            m.add(Item::new("管理擴充功能", "puzzle"), Act::Cmd("page.extensions"));
            m.add(Item::new("從 Chrome 重新同步", "sync"), Act::ExtSync);
        }
        Sub::Save => {
            let web = tab_of(wid, tid).is_some_and(|t| parse_web(&t.url).is_some());
            m.add(Item::new("另存網頁為…", "save").key("Ctrl+S").off(!web), Act::Cmd("save"));
            m.sep();
            m.add(Item::new("複製連結", "link").off(!web), Act::Cmd("copy.url"));
            m.add(Item::new("用 Chrome 開啟", "open").off(!web), Act::Cmd("tab.chrome"));
        }
        Sub::Tools => {
            let web = tab_of(wid, tid).is_some_and(|t| parse_web(&t.url).is_some());
            m.add(Item::new("工作管理員", "task").key("Shift+Esc"), Act::Cmd("taskmgr"));
            m.add(Item::new("檢視網頁原始碼", "code").key("Ctrl+U").off(!web), Act::Cmd("source"));
            m.sep();
            m.add(Item::new("清除這個網站的資料（Cookie、登入狀態）", "trash").danger().off(!web), Act::Cmd("site.clear"));
        }
        Sub::Entries(list) => m = ctx_menu(list),
    }
    m
}

/// 分頁的右鍵選單
fn tab_menu(wid: &str, tid: &str) -> Menu {
    let mut m = Menu::default();
    let Some(w) = get(wid) else { return m };
    let Some(i) = w.tabs.iter().position(|t| t.id == tid) else { return m };
    let tab = &w.tabs[i];
    let t = tid.to_string();
    let web = parse_web(&tab.url).is_some();
    let others = w.tabs.iter().any(|x| x.id != tid && !x.pinned);
    let right = w.tabs.iter().skip(i + 1).any(|x| !x.pinned);
    m.add(Item::new("在右側開啟新分頁", ""), Act::TabCmd(t.clone(), "tab.new_right"));
    m.add(Item::new("將分頁移至新視窗", "").off(w.tabs.len() < 2), Act::TabCmd(t.clone(), "tab.to_window"));
    m.sep();
    m.add(Item::new("重新載入", "").key("Ctrl+R"), Act::TabCmd(t.clone(), "tab.reload"));
    m.add(Item::new("複製", ""), Act::TabCmd(t.clone(), "tab.dup"));
    m.add(Item::new(if tab.pinned { "取消固定" } else { "固定" }, ""), Act::TabCmd(t.clone(), "tab.pin"));
    m.add(Item::new(if tab.muted { "將網站取消靜音" } else { "將網站設為靜音" }, ""), Act::TabCmd(t.clone(), "tab.mute"));
    m.sep();
    m.add(Item::new("關閉", "").key("Ctrl+W"), Act::TabCmd(t.clone(), "tab.close"));
    m.add(Item::new("關閉其他分頁", "").off(!others), Act::TabCmd(t.clone(), "tab.close_others"));
    m.add(Item::new("關閉右側分頁", "").off(!right), Act::TabCmd(t.clone(), "tab.close_right"));
    m.sep();
    m.add(Item::new("用 Chrome 開啟", "").off(!web), Act::TabCmd(t.clone(), "tab.chrome"));
    m.add(Item::new("清除這個網站的資料（Cookie、登入狀態）", "").danger().off(!web), Act::TabCmd(t, "site.clear"));
    m
}

/// 分頁列空白處的右鍵選單
fn strip_menu() -> Menu {
    let mut m = Menu::default();
    let closed = !st().file.closed.is_empty();
    m.add(Item::new("新分頁", "").key("Ctrl+T"), Act::Cmd("tab.new"));
    m.add(Item::new("重新開啟已關閉的分頁", "").key("Ctrl+Shift+T").off(!closed), Act::Cmd("tab.reopen"));
    m.add(Item::new("將所有分頁加入書籤…", ""), Act::Cmd("bm.all_tabs"));
    m.sep();
    m.add(Item::new("工作管理員", "").key("Shift+Esc"), Act::Cmd("taskmgr"));
    m
}

/// 工具列的「擴充功能」按鈕
fn ext_menu() -> Menu {
    let mut m = Menu::default();
    m.head("擴充功能");
    let list = ext_list();
    if list.is_empty() {
        m.add(Item::of("info", "還沒有擴充功能。在電腦的 Chrome 裡安裝後，這裡每次開啟都會自動同步過來。"), Act::Nothing);
    }
    for e in &list {
        let sub = if e.off { "已停用" } else if e.popup.is_empty() && e.options.is_empty() { "自動執行" } else { "" };
        let item = Item { kind: "ext", title: e.name.clone(), icon: e.icon.clone(), sub: sub.into(), off: e.off, check: e.pinned, ..Item::default() };
        m.add(item, Act::Ext(e.id.clone()));
    }
    m.sep();
    m.add(Item::new("管理擴充功能", "gear"), Act::Cmd("page.extensions"));
    m
}

/// 擴充功能的右鍵選單：選項、固定、停用／啟用
fn ext_ctx(id: &str) -> Option<Menu> {
    let e = ext_list().into_iter().find(|e| e.id == id)?;
    let mut m = Menu::default();
    m.add(Item::new(e.name.clone(), "").off(true), Act::Nothing);
    m.sep();
    if !e.popup.is_empty() {
        m.add(Item::new("開啟擴充功能選單", ""), Act::ExtPage(e.popup.clone()));
    }
    if !e.options.is_empty() {
        m.add(Item::new("選項", ""), Act::ExtPage(e.options.clone()));
    }
    if !e.off {
        m.add(Item::new(if e.pinned { "取消固定" } else { "固定" }, ""), Act::ExtPin(e.id.clone()));
    }
    m.sep();
    if e.off {
        m.add(Item::new("啟用（重新開啟視窗後生效）", ""), Act::ExtToggle(e.id.clone(), false));
    } else {
        m.add(Item::new("停用（重新開啟視窗後生效）", "").danger(), Act::ExtToggle(e.id.clone(), true));
    }
    m.add(Item::new("管理擴充功能", ""), Act::Cmd("page.extensions"));
    Some(m)
}

/// 網址列左邊的網站資訊
fn site_menu(wid: &str, tid: &str) -> Menu {
    let mut m = Menu::default();
    let Some(tab) = tab_of(wid, tid) else { return m };
    match url_kind(&tab.url) {
        "secure" => {
            m.head(webcore::host_of(&tab.url));
            m.add(Item::new("已建立安全連線", "lock").off(true).sub("你傳送給這個網站的資訊（例如密碼）不會外洩。"), Act::Nothing);
        }
        "insecure" => {
            m.head(webcore::host_of(&tab.url));
            m.add(Item::new("你與這個網站的連線不安全", "warn").off(true).sub("請勿在這個網站輸入密碼、信用卡這類機密資訊。"), Act::Nothing);
        }
        "ext" => {
            m.head("擴充功能的頁面");
            m.add(Item::new("管理擴充功能", "puzzle"), Act::Cmd("page.extensions"));
            return m;
        }
        _ => {
            m.head("瀏覽器");
            m.add(Item::new("這是瀏覽器本身的頁面，內容都在這台電腦上", "lock").off(true), Act::Nothing);
            return m;
        }
    }
    m.sep();
    m.add(Item::new("複製網址", "link"), Act::Cmd("copy.url"));
    m.add(Item::new("用 Chrome 開啟", "open"), Act::Cmd("tab.chrome"));
    m.add(Item::new("清除這個網站的資料（Cookie、登入狀態）", "trash").danger(), Act::Cmd("site.clear"));
    m
}

/// 右上角的人像：書籤與擴充功能從哪個 Chrome 使用者同步
fn profile_menu() -> Menu {
    let mut m = Menu::default();
    m.head("同步來源：這台電腦的 Chrome");
    let chosen = bm_read(|b| b.profile.clone());
    match find_profiles() {
        Ok((_, list)) => {
            let cur = pick_profile(&list, &chosen).map(|p| p.dir);
            for p in &list {
                let sub = if p.account.is_empty() { format!("{} 個書籤", p.count) } else { format!("{}・{} 個書籤", p.account, p.count) };
                m.add(Item::new(p.name.clone(), "user").sub(sub).check(cur.as_deref() == Some(p.dir.as_str())), Act::Profile(p.dir.clone()));
            }
        }
        Err(e) => m.add(Item::of("info", e), Act::Nothing),
    }
    m.sep();
    m.add(Item::of("info", "書籤與擴充功能每次開啟瀏覽器都會自動同步。"), Act::Nothing);
    m.add(Item::new("立即同步", "sync"), Act::SyncNow);
    m
}

/// 星號：書籤的提示框
fn bookmark_menu(wid: &str) -> Menu {
    let mut m = Menu::default();
    let Some(url) = get(wid).and_then(|w| w.tabs.into_iter().find(|t| t.id == w.active)).map(|t| t.url) else { return m };
    let found = bm_read(|b| {
        let id = bm_url_id(&b.bar, &url).or_else(|| bm_url_id(&b.other, &url))?;
        bm_find_any(b, &id).map(|x| (id.clone(), x.title.clone()))
    });
    let Some((id, title)) = found else {
        m.add(Item::new("將這個分頁加入書籤", "star").key("Ctrl+D"), Act::Cmd("bm.add"));
        return m;
    };
    m.head("已加入書籤");
    m.add(Item::new(title, "star-on").sub(webcore::display_url(&url)).off(true), Act::Nothing);
    m.sep();
    m.add(Item::new("重新命名…", "edit"), Act::BmRename(id));
    m.add(Item::new("書籤管理員", "folder"), Act::Cmd("bm.manager"));
    m.add(Item::new("移除", "trash").danger(), Act::Cmd("bm.remove"));
    m
}

/// 縮放的提示框
fn zoom_menu(wid: &str, tid: &str) -> Menu {
    let mut m = Menu::default();
    let pct = webcore::zoom_pct(tab_zoom(wid, tid)) as i64;
    m.add(Item { kind: "zoom", title: String::new(), n: pct, tag: "bubble".into(), ..Item::default() }, Act::Zoom);
    m
}

/// 分頁列左邊的「搜尋分頁」
fn tabs_menu(wid: &str) -> Menu {
    let mut m = Menu::default();
    let Some(w) = get(wid) else { return m };
    m.head("開啟的分頁");
    for t in &w.tabs {
        let title = if t.title.trim().is_empty() { webcore::display_url(&t.url) } else { t.title.clone() };
        let title = if title.is_empty() { "新分頁".to_string() } else { title };
        let icon = if t.icon.is_empty() { "globe".to_string() } else { t.icon.clone() };
        let item = Item { kind: "tab", title, icon, sub: webcore::host_of(&t.url), check: t.id == w.active, ..Item::default() };
        m.add(item, Act::SelectTab(t.id.clone()));
    }
    if !w.incognito && !st().file.closed.is_empty() {
        m.sep();
        m.head("最近關閉");
        closed_items(&mut m, 8);
    }
    m
}

/// 網頁的右鍵選單（項目已經排成 Chrome 的樣子）
fn ctx_menu(list: &[CtxEntry]) -> Menu {
    let mut m = Menu::default();
    for e in list {
        if e.sep {
            m.sep();
            continue;
        }
        let item = Item::new(e.title.clone(), "").key(&e.shortcut).off(!e.enabled).check(e.checked);
        if !e.children.is_empty() {
            m.add(item.more(), Act::Sub(Sub::Entries(e.children.clone())));
            continue;
        }
        let act = match &e.act {
            CtxAct::Native(id) => Act::Native(*id),
            CtxAct::OpenTab(u) => Act::NewTab(u.clone(), false),
            CtxAct::OpenWindow(u) => Act::NewWindow(u.clone(), false),
            CtxAct::OpenIncognito(u) => Act::NewWindow(u.clone(), true),
            CtxAct::Search(text) => Act::NewTab(webcore::search_url(engine(), text), true),
            CtxAct::ViewSource(u) => Act::NewTab(format!("view-source:{u}"), true),
            CtxAct::None => Act::Nothing,
        };
        m.add(item, act);
    }
    m
}

/// 上一頁／下一頁按鈕的清單（按右鍵）
fn nav_menu(json: &str, back: bool) -> Menu {
    let mut m = Menu::default();
    let v: Value = serde_json::from_str(json).unwrap_or_default();
    let cur = v["currentIndex"].as_u64().unwrap_or(0) as usize;
    let entries: Vec<(i64, String, String)> = v["entries"]
        .as_array()
        .map(|a| a.iter().map(|e| (e["id"].as_i64().unwrap_or(0), e["url"].as_str().unwrap_or("").to_string(), e["title"].as_str().unwrap_or("").to_string())).collect())
        .unwrap_or_default();
    let picked: Vec<&(i64, String, String)> =
        if back { entries.iter().take(cur).rev().take(12).collect() } else { entries.iter().skip(cur + 1).take(12).collect() };
    for (id, url, title) in picked {
        // 內部頁面顯示成 chrome://…
        let shown = Url::parse(url).ok().and_then(|u| tab_url(&u)).unwrap_or_else(|| url.clone());
        let page = webcore::virtual_page(&shown);
        let title = match page {
            Some(p) => webcore::ui_title(p).to_string(),
            None if title.trim().is_empty() => webcore::display_url(&shown),
            None => title.clone(),
        };
        let icon = hist_read(|h| h.icon_for(&shown)).filter(|i| !i.is_empty()).unwrap_or_else(|| "globe".to_string());
        m.add(Item::new(title, &icon).sub(shown), Act::NavEntry(*id));
    }
    if m.items.is_empty() {
        m.empty("沒有記錄");
    }
    m.sep();
    m.add(Item::new("顯示完整記錄", "history").key("Ctrl+H"), Act::Cmd("page.history"));
    m
}

/* ---------------- 開選單 ---------------- */

/// 網頁按了右鍵（WebView2 通知的）
pub fn open_context(app: &AppHandle, wid: &str, tid: &str, seq: u64, target: CtxTarget, items: Vec<NativeItem>) {
    let entries = webcore::build_context(&target, &items, engine(), is_incognito(wid));
    let pos = app.get_window(&win_label(wid)).and_then(|w| w.cursor_position().ok());
    let (Some(pos), false) = (pos, entries.is_empty()) else {
        // 顯示不了自己的選單：交還給 WebView2 顯示原本的
        let _ = app.run_on_main_thread(move || webnative::ctx_default(seq));
        return;
    };
    if ensure_window(app, 0).is_err() {
        let _ = app.run_on_main_thread(move || webnative::ctx_default(seq));
        return;
    }
    let spec = Spec {
        key: "ctx".into(),
        style: "menu",
        width: 0.0,
        menu: ctx_menu(&entries),
        place: Place::Cursor(pos.x.round() as i32, pos.y.round() as i32),
        keep: None,
        ctx: Some(seq),
    };
    open(app, wid, tid, spec);
}

/// 打開選單。kind：main（⋮）、tab、strip、folder、other、overflow、bm、bmbar、ext、extpin、site、profile、
/// bookmark、zoom、tabs、back、forward。rect = 按鈕在工具列裡的位置（左、上、右、下）；沒給就出現在滑鼠的位置
#[tauri::command]
pub async fn web_menu_open(
    app: AppHandle,
    webview: Webview,
    kind: String,
    key: String,
    target: Option<String>,
    ids: Option<Vec<String>>,
    rect: Option<[f64; 4]>,
) -> Result<(), String> {
    let wid = id_of(&webview).ok_or("不是瀏覽器視窗")?;
    // 同一個按鈕再點一下：關掉
    let same_open = lock(&POP).as_ref().is_some_and(|p| p.key == key && p.owner == wid);
    if same_open && kind != "zoom" {
        hide_all(&app, false);
        return Ok(());
    }
    let just_closed = lock(&CLOSED).as_ref().is_some_and(|(k, t)| *k == key && t.elapsed() < Duration::from_millis(400));
    if just_closed && rect.is_some() {
        return Ok(());
    }
    let w = get(&wid).ok_or("找不到這個視窗")?;
    let tid = w.active.clone();
    let target = target.unwrap_or_default();
    let win = app.get_window(&win_label(&wid)).ok_or("找不到這個視窗")?;
    let sf = win.scale_factor().unwrap_or(1.0).max(0.5);
    let place = match rect {
        Some([l, t, r, b]) => {
            let p = win.inner_position().map_err(|e| e.to_string())?;
            let f = |v: f64| (v * sf).round() as i32;
            let right = matches!(kind.as_str(), "main" | "ext" | "profile" | "bookmark" | "zoom" | "other");
            Place::Below { rect: (p.x + f(l), p.y + f(t), p.x + f(r), p.y + f(b)), right }
        }
        None => {
            let c = win.cursor_position().map_err(|e| e.to_string())?;
            Place::Cursor(c.x.round() as i32, c.y.round() as i32)
        }
    };
    // 上一頁／下一頁的清單要先問 WebView2
    if kind == "back" || kind == "forward" {
        let Some(page) = active_page(&app, &wid) else { return Ok(()) };
        let (app2, wid2, tid2, back) = (app.clone(), wid.clone(), tid.clone(), kind == "back");
        with_wv(&page, move |wv| {
            webnative::nav_entries(
                wv,
                Box::new(move |json| {
                    std::thread::spawn(move || {
                        let spec = Spec { key, style: "menu", width: 0.0, menu: nav_menu(&json, back), place, keep: None, ctx: None };
                        open(&app2, &wid2, &tid2, spec);
                    });
                }),
            );
        });
        return Ok(());
    }
    let (menu, style, tid) = match kind.as_str() {
        "main" => (main_menu(&wid, &tid), "menu", tid),
        "tab" => (tab_menu(&wid, &target), "menu", target.clone()),
        "strip" => (strip_menu(), "menu", tid),
        "folder" => (sub_menu(&wid, &tid, &Sub::BmFolder(target.clone())), "menu", tid),
        "other" => (sub_menu(&wid, &tid, &Sub::BmOther), "menu", tid),
        "overflow" => {
            let ids = ids.unwrap_or_default();
            let m = bm_read(|b| {
                let list: Vec<Bm> = b.bar.iter().filter(|x| ids.contains(&x.id)).cloned().collect();
                bm_menu(&list, &b.icons)
            });
            (m, "menu", tid)
        }
        "bm" => match bm_ctx_of(&target) {
            Some(m) => (m, "menu", tid),
            None => return Ok(()),
        },
        "bmbar" => (bmbar_menu(), "menu", tid),
        "ext" => (ext_menu(), "bubble", tid),
        "extpin" => match ext_ctx(&target) {
            Some(m) => (m, "menu", tid),
            None => return Ok(()),
        },
        "site" => (site_menu(&wid, &tid), "bubble", tid),
        "profile" => (profile_menu(), "bubble", tid),
        "bookmark" => (bookmark_menu(&wid), "bubble", tid),
        "zoom" => (zoom_menu(&wid, &tid), "bubble", tid),
        "tabs" => (tabs_menu(&wid), "bubble", tid),
        _ => return Ok(()),
    };
    if menu.items.is_empty() {
        return Ok(());
    }
    // 縮放的提示框已經開著：只更新數字，不要整個重開
    if kind == "zoom" && same_open {
        replace_level(&app, 0, menu);
    } else {
        open(&app, &wid, &tid, Spec { key: key.clone(), style, width: 0.0, menu, place, keep: None, ctx: None });
    }
    // 縮放的提示框：過一下自己消失（滑鼠在上面就先不消失）
    if kind == "zoom" {
        let app2 = app.clone();
        let stamp = next_gen();
        ZOOM_STAMP.store(stamp, Ordering::SeqCst);
        std::thread::spawn(move || loop {
            std::thread::sleep(Duration::from_millis(1800));
            if ZOOM_STAMP.load(Ordering::SeqCst) != stamp {
                return;
            }
            let rect = lock(&POP).as_ref().filter(|p| p.key == "zoom").and_then(|p| p.levels.first().map(|l| l.rect));
            let Some(_rect) = rect else { return };
            #[cfg(windows)]
            {
                let (x, y) = win32::cursor();
                if x >= _rect.0 && x < _rect.2 && y >= _rect.1 && y < _rect.3 {
                    continue;
                }
            }
            hide_all(&app2, false);
            return;
        });
    }
    Ok(())
}

static ZOOM_STAMP: AtomicU64 = AtomicU64::new(0);

/// 小視窗載入好了：拿目前要顯示的內容
#[tauri::command]
pub fn web_pop_ready(webview: Webview) -> Option<Model> {
    let i = level_of(&webview)?;
    let g = lock(&POP);
    let l = g.as_ref()?.levels.get(i)?;
    Some(Model { gen: l.gen, level: i, style: l.style, width: l.width, sel: l.sel, items: l.items.clone() })
}

/// 小視窗量好自己需要多大（邏輯像素）：算位置並顯示
#[tauri::command]
pub async fn web_pop_size(app: AppHandle, webview: Webview, gen: u64, w: f64, h: f64) {
    let Some(i) = level_of(&webview) else { return };
    let info = lock(&POP).as_ref().and_then(|p| p.levels.get(i).filter(|l| l.gen == gen).map(|l| (p.owner.clone(), p.sf, l.place.clone())));
    let Some((owner, sf, place)) = info else { return };
    let (pw, ph) = ((w * sf).ceil() as i32, (h * sf).ceil() as i32);
    let (ax, ay) = match &place {
        Place::Below { rect, .. } => (rect.0, rect.1),
        Place::Cursor(x, y) => (*x, *y),
        Place::Side { parent, top } => (parent.0, *top),
    };
    let rect = place_rect(&place, pw.max(8), ph.max(8), work_area(&app, &owner, ax, ay), (OVERLAP * sf).round() as i32);
    {
        let mut g = lock(&POP);
        let Some(l) = g.as_mut().and_then(|p| p.levels.get_mut(i)).filter(|l| l.gen == gen) else { return };
        l.rect = rect;
        l.visible = true;
    }
    show_window(&app, i, rect);
    // 換到縮放比例不同的螢幕時，Windows 會自動放大縮小一次：再設回正確的大小
    std::thread::sleep(Duration::from_millis(60));
    let still = lock(&POP).as_ref().and_then(|p| p.levels.get(i)).is_some_and(|l| l.gen == gen && l.visible && l.rect == rect);
    if still {
        show_window(&app, i, rect);
    }
}

/// 滑鼠停在某一項上：有下一層就開，沒有就把更深的關掉。rect = 那一項在小視窗裡的位置（邏輯像素）
#[tauri::command]
pub async fn web_pop_hover(app: AppHandle, webview: Webview, gen: u64, index: usize, rect: [f64; 4]) {
    let Some(i) = level_of(&webview) else { return };
    let info = {
        let g = lock(&POP);
        let Some(p) = g.as_ref() else { return };
        let Some(l) = p.levels.get(i).filter(|l| l.gen == gen && l.visible) else { return };
        let Some(act) = l.acts.get(index).cloned() else { return };
        let open_from = p.levels.get(i + 1).and_then(|n| n.from);
        (p.owner.clone(), p.tid.clone(), p.sf, l.rect, act, open_from)
    };
    let (owner, tid, sf, parent, act, open_from) = info;
    match act {
        Act::Sub(sub) => {
            if open_from == Some(index) {
                return;
            }
            let menu = sub_menu(&owner, &tid, &sub);
            // 跟上一層的項目對齊（上面有 8 像素的留白）
            let top = parent.1 + ((rect[1] - 8.0) * sf).round() as i32;
            push_level(&app, i, Some(index), menu, Place::Side { parent, top });
        }
        _ => close_deeper(&app, i),
    }
}

/// 網址列用方向鍵選建議
#[tauri::command]
pub fn web_omni_select(app: AppHandle, webview: Webview, index: i32) {
    let Some(wid) = id_of(&webview) else { return };
    let gen = {
        let mut g = lock(&POP);
        let Some(p) = g.as_mut().filter(|p| p.key == "omni" && p.owner == wid) else { return };
        let Some(l) = p.levels.first_mut() else { return };
        l.sel = index;
        l.gen
    };
    let _ = app.emit_to(label(0).as_str(), "web-pop-sel", serde_json::json!({ "gen": gen, "sel": index }));
}

/// 工具列要求關掉（網址列失去焦點、按 Esc）。key = 只關這個按鈕開的（沒給就全部）
#[tauri::command]
pub async fn web_pop_close(app: AppHandle, webview: Webview, key: Option<String>) {
    let Some(wid) = owner_of(&webview) else { return };
    let mine = lock(&POP).as_ref().is_some_and(|p| p.owner == wid && key.as_ref().is_none_or(|k| *k == p.key));
    if mine {
        hide_all(&app, false);
    }
}

fn open_ext(app: &AppHandle, owner: &str, id: &str, anchor: (i32, i32, i32, i32)) -> bool {
    let e = ext_list().into_iter().find(|e| e.id == id).filter(|e| !e.off);
    let popup = e.as_ref().and_then(|e| Url::parse(&e.popup).ok());
    let options = e.as_ref().and_then(|e| Url::parse(&e.options).ok());
    if let Some(u) = popup {
        // 像 Chrome 一樣開成小視窗蓋在網頁上，分頁不變，擴充功能才知道目前是哪個網站
        let _ = open_ext_popup(app, owner, u, anchor);
        true
    } else if let Some(u) = options {
        let _ = add_tab(app, owner, u.to_string(), true, None);
        true
    } else {
        false
    }
}

/// 工具列上固定的擴充功能圖示：點了開它的選單頁面（沒有就開選項）
#[tauri::command]
pub async fn web_ext_open(app: AppHandle, webview: Webview, id: String, rect: [f64; 4]) -> Result<(), String> {
    let wid = id_of(&webview).ok_or("不是瀏覽器視窗")?;
    hide_all(&app, false);
    // 同一個圖示再點一下：關掉
    if lock(&EXT_POPUP).as_ref().is_some_and(|(w, _)| *w == wid) {
        close_ext_popup(&app, &wid);
        return Ok(());
    }
    let win = app.get_window(&win_label(&wid)).ok_or("找不到這個視窗")?;
    let sf = win.scale_factor().unwrap_or(1.0).max(0.5);
    let p = win.inner_position().map_err(|e| e.to_string())?;
    let f = |v: f64| (v * sf).round() as i32;
    let anchor = (p.x + f(rect[0]), p.y + f(rect[1]), p.x + f(rect[2]), p.y + f(rect[3]));
    open_ext(&app, &wid, &id, anchor);
    Ok(())
}

/// 點了某一項。button：0 左鍵、1 中鍵、2 右鍵；part：那一項裡面的小按鈕（縮放的加減、擴充功能的固定…）
#[tauri::command]
pub async fn web_pop_pick(app: AppHandle, webview: Webview, gen: u64, index: usize, button: u8, part: Option<String>) {
    let Some(i) = level_of(&webview) else { return };
    let info = {
        let g = lock(&POP);
        let Some(p) = g.as_ref() else { return };
        let Some(l) = p.levels.get(i).filter(|l| l.gen == gen) else { return };
        let Some(act) = l.acts.get(index).cloned() else { return };
        let anchor = match &p.levels[0].place {
            Place::Below { rect, .. } => *rect,
            Place::Cursor(x, y) => (*x, *y, *x, *y),
            Place::Side { parent, .. } => *parent,
        };
        (p.owner.clone(), p.tid.clone(), act, p.ctx, anchor, p.key.clone())
    };
    let (owner, tid, act, ctx, anchor, key) = info;
    let part = part.unwrap_or_default();
    let cursor = || {
        app.get_window(&win_label(&owner)).and_then(|w| w.cursor_position().ok()).map(|c| Place::Cursor(c.x.round() as i32, c.y.round() as i32))
    };
    // 右鍵：書籤、擴充功能有自己的選單（再開一層）
    if button == 2 {
        let menu = match &act {
            Act::Link(id) => bm_ctx_of(id),
            Act::Sub(Sub::BmFolder(id)) => bm_ctx_of(id),
            Act::Ext(id) => ext_ctx(id),
            _ => None,
        };
        if let (Some(menu), Some(place)) = (menu, cursor()) {
            push_level(&app, i, None, menu, place);
        }
        return;
    }
    match act {
        Act::Nothing => {}
        Act::Cmd(c) => {
            hide_all(&app, false);
            run_cmd(&app, &owner, Some(&tid), c);
        }
        Act::TabCmd(t, c) => {
            hide_all(&app, false);
            run_cmd(&app, &owner, Some(&t), c);
        }
        // 點有下一層的項目：滑鼠停著就會開，這裡不用做事
        Act::Sub(_) => {}
        Act::Link(id) => {
            hide_all(&app, false);
            open_bm(&app, &owner, &id, button == 1);
        }
        Act::NewTab(u, activate) => {
            hide_all(&app, false);
            let _ = add_tab(&app, &owner, u, activate, Some(&tid));
        }
        Act::NewWindow(u, incognito) => {
            hide_all(&app, false);
            let _ = new_window(&app, Some(u), incognito, Some(&owner));
        }
        Act::BmRename(id) => {
            hide_all(&app, false);
            let _ = app.emit_to(bar_label(&owner).as_str(), "web-rename", serde_json::json!({ "id": owner, "bm": id }));
        }
        Act::BmDelete(id) => {
            hide_all(&app, false);
            bm_delete(&app, &id);
        }
        Act::BmOpenAll(id) => {
            hide_all(&app, false);
            fn links(list: &[Bm], out: &mut Vec<String>) {
                for b in list {
                    if b.url.is_empty() {
                        links(&b.children, out);
                    } else {
                        out.push(b.url.clone());
                    }
                }
            }
            let mut urls = Vec::new();
            bm_read(|b| {
                if let Some(f) = bm_find_any(b, &id) {
                    links(&f.children, &mut urls);
                }
            });
            // 一次最多開 20 個，避免不小心開太多
            for u in urls.into_iter().filter(|u| parse_web(u).is_some()).take(20) {
                let _ = add_tab(&app, &owner, u, false, None);
            }
        }
        Act::Ext(id) => match part.as_str() {
            "pin" => {
                ext_toggle_pin(&app, &id);
                replace_level(&app, i, ext_menu());
            }
            "more" => {
                if let (Some(menu), Some(place)) = (ext_ctx(&id), cursor()) {
                    push_level(&app, i, None, menu, place);
                }
            }
            _ => {
                // 有選單頁面就開；沒有就開選項；都沒有（自動執行的）就顯示右鍵選單
                if ext_list().iter().any(|e| e.id == id && !e.off && (!e.popup.is_empty() || !e.options.is_empty())) {
                    hide_all(&app, false);
                    open_ext(&app, &owner, &id, anchor);
                } else if let (Some(menu), Some(place)) = (ext_ctx(&id), cursor()) {
                    push_level(&app, i, None, menu, place);
                }
            }
        },
        Act::ExtPage(page) => {
            hide_all(&app, false);
            if let Ok(u) = Url::parse(&page) {
                if ext_list().iter().any(|e| e.popup == page) {
                    let _ = open_ext_popup(&app, &owner, u, anchor);
                } else {
                    let _ = add_tab(&app, &owner, u.to_string(), true, None);
                }
            }
        }
        Act::ExtToggle(id, off) => {
            hide_all(&app, false);
            let _ = ext_set_off(&id, off);
            emit_exts(&app);
        }
        Act::ExtPin(id) => {
            hide_all(&app, false);
            ext_toggle_pin(&app, &id);
        }
        Act::ExtSync => {
            hide_all(&app, false);
            let _ = sync_extensions();
            emit_exts(&app);
        }
        Act::Native(id) => {
            // 先把結果交給 WebView2，再關選單（關的時候就不會當成「沒選」）
            if let Some(seq) = ctx {
                finish_ctx(&app, seq, Some(id));
            }
            if let Some(p) = lock(&POP).as_mut() {
                p.ctx = None;
            }
            hide_all(&app, false);
        }
        Act::Reopen(n) => {
            hide_all(&app, false);
            reopen_tab(&app, &owner, n);
        }
        Act::SelectTab(t) => {
            if part == "close" {
                let _ = close_tab(&app, &owner, &t);
                // 視窗還在就更新清單
                if get(&owner).is_some_and(|w| w.open) && lock(&POP).as_ref().is_some_and(|p| p.key == key) {
                    replace_level(&app, i, tabs_menu(&owner));
                }
            } else {
                hide_all(&app, false);
                let _ = select_tab(&app, &owner, &t);
            }
        }
        Act::NavEntry(id) => {
            hide_all(&app, false);
            if let Some(p) = app.get_webview(&page_label(&owner, &tid)) {
                with_wv(&p, move |w| webnative::nav_to(w, id));
            }
        }
        Act::Profile(dir) => {
            hide_all(&app, false);
            bm(|b| b.profile = dir);
            let _ = import_chrome(&app, true);
            let _ = sync_extensions();
            emit_bm(&app);
            emit_exts(&app);
        }
        Act::SyncNow => {
            hide_all(&app, false);
            let _ = import_chrome(&app, true);
            let _ = sync_extensions();
            emit_bm(&app);
            emit_exts(&app);
        }
        Act::Sug(u) => {
            hide_all(&app, false);
            if button == 1 {
                let _ = add_tab(&app, &owner, u, false, Some(&tid));
            } else {
                navigate_tab(&app, &owner, &tid, &u);
                if let Some(p) = app.get_webview(&page_label(&owner, &tid)) {
                    let _ = p.set_focus();
                }
            }
            let _ = app.emit_to(bar_label(&owner).as_str(), "web-blur", owner.clone());
        }
        Act::Zoom => {
            let cur = tab_zoom(&owner, &tid);
            let z = match part.as_str() {
                "minus" => webcore::zoom_step(cur, false),
                "plus" => webcore::zoom_step(cur, true),
                "reset" => st().file.settings.zoom_for(""),
                "full" => {
                    hide_all(&app, false);
                    toggle_fs(&app, &owner);
                    return;
                }
                _ => return,
            };
            set_tab_zoom(&app, &owner, &tid, z);
            // 更新這一列顯示的百分比
            let pct = webcore::zoom_pct(tab_zoom(&owner, &tid)) as i64;
            {
                let mut g = lock(&POP);
                if let Some(it) = g.as_mut().and_then(|p| p.levels.get_mut(i)).and_then(|l| l.items.get_mut(index)) {
                    it.n = pct;
                }
            }
            let _ = app.emit_to(label(i).as_str(), "web-pop-zoom", serde_json::json!({ "gen": gen, "index": index, "pct": pct }));
            if key == "zoom" {
                // 還在調：提示框晚一點再消失
                ZOOM_STAMP.store(next_gen(), Ordering::SeqCst);
            }
        }
    }
}

/* ---------------- 網址列的建議 ---------------- */

#[derive(Serialize, Clone)]
pub struct OmniOut {
    gen: u64,
    list: Vec<Sug>,
}

/// 最新的一次輸入（搜尋引擎的建議比較慢才回來，回來時要確認還是同一次輸入）
static OMNI_GEN: AtomicU64 = AtomicU64::new(0);

fn sug_menu(list: &[Sug]) -> Menu {
    let mut m = Menu::default();
    for s in list {
        let item = Item { kind: "sug", title: s.text.clone(), sub: s.desc.clone(), icon: s.icon.clone(), tag: s.kind.to_string(), ..Item::default() };
        m.add(item, Act::Sug(s.url.clone()));
    }
    m
}

/// 顯示（或更新）建議清單。rect = 網址列在工具列裡的位置（邏輯像素）
fn show_omni(app: &AppHandle, wid: &str, list: &[Sug], rect: [f64; 4]) {
    let same = lock(&POP).as_ref().is_some_and(|p| p.key == "omni" && p.owner == wid);
    if list.is_empty() {
        if same {
            hide_all(app, false);
        }
        return;
    }
    let menu = sug_menu(list);
    if same {
        {
            let mut g = lock(&POP);
            if let Some(l) = g.as_mut().and_then(|p| p.levels.first_mut()) {
                l.sel = -1;
            }
        }
        replace_level(app, 0, menu);
        return;
    }
    let Some(win) = app.get_window(&win_label(wid)) else { return };
    let Ok(p) = win.inner_position() else { return };
    let sf = win.scale_factor().unwrap_or(1.0).max(0.5);
    let f = |v: f64| (v * sf).round() as i32;
    let pill = (p.x + f(rect[0]), p.y + f(rect[1]), p.x + f(rect[2]), p.y + f(rect[3]));
    let tid = get(wid).map(|w| w.active).unwrap_or_default();
    let spec = Spec {
        key: "omni".into(),
        style: "omni",
        width: (rect[2] - rect[0]).max(200.0),
        menu,
        // 貼在網址列下面一點
        place: Place::Below { rect: (pill.0, pill.1, pill.2, pill.3 + f(4.0)), right: false },
        keep: Some(pill),
        ctx: None,
    };
    open(app, wid, &tid, spec);
}

fn flat_bookmarks(list: &[Bm], out: &mut Vec<(String, String)>) {
    for b in list {
        if b.url.is_empty() {
            flat_bookmarks(&b.children, out);
        } else {
            out.push((b.title.clone(), b.url.clone()));
        }
    }
}

/// 網址列打字：回傳建議（輸入的網址／搜尋、歷史記錄、書籤），並在網址列下面顯示清單；
/// 設定有開的話，再去問搜尋引擎的建議（比較慢，回來後用 web-omni-list 通知）。
/// inline = 可以自動補完（刪字的時候不要）
#[tauri::command]
pub async fn web_omni_input(app: AppHandle, webview: Webview, text: String, inline: bool, rect: [f64; 4]) -> Result<OmniOut, String> {
    let wid = id_of(&webview).ok_or("不是瀏覽器視窗")?;
    let gen = next_gen();
    OMNI_GEN.store(gen, Ordering::SeqCst);
    let (bms, suggest, incognito, e) = {
        let s = st();
        let mut bms = Vec::new();
        flat_bookmarks(&s.file.bookmarks.bar, &mut bms);
        flat_bookmarks(&s.file.bookmarks.other, &mut bms);
        (bms, s.file.settings.suggest, s.file.wins.iter().any(|w| w.id == wid && w.incognito), webcore::engine(&s.file.settings.engine))
    };
    let list = hist_read(|h| webcore::suggest_local(&text, h, &bms, e, webcore::now_ms(), inline, 8)).unwrap_or_default();
    show_omni(&app, &wid, &list, rect);
    // 搜尋引擎的建議（打的字會送給搜尋引擎，跟 Chrome 一樣；無痕式視窗、設定關掉時不送）
    if suggest && !incognito && webcore::wants_remote(&text) && !list.is_empty() {
        let (app2, wid2, local) = (app.clone(), wid.clone(), list.clone());
        std::thread::spawn(move || {
            let url = webcore::suggest_url(e, &text);
            let body = ureq::get(&url).timeout(Duration::from_millis(1500)).set("User-Agent", "Mozilla/5.0").call().ok().and_then(|r| r.into_string().ok());
            let Some(body) = body else { return };
            let remote = webcore::parse_suggest(&body);
            if remote.is_empty() || OMNI_GEN.load(Ordering::SeqCst) != gen {
                return;
            }
            // 清單已經關掉（按了 Enter、離開網址列）就不用再顯示
            if !lock(&POP).as_ref().is_some_and(|p| p.key == "omni" && p.owner == wid2) {
                return;
            }
            let merged = webcore::merge_remote(local, &remote, e, 8);
            show_omni(&app2, &wid2, &merged, rect);
            let _ = app2.emit_to(bar_label(&wid2).as_str(), "web-omni-list", OmniOut { gen, list: merged });
        });
    }
    Ok(OmniOut { gen, list })
}

/* ---------------- 點到外面就關掉 ---------------- */

/// 選單開著時：點到選單外面就關掉（擴充功能蓋在網頁上的小視窗也是）
pub fn watch(app: &AppHandle) {
    #[cfg(windows)]
    {
        let app2 = app.clone();
        std::thread::spawn(move || {
            let mut prev_down = true;
            let mut seen: Option<Instant> = None;
            loop {
                let cur = lock(&POP)
                    .as_ref()
                    .map(|p| (p.levels.iter().filter(|l| l.visible).map(|l| l.rect).chain(p.keep).collect::<Vec<_>>(), p.shown_at, p.levels.iter().any(|l| l.visible)));
                let ext = lock(&EXT_POPUP).clone();
                if cur.is_none() && ext.is_none() {
                    seen = None;
                    prev_down = true;
                    std::thread::sleep(Duration::from_millis(120));
                    continue;
                }
                let down = win32::mouse_down();
                let inside = |r: &(i32, i32, i32, i32), x: i32, y: i32| x >= r.0 && x < r.2 && y >= r.1 && y < r.3;
                if let Some((rects, shown, visible)) = cur {
                    if seen != Some(shown) {
                        // 剛打開：打開它的那一下不算
                        if visible {
                            seen = Some(shown);
                        }
                    } else if down && !prev_down {
                        let (x, y) = win32::cursor();
                        if !rects.iter().any(|r| inside(r, x, y)) {
                            hide_all(&app2, true);
                        }
                    }
                }
                // 擴充功能的小視窗：點外面就關
                if let Some((wid, rect)) = ext {
                    if down && !prev_down {
                        let (x, y) = win32::cursor();
                        if !inside(&rect, x, y) {
                            close_ext_popup(&app2, &wid);
                        }
                    }
                }
                prev_down = down;
                std::thread::sleep(Duration::from_millis(20));
            }
        });
    }
    #[cfg(not(windows))]
    let _ = app;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn menus_have_matching_actions() {
        let m = strip_menu_for_test();
        assert_eq!(m.items.len(), m.acts.len());
        let mut m = Menu::default();
        m.sep();
        assert!(m.items.is_empty());
        m.add(Item::new("a", ""), Act::Nothing);
        m.sep();
        m.sep();
        assert_eq!(m.items.len(), 2);
        assert_eq!(m.trim().items.len(), 1);
    }

    fn strip_menu_for_test() -> Menu {
        let mut m = Menu::default();
        m.add(Item::new("新分頁", "").key("Ctrl+T"), Act::Cmd("tab.new"));
        m.head("x");
        m.empty("y");
        m
    }
}
