# web 視窗的實機測試（在 GitHub 的 Windows 測試機上執行）
# 用滑鼠、鍵盤實際操作，記錄每一步的截圖、視窗位置與檢查結果到 uitest-out/
import ctypes
import ctypes.wintypes as wt
import http.server
import json
import os
import shutil
import socketserver
import subprocess
import sys
import threading
import time
import traceback
import urllib.parse

from PIL import ImageGrab

# 測試機的命令列預設不是 UTF-8：中文改成可以印出來
try:
    sys.stdout.reconfigure(encoding="utf-8", errors="replace")
    sys.stderr.reconfigure(encoding="utf-8", errors="replace")
except Exception:
    pass

HERE = os.path.dirname(os.path.abspath(__file__))
OUT = os.path.abspath("uitest-out")
os.makedirs(OUT, exist_ok=True)
EXE = os.path.abspath(sys.argv[1])
PORT = 8765

user32 = ctypes.windll.user32
kernel32 = ctypes.windll.kernel32
try:
    ctypes.windll.shcore.SetProcessDpiAwareness(2)
except Exception:
    user32.SetProcessDPIAware()

HWND = ctypes.c_void_p
user32.GetForegroundWindow.restype = HWND
user32.GetWindow.restype = HWND
user32.GetWindow.argtypes = [HWND, ctypes.c_uint]
user32.GetTopWindow.restype = HWND
user32.GetTopWindow.argtypes = [HWND]
user32.GetParent.restype = HWND
user32.GetParent.argtypes = [HWND]
user32.GetAncestor.restype = HWND
user32.GetAncestor.argtypes = [HWND, ctypes.c_uint]
user32.WindowFromPoint.restype = HWND
user32.WindowFromPoint.argtypes = [wt.POINT]
for f in ("IsWindowVisible", "IsIconic", "IsZoomed", "IsWindow", "GetWindowTextLengthW", "SetForegroundWindow", "IsHungAppWindow", "GetDpiForWindow"):
    getattr(user32, f).argtypes = [HWND]
user32.GetWindowTextW.argtypes = [HWND, ctypes.c_wchar_p, ctypes.c_int]
user32.GetClassNameW.argtypes = [HWND, ctypes.c_wchar_p, ctypes.c_int]
user32.GetWindowRect.argtypes = [HWND, ctypes.POINTER(wt.RECT)]
user32.GetClientRect.argtypes = [HWND, ctypes.POINTER(wt.RECT)]
user32.ClientToScreen.argtypes = [HWND, ctypes.POINTER(wt.POINT)]
user32.GetWindowThreadProcessId.argtypes = [HWND, ctypes.POINTER(wt.DWORD)]
user32.ShowWindow.argtypes = [HWND, ctypes.c_int]
user32.MoveWindow.argtypes = [HWND, ctypes.c_int, ctypes.c_int, ctypes.c_int, ctypes.c_int, wt.BOOL]
user32.PostMessageW.argtypes = [HWND, ctypes.c_uint, ctypes.c_void_p, ctypes.c_void_p]
user32.SendMessageTimeoutW.argtypes = [HWND, ctypes.c_uint, ctypes.c_void_p, ctypes.c_void_p, ctypes.c_uint, ctypes.c_uint, ctypes.POINTER(ctypes.c_size_t)]
EnumProc = ctypes.WINFUNCTYPE(wt.BOOL, HWND, ctypes.c_void_p)
user32.EnumWindows.argtypes = [EnumProc, ctypes.c_void_p]
user32.EnumChildWindows.argtypes = [HWND, EnumProc, ctypes.c_void_p]

LOGF = open(os.path.join(OUT, "log.txt"), "w", encoding="utf-8")
RESULTS = []


def log(*a):
    s = " ".join(str(x) for x in a)
    try:
        print(s, flush=True)
    except Exception:
        pass
    LOGF.write(s + "\n")
    LOGF.flush()


