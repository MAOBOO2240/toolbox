# web 視窗的實機測試（在 GitHub 的 Windows 測試機上執行）
# 用滑鼠、鍵盤實際操作，記錄每一步的截圖、視窗位置與檢查結果到 uitest-out/
import ctypes
import ctypes.wintypes as wt
import http.server
import json
import os
import shutil
import socketserver
import struct
import subprocess
import sys
import threading
import time
import traceback
import urllib.parse
import urllib.request

from PIL import ImageGrab

try:
    import websocket  # websocket-client：讀工具列裡按鈕的實際位置
except Exception:
    websocket = None

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
user32.EnumDisplaySettingsW.argtypes = [ctypes.c_wchar_p, wt.DWORD, ctypes.c_void_p]
user32.ChangeDisplaySettingsW.argtypes = [ctypes.c_void_p, wt.DWORD]
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
# 測試用的擴充功能（放在待安裝的資料夾）
EXT_ID = "eamehjingoajmedombhecpkpagoadodo"
shutil.copytree(os.path.join(HERE, "ext"), os.path.join(data_dir, "web-ext", "b1", EXT_ID))
with open(os.path.join(data_dir, "web-ext.json"), "w", encoding="utf-8") as f:
    json.dump({"exts": {EXT_ID: {"ver": "1.0", "dir": f"b1/{EXT_ID}", "name": "測試擴充功能", "off": False, "loaded": False}}, "pending": "b1"}, f, ensure_ascii=False)

# ---------- 開假的遊戲、開程式 ----------
game_proc = subprocess.Popen([sys.executable, os.path.join(HERE, "game.py"), GAME_LOG])
game = wait_for(lambda: next((h for h in top_windows() if title(h) == "MapleStory" and user32.IsWindowVisible(h)), 0), 15)
log("game hwnd", game, wrect(game) if game else None)

# 只有測試時打開 WebView2 的遠端偵錯，用來讀工具列裡按鈕、分頁的實際位置。
# 程式沒有系統管理員權限時會用「以系統管理員身分執行」重開自己，環境變數不會帶過去，所以也寫進 WebView2 的登錄設定
DEBUG_ARGS = "--remote-debugging-port=9222"
try:
    import winreg
    k = winreg.CreateKeyEx(winreg.HKEY_CURRENT_USER, r"Software\Policies\Microsoft\Edge\WebView2\AdditionalBrowserArguments", 0, winreg.KEY_SET_VALUE)
    winreg.SetValueEx(k, os.path.basename(EXE), 0, winreg.REG_SZ, DEBUG_ARGS)
    winreg.CloseKey(k)
except Exception as e:
    log("寫登錄失敗", e)
app_env = dict(os.environ)
app_env["WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS"] = DEBUG_ARGS
log("測試程式是系統管理員", bool(ctypes.windll.shell32.IsUserAnAdmin()))
app_proc = subprocess.Popen([EXE], env=app_env)
KNOWN = ("工具箱", "web 選單", "計時器浮動視窗", "")


def find_web():
    for h in app_windows():
        if user32.IsWindowVisible(h) and title(h) not in KNOWN and len(children(h)) >= 2:
            return h
    return 0


web = wait_for(find_web, 40)
WEB_AT = time.time()
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


# ---------- 讀工具列裡元素的實際位置（WebView2 遠端偵錯） ----------
APPID = "com.toolbox.desktop"


def devtools_ports():
    ports = []
    for root in (os.environ.get("LOCALAPPDATA", ""), os.environ.get("APPDATA", "")):
        top = os.path.join(root, APPID)
        for dp, dn, fn in os.walk(top):
            if dp[len(top):].count(os.sep) >= 3:
                dn[:] = []
            if "DevToolsActivePort" in fn:
                try:
                    with open(os.path.join(dp, "DevToolsActivePort")) as f:
                        ports.append(int(f.read().split()[0]))
                except Exception:
                    pass
    return ports


def cdp_targets():
    res = []
    for port in [9222] + [p for p in devtools_ports() if p != 9222]:
        try:
            with urllib.request.urlopen(f"http://127.0.0.1:{port}/json/list", timeout=3) as r:
                for t in json.load(r):
                    t["port"] = port
                    res.append(t)
        except Exception:
            pass
    return res


