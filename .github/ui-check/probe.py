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


def hover(x, y, settle=0.5):
    """滑鼠移過去並停住（分幾步走，網頁才會收到移動）"""
    p = wt.POINT()
    user32.GetCursorPos(ctypes.byref(p))
    for i in range(1, 6):
        move(p.x + (x - p.x) * i / 5, p.y + (y - p.y) * i / 5)
        time.sleep(0.03)
    move(x + 1, y)
    time.sleep(0.03)
    move(x, y)
    time.sleep(settle)


def chord(mods, vk, wait=0.5):
    for m in mods:
        user32.keybd_event(m, 0, 0, 0)
    time.sleep(0.04)
    key(vk)
    time.sleep(0.04)
    for m in reversed(mods):
        user32.keybd_event(m, 0, 2, 0)
    time.sleep(wait)


VK_ENTER, VK_ESC, VK_TAB, VK_CTRL, VK_SHIFT, VK_ALT = 0x0D, 0x1B, 0x09, 0x11, 0x10, 0x12
VK_LEFT, VK_UP, VK_RIGHT, VK_DOWN, VK_F5, VK_F11 = 0x25, 0x26, 0x27, 0x28, 0x74, 0x7A
VK_PLUS, VK_MINUS = 0xBB, 0xBD

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
        if u.path == "/dl.bin":
            body = b"x" * 300000
            self.send_response(200)
            self.send_header("Content-Type", "application/octet-stream")
            self.send_header("Content-Disposition", 'attachment; filename="ui-check.bin"')
            self.send_header("Content-Length", str(len(body)))
            self.end_headers()
            self.wfile.write(body)
            return
        super().do_GET()

    def log_message(self, *a):
        pass


socketserver.ThreadingTCPServer.allow_reuse_address = True
srv = socketserver.ThreadingTCPServer(("0.0.0.0", PORT), Handler)
threading.Thread(target=srv.serve_forever, daemon=True).start()


def page_log():
    return open(PAGE_LOG, encoding="utf-8").read()


def page_log_since(t0):
    res = []
    for l in page_log().splitlines():
        try:
            if float(l.split()[0]) >= t0:
                res.append(l)
        except Exception:
            pass
    return res


GAME_LOG = os.path.join(OUT, "game_keys.txt")
open(GAME_LOG, "w").close()


def game_keys():
    return open(GAME_LOG, encoding="utf-8").read()


# ---------- 使用統計不要送到作者的後台（這台測試機不該出現在那裡） ----------
try:
    src = open(os.path.join(HERE, "..", "..", "src-tauri", "src", "stats.rs"), encoding="utf-8").read()
    host = src.split('STATS_URL: &str = "https://')[1].split('"')[0].split("/")[0]
    with open(os.path.join(os.environ["SystemRoot"], "System32", "drivers", "etc", "hosts"), "a") as f:
        f.write(f"\r\n0.0.0.0 {host}\r\n")
    subprocess.run(["ipconfig", "/flushdns"], capture_output=True)
    log("stats host blocked")
except Exception as e:
    log("hosts 寫入失敗", e)

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
        "next_id": 5,
    },
}
with open(os.path.join(data_dir, "web.json"), "w", encoding="utf-8") as f:
    json.dump(web_json, f, ensure_ascii=False)
# 測試用的使用者腳本
os.makedirs(os.path.join(data_dir, "web-scripts"), exist_ok=True)
with open(os.path.join(data_dir, "web-scripts", "t1.user.js"), "w", encoding="utf-8") as f:
    f.write("// ==UserScript==\n// @name 測試腳本一\n// @match http://127.0.0.1:8765/test.html*\n// @grant GM_setValue\n// ==/UserScript==\n"
            "GM_setValue('n', (GM_getValue('n', 0) || 0) + 1);\nfetch('/log?m=' + encodeURIComponent('[us] end ran n=' + GM_getValue('n') + ' ' + location.pathname));\n")
with open(os.path.join(data_dir, "web-scripts", "t2.user.js"), "w", encoding="utf-8") as f:
    f.write("// ==UserScript==\n// @name 測試腳本二\n// @match http://127.0.0.1:8765/*\n// @match http://localhost:8765/*\n// @exclude http://localhost:8765/frame.html\n// @run-at document-start\n// ==/UserScript==\n"
            "fetch('/log?m=' + encodeURIComponent('[us] start ran ' + location.pathname + ' body=' + !!document.body));\n")