def check(name, ok, detail=""):
    RESULTS.append((name, bool(ok), detail))
    log(("PASS" if ok else "FAIL"), name, detail)


_shot = [0]


def shot(name):
    _shot[0] += 1
    path = os.path.join(OUT, f"{_shot[0]:02d}_{name}.png")
    try:
        ImageGrab.grab(all_screens=True).save(path)
    except Exception as e:
        log("screenshot failed", e)


def H(h):
    return int(h or 0)


def proc_name(pid):
    h = kernel32.OpenProcess(0x1000, False, pid)
    if not h:
        return ""
    buf = ctypes.create_unicode_buffer(1024)
    size = wt.DWORD(1024)
    ok = kernel32.QueryFullProcessImageNameW(h, 0, buf, ctypes.byref(size))
    kernel32.CloseHandle(h)
    return os.path.basename(buf.value) if ok else ""


def title(h):
    n = user32.GetWindowTextLengthW(h)
    b = ctypes.create_unicode_buffer(n + 1)
    user32.GetWindowTextW(h, b, n + 1)
    return b.value


def cls(h):
    b = ctypes.create_unicode_buffer(256)
    user32.GetClassNameW(h, b, 256)
    return b.value


def wrect(h):
    r = wt.RECT()
    user32.GetWindowRect(h, ctypes.byref(r))
    return (r.left, r.top, r.right, r.bottom)


def client(h):
    p = wt.POINT(0, 0)
    user32.ClientToScreen(h, ctypes.byref(p))
    r = wt.RECT()
    user32.GetClientRect(h, ctypes.byref(r))
    return (p.x, p.y, r.right, r.bottom)


def pid_of(h):
    pid = wt.DWORD()
    user32.GetWindowThreadProcessId(h, ctypes.byref(pid))
    return pid.value


def top_windows():
    res = []
    user32.EnumWindows(EnumProc(lambda h, l: res.append(H(h)) or True), None)
    return res


def app_windows():
    exe = os.path.basename(EXE).lower()
    return [h for h in top_windows() if proc_name(pid_of(h)).lower() == exe]


def children(h):
    res = []
    user32.EnumChildWindows(h, EnumProc(lambda c, l: res.append(H(c)) or True), None)
    return [c for c in res if H(user32.GetParent(c)) == h]


def dump(h, label):
    if not h or not user32.IsWindow(h):
        log(f"[{label}] window gone")
        return
    log(f"[{label}] title={title(h)!r} rect={wrect(h)} client={client(h)} zoomed={bool(user32.IsZoomed(h))} iconic={bool(user32.IsIconic(h))}")
    for c in children(h):
        log(f"    child {cls(c)} visible={bool(user32.IsWindowVisible(c))} rect={wrect(c)}")


def zorder():
    order = []
    h = user32.GetTopWindow(None)
    while h:
        if user32.IsWindowVisible(h):
            order.append(H(h))
        h = user32.GetWindow(h, 2)  # GW_HWNDNEXT
    return order


def above(a, b):
    z = zorder()
    return a in z and b in z and z.index(a) < z.index(b)


def responsive(h):
    res = ctypes.c_size_t()
    return bool(user32.SendMessageTimeoutW(h, 0, None, None, 0x0002, 3000, ctypes.byref(res)))


def fg():
    return H(user32.GetForegroundWindow())


def move(x, y):
    user32.SetCursorPos(int(x), int(y))


def click(x, y, button="left", double=False, wait=0.35):
    move(x, y)
    time.sleep(0.08)
    down, up = {"left": (2, 4), "right": (8, 16), "middle": (0x20, 0x40)}[button]
    for _ in range(2 if double else 1):
        user32.mouse_event(down, 0, 0, 0, 0)
        time.sleep(0.04)
        user32.mouse_event(up, 0, 0, 0, 0)
        time.sleep(0.06)
    time.sleep(wait)