_cdp_n = [0]


def cdp_eval(match, expr):
    """在網址含 match 的頁面執行 JS，回傳結果；讀不到回傳 None"""
    if websocket is None:
        return None
    t = next((t for t in cdp_targets() if t.get("type") == "page" and match in t.get("url", "") and t.get("webSocketDebuggerUrl")), None)
    if not t:
        return None
    try:
        ws = websocket.create_connection(t["webSocketDebuggerUrl"], timeout=5, suppress_origin=True)
    except Exception as e:
        log("cdp 連不上", e)
        return None
    try:
        _cdp_n[0] += 1
        n = _cdp_n[0]
        ws.send(json.dumps({"id": n, "method": "Runtime.evaluate", "params": {"expression": expr, "returnByValue": True}}))
        while True:
            m = json.loads(ws.recv())
            if m.get("id") == n:
                return m.get("result", {}).get("result", {}).get("value")
    except Exception as e:
        log("cdp 執行失敗", e)
        return None
    finally:
        try:
            ws.close()
        except Exception:
            pass


BAR = "/web.html"


def bar_eval(expr):
    return cdp_eval(BAR, expr)


def bar_pt(el_js, fallback, dy=None):
    """工具列裡某個元素的中心（CSS 像素）；讀不到時用估計的位置"""
    b = bar_eval(f"(() => {{ const e = {el_js}; if (!e) return null; const r = e.getBoundingClientRect(); return [r.left, r.top, r.width, r.height]; }})()")
    if b and b[2] > 0:
        return (b[0] + b[2] / 2, b[1] + dy if dy is not None else b[1] + b[3] / 2)
    log("（讀不到元素位置，改用估計的）", el_js)
    return fallback


def row_pixels(y_css):
    cx, cy, cw, ch = client(web)
    y = int(cy + y_css * scale)
    img = ImageGrab.grab(bbox=(cx, y, cx + cw, y + 1), all_screens=True).convert("RGB")
    return [img.getpixel((i, 0)) for i in range(img.width)]


def find_tab_icons():
    """從截圖找每個分頁的網站圖示（彩色小方塊），回傳內容區的 x（CSS 像素），由左到右"""
    best = []
    for ycss in (17, 18, 19, 20):
        px = row_pixels(ycss) + [(0, 0, 0)]
        found, run = [], 0
        for i, (r, g, b) in enumerate(px):
            mx, mn = max(r, g, b), min(r, g, b)
            if mx > 80 and (mx - mn) / mx > 0.35:
                run += 1
            else:
                if 6 <= run <= 22:
                    found.append((i - run) / scale)
                run = 0
        found = [x for x in found if x < W() - 150]
        if len(found) > len(best):
            best = found
    return best


def find_plus():
    """從截圖找「新分頁」的 +（淺藍色的橫線，在最後一個分頁右邊），回傳中心 x（CSS 像素）"""
    icons = find_tab_icons()
    lo = (icons[-1] + 24) * scale if icons else 0
    best = None
    for ycss in (20, 21, 22, 23, 24, 25):
        px = row_pixels(ycss)
        run = 0
        for i, (r, g, b) in enumerate(px + [(0, 0, 0)]):
            if i >= lo and abs(r - 125) < 45 and abs(g - 211) < 45 and abs(b - 252) < 45:
                run += 1
                continue
            if run >= 6 and (best is None or run > best[0]):
                best = (run, (i - run + i - 1) / 2)
            run = 0
    return best[1] / scale if best else None


def tab_x(i, n=None):
    icons = find_tab_icons()
    if i < len(icons):
        return icons[i] + 8
    log(f"（截圖找不到第 {i + 1} 個分頁，找到 {len(icons)} 個）")
    return 8 + 120 * i + 60


def tab_count():
    return len(find_tab_icons())


def plus_x():
    x = find_plus()
    if x is None:
        log("（截圖找不到 +）")
        return 8 + 120 * tab_count() + 20
    return x


def TAB(i):
    return f"document.querySelectorAll('#tabs .tab')[{i}]"