with open(os.path.join(data_dir, "web-scripts", "t3.user.js"), "w", encoding="utf-8") as f:
    f.write("// ==UserScript==\n// @name 不該跑\n// @match https://example.com/*\n// ==/UserScript==\nfetch('http://127.0.0.1:8765/log?m=' + encodeURIComponent('[us] WRONG'));\n")

# 一個 Chrome 使用者（書籤、一個擴充功能）：打開視窗時會自動同步進來
EXT_ID = "eamehjingoajmedombhecpkpagoadodo"
chrome_root = os.path.join(os.environ["LOCALAPPDATA"], "Google", "Chrome", "User Data")
CHROME_SEEDED = not os.path.exists(os.path.join(chrome_root, "Default", "Bookmarks"))
if CHROME_SEEDED:
    os.makedirs(os.path.join(chrome_root, "Default"), exist_ok=True)
    chrome_bm = {
        "roots": {
            "bookmark_bar": {"type": "folder", "id": "1", "name": "書籤列", "children": [
                {"type": "url", "id": "5", "name": "YouTube", "url": "https://www.youtube.com/"},
                {"type": "folder", "id": "7", "name": "楓之谷", "children": [
                    {"type": "url", "id": "8", "name": "官網", "url": "https://maplestory.beanfun.com/"},
                    {"type": "folder", "id": "9", "name": "攻略", "children": [
                        {"type": "url", "id": "10", "name": "短影片", "url": f"{base}/snap.html?deep"},
                    ]},
                    {"type": "url", "id": "11", "name": "IANA", "url": "https://www.iana.org/"},
                ]},
            ]},
            "other": {"type": "folder", "id": "2", "name": "其他書籤", "children": [
                {"type": "url", "id": "14", "name": "MDN", "url": "https://developer.mozilla.org/"},
            ]},
            "synced": {"type": "folder", "id": "3", "name": "行動裝置書籤", "children": []},
        },
        "version": 1,
    }
    with open(os.path.join(chrome_root, "Default", "Bookmarks"), "w", encoding="utf-8") as f:
        json.dump(chrome_bm, f, ensure_ascii=False)
    if not os.path.exists(os.path.join(chrome_root, "Local State")):
        with open(os.path.join(chrome_root, "Local State"), "w", encoding="utf-8") as f:
            json.dump({"profile": {"info_cache": {"Default": {"name": "測試使用者"}}, "last_used": "Default"}}, f, ensure_ascii=False)
    shutil.copytree(os.path.join(HERE, "ext"), os.path.join(chrome_root, "Default", "Extensions", EXT_ID, "1.0_0"), dirs_exist_ok=True)
log("chrome profile seeded", CHROME_SEEDED)

# ---------- 開假的遊戲、開程式 ----------
game_proc = subprocess.Popen([sys.executable, os.path.join(HERE, "game.py"), GAME_LOG])
game = wait_for(lambda: next((h for h in top_windows() if title(h) == "MapleStory" and user32.IsWindowVisible(h)), 0), 15)
log("game hwnd", game, wrect(game) if game else None)

# 只有測試時打開 WebView2 的遠端偵錯，用來讀工具列、選單、頁面裡元素的實際位置和內容。
# 連接埠用 0（每個瀏覽器程序自己挑一個，寫在資料夾的 DevToolsActivePort）：工具列／選單和網頁是兩個不同的瀏覽器程序
DEBUG_ARGS = "--remote-debugging-port=0"
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
MENU_TITLE = "瀏覽器選單"
KNOWN = ("工具箱", MENU_TITLE, "計時器浮動視窗", "", "com.toolbox.desktop-siw")


def find_web():
    for h in app_windows():
        if user32.IsWindowVisible(h) and title(h) not in KNOWN and len(children(h)) >= 2:
            return h
    return 0


web = wait_for(find_web, 45)
WEB_AT = time.time()
log("web hwnd", web)
if not web:
    for h in app_windows():
        log("app window", h, repr(title(h)), cls(h), user32.IsWindowVisible(h), wrect(h))
    shot("no_web_window")
    sys.exit(1)
time.sleep(5)
scale = (user32.GetDpiForWindow(web) or 96) / 96
log("dpi scale", scale, "screen", user32.GetSystemMetrics(0), user32.GetSystemMetrics(1))

# 工具列的高度（CSS 像素）：分頁列 48（放到最大 40）＋工具列 46＋書籤列 34＋線 1；網頁左右下各留 5 給拉大小
BAR_H, BAR_H_MAX, EDGE = 129, 121, 5