def drag(x1, y1, x2, y2, steps=30, wait=0.6):
    move(x1, y1)
    time.sleep(0.1)
    user32.mouse_event(2, 0, 0, 0, 0)
    time.sleep(0.15)
    for i in range(1, steps + 1):
        move(x1 + (x2 - x1) * i / steps, y1 + (y2 - y1) * i / steps)
        time.sleep(0.02)
    time.sleep(0.3)
    user32.mouse_event(4, 0, 0, 0, 0)
    time.sleep(wait)


def wheel(x, y, notches):
    move(x, y)
    time.sleep(0.05)
    user32.mouse_event(0x0800, 0, 0, (-120 * notches) & 0xFFFFFFFF, 0)
    time.sleep(0.4)


def key(vk):
    user32.keybd_event(vk, 0, 0, 0)
    time.sleep(0.03)
    user32.keybd_event(vk, 0, 2, 0)
    time.sleep(0.03)


def type_text(s):
    for ch in s:
        vk = user32.VkKeyScanW(ord(ch)) & 0xFF
        shift = (user32.VkKeyScanW(ord(ch)) >> 8) & 1
        if shift:
            user32.keybd_event(0x10, 0, 0, 0)
        key(vk)
        if shift:
            user32.keybd_event(0x10, 0, 2, 0)
    time.sleep(0.3)


def wait_for(fn, timeout=10.0, step=0.2):
    end = time.time() + timeout
    while time.time() < end:
        v = fn()
        if v:
            return v
        time.sleep(step)
    return fn()


# ---------- 本機測試網頁（記錄網頁送來的訊息） ----------
PAGE_LOG = os.path.join(OUT, "page_log.txt")
open(PAGE_LOG, "w").close()


class Handler(http.server.SimpleHTTPRequestHandler):
    def __init__(self, *a, **k):
        super().__init__(*a, directory=os.path.join(HERE, "pages"), **k)

    def do_GET(self):
        u = urllib.parse.urlparse(self.path)
        if u.path == "/log":
            m = urllib.parse.parse_qs(u.query).get("m", [""])[0]
            with open(PAGE_LOG, "a", encoding="utf-8") as f:
                f.write(f"{time.time():.2f} {m}\n")
            self.send_response(204)
            self.send_header("Access-Control-Allow-Origin", "*")
            self.end_headers()
            return
        super().do_GET()

    def log_message(self, *a):
        pass


socketserver.ThreadingTCPServer.allow_reuse_address = True
srv = socketserver.ThreadingTCPServer(("0.0.0.0", PORT), Handler)
threading.Thread(target=srv.serve_forever, daemon=True).start()


def page_log():
    return open(PAGE_LOG, encoding="utf-8").read()


GAME_LOG = os.path.join(OUT, "game_keys.txt")
open(GAME_LOG, "w").close()


def game_keys():
    return open(GAME_LOG, encoding="utf-8").read()


# ---------- 準備程式資料：一個開著的 web 視窗、三個分頁、書籤 ----------
data_dir = os.path.join(os.environ["APPDATA"], "com.toolbox.desktop")
shutil.rmtree(data_dir, ignore_errors=True)
os.makedirs(data_dir, exist_ok=True)
base = f"http://127.0.0.1:{PORT}"
web_json = {
    "next_id": 1,
    "wins": [{
        "id": "w1", "open": True, "x": 80, "y": 60, "w": 900, "h": 620, "opacity": 100,
        "tabs": [
            {"id": "t1", "url": f"{base}/test.html", "title": "測試頁"},
            {"id": "t2", "url": f"{base}/snap.html", "title": "短影片測試"},
            {"id": "t3", "url": "https://example.com/", "title": "Example"},
        ],
        "active": "t1", "next_tab": 3,
    }],
    "bookmarks": {
        "bar": [
            {"id": "b1", "title": "Example", "url": "https://example.com/"},
            {"id": "b2", "title": "資料夾", "url": "", "children": [
                {"id": "b3", "title": "測試頁", "url": f"{base}/test.html?bm"},
                {"id": "b5", "title": "Example", "url": "https://example.com/?bm"},
            ]},
        ],
        "other": [{"id": "b4", "title": "Other", "url": "https://example.org/"}],
        "next_id": 5, "sync": False,
    },
}
with open(os.path.join(data_dir, "web.json"), "w", encoding="utf-8") as f:
    json.dump(web_json, f, ensure_ascii=False)