BTN_MAX = "document.getElementById('cMax')"
BTN_MIN = "document.getElementById('cMin')"
BTN_CLOSE = "document.getElementById('cClose')"
DRAG = "document.getElementById('drag')"
NEWTAB = "document.getElementById('newTab')"
FOLDER = "[...document.querySelectorAll('#bmList .bm')].find((b) => b.textContent.includes('資料夾'))"
OTHER = "document.getElementById('bmOther')"


def click_bar(el_js, fallback, **kw):
    click(*C(*bar_pt(el_js, fallback)), **kw)


def active_tab():
    """目前的分頁是第幾個（讀不到回傳 None）"""
    return bar_eval("[...document.querySelectorAll('#tabs .tab')].findIndex((t) => t.classList.contains('active'))")


def active_url():
    return bar_eval("document.getElementById('addr').title")


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


def diag():
    try:
        ps = ("Get-CimInstance Win32_Process | Where-Object { $_.Name -eq 'toolbox.exe' -or ($_.Name -eq 'msedgewebview2.exe' -and $_.CommandLine -notmatch '--type=') } | "
              "ForEach-Object { \"$($_.ProcessId) parent=$($_.ParentProcessId) $($_.Name) :: $($_.CommandLine)\" }")
        out = subprocess.run(["powershell", "-NoProfile", "-Command", ps], capture_output=True, text=True, timeout=60).stdout
        log("程序：\n" + "\n".join(l[:600] for l in out.splitlines() if l.strip()))
    except Exception as e:
        log("讀程序失敗", e)
    try:
        out = subprocess.run(["netstat", "-ano"], capture_output=True, text=True, timeout=30).stdout
        log("9222：", [l.strip() for l in out.splitlines() if ":9222" in l])
    except Exception as e:
        log("netstat 失敗", e)
    log("app_proc", app_proc.pid, "還在跑" if app_proc.poll() is None else f"已結束 {app_proc.poll()}")


def s_start():
    diag()
    log("devtools ports", devtools_ports(), "targets", [(t.get("port"), t.get("type"), t.get("url", "")[:60]) for t in cdp_targets()])
    log("工具列讀得到", bar_eval("1 + 1") == 2)
    dump(web, "start")
    kids = [c for c in children(web) if user32.IsWindowVisible(c)]
    check("一開始：看得到工具列和網頁", len(kids) >= 2, f"{len(kids)} 個可見子視窗")
    r, c = wrect(web), client(web)
    check("一開始：整個視窗都是內容（沒有看不到的邊）", (r[0], r[1], r[2] - r[0], r[3] - r[1]) == c, f"rect={r} client={c}")
    check("一開始：大小和上次記住的一樣（900×620）", (c[2], c[3]) == (round(900 * scale), round(620 * scale)), f"client={c}")


def s_max_button():
    before = wrect(web)
    log("按之前的前景視窗", fg(), repr(title(fg())), "web", web, "game", game)
    log(f"web 視窗出現後 {time.time() - WEB_AT:.1f} 秒")
    for k in range(3):
        t0 = time.time()
        click(*C(W() - 69, 20), wait=0.05)
        z = wait_for(lambda: user32.IsZoomed(web), 5, 0.05)
        log(f"第 {k + 1} 次按放到最大：{time.time() - t0:.2f} 秒" if z else "5 秒內沒有放到最大", "前景", fg())
        time.sleep(0.8)
        if k == 2:
            break
        t0 = time.time()
        click(*C(W() - 69, 20), wait=0.05)
        z = wait_for(lambda: not user32.IsZoomed(web), 5, 0.05)
        log(f"第 {k + 1} 次按還原：{time.time() - t0:.2f} 秒" if z else "5 秒內沒有還原")
        time.sleep(0.8)
    check("按放到最大按鈕：視窗放到最大", user32.IsZoomed(web), f"before={before} after={wrect(web)}")
    vis = [wrect(c) for c in children(web) if user32.IsWindowVisible(c)]
    tops = sorted(r[1] for r in vis)
    check("放到最大後：工具列在最上面、網頁在下面", len(vis) >= 2 and tops[0] <= client(web)[1] + 2, f"{vis}")
    # 放到最大時滑鼠移到螢幕最上緣也點得到分頁
    x, _ = bar_pt(TAB(1), (tab_x(1), 23))
    sx, _ = C(x, 0)
    click(sx, client(web)[1], wait=1.5)
    check("放到最大時點螢幕最上緣：點得到分頁", "短影片" in title(web), f"title={title(web)}")
    x, _ = bar_pt(TAB(0), (tab_x(0), 23))
    sx, _ = C(x, 0)
    click(sx, client(web)[1], wait=1.0)
    check("放到最大時點螢幕最上緣：切回第一個分頁", "測試頁" in title(web), f"title={title(web)}")