def C(x, y):
    """web 視窗內容區的座標（CSS 像素）→ 螢幕座標"""
    cx, cy, cw, ch = client(web)
    return cx + x * scale, cy + y * scale


def W():
    return client(web)[2] / scale


def vis_kids(h=None):
    return [c for c in children(h or web) if user32.IsWindowVisible(c)]


def page_rect():
    """目前顯示的網頁（子視窗）的範圍：不是工具列的那個、最大的"""
    cx, cy, cw, ch = client(web)
    best = None
    for c in vis_kids():
        r = wrect(c)
        is_bar = r[1] == cy and r[0] == cx and (r[3] - r[1]) <= 140 * scale
        if is_bar:
            continue
        if best is None or (r[2] - r[0]) * (r[3] - r[1]) > (best[2] - best[0]) * (best[3] - best[1]):
            best = r
    if best is None:
        return (cx + EDGE * scale, cy + BAR_H * scale, cx + cw - EDGE * scale, cy + ch - EDGE * scale)
    return best


def PG(x, y):
    """目前網頁裡的座標（CSS 像素）→ 螢幕座標"""
    r = page_rect()
    return r[0] + x * scale, r[1] + y * scale



# ---------- 探測：UI Automation 讀不讀得到 WebView2 裡的東西 ----------
import uiautomation as auto

auto.SetGlobalSearchTimeout(2)


def uia_dump(hwnd, tag, max_depth=40, limit=500):
    log(f"\n--- UIA {tag} hwnd={hwnd} title={title(hwnd)!r} rect={wrect(hwnd)}")
    t0 = time.time()
    n = [0]

    def walk(c, d):
        if n[0] >= limit or d > max_depth:
            return
        n[0] += 1
        try:
            r = c.BoundingRectangle
            val = ""
            if c.ControlTypeName == "EditControl":
                try:
                    val = c.GetValuePattern().Value
                except Exception as e:
                    val = f"(value err {e})"
            log("  " * d + f"{c.ControlTypeName} name={c.Name!r} rect=({r.left},{r.top},{r.right},{r.bottom}) en={c.IsEnabled} off={c.IsOffscreen} foc={c.HasKeyboardFocus} class={c.ClassName!r} id={c.AutomationId!r} {val!r}")
            for ch in c.GetChildren():
                walk(ch, d + 1)
        except Exception as e:
            log("  " * d + f"(err {e})")

    try:
        walk(auto.ControlFromHandle(hwnd), 0)
    except Exception:
        log(traceback.format_exc())
    log(f"--- {n[0]} nodes in {time.time() - t0:.2f}s")


def procs():
    try:
        exe = os.path.basename(EXE)
        ps = ("Get-CimInstance Win32_Process | Where-Object { $_.Name -eq '" + exe + "' -or ($_.Name -eq 'msedgewebview2.exe' -and $_.CommandLine -notmatch '--type=') } | "
              "ForEach-Object { \"$($_.ProcessId) parent=$($_.ParentProcessId) $($_.Name) :: $($_.CommandLine)\" }")
        out = subprocess.run(["powershell", "-NoProfile", "-Command", ps], capture_output=True, text=True, timeout=60).stdout
        log("程序：\n" + "\n".join(l[:700] for l in out.splitlines() if l.strip()))
    except Exception as e:
        log("讀程序失敗", e)


procs()
shot("start")
for h in app_windows():
    if user32.IsWindowVisible(h):
        log("app window", h, repr(title(h)), cls(h), wrect(h), [(cls(c), wrect(c), bool(user32.IsWindowVisible(c))) for c in children(h)])
uia_dump(web, "web 第一次")
time.sleep(1.5)
uia_dump(web, "web 第二次")
# 右上角的 ⋮
click(*C(W() - 23, 71), wait=1.5)
shot("menu")
for h in app_windows():
    if title(h) == MENU_TITLE and user32.IsWindowVisible(h):
        uia_dump(h, "選單 第一次")
        time.sleep(1.0)
        uia_dump(h, "選單 第二次")
click(*C(300, BAR_H + 330), wait=1.0)
# 網頁按右鍵
click(*C(300, BAR_H + 330), button="right", wait=1.5)
shot("context")
for h in app_windows():
    if title(h) == MENU_TITLE and user32.IsWindowVisible(h):
        uia_dump(h, "右鍵選單")
click(*C(400, BAR_H + 300), wait=1.0)
shot("end")
subprocess.run(["taskkill", "/IM", os.path.basename(EXE), "/F"], capture_output=True)
game_proc.kill()
os._exit(0)