# ---------- 開假的遊戲、開程式 ----------
game_proc = subprocess.Popen([sys.executable, os.path.join(HERE, "game.py"), GAME_LOG])
game = wait_for(lambda: next((h for h in top_windows() if title(h) == "MapleStory" and user32.IsWindowVisible(h)), 0), 15)
log("game hwnd", game, wrect(game) if game else None)

app_proc = subprocess.Popen([EXE])
KNOWN = ("工具箱", "web 選單", "計時器浮動視窗", "")


def find_web():
    for h in app_windows():
        if user32.IsWindowVisible(h) and title(h) not in KNOWN and len(children(h)) >= 2:
            return h
    return 0


web = wait_for(find_web, 40)
log("web hwnd", web)
if not web:
    for h in app_windows():
        log("app window", h, repr(title(h)), cls(h), user32.IsWindowVisible(h), wrect(h))
    shot("no_web_window")
    sys.exit(1)
time.sleep(4)
scale = (user32.GetDpiForWindow(web) or 96) / 96
log("dpi scale", scale, "screen", user32.GetSystemMetrics(0), user32.GetSystemMetrics(1))


def C(x, y):
    """web 視窗內容區的座標（CSS 像素）→ 螢幕座標"""
    cx, cy, cw, ch = client(web)
    return cx + x * scale, cy + y * scale


def W():
    return client(web)[2] / scale


def tab_x(i, n):
    total = 8 + n * 240 + 38 + 40 + 138
    over = max(0, total - W())
    tw = (n * 240 - over * (n * 240) / (n * 240 + 40)) / n
    return 8 + tw * i + tw * 0.3


def menu_win():
    return next((h for h in app_windows() if title(h) == "web 選單"), 0)


def step(name, fn):
    log(f"\n===== {name} =====")
    try:
        fn()
    except Exception:
        log(traceback.format_exc())
        check(name + " (沒有錯誤)", False, "exception")
    shot(name)
    dump(web, name)
    check(name + "：程式有回應", responsive(web) if user32.IsWindow(web) else True)


def s_start():
    dump(web, "start")
    kids = [c for c in children(web) if user32.IsWindowVisible(c)]
    check("一開始：看得到工具列和網頁", len(kids) >= 2, f"{len(kids)} 個可見子視窗")


def s_max_button():
    before = wrect(web)
    x, y = C(W() - 69, 20)
    click(x, y, wait=1.2)
    check("按放到最大按鈕：視窗放到最大", user32.IsZoomed(web), f"before={before} after={wrect(web)}")
    vis = [wrect(c) for c in children(web) if user32.IsWindowVisible(c)]
    tops = sorted(r[1] for r in vis)
    check("放到最大後：工具列在最上面、網頁在下面", len(vis) >= 2 and tops[0] <= client(web)[1] + 2, f"{vis}")


def s_restore_button():
    x, y = C(W() - 69, 20)
    click(x, y, wait=1.2)
    check("再按一次：還原", not user32.IsZoomed(web), f"{wrect(web)}")


def s_snap_resize():
    x, y = C(tab_x(1, 3), 23)
    click(x, y, wait=1.5)
    px, py = C(400, 300)
    for _ in range(4):
        wheel(px, py, 1)
        time.sleep(0.5)
    time.sleep(1)
    log("page log (snap):", page_log()[-600:])
    mx, my = C(W() - 69, 20)
    click(mx, my, wait=1.5)
    shot("snap_maximized")
    click(*C(W() - 69, 20), wait=1.5)
    time.sleep(1)
    lines = [l for l in page_log().splitlines() if "[snap]" in l]
    log("snap lines:", lines[-8:])
    slides = [l.split("slide:")[1].split()[0] for l in lines if "slide:" in l]
    check("短影片頁面：放大、還原後還是同一張", len(set(slides[-3:])) <= 1 if len(slides) >= 2 else False, f"{slides}")