def s_restore_button():
    click_bar(BTN_MAX, (W() - 69, 20), wait=1.2)
    check("再按一次：還原", not user32.IsZoomed(web), f"{wrect(web)}")
    r, c = wrect(web), client(web)
    check("還原後：大小回到原本的（900×620）", (c[2], c[3]) == (round(900 * scale), round(620 * scale)), f"rect={r} client={c}")


def slides_since(t0):
    res = []
    for l in page_log().splitlines():
        try:
            t = float(l.split()[0])
        except Exception:
            continue
        if t >= t0 and "[snap]" in l and "slide:" in l:
            res.append(int(l.split("slide:")[1].split()[0]))
    return res


def s_snap_resize():
    click_bar(TAB(1), (tab_x(1), 23), wait=2.5)
    check("切到短影片分頁", "短影片" in title(web), title(web))
    px, py = C(400, 300)
    for _ in range(3):
        wheel(px, py, 1)
        time.sleep(0.7)
    time.sleep(1)
    cur = (slides_since(0) or [1])[-1]
    log("捲動後是第", cur, "張")
    t0 = time.time()
    click_bar(BTN_MAX, (W() - 69, 20), wait=2.0)
    shot("snap_maximized")
    click_bar(BTN_MAX, (W() - 69, 20), wait=2.0)
    time.sleep(1)
    after = slides_since(t0)
    log("放大、還原之後：", after)
    check("短影片頁面：捲動有換到下一張", cur > 1, f"{cur}")
    check("短影片頁面：放大、還原後還是同一張", len(after) > 0 and all(s == cur for s in after), f"捲動後={cur} 之後={after}")


def s_double_click():
    click_bar(DRAG, (W() - 200, 12), double=True, wait=1.5)
    check("按兩下分頁列空白處：放到最大", user32.IsZoomed(web), f"{wrect(web)}")
    click_bar(DRAG, (W() - 200, 12), double=True, wait=1.5)
    check("再按兩下：還原", not user32.IsZoomed(web), f"{wrect(web)}")


def s_drag_snap():
    before = wrect(web)
    x, y = C(*bar_pt(DRAG, (W() - 200, 15)))
    drag(x, y, x, 2, wait=1.5)
    check("拖到螢幕頂端：放到最大", user32.IsZoomed(web), f"{wrect(web)}")
    shot("drag_top")
    x, y = C(*bar_pt(DRAG, (W() / 2, 15)))
    drag(x, y, x - 300, y + 150, wait=1.5)
    after = wrect(web)
    check("放到最大時往下拖：還原成原本大小", not user32.IsZoomed(web) and abs((after[2] - after[0]) - (before[2] - before[0])) < 30
          and abs((after[3] - after[1]) - (before[3] - before[1])) < 30, f"before={before} after={after}")
    # 放回原位，後面的測試才在螢幕裡
    user32.MoveWindow(web, before[0], before[1], before[2] - before[0], before[3] - before[1], True)
    time.sleep(1.0)
    dump(web, "moved back")
    vis = [wrect(c) for c in children(web) if user32.IsWindowVisible(c)]
    check("用別的程式移動視窗：網頁跟著放好", any(r[1] == before[1] for r in vis) and any(r[1] == before[1] + round(108 * scale) for r in vis), f"{vis}")