def s_double_click():
    x, y = C(W() - 200, 12)
    click(x, y, double=True, wait=1.2)
    check("按兩下分頁列空白處：放到最大", user32.IsZoomed(web))
    x, y = C(W() - 200, 12)
    click(x, y, double=True, wait=1.2)
    check("再按兩下：還原", not user32.IsZoomed(web))


def s_drag_snap():
    before = wrect(web)
    x, y = C(W() - 200, 15)
    drag(x, y, x, 2, wait=1.5)
    check("拖到螢幕頂端：放到最大", user32.IsZoomed(web), f"{wrect(web)}")
    shot("drag_top")
    x, y = C(W() / 2, 15)
    drag(x, y, x + 50, y + 250, wait=1.5)
    after = wrect(web)
    check("放到最大時往下拖：還原成原本大小", not user32.IsZoomed(web) and abs((after[2] - after[0]) - (before[2] - before[0])) < 30, f"before={before} after={after}")


def s_tabs():
    click(*C(tab_x(2, 3), 23), wait=2.5)
    shot("tab3")
    click(*C(tab_x(0, 3), 23), wait=1.5)
    kids = [c for c in children(web) if user32.IsWindowVisible(c)]
    check("切換分頁：只有一個網頁顯示", len(kids) == 2, f"{[wrect(c) for c in kids]}")


def s_menu():
    click(*C(143, 93), wait=1.2)
    m = menu_win()
    check("點書籤資料夾：出現選單", m and user32.IsWindowVisible(m), f"menu={m}")
    shot("menu_open")
    click(*C(W() / 2, 400), wait=1.0)
    check("點外面：選單關掉", not (m and user32.IsWindowVisible(m)))
    check("選單關掉後程式有回應", responsive(web))
    click(*C(W() - 55, 93), wait=1.2)
    m = menu_win()
    check("點所有書籤：出現選單", m and user32.IsWindowVisible(m))
    shot("menu_other")
    click(*C(W() - 55, 93), wait=1.0)
    check("再點一次所有書籤：選單關掉", not (m and user32.IsWindowVisible(m)))
    check("所有書籤操作後程式有回應", responsive(web))


def s_new_tab():
    n = 3
    click(*C(8 + (tab_x(1, 3) - tab_x(0, 3)) * n + 15, 23), wait=1.0)
    type_text(f"127.0.0.1:{PORT}/test.html?new")
    key(0x0D)
    time.sleep(2.5)
    shot("new_tab_loaded")
    check("新分頁輸入網址：有載入", "test.html?new" in title(web) or "測試頁" in title(web), title(web))


def s_game():
    gx, gy = wrect(game)[0] + 60, wrect(game)[3] - 40
    click(gx, gy, wait=1.0)
    check("點遊戲：遊戲在前景", fg() == game, f"fg={fg()} game={game}")
    check("遊戲在前景：web 視窗在遊戲上面", above(web, game))
    click(*C(5 + 200, 108 + 320), wait=0.8)
    check("點網頁空白處：鍵盤還在遊戲", fg() == game, f"fg={fg()}")
    key(0x41)
    time.sleep(0.4)
    check("按 A：遊戲收到", "key:a" in game_keys(), game_keys()[-80:])
    click(*C(5 + 200, 108 + 40), wait=1.0)
    check("點網頁輸入框：鍵盤交給 web", fg() == web, f"fg={fg()}")
    type_text("hello")
    time.sleep(0.5)
    check("在輸入框打字：網頁收到", "input:hello" in page_log(), page_log()[-200:])
    key(0x0D)
    time.sleep(1.0)
    check("按 Enter：鍵盤還給遊戲", fg() == game, f"fg={fg()}")
    key(0x42)
    time.sleep(0.4)
    check("按 B：遊戲收到", "key:b" in game_keys())
    click(*C(5 + 200, 108 + 40), wait=1.0)
    type_text("xy")
    move(gx, gy)
    time.sleep(1.2)
    check("打字中滑鼠移出 web：鍵盤還給遊戲", fg() == game, f"fg={fg()}")
    click(*C(5 + 20 + 2 + 160, 108 + 90 + 2 + 28), wait=1.0)
    check("點內嵌框裡的輸入框：鍵盤交給 web", fg() == web, f"fg={fg()}")
    type_text("fr")
    time.sleep(0.5)
    check("內嵌框打字：收到", "[frame] input:fr" in page_log())
    move(gx, gy)
    time.sleep(1.2)
    before = set(app_windows())
    click(*C(5 + 100, 108 + 200), wait=2.5)
    new = [h for h in app_windows() if h not in before and user32.IsWindowVisible(h)]
    shot("popup")
    check("登入小視窗：開成另一個視窗", len(new) >= 1, f"{[(title(h), wrect(h)) for h in new]}")
    if new:
        type_text("pp")
        time.sleep(0.5)
        check("登入小視窗：可以打字", "[popup] input:pp" in page_log(), page_log()[-200:])
        user32.PostMessageW(new[0], 0x0010, None, None)
        time.sleep(1)


def s_notepad():
    np_proc = subprocess.Popen(["notepad.exe"])
    np = wait_for(lambda: next((h for h in top_windows() if pid_of(h) == np_proc.pid and user32.IsWindowVisible(h)), 0), 10)
    time.sleep(1)
    if np:
        user32.ShowWindow(np, 1)
        user32.MoveWindow(np, 100, 100, 600, 400, True)
        click(wrect(np)[0] + 200, wrect(np)[1] + 200, wait=1.0)
        check("記事本在前景時可以蓋住 web 視窗", above(np, web), f"fg={fg()} np={np}")
        shot("notepad_over_web")
        x, y = C(tab_x(0, 4), 23)
        if not (wrect(np)[0] <= x <= wrect(np)[2] and wrect(np)[1] <= y <= wrect(np)[3]):
            click(x, y, wait=1.0)
            check("點 web 視窗：變成前景（一般視窗）", fg() == web and above(web, np), f"fg={fg()}")
    np_proc.kill()


def s_minimize():
    click(*C(W() - 115, 20), wait=1.2)
    check("按縮小：縮到工作列", user32.IsIconic(web))
    user32.ShowWindow(web, 9)
    time.sleep(1.2)
    check("從工作列還原", not user32.IsIconic(web) and user32.IsWindowVisible(web))


def s_close():
    click(*C(W() - 23, 20), wait=1.5)
    check("按關閉：視窗關掉", not user32.IsWindow(web) or not user32.IsWindowVisible(web))


for name, fn in [
    ("start", s_start),
    ("max_button", s_max_button),
    ("restore_button", s_restore_button),
    ("snap_resize", s_snap_resize),
    ("double_click", s_double_click),
    ("drag_snap", s_drag_snap),
    ("tabs", s_tabs),
    ("menu", s_menu),
    ("new_tab", s_new_tab),
    ("game", s_game),
    ("notepad", s_notepad),
    ("minimize", s_minimize),
    ("close", s_close),
]:
    step(name, fn)

log("\n===== 結果 =====")
for name, ok, detail in RESULTS:
    log(("PASS" if ok else "FAIL"), name, detail)
log(f"{sum(1 for r in RESULTS if r[1])}/{len(RESULTS)} passed")
log("\n----- page log -----\n" + page_log())
log("\n----- game keys -----\n" + game_keys())
subprocess.run(["taskkill", "/IM", os.path.basename(EXE), "/F"], capture_output=True)
game_proc.kill()
os._exit(0)