def s_tabs():
    click_bar(TAB(2), (tab_x(2), 23), wait=2.5)
    shot("tab3")
    a = active_tab()
    check("點第三個分頁：切過去", a == 2 if a is not None else "Example" in title(web), f"active={a} title={title(web)}")
    click_bar(TAB(0), (tab_x(0), 23), wait=1.5)
    kids = [c for c in children(web) if user32.IsWindowVisible(c)]
    check("切換分頁：只有一個網頁顯示", len(kids) == 2, f"{[wrect(c) for c in kids]}")
    check("切回第一個分頁", "測試頁" in title(web), title(web))


def s_menu():
    click_bar(FOLDER, (143, 93), wait=1.2)
    m = menu_win()
    check("點書籤資料夾：出現選單", m and user32.IsWindowVisible(m), f"menu={m}")
    shot("menu_open")
    click(*C(W() / 2, 400), wait=1.0)
    check("點外面：選單關掉", not (m and user32.IsWindowVisible(m)))
    check("選單關掉後程式有回應", responsive(web))
    click_bar(OTHER, (W() - 55, 93), wait=1.2)
    m = menu_win()
    check("點所有書籤：出現選單", m and user32.IsWindowVisible(m))
    shot("menu_other")
    click_bar(OTHER, (W() - 55, 93), wait=1.0)
    check("再點一次所有書籤：選單關掉", not (m and user32.IsWindowVisible(m)))
    check("所有書籤操作後程式有回應", responsive(web))


def s_new_tab():
    n0 = tab_count()
    click_bar(NEWTAB, (plus_x(), 23), wait=1.0)
    type_text(f"http://127.0.0.1:{PORT}/test.html?new")
    key(0x0D)
    time.sleep(2.5)
    shot("new_tab_loaded")
    check("按 +：多一個分頁", tab_count() == n0 + 1, f"{n0} → {tab_count()}")
    check("新分頁輸入網址：有載入", "?new" in page_log(), page_log()[-300:])


def s_game():
    # 回到第一個分頁（測試頁）
    click_bar(TAB(0), (tab_x(0), 23), wait=1.5)
    # 遊戲上沒有被 web 視窗蓋住、也不在工作列上的點
    g, r = wrect(game), wrect(web)
    sh = user32.GetSystemMetrics(1) - 60
    cands = [(g[0] + 40, g[1] + 60), (g[2] - 40, g[1] + 60), (g[0] + 40, min(g[3], sh) - 30), (g[2] - 40, min(g[3], sh) - 30)]
    gx, gy = next(((x, y) for x, y in cands if not (r[0] <= x < r[2] and r[1] <= y < r[3])), cands[0])
    log("遊戲要點的位置", (gx, gy), "game", g, "web", r)
    click(gx, gy, wait=1.0)
    check("點遊戲：遊戲在前景", fg() == game, f"fg={fg()} game={game}")
    check("遊戲在前景：web 視窗在遊戲上面", above(web, game), f"z={zorder()[:6]}")
    click(*C(5 + 200, 108 + 320), wait=0.8)
    check("點網頁空白處：鍵盤還在遊戲", fg() == game, f"fg={fg()}")
    check("點網頁空白處：web 視窗還在遊戲上面", above(web, game))
    n0 = tab_count()
    click_bar(NEWTAB, (plus_x(), 23), wait=1.2)
    n1 = tab_count()
    check("遊戲在前景時點 web 的按鈕：第一下就有作用", n1 == n0 + 1, f"{n0} → {n1}")
    # 中鍵關掉剛開的分頁，回到測試頁
    if n1 == n0 + 1:
        click(*C(tab_x(n1 - 1), 23), button="middle", wait=1.0)
        click_bar(TAB(0), (tab_x(0), 23), wait=1.0)
        check("中鍵關掉分頁", tab_count() == n0, f"{tab_count()}")
    click(gx, gy, wait=1.0)
    check("再點遊戲：遊戲在前景", fg() == game, f"fg={fg()}")
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
    # 打字中自己點遊戲（不是打完）：之後從遊戲再點輸入框，還是要把鍵盤交給 web
    click(*C(5 + 200, 108 + 40), wait=1.0)
    check("再點網頁輸入框：鍵盤交給 web", fg() == web, f"fg={fg()}")
    click(gx, gy, wait=1.0)
    check("打字中點遊戲：遊戲在前景", fg() == game, f"fg={fg()}")
    click(*C(5 + 200, 108 + 40), wait=1.0)
    check("點遊戲後再點輸入框：鍵盤又交給 web", fg() == web, f"fg={fg()}")
    type_text("w")
    time.sleep(0.5)
    check("再打字：網頁收到", "input:helloxyw" in page_log(), page_log()[-160:])
    key(0x0D)
    time.sleep(1.0)
    check("再按 Enter：鍵盤還給遊戲", fg() == game, f"fg={fg()}")
    move(gx, gy)
    time.sleep(0.8)
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
        # 記事本放在 web 視窗的右下（蓋住一部分網頁，不蓋到分頁）
        r = wrect(web)
        user32.MoveWindow(np, r[0] + 300, r[1] + 200, 500, 350, True)
        time.sleep(0.5)
        click(wrect(np)[0] + 250, wrect(np)[1] + 200, wait=1.0)
        check("記事本在前景時可以蓋住 web 視窗", above(np, web), f"fg={fg()} np={np}")
        shot("notepad_over_web")
        click_bar(TAB(0), (tab_x(0), 23), wait=1.0)
        check("點 web 視窗：變成前景（一般視窗）", fg() == web and above(web, np), f"fg={fg()}")
    np_proc.kill()


# ---------- 改變螢幕縮放比例（模擬視窗移到縮放比例不同的螢幕） ----------
class LUID(ctypes.Structure):
    _fields_ = [("LowPart", wt.DWORD), ("HighPart", wt.LONG)]


class DCHEADER(ctypes.Structure):
    _fields_ = [("type", ctypes.c_int), ("size", ctypes.c_uint32), ("adapterId", LUID), ("id", ctypes.c_uint32)]


class DPI_GET(ctypes.Structure):
    _fields_ = [("header", DCHEADER), ("minScaleRel", ctypes.c_int32), ("curScaleRel", ctypes.c_int32), ("maxScaleRel", ctypes.c_int32)]


class DPI_SET(ctypes.Structure):
    _fields_ = [("header", DCHEADER), ("scaleRel", ctypes.c_int32)]


class DEVMODEW(ctypes.Structure):
    _fields_ = [
        ("dmDeviceName", ctypes.c_wchar * 32),
        ("dmSpecVersion", wt.WORD), ("dmDriverVersion", wt.WORD), ("dmSize", wt.WORD), ("dmDriverExtra", wt.WORD),
        ("dmFields", wt.DWORD),
        ("dmPositionX", wt.LONG), ("dmPositionY", wt.LONG), ("dmDisplayOrientation", wt.DWORD), ("dmDisplayFixedOutput", wt.DWORD),
        ("dmColor", ctypes.c_short), ("dmDuplex", ctypes.c_short), ("dmYResolution", ctypes.c_short), ("dmTTOption", ctypes.c_short),
        ("dmCollate", ctypes.c_short),
        ("dmFormName", ctypes.c_wchar * 32),
        ("dmLogPixels", wt.WORD), ("dmBitsPerPel", wt.DWORD), ("dmPelsWidth", wt.DWORD), ("dmPelsHeight", wt.DWORD),
        ("dmDisplayFlags", wt.DWORD), ("dmDisplayFrequency", wt.DWORD),
        ("dmICMMethod", wt.DWORD), ("dmICMIntent", wt.DWORD), ("dmMediaType", wt.DWORD), ("dmDitherType", wt.DWORD),
        ("dmReserved1", wt.DWORD), ("dmReserved2", wt.DWORD), ("dmPanningWidth", wt.DWORD), ("dmPanningHeight", wt.DWORD),
    ]


def display_source():
    np_, nm = wt.UINT(0), wt.UINT(0)
    if user32.GetDisplayConfigBufferSizes(2, ctypes.byref(np_), ctypes.byref(nm)) != 0:
        return None
    paths = (ctypes.c_ubyte * (72 * max(1, np_.value)))()
    modes = (ctypes.c_ubyte * (64 * max(1, nm.value)))()
    if user32.QueryDisplayConfig(2, ctypes.byref(np_), paths, ctypes.byref(nm), modes, None) != 0:
        return None
    low, high, sid = struct.unpack_from("<IiI", bytes(paths[:12]))
    return low, high, sid


def dpi_info():
    src = display_source()
    if not src:
        return None
    g = DPI_GET()
    g.header.type = -3
    g.header.size = ctypes.sizeof(DPI_GET)
    g.header.adapterId = LUID(src[0], src[1])
    g.header.id = src[2]
    if user32.DisplayConfigGetDeviceInfo(ctypes.byref(g)) != 0:
        return None
    return src, g.minScaleRel, g.curScaleRel, g.maxScaleRel


def set_dpi_rel(src, rel):
    st_ = DPI_SET()
    st_.header.type = -4
    st_.header.size = ctypes.sizeof(DPI_SET)
    st_.header.adapterId = LUID(src[0], src[1])
    st_.header.id = src[2]
    st_.scaleRel = rel
    return user32.DisplayConfigSetDeviceInfo(ctypes.byref(st_))


def set_resolution(w, h):
    dm = DEVMODEW()
    dm.dmSize = ctypes.sizeof(DEVMODEW)
    modes = set()
    i = 0
    while user32.EnumDisplaySettingsW(None, i, ctypes.byref(dm)):
        modes.add((dm.dmPelsWidth, dm.dmPelsHeight))
        i += 1
    log("可用解析度", sorted(modes))
    if (w, h) not in modes:
        cands = sorted([m for m in modes if m[0] >= 1600], key=lambda m: m[0] * m[1])
        if not cands:
            return False
        w, h = cands[0]
    dm = DEVMODEW()
    dm.dmSize = ctypes.sizeof(DEVMODEW)
    user32.EnumDisplaySettingsW(None, -1, ctypes.byref(dm))
    dm.dmPelsWidth, dm.dmPelsHeight = w, h
    dm.dmFields = 0x00080000 | 0x00100000
    r = user32.ChangeDisplaySettingsW(ctypes.byref(dm), 0)
    log("改解析度", (w, h), "結果", r)
    return r == 0


def layout_ok(sc):
    vis = [wrect(c) for c in children(web) if user32.IsWindowVisible(c)]
    cx, cy, cw, ch = client(web)
    bar_h = round(108 * sc)
    bar = [r for r in vis if r[1] == cy and r[3] - r[1] == bar_h]
    page = [r for r in vis if abs(r[1] - (cy + bar_h)) <= 1 and r[3] <= cy + ch]
    return bool(bar) and bool(page), vis


def s_dpi():
    user32.SetProcessDPIAware()
    info = dpi_info()
    log("縮放比例資訊", info)
    if not info or info[3] <= info[2]:
        if not set_resolution(1920, 1080):
            log("沒辦法改解析度，略過縮放比例測試")
            return
        time.sleep(3)
        info = dpi_info()
        log("改解析度後的縮放比例資訊", info)
    if not info or info[3] <= info[2]:
        log("這台測試機不能調高縮放比例，略過")
        return
    src, lo, cur, hi = info
    # 切到短影片分頁（捲到第 4 張了），看縮放比例改變後還是不是同一張
    click(*C(tab_x(1), 23), wait=1.5)
    t0 = time.time()
    d0 = user32.GetDpiForWindow(web)
    r = set_dpi_rel(src, cur + 1)
    time.sleep(4)
    d1 = user32.GetDpiForWindow(web)
    log("改縮放比例", r, "DPI", d0, "→", d1)
    shot("dpi_up")
    dump(web, "dpi up")
    ok, vis = layout_ok(d1 / 96)
    check("縮放比例改變後：視窗還看得到", user32.IsWindowVisible(web) and not user32.IsIconic(web))
    check("縮放比例改變後：工具列和網頁位置正確", ok, f"dpi={d1} {vis}")
    after = slides_since(t0)
    check("縮放比例改變後：短影片還是同一張", all(x == 4 for x in after), f"{after}")
    check("縮放比例改變後：程式有回應", responsive(web))
    t0 = time.time()
    set_dpi_rel(src, cur)
    time.sleep(4)
    d2 = user32.GetDpiForWindow(web)
    log("縮放比例改回來 DPI", d2)
    shot("dpi_back")
    dump(web, "dpi back")
    ok, vis = layout_ok(d2 / 96)
    check("縮放比例改回來：工具列和網頁位置正確", ok, f"dpi={d2} {vis}")
    after = slides_since(t0)
    check("縮放比例改回來：短影片還是同一張", all(x == 4 for x in after), f"{after}")
    click(*C(tab_x(0), 23), wait=1.0)


def s_fullscreen():
    click_bar(TAB(0), (tab_x(0), 23), wait=1.0)
    n0 = len([c for c in children(web) if user32.IsWindowVisible(c)])
    click(*C(5 + 100, 108 + 420), wait=2.0)
    shot("fullscreen")
    r = wrect(web)
    sw, sh = user32.GetSystemMetrics(0), user32.GetSystemMetrics(1)
    vis = [wrect(c) for c in children(web) if user32.IsWindowVisible(c)]
    check("影片全螢幕：視窗蓋滿整個螢幕", r == (0, 0, sw, sh), f"{r} screen={sw}x{sh}")
    check("影片全螢幕：工具列藏起來、網頁蓋滿", len(vis) == 1 and vis[0] == (0, 0, sw, sh), f"{vis}")
    check("影片全螢幕：網頁收到", "fullscreen on" in page_log() or "fullscreenchange true" in page_log(), page_log()[-200:])
    key(0x1B)
    time.sleep(2.0)
    shot("fullscreen_off")
    r = wrect(web)
    vis = [wrect(c) for c in children(web) if user32.IsWindowVisible(c)]
    check("按 Esc：回到原本的大小", r == (80, 60, 980, 680), f"{r}")
    check("按 Esc：工具列回來", len(vis) == n0 and any(x[1] == r[1] for x in vis), f"{vis}")


def s_ext():
    before = [c for c in children(web) if user32.IsWindowVisible(c)]
    click(*C(W() - 23, 59), wait=1.5)
    m = menu_win()
    check("按擴充功能按鈕：出現選單", m and user32.IsWindowVisible(m), f"{m}")
    shot("ext_menu")
    if m:
        mr = wrect(m)
        # 第一列是「擴充功能」標題，第二列才是第一個擴充功能
        click(mr[0] + 60, mr[1] + 5 + 30 + 15, wait=3.0)
    shot("ext_popup")
    after = [c for c in children(web) if user32.IsWindowVisible(c)]
    check("點擴充功能：小視窗蓋在網頁上", len(after) == len(before) + 1, f"{[wrect(c) for c in after]}")
    log("ext log:", [l for l in page_log().splitlines() if "[ext]" in l][-6:])
    # WebView2 把每個網頁當成各自的視窗，擴充功能的小視窗查不到「目前的分頁」（只記錄，不算失敗）
    act = next((l for l in page_log().splitlines() if "[ext] active:" in l), "")
    check("擴充功能看得到目前的網頁（active tab）", "test.html" in act and "popup.html" not in act, act[-200:])
    lf = next((l for l in page_log().splitlines() if "[ext] lastFocused:" in l), "")
    check("擴充功能看得到目前的網頁（lastFocused）", "test.html" in lf and "popup.html" not in lf, lf[-200:])
    click(*C(5 + 100, 108 + 320), wait=1.0)
    after2 = [c for c in children(web) if user32.IsWindowVisible(c)]
    check("點外面：小視窗關掉", len(after2) == len(before), f"{len(after2)}")


def s_minimize():
    click_bar(BTN_MIN, (W() - 115, 20), wait=1.2)
    check("按縮小：縮到工作列", user32.IsIconic(web))
    user32.ShowWindow(web, 9)
    time.sleep(1.2)
    check("從工作列還原", not user32.IsIconic(web) and user32.IsWindowVisible(web))
    r, c = wrect(web), client(web)
    check("還原後：大小沒變", (c[2], c[3]) == (round(900 * scale), round(620 * scale)), f"rect={r} client={c}")


def s_close():
    click_bar(BTN_CLOSE, (W() - 23, 20), wait=1.5)
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
    ("dpi", s_dpi),
    ("fullscreen", s_fullscreen),
    ("ext", s_ext),
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
