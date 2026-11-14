# 瀏覽器視窗的實機檢查（在 GitHub 的 Windows 測試機上執行，只有 ui-check 分支會跑）
# 把測試版程式實際打開，用滑鼠、鍵盤操作，記錄每一步的截圖、視窗位置與檢查結果到 uitest-out/
import ctypes
import ctypes.wintypes as wt
import http.server
import json
import os
import re
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

from PIL import Image, ImageGrab

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
        # 256 色就夠看（檔案小很多）
        ImageGrab.grab(all_screens=True).convert("RGB").convert("P", palette=Image.ADAPTIVE, colors=256).save(path, optimize=True)
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


# 「從網址安裝」測試用的腳本：版本可以改（測更新）
INST = {"ver": "1.0"}


def inst_script():
    v = INST["ver"]
    return ("// ==UserScript==\n// @name         安裝測試腳本\n// @version      " + v + "\n// @description  從網址安裝的測試\n"
            "// @match        http://127.0.0.1:8765/us.html*\n// @grant        none\n// @downloadURL  http://127.0.0.1:8765/inst.user.js\n// ==/UserScript==\n"
            "fetch('/log?m=' + encodeURIComponent('[inst] ran v=" + v + " ' + location.search));\n")


class Handler(http.server.SimpleHTTPRequestHandler):
    def __init__(self, *a, **k):
        super().__init__(*a, directory=os.path.join(HERE, "pages"), **k)

    def xecho(self, method, body):
        """使用者腳本的連線：回報收到的方法、標頭、內容（不給跨網站許可，網頁自己連會被擋）"""
        h = self.headers
        out = json.dumps({"method": method, "referer": h.get("Referer"), "xtest": h.get("X-Test"), "cookie": h.get("Cookie"), "ua": h.get("User-Agent"),
                          "ctype": h.get("Content-Type"), "body": body}, ensure_ascii=False).encode("utf-8")
        self.send_response(200)
        self.send_header("Content-Type", "application/json; charset=utf-8")
        self.send_header("Content-Length", str(len(out)))
        self.end_headers()
        self.wfile.write(out)

    def plain(self, body, ctype, extra=()):
        body = body.encode("utf-8")
        self.send_response(200)
        self.send_header("Content-Type", ctype)
        self.send_header("Content-Length", str(len(body)))
        self.send_header("Cache-Control", "no-store")
        for k, v in extra:
            self.send_header(k, v)
        self.end_headers()
        self.wfile.write(body)

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
        if u.path == "/login.html":
            with open(PAGE_LOG, "a", encoding="utf-8") as f:
                f.write(f"{time.time():.2f} [hdr] ua={self.headers.get('User-Agent')!r} sec-ch-ua={self.headers.get('Sec-CH-UA')!r}\n")
        if u.path == "/post-echo":
            return self.echo("GET", u.query)
        if u.path == "/xhr-echo":
            return self.xecho("GET", "")
        if u.path == "/xhr-stream":
            # 一段一段送，中間停一下（沒有長度，送完就關）
            self.send_response(200)
            self.send_header("Content-Type", "text/event-stream")
            self.end_headers()
            for part in ("data: 1\n", "data: 2\n", "data: 3\n"):
                self.wfile.write(part.encode())
                self.wfile.flush()
                time.sleep(0.4)
            return
        if u.path == "/setck":
            return self.plain("<!doctype html><meta charset=utf-8><title>Cookie</title>ok", "text/html; charset=utf-8", [("Set-Cookie", "mbck=abc; Path=/")])
        if u.path == "/inst.user.js":
            return self.plain(inst_script(), "text/javascript; charset=utf-8")
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

    def do_POST(self):
        n = int(self.headers.get("Content-Length") or 0)
        body = self.rfile.read(n).decode("utf-8", "replace") if n else ""
        if urllib.parse.urlparse(self.path).path == "/post-echo":
            return self.echo("POST", body)
        if urllib.parse.urlparse(self.path).path == "/xhr-echo":
            return self.xecho("POST", body)
        self.send_response(404)
        self.end_headers()

    def echo(self, method, body):
        """收到表單：回一個網頁，網頁自己回報收到了什麼"""
        info = json.dumps(f"method={method} body={body}")
        html = ("<!doctype html><meta charset=utf-8><title>收到表單</title><body>" + method + "<script>"
                "let op='none';try{op=window.opener?'true':'false'}catch(e){op='error'}"
                "fetch('/log?m='+encodeURIComponent('[post] '+" + info + "+' opener='+op))</script>").encode("utf-8")
        self.send_response(200)
        self.send_header("Content-Type", "text/html; charset=utf-8")
        self.send_header("Content-Length", str(len(html)))
        self.end_headers()
        self.wfile.write(html)

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
    f.write("// ==UserScript==\n// @name 測試腳本一\n// @match http://127.0.0.1:8765/test.html*\n// @grant GM_setValue\n// @grant GM_getValue\n// ==/UserScript==\n"
            "GM_setValue('n', (GM_getValue('n', 0) || 0) + 1);\nfetch('/log?m=' + encodeURIComponent('[us] end ran n=' + GM_getValue('n') + ' ' + location.pathname));\n")
with open(os.path.join(data_dir, "web-scripts", "t2.user.js"), "w", encoding="utf-8") as f:
    f.write("// ==UserScript==\n// @name 測試腳本二\n// @match http://127.0.0.1:8765/*\n// @match http://localhost:8765/*\n// @exclude http://localhost:8765/frame.html\n// @run-at document-start\n// ==/UserScript==\n"
            "window.__us2ran = true;\nfetch('/log?m=' + encodeURIComponent('[us] start ran ' + location.pathname + ' body=' + !!document.body));\n")
for fn in os.listdir(os.path.join(HERE, "userscripts")):
    shutil.copyfile(os.path.join(HERE, "userscripts", fn), os.path.join(data_dir, "web-scripts", fn))
with open(os.path.join(data_dir, "web-scripts", "t3.user.js"), "w", encoding="utf-8") as f:
    f.write("// ==UserScript==\n// @name 不該跑\n// @match https://not-visited.invalid/*\n// ==/UserScript==\nfetch('http://127.0.0.1:8765/log?m=' + encodeURIComponent('[us] WRONG'));\n")

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

log("測試程式是系統管理員", bool(ctypes.windll.shell32.IsUserAnAdmin()))
try:
    ONLY_EARLY = open(os.path.join(HERE, "only.txt"), encoding="utf-8").read().split()
except Exception:
    ONLY_EARLY = []
if "xprobe" in ONLY_EARLY:
    # 跟使用者的電腦一樣：深色模式、語言是繁體中文；再放一個查問題用的腳本（記下網頁在等哪些瀏覽器功能）
    for v in ("AppsUseLightTheme", "SystemUsesLightTheme"):
        subprocess.run(["reg", "add", r"HKCU\Software\Microsoft\Windows\CurrentVersion\Themes\Personalize", "/v", v, "/t", "REG_DWORD", "/d", "0", "/f"], capture_output=True)
    anim = wt.BOOL(0)
    user32.SystemParametersInfoW(0x1042, 0, ctypes.byref(anim), 0)
    was = bool(anim.value)
    user32.SystemParametersInfoW(0x1043, 0, ctypes.c_void_p(1), 3)  # SPI_SETCLIENTAREAANIMATION = 開
    user32.SystemParametersInfoW(0x1042, 0, ctypes.byref(anim), 0)
    log("xprobe：Windows 的動畫效果 原本", was, "→ 現在", bool(anim.value), "遠端工作階段", user32.GetSystemMetrics(0x1000))
    prof = os.path.join(data_dir, "web-profile", "EBWebView", "Default")
    os.makedirs(prof, exist_ok=True)
    with open(os.path.join(prof, "Preferences"), "w", encoding="utf-8") as f:
        json.dump({"intl": {"accept_languages": "zh-TW,zh,en-US,en", "selected_languages": "zh-TW,zh,en-US,en"}}, f)
    XDIAG = open(os.path.join(HERE, "xdiag.js"), encoding="utf-8").read()
    with open(os.path.join(data_dir, "web-scripts", "x-diag.user.js"), "w", encoding="utf-8") as f:
        f.write("// ==UserScript==\n// @name 0 查 X 卡住\n// @match https://x.com/*\n// @run-at document-start\n// @noframes\n// @grant none\n// ==/UserScript==\n" + XDIAG + """
;(function () {
  if (window.top !== window) return;
  var show = function () {
    try {
      var s = 'URL=' + location.href + ' ' + window.__mbSummary();
      var p = document.getElementById('__mbdiag');
      if (!p) { p = document.createElement('div'); p.id = '__mbdiag'; document.documentElement.insertBefore(p, document.documentElement.firstChild); }
      p.style.cssText = 'position:fixed;left:0;top:0;right:0;z-index:2147483647;background:#fff;color:#000;font:10px monospace;max-height:45%;overflow:auto;padding:2px;opacity:.92';
      p.textContent = '';
      for (var i = 0, k = 0; i < s.length; i += 380, k++) { var d = document.createElement('div'); d.textContent = 'MB' + (k < 10 ? '0' : '') + k + ':' + s.slice(i, i + 380); p.appendChild(d); }
    } catch (e) {}
  };
  setTimeout(show, 15000);
  setTimeout(show, 26000);
})();
""")
    log("xprobe：深色模式、zh-TW、放好查問題的腳本")
    # 使用者公開的油猴整合包：只在網址後面有 #mbb 的 x.com 才跑，看它會不會讓 X 停在標誌
    try:
        with urllib.request.urlopen("https://raw.githubusercontent.com/MAOBOO2240/userscripts/main/maoboo.user.js", timeout=40) as r:
            bundle = r.read().decode("utf-8")
        assert "// @match        *://*/*" in bundle
        bundle = bundle.replace("// @match        *://*/*", "// @include      /^https:\\/\\/x\\.com\\/.*#mbb/", 1)
        with open(os.path.join(data_dir, "web-scripts", "maoboo.user.js"), "w", encoding="utf-8") as f:
            f.write(bundle)
        log("xprobe：整合包放好了", len(bundle), [l for l in bundle.splitlines()[:16] if "@include" in l or "@version" in l])
    except Exception as e:
        log("xprobe：整合包拿不到", repr(e))
if "xuser" in ONLY_EARLY:
    for v in ("AppsUseLightTheme", "SystemUsesLightTheme"):
        subprocess.run(["reg", "add", r"HKCU\Software\Microsoft\Windows\CurrentVersion\Themes\Personalize", "/v", v, "/t", "REG_DWORD", "/d", "0", "/f"], capture_output=True)
    user32.SystemParametersInfoW(0x1043, 0, ctypes.c_void_p(1), 3)
    prof = os.path.join(data_dir, "web-profile", "EBWebView", "Default")
    os.makedirs(prof, exist_ok=True)
    with open(os.path.join(prof, "Preferences"), "w", encoding="utf-8") as f:
        json.dump({"intl": {"accept_languages": "zh-TW,zh,en-US,en", "selected_languages": "zh-TW,zh,en-US,en"}}, f)
    shutil.copyfile(os.path.join(HERE, "x-diag.user.js"), os.path.join(data_dir, "web-scripts", "X 診斷（工具箱）.user.js"))
    log("xuser：放好要給使用者的診斷腳本", os.path.getsize(os.path.join(HERE, "x-diag.user.js")))
    with open(os.path.join(data_dir, "web-scripts", "0-freeze.user.js"), "w", encoding="utf-8") as f:
        f.write("""// ==UserScript==
// @name 0 測試用：把網頁卡死
// @match https://x.com/*
// @run-at document-start
// @noframes
// @grant none
// ==/UserScript==
if (/mbfreeze/.test(location.hash)) setTimeout(function () {
  var c = document.createElement('canvas'), gl = c.getContext('webgl');
  if (gl) gl.getParameter(gl.VERSION);
  for (;;) {}
}, 2500);
""")
if "extprobe" in ONLY_EARLY:
    # （查問題用）跟使用者一樣的擴充功能：從 Chrome 線上應用程式商店拿，放進假的 Chrome 使用者資料夾，
    # 程式打開瀏覽器視窗時會自動同步進來
    import base64
    import hashlib
    import io
    import zipfile
    os.environ["WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS"] = "--remote-debugging-port=0 --remote-allow-origins=* --mb-from=env"
    os.environ["MAOBOO_DEBUG"] = "1"
    for root_ in ("HKLM", "HKCU"):
        for name_, mark_ in (("MAOBOO.exe", "exe"), ("*", "star")):
            r_ = subprocess.run(["reg", "add", rf"{root_}\Software\Policies\Microsoft\Edge\WebView2\AdditionalBrowserArguments", "/v", name_, "/t", "REG_SZ",
                                 "/d", f"--remote-debugging-port=0 --remote-allow-origins=* --mb-from={root_}-{mark_}", "/f"], capture_output=True, text=True)
            log("extprobe：登錄檔", root_, name_, r_.returncode, (r_.stdout + r_.stderr).strip()[:120])
    TM_ID = "dhdgffkkebhmkfjojejmpbldmpobfkfo"
    REAL_EXTS = [TM_ID, "bgnkhhnnamicmpeenaelnjfhikgbkllg", "mnjggcdmjocbbbhaepdhchncahnbgone", "gebbhagfogifgggkldgodflihgfeippi",
                 "cgakmbkklpmijhckpolanjijghgfpeoa", "mgojgddhfhekopdpkocobommepgdeffb", "fcoeoabgfenejglbffodgkkbkcdhcgfn",
                 "ghbmnnjooekpmoecnnnilnnbdlolhkhi", "inomeogfingihgjfjlpeplalcfajhgai"]
    try:
        only_ids = open(os.path.join(HERE, "extprobe-ids.txt"), encoding="utf-8").read().split()
        if only_ids:
            REAL_EXTS = only_ids
    except Exception:
        pass

    def _pb_fields(buf):
        """很小的 protobuf 讀法：回傳 [(欄位編號, 型別, 值)]"""
        i, out = 0, []
        while i < len(buf):
            key, shift = 0, 0
            while True:
                b = buf[i]; i += 1
                key |= (b & 0x7F) << shift; shift += 7
                if not b & 0x80:
                    break
            num, wt_ = key >> 3, key & 7
            if wt_ == 0:
                v, shift = 0, 0
                while True:
                    b = buf[i]; i += 1
                    v |= (b & 0x7F) << shift; shift += 7
                    if not b & 0x80:
                        break
                out.append((num, wt_, v))
            elif wt_ == 2:
                ln, shift = 0, 0
                while True:
                    b = buf[i]; i += 1
                    ln |= (b & 0x7F) << shift; shift += 7
                    if not b & 0x80:
                        break
                out.append((num, wt_, buf[i:i + ln])); i += ln
            elif wt_ == 1:
                out.append((num, wt_, buf[i:i + 8])); i += 8
            elif wt_ == 5:
                out.append((num, wt_, buf[i:i + 4])); i += 4
            else:
                break
        return out

    def _crx_id(pub):
        return "".join(chr(ord("a") + int(c, 16)) for c in hashlib.sha256(pub).hexdigest()[:32])

    pv = "150.0.0.0"
    try:
        out = subprocess.run(["reg", "query", r"HKLM\SOFTWARE\WOW6432Node\Microsoft\EdgeUpdate\Clients\{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}", "/v", "pv"], capture_output=True, text=True).stdout
        pv = next((w for w in out.split() if w.count(".") == 3 and w[0].isdigit()), pv)
    except Exception:
        pass
    log("extprobe：這台機器的 WebView2 版本", pv)
    REAL_INFO = {}
    for eid in REAL_EXTS:
        try:
            url = f"https://clients2.google.com/service/update2/crx?response=redirect&prodversion={pv}&acceptformat=crx2,crx3&x=id%3D{eid}%26uc"
            req = urllib.request.Request(url, headers={"User-Agent": f"Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/{pv} Safari/537.36"})
            with urllib.request.urlopen(req, timeout=180) as r:
                data = r.read()
            if data[:4] != b"Cr24":
                log("extprobe：不是 CRX", eid, len(data), data[:40])
                continue
            ver = struct.unpack("<I", data[4:8])[0]
            pub = None
            if ver == 3:
                hlen = struct.unpack("<I", data[8:12])[0]
                header, body = data[12:12 + hlen], data[12 + hlen:]
                keys = []
                for num, wt_, v in _pb_fields(header):
                    if num in (2, 3) and wt_ == 2:
                        for n2, w2, v2 in _pb_fields(v):
                            if n2 == 1 and w2 == 2:
                                keys.append(bytes(v2))
                pub = next((k for k in keys if _crx_id(k) == eid), None)
            elif ver == 2:
                klen, slen = struct.unpack("<II", data[8:16])
                pub, body = data[16:16 + klen], data[16 + klen + slen:]
            else:
                log("extprobe：不認得的 CRX 版本", eid, ver)
                continue
            z = zipfile.ZipFile(io.BytesIO(body))
            man = json.loads(z.read("manifest.json").decode("utf-8-sig"))
            dst = os.path.join(chrome_root, "Default", "Extensions", eid, man.get("version", "1") + "_0")
            os.makedirs(dst, exist_ok=True)
            z.extractall(dst)
            if pub is not None and "key" not in man:
                man["key"] = base64.b64encode(pub).decode()
                with open(os.path.join(dst, "manifest.json"), "w", encoding="utf-8") as f:
                    json.dump(man, f, ensure_ascii=False)
            REAL_INFO[eid] = {"name": man.get("name"), "version": man.get("version"), "mv": man.get("manifest_version"), "key": pub is not None,
                              "perms": man.get("permissions"), "opt": man.get("optional_permissions"), "bg": man.get("background"),
                              "action": (man.get("action") or man.get("browser_action") or {}).get("default_popup"),
                              "options": man.get("options_page") or (man.get("options_ui") or {}).get("page"), "side": man.get("side_panel"),
                              "min": man.get("minimum_chrome_version"), "cs": len(man.get("content_scripts") or [])}
            log("extprobe：放好", eid, len(data), json.dumps(REAL_INFO[eid], ensure_ascii=False)[:700])
        except Exception as e:
            log("extprobe：拿不到", eid, repr(e)[:300])
    try:
        import re as _re
        ad = os.path.join(chrome_root, "Default", "Extensions", "bgnkhhnnamicmpeenaelnjfhikgbkllg")
        for root_, _dirs, files_ in os.walk(ad):
            for fn_ in files_:
                if not fn_.endswith(".js"):
                    continue
                try:
                    src_ = open(os.path.join(root_, fn_), encoding="utf-8", errors="replace").read()
                except Exception:
                    continue
                if fn_ != "background.js":
                    continue
                for word_, n_ in (("openSettingsPage", 2), ("static async openTab", 1), ("static async findOne", 1), ("static async focus", 1), ("OpenSettingsTab,", 2), ("settingsUrl", 2)):
                    hits_ = [m.start() for m in _re.finditer(_re.escape(word_), src_)][:n_]
                    for i_ in hits_:
                        log(f"extprobe：AdGuard background.js 裡的 {word_}：", src_[max(0, i_ - 150):i_ + 900].replace("\n", " "))
    except Exception as e:
        log("extprobe：讀 AdGuard 的程式失敗", repr(e))
    try:
        pref = os.path.join(chrome_root, "Default", "Preferences")
        if not os.path.exists(pref):
            with open(pref, "w", encoding="utf-8") as f:
                json.dump({"extensions": {"pinned_extensions": [TM_ID]}}, f)
    except Exception as e:
        log("extprobe：Preferences 寫不了", e)
if "xcom" in ONLY_EARLY:
    os.environ["WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS"] = "--remote-debugging-port=0 --remote-allow-origins=*"
    log("xcom：打開瀏覽器元件的偵錯埠")
APP_ERR = open(os.path.join(OUT, "app_stderr.txt"), "w", encoding="utf-8", errors="replace")
app_proc = subprocess.Popen([EXE], stdout=APP_ERR, stderr=APP_ERR)
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


# ---------- 讀畫面上的東西：UI Automation（WebView2 會把網頁裡每個元素的 id、class、文字、位置報出來） ----------
import uiautomation as auto

auto.SetGlobalSearchTimeout(2)


class N:
    """畫面上的一個元素"""
    __slots__ = ("type", "name", "l", "t", "r", "b", "en", "foc", "cls", "id", "kids", "ctrl")

    def __repr__(self):
        return f"<{self.type} {self.name!r} #{self.id} .{self.cls} ({self.l},{self.t},{self.r},{self.b})>"

    @property
    def classes(self):
        return self.cls.split()

    def has(self, c):
        return c in self.cls.split()

    @property
    def w(self):
        return self.r - self.l

    @property
    def h(self):
        return self.b - self.t

    @property
    def center(self):
        return ((self.l + self.r) / 2, (self.t + self.b) / 2)

    def walk(self):
        yield self
        for k in self.kids:
            yield from k.walk()

    def texts(self):
        return [n.name for n in self.walk() if n.type == "TextControl" and n.name]


def _node(c, depth, count, limit):
    n = N()
    n.ctrl = c
    n.type = c.ControlTypeName
    n.name = c.Name or ""
    r = c.BoundingRectangle
    n.l, n.t, n.r, n.b = r.left, r.top, r.right, r.bottom
    n.en = bool(c.IsEnabled)
    n.foc = bool(c.HasKeyboardFocus)
    n.cls = c.ClassName or ""
    n.id = c.AutomationId or ""
    n.kids = []
    count[0] += 1
    if depth < 40 and count[0] < limit:
        try:
            for ch in c.GetChildren():
                try:
                    n.kids.append(_node(ch, depth + 1, count, limit))
                except Exception:
                    pass
        except Exception:
            pass
    return n


def render_hwnds(top):
    """一個視窗裡看得到的網頁內容（每個 webview 一個）"""
    res = []

    def rec(h, d):
        for c in children(h):
            if cls(c) == "Chrome_RenderWidgetHostHWND":
                if user32.IsWindowVisible(c):
                    res.append(c)
            elif d < 6:
                rec(c, d + 1)

    rec(top, 0)
    return res


def snap(rh, tries=6, limit=1500, need=None):
    """讀一個網頁內容的元素樹（剛載入時要等一下才讀得到）。need(root) 回傳 True 才算讀好"""
    root = None
    for i in range(tries):
        try:
            pane = auto.ControlFromHandle(rh)
            docs = [c for c in pane.GetChildren() if c.ControlTypeName == "DocumentControl"]
            if docs:
                root = _node(docs[0], 0, [0], limit)
                if root.kids and (need is None or need(root)):
                    return root
        except Exception as e:
            if i == tries - 1:
                log("（讀元素失敗）", e)
        time.sleep(0.35)
    return root


def find_id(root, i):
    if root is None:
        return None
    for n in root.walk():
        if n.id == i:
            return n
    return None


def find_all(root, pred):
    return [n for n in root.walk() if pred(n)] if root is not None else []


def bar_hwnd(win=None):
    win = win or web
    cx, cy, cw, ch = client(win)
    for h in render_hwnds(win):
        r = wrect(h)
        if r[1] == cy and r[0] == cx and (r[3] - r[1]) <= 140 * scale:
            return h
    return 0


def page_hwnd(win=None):
    win = win or web
    bh = bar_hwnd(win)
    best = 0
    for h in render_hwnds(win):
        if h == bh:
            continue
        r = wrect(h)
        if not best or (r[2] - r[0]) * (r[3] - r[1]) > (wrect(best)[2] - wrect(best)[0]) * (wrect(best)[3] - wrect(best)[1]):
            best = h
    return best


def _alive():
    """程式沒有回應就不用再讀畫面了（每讀一次都要等很久）：直接結束這一步"""
    if user32.IsWindow(web) and not responsive(web):
        time.sleep(2.5)
        if not responsive(web):
            raise RuntimeError("程式沒有回應")


def bar_tree(win=None):
    if win is None:
        _alive()
    h = bar_hwnd(win)
    return snap(h, need=lambda r: find_id(r, "tool") is not None) if h else None


def page_tree(win=None, tries=6):
    h = page_hwnd(win)
    return snap(h, tries=tries) if h else None


def page_text(win=None):
    """目前網頁上看得到的文字"""
    root = page_tree(win)
    if root is None:
        return ""
    return "\n".join(n.name for n in root.walk() if n.name and n.type in ("TextControl", "ButtonControl", "HyperlinkControl", "EditControl", "CheckBoxControl", "ListItemControl", "MenuItemControl", "ComboBoxControl"))


def css_rect(n):
    """元素的位置（相對 web 視窗內容區的 CSS 像素）：[x, y, 寬, 高]"""
    cx, cy, cw, ch = client(web)
    return [(n.l - cx) / scale, (n.t - cy) / scale, n.w / scale, n.h / scale]


def E(i):
    return lambda root: find_id(root, i)


def tab_nodes(root):
    box = find_id(root, "tabs")
    return [k for k in box.kids if k.has("tab")] if box else []


def TAB(i):
    return lambda root: (tab_nodes(root)[i] if i < len(tab_nodes(root)) else None)


def TABX(i):
    """第 i 個分頁上的 ×"""
    def f(root):
        ts = tab_nodes(root)
        return next((k for k in ts[i].kids if k.has("tx")), None) if i < len(ts) else None
    return f


def bm_nodes(root):
    box = find_id(root, "bmList")
    return [k for k in box.kids if k.has("bm")] if box else []


def BM(name):
    return lambda root: next((k for k in bm_nodes(root) if k.name.strip() == name), None)


def EXTICON(i):
    def f(root):
        box = find_id(root, "exts")
        ks = [k for k in box.kids if k.w > 0] if box else []
        return ks[i] if i < len(ks) else None
    return f


BTN_MAX, BTN_MIN, BTN_CLOSE, DRAG, NEWTAB, OTHER = E("cMax"), E("cMin"), E("cClose"), E("drag"), E("newTab"), E("bmOther")
ADDR, BACK, FWD, RELOAD, MENU_BTN, EXT_BTN, PROFILE, SITE, STAR, TABSEARCH = E("omni"), E("back"), E("fwd"), E("reload"), E("menu"), E("ext"), E("profile"), E("site"), E("star"), E("tabSearch")


def rect_of(el, root=None):
    """工具列裡某個元素的位置（CSS 像素）；找不到或沒顯示回傳 None"""
    root = root or bar_tree()
    n = el(root) if root is not None else None
    if n is None or n.w <= 0 or n.h <= 0:
        return None
    return css_rect(n)


def bar_pt(el, fallback=None, dy=None):
    """工具列裡某個元素的中心（CSS 像素）；讀不到時用估計的位置（沒有估計就回傳 None）"""
    b = rect_of(el)
    if b:
        return (b[0] + b[2] / 2, b[1] + dy if dy is not None else b[1] + b[3] / 2)
    log("（讀不到元素位置）→", fallback)
    return fallback


def click_bar(el, fallback=None, **kw):
    p = bar_pt(el, fallback)
    if not p:
        return False
    click(*C(*p), **kw)
    return True


def tabs_info(root=None):
    root = root or bar_tree()
    res = []
    for t in tab_nodes(root):
        tx = [k for k in t.kids if k.type == "TextControl" and k.name]
        res.append({"active": t.has("active"), "pinned": t.has("pinned"), "title": tx[-1].name if tx else "", "w": t.w / scale})
    return res


def tab_count():
    return len(tabs_info())


def active_tab():
    """目前的分頁是第幾個（讀不到回傳 None）"""
    ts = tabs_info()
    return next((i for i, t in enumerate(ts) if t["active"]), None)


def bm_names():
    return [k.name.strip() for k in bm_nodes(bar_tree())]


def edit_value(n):
    try:
        return n.ctrl.GetValuePattern().Value or ""
    except Exception:
        return ""


def edit_selection(n):
    """輸入框裡選起來的文字（讀不到回傳 None）"""
    try:
        sel = n.ctrl.GetTextPattern().GetSelection()
        return "".join(r.GetText(-1) for r in sel)
    except Exception as e:
        log("（讀不到選取範圍）", e)
        return None


def bar_state():
    root = bar_tree()
    if root is None:
        return {}
    g = lambda i: find_id(root, i)
    addr, shown, star, zoom, site, bms = g("addr"), g("shown"), g("star"), g("zoomBtn"), g("site"), g("bms")
    vis = lambda n: n is not None and n.w > 0 and n.h > 0
    return {
        "back": bool(g("back") and g("back").en), "fwd": bool(g("fwd") and g("fwd").en),
        "addr": edit_value(addr) if addr else "", "shown": "".join(shown.texts()) if shown else "",
        "focus": "addr" if addr is not None and addr.foc else "",
        "zoom": zoom.name if vis(zoom) else "", "star": vis(star), "starOn": bool(vis(star) and star.has("on")),
        "dl": vis(g("dl")), "chip": (site.name if site is not None else ""), "bm": vis(bms),
    }


def saved():
    try:
        return json.load(open(os.path.join(data_dir, "web.json"), encoding="utf-8"))
    except Exception:
        return {}


def tab_ids(n=None):
    """分頁的編號（照順序）。程式改了之後一秒左右才存檔，所以等到數量對得上"""
    n = tab_count() if n is None else n
    ids = []
    for _ in range(12):
        w = next((w for w in saved().get("wins", []) if w.get("id") == "w1"), {})
        ids = [t.get("id") for t in w.get("tabs", [])]
        if len(ids) == n:
            break
        time.sleep(0.3)
    return ids


# ---------- 選單（每一層是一個小視窗） ----------
_levels = {}


def menu_wins():
    return [h for h in app_windows() if title(h) == MENU_TITLE and user32.IsWindowVisible(h)]


def _level_order(wins):
    """由外到內排：每個小視窗固定是某一層，第一次看到時記下來（外層一定先出現）"""
    known = [h for h in wins if h in _levels]
    for h in wins:
        if h not in _levels:
            _levels[h] = (max([_levels[k] for k in known], default=-1) + 1) if known else len([x for x in wins if x in _levels])
            known.append(h)
    return sorted(wins, key=lambda h: _levels[h])


def parse_menu(mroot):
    """選單裡一列一列的內容。能點的是一組（class 是 mi、sug、row…）；不能點的（停用、標題、說明）只有文字"""
    items = []
    if mroot is None:
        return items
    for k in mroot.kids:
        if k.type == "GroupControl" and k.cls:
            tx = [n for n in k.walk() if n.type == "TextControl" and n.name]
            # 第一個字是圖示（網站沒有圖示時用第一個字代替）：跳過
            if len(tx) >= 2 and len(tx[0].name) == 1 and tx[0].w <= 16 * scale and tx[1].l - k.l <= 70 * scale:
                tx = tx[1:]
            btns = [n for n in k.walk() if n.type == "ButtonControl"]
            items.append({"cls": k.cls, "t": tx[0].name.strip() if tx else k.name.strip(), "texts": [n.name for n in tx], "off": k.has("off"), "rect": (k.l, k.t, k.r, k.b),
                          "btns": {b.name: b for b in btns}, "node": k})
        elif k.type in ("TextControl", "ButtonControl") and k.name:
            # 同一列的零散文字、按鈕併在一起
            if items and items[-1]["cls"] == "loose" and abs((k.t + k.b) / 2 - (items[-1]["rect"][1] + items[-1]["rect"][3]) / 2) <= 12 * scale:
                it = items[-1]
                if k.type == "TextControl":
                    it["texts"].append(k.name)
                else:
                    it["btns"][k.name] = k
                it["rect"] = (min(it["rect"][0], k.l), min(it["rect"][1], k.t), max(it["rect"][2], k.r), max(it["rect"][3], k.b))
            else:
                items.append({"cls": "loose", "t": k.name.strip(), "texts": [k.name] if k.type == "TextControl" else [], "off": True, "rect": (k.l, k.t, k.r, k.b),
                              "btns": {k.name: k} if k.type == "ButtonControl" else {}, "node": k})
    for it in items:
        # 有按鈕的零散列是「縮放」那一排（可以按）
        if it["cls"] == "loose" and it["btns"]:
            it["off"] = False
    return items


def menus():
    """畫面上的選單，由外到內：[(視窗, 內容)]"""
    res = []
    for h in _level_order(menu_wins()):
        rhs = render_hwnds(h)
        root = snap(rhs[0], tries=5, need=lambda r: bool(find_id(r, "menu") and find_id(r, "menu").kids)) if rhs else None
        res.append((h, {"items": parse_menu(find_id(root, "menu")), "rect": wrect(h), "level": _levels.get(h, 0)}))
    return res


def menu_titles(ms=None):
    ms = menus() if ms is None else ms
    return [[it["t"] + ("（不能按）" if it["off"] else "") for it in m["items"]] for _, m in ms]


def names_of(m):
    return [it["t"] for it in m["items"]]


def menu_find(text, exact=True, ms=None):
    for h, m in (menus() if ms is None else ms):
        for it in m["items"]:
            if (it["t"] == text) if exact else (text in it["t"]):
                l, t, r, b = it["rect"]
                return h, it, (l + min((r - l) / 2, 90 * scale), (t + b) / 2), m
    return None


def menu_click(text, button="left", exact=True, wait=0.8):
    f = menu_find(text, exact)
    if not f:
        log("（選單裡找不到）", text, menu_titles())
        return False
    click(*f[2], button=button, wait=wait)
    return True


def menu_hover(text, exact=True, settle=0.8):
    f = menu_find(text, exact)
    if not f:
        log("（選單裡找不到）", text, menu_titles())
        return False
    hover(*f[2], settle=settle)
    return True


def blank_point():
    """網頁上一個空白、沒被選單蓋住的點（螢幕座標）"""
    rects = [wrect(h) for h in menu_wins()]
    pr = page_rect()
    cx, cy, cw, ch = client(web)
    for c in vis_kids():
        r = wrect(c)
        is_bar = r[1] == cy and r[0] == cx and (r[3] - r[1]) <= 140 * scale
        if not is_bar and r != pr:
            rects.append(r)
    for x, y in ((620, 330), (700, 250), (560, 420), (780, 380), (300, 330), (640, 120), (420, 200), (100, 330), (40, 240)):
        sx, sy = PG(x, y)
        if sx >= pr[2] - 6 or sy >= pr[3] - 6:
            continue
        if not any(r[0] - 6 <= sx <= r[2] + 6 and r[1] - 6 <= sy <= r[3] + 6 for r in rects):
            return sx, sy
    return PG(620, 330)


def click_blank(wait=0.6):
    """點一下網頁的空白處（鍵盤交給網頁／把選單關掉）"""
    click(*blank_point(), wait=wait)


def close_menus():
    """點網頁空白處把選單關掉"""
    if menu_wins():
        click_blank(0.7)
    if menu_wins():
        log("（選單還開著）", [wrect(h) for h in menu_wins()])
        click_blank(0.9)
    return not menu_wins()


def open_menu(el, name, button="left"):
    """按工具列上的某個東西打開選單，回傳選單內容（沒出現回傳 []）"""
    if not click_bar(el, button=button, wait=0.9):
        check(f"{name}：找得到按鈕", False)
        return []
    wait_for(menu_wins, 3, 0.2)
    time.sleep(0.4)
    ms = menus()
    check(f"{name}：出現選單", bool(ms) and bool(ms[0][1]["items"]), f"{menu_titles(ms)}")
    return ms


def below(anchor_css, h, side="left"):
    """選單是不是貼在某個元素下面（左邊或右邊對齊）"""
    if not anchor_css:
        return False, "讀不到元素"
    ax, ay = C(anchor_css[0], anchor_css[1] + anchor_css[3])
    ar = C(anchor_css[0] + anchor_css[2], 0)[0]
    r = wrect(h)
    ok_y = -2 * scale <= r[1] - ay <= 12 * scale
    ok_x = abs(r[0] - ax) <= 12 * scale if side == "left" else abs(r[2] - ar) <= 12 * scale
    return ok_y and ok_x, f"元素下緣=({ax:.0f}..{ar:.0f}, {ay:.0f}) 選單={r}"


HUNG = {"v": False}


def stderr_text():
    """程式到目前為止印出來的訊息（程式還開著那個檔案，直接讀內容）"""
    try:
        APP_ERR.flush()
    except Exception:
        pass
    try:
        return open(os.path.join(OUT, "app_stderr.txt"), encoding="utf-8", errors="replace").read()
    except Exception as e:
        return f"（讀不到：{e!r}）"


def hang_report(where):
    """程式沒有回應了：把查得到的都記下來（程式自己印的訊息、每個執行緒停在哪裡），後面的步驟不用再跑"""
    if HUNG["v"]:
        return
    HUNG["v"] = True
    log(f"\n!!!!! 程式沒有回應（{where}）")
    shot("hang")
    # 等程式自己的看門狗印出哪些鎖被佔著
    time.sleep(10)
    txt = stderr_text()
    log("程式的訊息（最後 250 行）：\n" + "\n".join("    " + l for l in txt.splitlines()[-250:]))
    try:
        with open(os.path.join(OUT, "app_stderr_copy.txt"), "w", encoding="utf-8") as f:
            f.write(txt)
    except Exception:
        pass
    cdb = next((c for c in (r"C:\Program Files (x86)\Windows Kits\10\Debuggers\x64\cdb.exe", r"C:\Program Files\Windows Kits\10\Debuggers\x64\cdb.exe") if os.path.exists(c)), None)
    log("cdb：", cdb)
    if cdb:
        try:
            r = subprocess.run([cdb, "-pv", "-p", str(app_proc.pid), "-y", r"srv*C:\symcache*https://msdl.microsoft.com/download/symbols", "-c", "~*k 60; qd"],
                               capture_output=True, text=True, timeout=300, errors="replace")
            with open(os.path.join(OUT, "hang_stacks.txt"), "w", encoding="utf-8") as f:
                f.write(r.stdout + "\n" + r.stderr)
            log("每個執行緒停在哪裡（詳細的在 hang_stacks.txt）：\n" + r.stdout[-12000:])
        except Exception as e:
            log("cdb 失敗", repr(e))


def step(name, fn):
    log(f"\n===== {name} =====")
    if HUNG["v"]:
        log("（程式已經沒有回應，這一步不跑）")
        return
    try:
        fn()
    except Exception:
        log(traceback.format_exc())
        check(name + " (沒有錯誤)", False, "exception")
    shot(name)
    dump(web, name)
    ok = responsive(web) if user32.IsWindow(web) else True
    if not ok:
        time.sleep(4)
        ok = responsive(web) if user32.IsWindow(web) else True
        if not ok:
            hang_report(name)
    check(name + "：程式有回應", ok)
    try:
        if user32.IsWindow(web) and menu_wins():
            log("（這一步結束時選單還開著，關掉）")
            close_menus()
    except Exception:
        pass


def diag():
    try:
        exe = os.path.basename(EXE)
        ps = ("Get-CimInstance Win32_Process | Where-Object { $_.Name -eq '" + exe + "' -or ($_.Name -eq 'msedgewebview2.exe' -and $_.CommandLine -notmatch '--type=') } | "
              "ForEach-Object { \"$($_.ProcessId) parent=$($_.ParentProcessId) $($_.Name) :: $($_.CommandLine)\" }")
        out = subprocess.run(["powershell", "-NoProfile", "-Command", ps], capture_output=True, text=True, timeout=60).stdout
        log("程序：\n" + "\n".join(l[:300] for l in out.splitlines() if l.strip()))
    except Exception as e:
        log("讀程序失敗", e)
    log("app_proc", app_proc.pid, "還在跑" if app_proc.poll() is None else f"已結束 {app_proc.poll()}")


def select_tab(i, wait=1.2):
    click_bar(TAB(i), wait=wait)


def tab_index(part):
    return next((i for i, t in enumerate(tabs_info()) if part in t["title"]), None)


def select_title(part, wait=1.2):
    """切到標題含這些字的分頁（第一個）"""
    ts = tabs_info()
    i = next((k for k, t in enumerate(ts) if part in t["title"]), None)
    if i is None:
        log("（找不到分頁）", part, [t["title"] for t in ts])
        return False
    if not ts[i]["active"]:
        click_bar(TAB(i), wait=wait)
    return True


def ensure_web_fg():
    """鍵盤要在 web 視窗上（它是前景視窗）。遊戲在前景時點 web 不會搶鍵盤，所以用系統的方式叫到前面"""
    if fg() == web:
        return True
    # 先按一下 Alt：Windows 才允許別的程式把視窗叫到前景
    user32.keybd_event(VK_ALT, 0, 0, 0)
    user32.keybd_event(VK_ALT, 0, 2, 0)
    user32.SetForegroundWindow(web)
    time.sleep(0.6)
    if fg() != web:
        click_bar(DRAG, wait=0.6)
    log("叫 web 視窗到前景：", fg() == web, "fg", fg(), repr(title(fg())))
    return fg() == web


def home(wait=1.2):
    """每一步開始前：選單關掉、回到測試頁那個分頁"""
    if menu_wins():
        close_menus()
    if not select_title("測試頁", wait):
        select_tab(0, wait)


def restore_tabs(keep):
    """只留下原本有的分頁（多出來的用中鍵關掉）"""
    for _ in range(10):
        n = tab_count()
        ids = tab_ids(n)
        if len(ids) != n:
            log("（分頁編號對不上，不關）", ids, n)
            break
        extra = [i for i, t in enumerate(ids) if t not in keep]
        if not extra:
            break
        click_bar(TAB(extra[-1]), button="middle", wait=1.3)
    return tab_count()


def go(url, wait=2.5):
    """在網址列打網址前往（目前分頁）"""
    click_bar(ADDR, wait=0.6)
    type_text(url)
    key(VK_ENTER)
    time.sleep(wait)


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
    vis = [wrect(c) for c in vis_kids()]
    cx, cy, cw, ch = client(web)
    bar_h = round(BAR_H * sc)
    bar = [r for r in vis if r[1] == cy and r[3] - r[1] == bar_h]
    page = [r for r in vis if abs(r[1] - (cy + bar_h)) <= 1 and r[3] <= cy + ch]
    return bool(bar) and bool(page), vis


SIZE0 = (900, 620)


class _Empty:
    kids = []


N_EMPTY = _Empty()
SNAP = {"cur": 1}


# =====================================================================================
# 原本就有的行為（視窗、分頁、選單、遊戲並行…）
# =====================================================================================
def s_start():
    diag()
    root = bar_tree()
    check("讀得到工具列裡的元素", root is not None and find_id(root, "menu") is not None, f"{root}")
    dump(web, "start")
    kids = vis_kids()
    check("一開始：看得到工具列和網頁", len(kids) >= 2, f"{len(kids)} 個可見子視窗")
    r, c = wrect(web), client(web)
    check("一開始：整個視窗都是內容（沒有看不到的邊）", (r[0], r[1], r[2] - r[0], r[3] - r[1]) == c, f"rect={r} client={c}")
    check("一開始：大小和上次記住的一樣（900×620）", (c[2], c[3]) == (round(SIZE0[0] * scale), round(SIZE0[1] * scale)), f"client={c}")
    ok, vis = layout_ok(scale)
    check("一開始：工具列高 129、網頁在它下面", ok, f"{vis}")
    hs = [rect_of(E(i)) for i in ("strip", "tool", "bms")]
    check("工具列三排的高度：分頁列 48、工具列 46、書籤列 34", bool(all(hs)) and [round(h[3]) for h in hs][:2] == [48, 46] and 33 <= hs[2][3] <= 35, f"{hs}")
    ts = tabs_info()
    check("一開始：三個分頁、第一個是目前的", len(ts) == 3 and ts[0]["active"] and "測試頁" in ts[0]["title"], f"{ts}")
    check("一開始：視窗標題是分頁的標題", title(web) == "測試頁", repr(title(web)))
    st = bar_state()
    log("bar state", st)
    check("一開始：網址列顯示目前的網址", "127.0.0.1:8765/test.html" in (st.get("shown") or ""), f"{st.get('shown')!r}")
    check("一開始：沒有上一頁、下一頁可以按", st.get("back") is False and st.get("fwd") is False, f"{st}")


def s_chrome_sync():
    names = bm_names()
    log("書籤列", names)
    check("書籤列：原本的書籤還在", "Example" in names and "資料夾" in names, f"{names}")
    if CHROME_SEEDED:
        check("書籤列：打開視窗時自動帶進 Chrome 的書籤", "YouTube" in names and "楓之谷" in names, f"{names}")
    check("書籤列右邊有「所有書籤」", rect_of(OTHER) is not None)
    try:
        st = json.load(open(os.path.join(data_dir, "web-ext.json"), encoding="utf-8"))
    except Exception as e:
        st = {"error": str(e)}
    log("web-ext.json", json.dumps(st, ensure_ascii=False)[:500])
    if CHROME_SEEDED:
        mk = (st.get("exts") or {}).get(EXT_ID) or {}
        check("擴充功能：打開視窗時自動從 Chrome 同步", bool(mk.get("dir")) and mk.get("ver") == "1.0_0", f"{mk}")


def s_max_button():
    before = wrect(web)
    log("按之前的前景視窗", fg(), repr(title(fg())), "web", web, "game", game)
    log(f"web 視窗出現後 {time.time() - WEB_AT:.1f} 秒")
    for k in range(3):
        t0 = time.time()
        click_bar(BTN_MAX, (W() - 69, 20), wait=0.05)
        z = wait_for(lambda: user32.IsZoomed(web), 5, 0.05)
        log(f"第 {k + 1} 次按放到最大：{time.time() - t0:.2f} 秒" if z else "5 秒內沒有放到最大", "前景", fg())
        time.sleep(0.8)
        if k == 2:
            break
        t0 = time.time()
        click_bar(BTN_MAX, (W() - 69, 16), wait=0.05)
        z = wait_for(lambda: not user32.IsZoomed(web), 5, 0.05)
        log(f"第 {k + 1} 次按還原：{time.time() - t0:.2f} 秒" if z else "5 秒內沒有還原")
        time.sleep(0.8)
    check("按放到最大按鈕：視窗放到最大", user32.IsZoomed(web), f"before={before} after={wrect(web)}")
    vis = [wrect(c) for c in vis_kids()]
    tops = sorted(r[1] for r in vis)
    check("放到最大後：工具列在最上面、網頁在下面", len(vis) >= 2 and tops[0] <= client(web)[1] + 2, f"{vis}")
    cx, cy, cw, ch = client(web)
    pr = page_rect()
    check("放到最大後：網頁貼齊左右下（不留邊）、分頁列變矮", pr[0] == cx and pr[2] == cx + cw and pr[3] == cy + ch and pr[1] == cy + round(BAR_H_MAX * scale), f"page={pr} client={client(web)}")
    shot("maximized")
    # 放到最大時滑鼠移到螢幕最上緣也點得到分頁
    p = bar_pt(TAB(1), (40 + 232 + 100, 23))
    click(C(p[0], 0)[0], client(web)[1], wait=1.5)
    check("放到最大時點螢幕最上緣：點得到分頁", "短影片" in title(web), f"title={title(web)}")
    p = bar_pt(TAB(0), (40 + 100, 23))
    click(C(p[0], 0)[0], client(web)[1], wait=1.0)
    check("放到最大時點螢幕最上緣：切回第一個分頁", "測試頁" in title(web), f"title={title(web)}")


def s_restore_button():
    click_bar(BTN_MAX, (W() - 69, 16), wait=1.2)
    check("再按一次：還原", not user32.IsZoomed(web), f"{wrect(web)}")
    r, c = wrect(web), client(web)
    check("還原後：大小回到原本的（900×620）", (c[2], c[3]) == (round(SIZE0[0] * scale), round(SIZE0[1] * scale)), f"rect={r} client={c}")


def slides_since(t0):
    res = []
    for l in page_log().splitlines():
        try:
            t = float(l.split()[0])
        except Exception:
            continue
        if t >= t0 and "[snap]" in l and "slide:" in l:
            try:
                res.append(int(l.split("slide:")[1].split()[0]))
            except Exception:
                pass
    return res


def s_snap_resize():
    select_title("短影片", 2.5)
    check("切到短影片分頁", "短影片" in title(web), title(web))
    px, py = PG(400, 300)
    for _ in range(3):
        wheel(px, py, 1)
        time.sleep(0.7)
    time.sleep(1)
    cur = (slides_since(0) or [1])[-1]
    SNAP["cur"] = cur
    log("捲動後是第", cur, "張")
    t0 = time.time()
    click_bar(BTN_MAX, (W() - 69, 20), wait=2.0)
    shot("snap_maximized")
    click_bar(BTN_MAX, (W() - 69, 16), wait=2.0)
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
    # 拖曳視窗之後分頁列還會更新（按著分頁列空白處拖視窗，放開的事件網頁收不到）
    t_before = tabs_info()
    select_tab(2, wait=1.5)
    a = active_tab()
    check("拖完視窗後點分頁：分頁列有跟著更新", a == 2, f"active={a} before={t_before}")
    select_title("短影片", 1.0)
    # 放回原位，後面的測試才在螢幕裡
    user32.MoveWindow(web, before[0], before[1], before[2] - before[0], before[3] - before[1], True)
    time.sleep(1.0)
    dump(web, "moved back")
    vis = [wrect(c) for c in vis_kids()]
    check("用別的程式移動視窗：網頁跟著放好", any(r[1] == before[1] for r in vis) and any(r[1] == before[1] + round(BAR_H * scale) for r in vis), f"{vis}")


def s_tabs():
    select_tab(2, wait=2.5)
    shot("tab3")
    a = active_tab()
    check("點第三個分頁：切過去", a == 2 if a is not None else "Example" in title(web), f"active={a} title={title(web)}")
    home(1.5)
    kids = vis_kids()
    check("切換分頁：只有一個網頁顯示", len(kids) == 2, f"{[wrect(c) for c in kids]}")
    check("切回第一個分頁", "測試頁" in title(web), title(web))
    # 拖曳分頁排順序：第一個拖到第二個後面，再拖回來
    r0, r1 = rect_of(TAB(0)), rect_of(TAB(1))
    if r0 and r1:
        y = r0[1] + r0[3] / 2
        drag(*C(r0[0] + 60, y), *C(r1[0] + r1[2] - 20, y), wait=1.0)
        ts = tabs_info()
        check("拖曳分頁：順序換了", len(ts) == 3 and "短影片" in ts[0]["title"] and "測試頁" in ts[1]["title"], f"{[t['title'] for t in ts]}")
        r0, r1 = rect_of(TAB(0)), rect_of(TAB(1))
        drag(*C(r1[0] + 60, y), *C(r0[0] + 20, y), wait=1.0)
        ts = tabs_info()
        check("拖曳分頁：拖回原本的順序", len(ts) == 3 and "測試頁" in ts[0]["title"], f"{[t['title'] for t in ts]}")
    home(1.0)


def s_bm_menu():
    ms = open_menu(BM("資料夾"), "點書籤資料夾")
    shot("menu_open")
    if ms:
        check("書籤資料夾：裡面的書籤都列出來", names_of(ms[0][1]) == ["測試頁", "Example"], f"{menu_titles(ms)}")
        ok, d = below(rect_of(BM("資料夾")), ms[0][0])
        check("書籤資料夾：選單貼在資料夾下面", ok, d)
    click_blank(1.0)
    check("點外面：選單關掉", not menu_wins())
    check("選單關掉後程式有回應", responsive(web))
    ms = open_menu(OTHER, "點所有書籤")
    shot("menu_other")
    if ms:
        ok, d = below(rect_of(OTHER), ms[0][0], side="right")
        check("所有書籤：選單貼在按鈕下面、靠右對齊", ok, d)
        log("所有書籤", menu_titles())
    click_bar(OTHER, wait=1.0)
    check("再點一次所有書籤：選單關掉", not menu_wins())
    check("所有書籤操作後程式有回應", responsive(web))
    if CHROME_SEEDED:
        # 資料夾裡還有資料夾：滑過去打開下一層
        ms = open_menu(BM("楓之谷"), "點 Chrome 同步來的資料夾")
        if ms:
            check("楓之谷：官網、攻略、IANA", names_of(ms[0][1]) == ["官網", "攻略", "IANA"], f"{menu_titles(ms)}")
            menu_hover("攻略")
            wait_for(lambda: len(menu_wins()) >= 2, 3, 0.2)
            time.sleep(0.4)
            ms2 = menus()
            shot("menu_nested")
            check("滑到裡面的資料夾：打開下一層", len(ms2) >= 2 and names_of(ms2[1][1]) == ["短影片"], f"{menu_titles(ms2)}")
            if ms2 and len(ms2) >= 2:
                a, b = wrect(ms2[0][0]), wrect(ms2[1][0])
                check("下一層開在右邊、緊貼著", abs(b[0] - a[2]) <= 8 * scale and abs(b[1] - a[1]) < (a[3] - a[1]), f"{a} {b}")
                # 滑鼠移進下一層：兩層都還在
                menu_hover("短影片")
                check("滑鼠移進下一層：選單都還開著", len(menu_wins()) >= 2, f"{len(menu_wins())}")
                t0 = time.time()
                menu_click("短影片", wait=2.5)
                check("點最裡面的書籤：選單全部關掉", not menu_wins(), f"{len(menu_wins())}")
                check("點最裡面的書籤：在目前分頁打開", any("?deep" in l or "[snap] load" in l for l in page_log_since(t0)) and "短影片" in title(web), f"{title(web)} {page_log_since(t0)[-3:]}")
                click_bar(BACK, wait=2.0)
                check("按上一頁：回到測試頁", "測試頁" in title(web), title(web))
        close_menus()


def s_new_tab():
    n0 = tab_count()
    click_bar(NEWTAB, wait=1.5)
    st = bar_state()
    check("按 +：多一個分頁、標題是新分頁", tab_count() == n0 + 1 and title(web) == "新分頁", f"{n0} → {tab_count()} title={title(web)!r}")
    check("按 +：網址列可以直接打字", st.get("focus") == "addr", f"{st}")
    shot("new_tab")
    type_text(f"http://127.0.0.1:{PORT}/test.html?new")
    key(VK_ENTER)
    time.sleep(2.5)
    shot("new_tab_loaded")
    check("新分頁輸入網址：有載入", "?new" in page_log(), page_log()[-300:])
    check("新分頁輸入網址：標題換成網頁的", "測試頁" in title(web), title(web))
    st = bar_state()
    ms = ctx_at(*PG(620, 330))
    shot("new_tab_context")
    if ms:
        names = {it["t"]: it for it in ms[0][1]["items"]}
        back_off = any("上一頁" in n and it["off"] for n, it in names.items())
        fwd_off = any("下一頁" in n and it["off"] for n, it in names.items())
        check("剛逛到的網頁按右鍵：可以上一頁、不能下一頁（跟工具列的箭頭一樣）", (not back_off) and fwd_off and st.get("back") is True and st.get("fwd") is False, f"選單 back_off={back_off} fwd_off={fwd_off} 工具列={st.get('back')},{st.get('fwd')}")
    else:
        check("剛逛到的網頁按右鍵：出現選單", False)
    close_menus()


# =====================================================================================
# 比照 Chrome 做的部分
# =====================================================================================
def s_omnibox():
    home()
    click_bar(ADDR, wait=0.7)
    st = bar_state()
    a = find_id(bar_tree(), "addr")
    val, sel = (edit_value(a), edit_selection(a)) if a else ("", None)
    check("點網址列：游標進去、整個網址選起來", st.get("focus") == "addr" and bool(val) and sel == val, f"value={val!r} selected={sel!r}")
    check("點網址列：顯示完整網址（有 http://）", val.startswith("http://127.0.0.1"), f"{val!r}")
    type_text("127")
    wait_for(menu_wins, 4, 0.2)
    time.sleep(1.5)
    ms = menus()
    shot("omni_suggest")
    check("網址列打字：下面出現建議", bool(ms) and any("sug" in it["cls"].split() for it in ms[0][1]["items"]), f"{menu_titles(ms)}")
    if ms:
        h, m = ms[0]
        pill = rect_of(ADDR)
        if pill:
            px, py = C(pill[0], pill[1] + pill[3])
            r = wrect(h)
            check("建議清單：貼在網址列下面、一樣寬", abs(r[0] - px) <= 3 * scale and 0 <= r[1] - py <= 8 * scale and abs((r[2] - r[0]) - pill[2] * scale) <= 4 * scale, f"pill={pill} menu={r}")
        log("建議", [it["texts"] for it in m["items"]])
        check("建議清單：有逛過的網頁（127.0.0.1）", any("127.0.0.1" in " ".join(it["texts"]) for it in m["items"]), f"{[it['texts'] for it in m['items']]}")
        on = lambda mm: next((i for i, it in enumerate(mm[0][1]["items"]) if "on" in it["cls"].split()), None) if mm else None
        on0 = on(ms)
        v0 = bar_state().get("addr")
        key(VK_DOWN)
        time.sleep(0.6)
        ms1 = menus()
        on1 = on(ms1)
        v1 = bar_state().get("addr")
        want1 = ms1[0][1]["items"][1]["texts"] if ms1 and len(ms1[0][1]["items"]) > 1 else []
        key(VK_DOWN)
        time.sleep(0.6)
        ms2 = menus()
        on2 = on(ms2)
        v2 = bar_state().get("addr")
        want2 = ms2[0][1]["items"][2]["texts"] if ms2 and len(ms2[0][1]["items"]) > 2 else []
        log("向下鍵", (on0, v0), (on1, v1, want1), (on2, v2, want2))
        # 搜尋建議：網址列是那個關鍵字；逛過的網頁：網址列是它的網址
        fits = lambda v, want: bool(want) and (v == want[0] if "Google 搜尋" in want[-1] else want[-1] in (v or ""))
        check("按向下鍵：一個一個往下選，網址列換成選到的那一個", (on0, on1, on2) == (0, 1, 2) and fits(v1, want1) and fits(v2, want2), f"{(on0, v0)} {(on1, v1, want1)} {(on2, v2, want2)}")
        key(VK_UP)
        time.sleep(0.3)
        key(VK_UP)
        time.sleep(0.3)
    key(VK_ESC)
    time.sleep(0.5)
    st = bar_state()
    check("按 Esc：建議關掉", not menu_wins(), f"{st}")
    key(VK_ESC)
    time.sleep(0.5)
    st = bar_state()
    check("再按 Esc：網址列回到目前的網址、游標離開", st.get("focus") != "addr" and "127.0.0.1:8765/test.html" in (st.get("shown") or ""), f"{st}")
    # 打網址前往
    t0 = time.time()
    go(f"127.0.0.1:{PORT}/snap.html?omni")
    check("打網址（沒打 http://）按 Enter：前往", "短影片" in title(web) and any("[snap] load" in l for l in page_log_since(t0)), f"{title(web)} {page_log_since(t0)[-2:]}")
    st = bar_state()
    check("前往後：可以按上一頁", st.get("back") is True and st.get("fwd") is False, f"{st}")
    check("前往後：選單都關了、游標離開網址列", not menu_wins() and st.get("focus") != "addr", f"{st}")
    click_bar(BACK, wait=2.0)
    st = bar_state()
    check("按上一頁：回到測試頁、可以按下一頁", "測試頁" in title(web) and st.get("fwd") is True, f"{title(web)} {st}")
    click_bar(FWD, wait=2.0)
    check("按下一頁：又到短影片", "短影片" in title(web), title(web))
    # 上一頁按右鍵：歷程清單
    ms = open_menu(BACK, "上一頁按右鍵", button="right")
    shot("back_menu")
    if ms:
        log("上一頁的清單", menu_titles())
        check("上一頁的清單：有測試頁", any("測試頁" in it["t"] for it in ms[0][1]["items"]), f"{menu_titles(ms)}")
        menu_click("測試頁", exact=False, wait=2.0)
        check("點清單裡的測試頁：回去了", "測試頁" in title(web), title(web))
    close_menus()
    if "測試頁" not in title(web):
        click_bar(BACK, wait=2.0)
    # 重新載入按鈕
    t0 = time.time()
    click_bar(RELOAD, wait=2.0)
    check("按重新載入：網頁重新載入", any("[test] load" in l for l in page_log_since(t0)), f"{page_log_since(t0)[-2:]}")


MAIN = {"w": 0}
MAIN_ITEMS = ["新分頁", "新視窗", "新增無痕式視窗", "歷史記錄", "下載", "書籤和清單", "擴充功能", "刪除瀏覽資料…", "縮放", "列印…", "尋找…", "儲存及分享", "更多工具", "設定", "結束"]


def s_main_menu():
    home()
    ms = open_menu(MENU_BTN, "按右上角的 ⋮")
    shot("main_menu")
    if not ms:
        return
    h, m = ms[0]
    names = names_of(m)
    check("⋮ 選單：項目和順序", names == MAIN_ITEMS, f"{names}")
    log("⋮ 選單每一列的文字", [it["texts"] for it in m["items"]])
    MAIN["w"] = (wrect(h)[2] - wrect(h)[0]) / scale
    ok, d = below(rect_of(MENU_BTN), h, side="right")
    check("⋮ 選單：開在按鈕下面、靠右對齊", ok, d)
    sw, sh = user32.GetSystemMetrics(0), user32.GetSystemMetrics(1)
    r = wrect(h)
    check("⋮ 選單：整個在螢幕裡", r[0] >= 0 and r[1] >= 0 and r[2] <= sw and r[3] <= sh, f"{r} screen={sw}x{sh}")
    for sub in ["歷史記錄", "書籤和清單", "擴充功能", "儲存及分享", "更多工具"]:
        menu_hover(sub)
        wait_for(lambda: len(menu_wins()) >= 2, 3, 0.2)
        time.sleep(0.5)
        ms2 = menus()
        check(f"滑到「{sub}」：打開下一層", len(ms2) >= 2 and bool(ms2[1][1]["items"]), f"{menu_titles(ms2)[1:]}")
        if len(ms2) >= 2:
            log(f"「{sub}」的下一層", menu_titles(ms2)[1])
            a, b = wrect(ms2[0][0]), wrect(ms2[1][0])
            check(f"「{sub}」的下一層：緊貼在旁邊、整個在螢幕裡", (abs(b[2] - a[0]) <= 8 * scale or abs(b[0] - a[2]) <= 8 * scale) and b[0] >= 0 and b[3] <= sh and b[1] >= 0, f"{a} {b}")
        shot("main_sub_" + {"歷史記錄": "history", "書籤和清單": "bookmarks", "擴充功能": "ext", "儲存及分享": "save", "更多工具": "tools"}[sub])
    # 沒有下一層的項目：滑過去時把下一層關掉
    menu_hover("設定")
    time.sleep(0.6)
    check("滑到沒有下一層的項目：下一層關掉", len(menu_wins()) == 1, f"{len(menu_wins())}")
    # 三層：書籤和清單 → 楓之谷 → 攻略
    if CHROME_SEEDED:
        menu_hover("書籤和清單")
        wait_for(lambda: len(menu_wins()) >= 2, 3, 0.2)
        menu_hover("楓之谷")
        wait_for(lambda: len(menu_wins()) >= 3, 3, 0.2)
        menu_hover("攻略")
        wait_for(lambda: len(menu_wins()) >= 4, 3, 0.2)
        shot("main_menu_deep")
        time.sleep(0.4)
        t = menu_titles()
        check("書籤和清單 → 楓之谷 → 攻略：一層一層打開", len(t) >= 4 and t[-1] == ["短影片"], f"{t[1:]}")
        rs = [m_["rect"] for _, m_ in menus()]
        # 第 n 層和第 n-2 層不能重疊（一路往同一個方向展開）
        over = lambda a, b: min(a[2], b[2]) - max(a[0], b[0]) > 12 * scale and min(a[3], b[3]) - max(a[1], b[1]) > 12 * scale
        bad = [(i, i + 2) for i in range(len(rs) - 2) if over(rs[i], rs[i + 2])]
        check("好幾層選單：裡面的不會蓋到外面的", len(rs) >= 4 and not bad, f"{rs} 重疊={bad}")
        menu_hover("設定")
        time.sleep(0.6)
    # 縮放那一排：按 + 放大、按 - 縮回來
    def zoom_row():
        mm = menus()
        it = next((i for i in mm[0][1]["items"] if i["t"] == "縮放"), None) if mm else None
        return it, (next((t for t in it["texts"] if t.endswith("%")), "") if it else "")
    z, pct = zoom_row()
    log("縮放那一排", z and (z["texts"], list(z["btns"])))
    if z and "放大" in z["btns"] and "縮小" in z["btns"]:
        click(*z["btns"]["放大"].center, wait=1.2)
        z2, pct = zoom_row()
        st = bar_state()
        check("⋮ 選單按放大：變 110%、選單還開著、網址列出現縮放圖示", pct == "110%" and len(menu_wins()) >= 1 and "110" in st.get("zoom", ""), f"pct={pct} {st}")
        shot("main_menu_zoom")
        if z2 and "縮小" in z2["btns"]:
            click(*z2["btns"]["縮小"].center, wait=1.2)
        z3, pct = zoom_row()
        check("⋮ 選單按縮小：回到 100%", pct == "100%", f"pct={pct}")
    else:
        check("⋮ 選單的縮放那一排：有縮小、放大按鈕", False, f"{z}")
    # 點外面關掉
    click_blank(1.0)
    check("點網頁：選單全部關掉", not menu_wins(), f"{len(menu_wins())}")
    # 再按一次按鈕也是關掉
    open_menu(MENU_BTN, "再按 ⋮")
    click_bar(MENU_BTN, wait=0.9)
    check("選單開著時再按 ⋮：關掉", not menu_wins(), f"{len(menu_wins())}")
    # 點項目：新分頁
    n0, keep = tab_count(), tab_ids()
    open_menu(MENU_BTN, "再按 ⋮（要點新分頁）")
    menu_click("新分頁", wait=1.5)
    check("⋮ → 新分頁：多一個分頁、選單關掉", tab_count() == n0 + 1 and not menu_wins(), f"{n0} → {tab_count()}")
    restore_tabs(keep)
    home()


def ctx_at(x, y):
    """在螢幕上某一點按右鍵，回傳出現的選單"""
    click(x, y, button="right", wait=0.9)
    wait_for(menu_wins, 3, 0.2)
    time.sleep(0.4)
    return menus()


def s_context():
    home()
    st = bar_state()
    ms = ctx_at(*PG(300, 330))
    shot("ctx_page")
    check("網頁按右鍵：出現選單", bool(ms), f"{menu_titles(ms)}")
    if ms:
        h, m = ms[0]
        log("網頁右鍵選單", menu_titles(ms))
        names = {it["t"]: it for it in m["items"]}
        check("網頁右鍵選單：上一頁、下一頁、重新載入、另存新檔、列印、檢視網頁原始碼", all(any(k in n for n in names) for k in ["上一頁", "下一頁", "重新載入", "另存新檔", "列印", "檢視網頁原始碼"]), f"{list(names)}")
        back_off = any("上一頁" in n and it["off"] for n, it in names.items())
        fwd_off = any("下一頁" in n and it["off"] for n, it in names.items())
        check("網頁右鍵選單：上一頁／下一頁能不能按，跟工具列的箭頭一致", back_off == (not st.get("back")) and fwd_off == (not st.get("fwd")), f"選單 back_off={back_off} fwd_off={fwd_off} 工具列={st.get('back')},{st.get('fwd')}")
        r = wrect(h)
        mx, my = PG(300, 330)
        check("網頁右鍵選單：開在滑鼠的位置（下面放不下就往上開）", abs(r[0] - mx) <= 6 * scale and (abs(r[1] - my) <= 6 * scale or abs(r[3] - my) <= 6 * scale), f"{r} mouse=({mx:.0f},{my:.0f})")
        t0 = time.time()
        menu_click("重新載入", exact=False, wait=2.0)
        check("右鍵 → 重新載入：網頁重新載入、選單關掉", any("[test] load" in l for l in page_log_since(t0)) and not menu_wins(), f"{page_log_since(t0)[-2:]}")
    close_menus()
    # 連結
    n0, keep = tab_count(), tab_ids()
    a0 = active_tab()
    ms = ctx_at(*PG(235, 200))
    shot("ctx_link")
    check("連結按右鍵：出現選單", bool(ms), f"{menu_titles(ms)}")
    if ms:
        log("連結右鍵選單", menu_titles(ms))
        names = names_of(ms[0][1])
        check("連結右鍵選單：在新分頁／新視窗／無痕式視窗開啟、複製連結網址", all(any(k in n for n in names) for k in ["新分頁", "新視窗", "無痕", "複製連結"]), f"{names}")
        menu_click("在新分頁中開啟連結", exact=False, wait=2.0)
        check("右鍵 → 在新分頁中開啟連結：多一個分頁、留在原本的分頁", tab_count() == n0 + 1 and active_tab() == a0, f"{n0} → {tab_count()} active={active_tab()}")
    close_menus()
    restore_tabs(keep)
    home()
    # 輸入框
    ms = ctx_at(*PG(200, 40))
    shot("ctx_input")
    check("輸入框按右鍵：出現選單", bool(ms), f"{menu_titles(ms)}")
    if ms:
        log("輸入框右鍵選單", menu_titles(ms))
        names = names_of(ms[0][1])
        check("輸入框右鍵選單：剪下、複製、貼上、全選", all(any(k in n for n in names) for k in ["剪下", "複製", "貼上", "全選"]), f"{names}")
    close_menus()
    key(VK_ESC)
    click_blank(0.5)
    # 分頁
    ms = open_menu(TAB(0), "分頁按右鍵", button="right")
    shot("ctx_tab")
    if ms:
        log("分頁右鍵選單", menu_titles(ms))
        names = names_of(ms[0][1])
        check("分頁右鍵選單：新分頁、重新載入、複製、固定、靜音、關閉、關閉其他分頁、關閉右側分頁", all(any(k in n for n in names) for k in ["新分頁", "重新載入", "複製", "固定", "靜音", "關閉", "關閉其他分頁", "右側"]), f"{names}")
        n0 = tab_count()
        menu_click("複製", wait=2.5)
        ts = tabs_info()
        check("分頁右鍵 → 複製：多一個一樣的分頁（在它右邊、切過去）", len(ts) == n0 + 1 and "測試頁" in ts[1]["title"] and ts[1]["active"], f"{[(t['title'], t['active']) for t in ts]}")
        # 固定分頁：變成小圖示、排到最前面；再取消
        ms = open_menu(TAB(1), "複製出來的分頁按右鍵", button="right")
        menu_click("固定", wait=1.2)
        ts = tabs_info()
        shot("tab_pinned")
        check("分頁右鍵 → 固定：排到最前面、變窄只剩圖示", bool(ts) and ts[0]["pinned"] and ts[0]["w"] < 60, f"{[(t['title'], t['pinned'], t['w']) for t in ts]}")
        ms = open_menu(TAB(0), "固定的分頁按右鍵", button="right")
        if ms:
            log("固定的分頁右鍵選單", menu_titles(ms))
        menu_click("取消固定", exact=False, wait=1.2)
        ts = tabs_info()
        check("取消固定：變回一般分頁", bool(ts) and not any(t["pinned"] for t in ts), f"{[(t['title'], t['pinned']) for t in ts]}")
        # 把多出來的那個關掉（按分頁上的 ×）
        idx = next((i for i, t in enumerate(ts) if t["active"]), 0)
        x = rect_of(TABX(idx))
        if x:
            click(*C(x[0] + x[2] / 2, x[1] + x[3] / 2), wait=1.2)
        check("按分頁上的 ×：關掉那個分頁", tab_count() == n0, f"{tab_count()}")
    close_menus()
    # 分頁列空白處
    ms = open_menu(DRAG, "分頁列空白處按右鍵", button="right")
    if ms:
        log("分頁列空白處右鍵選單", menu_titles(ms))
    shot("ctx_strip")
    close_menus()
    # 書籤
    ms = open_menu(BM("Example"), "書籤按右鍵", button="right")
    if ms:
        log("書籤右鍵選單", menu_titles(ms))
        names = names_of(ms[0][1])
        check("書籤右鍵選單：在新分頁／新視窗／無痕式視窗開啟、重新命名、刪除、書籤管理員", all(any(k in n for n in names) for k in ["新分頁", "新視窗", "無痕", "重新命名", "刪除", "書籤管理員"]), f"{names}")
    shot("ctx_bookmark")
    close_menus()
    home()


PAGES = [
    ("history", "歷史記錄", ["測試頁"]),
    ("downloads", "下載", []),
    ("bookmarks", "書籤", ["資料夾", "Example"]),
    ("extensions", "擴充功能", ["測試腳本一"]),
    ("settings", "設定", ["搜尋引擎"]),
]


def s_internal_pages():
    keep = tab_ids()
    for p, ttl, expect in PAGES:
        click_bar(NEWTAB, wait=1.2)
        type_text(f"chrome://{p}")
        key(VK_ENTER)
        time.sleep(2.5)
        txt = page_text()
        st = bar_state()
        shot("page_" + p)
        check(f"chrome://{p}：打得開、標題是「{ttl}」", ttl in title(web), f"title={title(web)!r}")
        check(f"chrome://{p}：網址列顯示 chrome://{p}", f"chrome://{p}" in (st.get("shown") or ""), f"{st.get('shown')!r} chip={st.get('chip')!r}")
        if txt:
            log(f"chrome://{p} 內容：", " | ".join(x.strip() for x in txt.splitlines() if x.strip())[:700])
            check(f"chrome://{p}：內容有 {expect}", all(e in txt for e in expect), txt[:200].replace("\n", " "))
        else:
            check(f"chrome://{p}：讀得到內容", False)
        restore_tabs(keep)
    # 設定：關掉「顯示首頁按鈕」，工具列上的首頁按鈕就不見；再打開
    click_bar(NEWTAB, wait=1.2)
    type_text("chrome://settings")
    key(VK_ENTER)
    time.sleep(2.5)

    def home_switch():
        root = page_tree()
        label = next((n for n in find_all(root, lambda n: n.type == "TextControl" and n.name == "顯示首頁按鈕")), None)
        if label is None:
            return None
        cy = (label.t + label.b) / 2
        sws = [n for n in find_all(root, lambda n: n.has("sw") or n.type == "CheckBoxControl") if n.w > 0 and abs((n.t + n.b) / 2 - cy) <= 30 * scale]
        return sws[0] if sws else None

    sw = home_switch()
    had = rect_of(E("home")) is not None
    if sw is not None:
        click(*sw.center, wait=1.0)
        gone = rect_of(E("home")) is None
        sw2 = home_switch()
        if sw2 is not None:
            click(*sw2.center, wait=1.0)
        back = rect_of(E("home")) is not None
        check("設定 → 關掉「顯示首頁按鈕」：工具列上的首頁按鈕不見；再打開就回來", had and gone and back, f"原本={had} 關掉後不見={gone} 再打開={back}")
    else:
        check("設定頁：找得到「顯示首頁按鈕」的開關", False)
    restore_tabs(keep)
    # 新分頁頁面：搜尋框可以打字前往
    click_bar(NEWTAB, wait=1.5)
    txt = page_text()
    log("新分頁內容：", " | ".join(x.strip() for x in txt.splitlines() if x.strip())[:400])
    shot("page_newtab")
    box = next((n for n in find_all(page_tree(), lambda n: n.type == "EditControl")), None)
    if box is not None:
        click(*box.center, wait=0.8)
        type_text(f"127.0.0.1:{PORT}/snap.html?ntp")
        key(VK_ENTER)
        time.sleep(2.5)
        check("新分頁頁面中間的搜尋框：打網址按 Enter 會前往", "短影片" in title(web), title(web))
    else:
        check("新分頁頁面：找得到搜尋框", False)
    restore_tabs(keep)
    home()


def s_shortcuts():
    home()
    click_blank(0.6)
    n0, keep = tab_count(), tab_ids()
    chord([VK_CTRL], 0x54, wait=1.2)  # Ctrl+T
    st = bar_state()
    check("Ctrl+T：開新分頁、可以直接打網址", tab_count() == n0 + 1 and st.get("focus") == "addr", f"{tab_count()} {st}")
    type_text(f"127.0.0.1:{PORT}/snap.html?keys")
    key(VK_ENTER)
    time.sleep(2.0)
    check("新分頁打網址：前往", "短影片" in title(web), title(web))
    click_blank(0.5)
    chord([VK_CTRL], 0x57, wait=1.2)  # Ctrl+W
    check("Ctrl+W：關掉目前的分頁", tab_count() == n0, f"{tab_count()}")
    chord([VK_CTRL, VK_SHIFT], 0x54, wait=2.0)  # Ctrl+Shift+T
    check("Ctrl+Shift+T：重新開啟剛關掉的分頁", tab_count() == n0 + 1 and "短影片" in title(web), f"{tab_count()} {title(web)}")
    click_blank(0.5)
    a0 = active_tab()
    chord([VK_CTRL], VK_TAB, wait=0.9)
    a1 = active_tab()
    chord([VK_CTRL, VK_SHIFT], VK_TAB, wait=0.9)
    a2 = active_tab()
    check("Ctrl+Tab／Ctrl+Shift+Tab：切到下一個／上一個分頁", a1 is not None and a1 != a0 and a2 == a0, f"{a0} → {a1} → {a2}")
    chord([VK_CTRL], 0x31, wait=0.9)  # Ctrl+1
    b1 = active_tab()
    chord([VK_CTRL], 0x39, wait=0.9)  # Ctrl+9
    b9 = active_tab()
    check("Ctrl+1／Ctrl+9：第一個／最後一個分頁", b1 == 0 and b9 == tab_count() - 1, f"{b1} {b9}")
    if tab_count() == n0 + 1:
        chord([VK_CTRL], 0x57, wait=1.2)
        # 連續關：關掉之後不用再點網頁，快捷鍵還是有作用
        chord([VK_CTRL, VK_SHIFT], 0x54, wait=1.8)
        check("Ctrl+W 之後馬上 Ctrl+Shift+T：又開回來（鍵盤沒有掉）", tab_count() == n0 + 1, f"{tab_count()}")
        chord([VK_CTRL], 0x57, wait=1.2)
    check("關掉多的分頁：回到原本的數量", tab_count() == n0, f"{tab_count()}")
    home()
    click_blank(0.5)
    chord([VK_CTRL], 0x4C, wait=0.7)  # Ctrl+L
    st = bar_state()
    check("Ctrl+L：游標到網址列", st.get("focus") == "addr", f"{st}")
    key(VK_ESC)
    time.sleep(0.3)
    key(VK_ESC)
    click_blank(0.5)
    # 縮放
    chord([VK_CTRL], VK_PLUS, wait=1.0)
    st = bar_state()
    shot("zoom_110")
    check("Ctrl + 加號：放大到 110%，網址列出現縮放圖示", "110" in st.get("zoom", ""), f"{st}")
    close_menus()
    click_blank(0.5)
    chord([VK_CTRL], 0x30, wait=1.0)  # Ctrl+0
    time.sleep(1.5)
    close_menus()
    st = bar_state()
    check("Ctrl+0：回到 100%，縮放圖示消失", st.get("zoom") == "", f"{st}")
    # 重新載入
    click_blank(0.5)
    t0 = time.time()
    key(VK_F5)
    time.sleep(2.0)
    check("F5：重新載入", any("[test] load" in l for l in page_log_since(t0)), f"{page_log_since(t0)[-2:]}")
    # 書籤列開關
    click_blank(0.5)
    top0 = page_rect()[1]
    chord([VK_CTRL, VK_SHIFT], 0x42, wait=1.2)  # Ctrl+Shift+B
    top1 = page_rect()[1]
    shot("no_bookmark_bar")
    chord([VK_CTRL, VK_SHIFT], 0x42, wait=1.2)
    top2 = page_rect()[1]
    check("Ctrl+Shift+B：書籤列收起來（網頁往上 34）、再按一次回來", top0 - top1 == round(34 * scale) and top2 == top0, f"{top0} → {top1} → {top2}")
    # Ctrl+D：加入書籤的小視窗
    click_blank(0.5)
    chord([VK_CTRL], 0x44, wait=1.2)  # Ctrl+D
    ms = menus()
    st = bar_state()
    shot("bookmark_bubble")
    check("Ctrl+D：加入書籤、出現小視窗、星星變實心", bool(ms) and st.get("starOn") is True, f"{menu_titles(ms)} {st}")
    if ms:
        log("加入書籤的小視窗", menu_titles(ms))
        menu_click("移除", wait=1.0)
    close_menus()
    st = bar_state()
    check("小視窗按移除：書籤拿掉、星星變回空心", st.get("starOn") is False, f"{st}")
    # 歷史記錄、下載
    for vk, ttl in ((0x48, "歷史記錄"), (0x4A, "下載")):
        click_blank(0.5)
        n1 = tab_count()
        chord([VK_CTRL], vk, wait=2.0)
        check(f"Ctrl+{chr(vk)}：在新分頁開「{ttl}」", ttl in title(web) and tab_count() == n1 + 1, f"{title(web)} {tab_count()}")
        restore_tabs(keep)
        home()
    # 尋找
    click_blank(0.5)
    chord([VK_CTRL], 0x46, wait=1.5)  # Ctrl+F
    shot("find_bar")
    log("Ctrl+F 之後的前景視窗", fg(), repr(title(fg())), proc_name(pid_of(fg())) if fg() else "")
    key(VK_ESC)
    time.sleep(0.6)
    click_blank(0.5)
    # Alt+左右
    go(f"127.0.0.1:{PORT}/snap.html?alt")
    click_blank(0.5)
    chord([VK_ALT], VK_LEFT, wait=2.0)
    t1 = title(web)
    chord([VK_ALT], VK_RIGHT, wait=2.0)
    t2 = title(web)
    check("Alt+←／Alt+→：上一頁／下一頁", "測試頁" in t1 and "短影片" in t2, f"{t1} → {t2}")
    click_bar(BACK, wait=2.0)
    restore_tabs(keep)
    home()


def tip_diag(el, tag):
    """（查問題用）工具列某個元素現在的名稱、樣式、說明文字，和滑鼠底下是什麼"""
    try:
        n = el(bar_tree())
        if n is None:
            log(f"  [{tag}] 找不到元素")
        else:
            extra = []
            for attr in ("HelpText", "AcceleratorKey", "ItemStatus"):
                try:
                    extra.append(f"{attr}={getattr(n.ctrl, attr)!r}")
                except Exception as e:
                    extra.append(f"{attr}=?")
            try:
                lp = n.ctrl.GetLegacyIAccessiblePattern()
                extra.append(f"legacy name={lp.Name!r} desc={lp.Description!r} help={lp.Help!r}")
            except Exception as e:
                extra.append(f"legacy=? {e}")
            log(f"  [{tag}] name={n.name!r} cls={n.cls!r} en={n.en} rect={(n.l, n.t, n.r, n.b)} kids={[(k.type, k.name, k.cls) for k in n.kids]} {' '.join(extra)}")
        pt = wt.POINT()
        user32.GetCursorPos(ctypes.byref(pt))
        try:
            c = auto.ControlFromPoint(pt.x, pt.y)
            log(f"  [{tag}] 滑鼠 {(pt.x, pt.y)} 底下：{c.ControlTypeName} name={c.Name!r} cls={c.ClassName!r} id={c.AutomationId!r}")
        except Exception as e:
            log(f"  [{tag}] 滑鼠底下讀不到：{e}")
    except Exception as e:
        log(f"  [{tag}] 讀不到：{e}")


def tips_now():
    res = []
    for h in top_windows():
        if not user32.IsWindowVisible(h) or title(h) == MENU_TITLE:
            continue
        c = cls(h)
        r = wrect(h)
        w_, h_ = r[2] - r[0], r[3] - r[1]
        if w_ <= 0 or h_ <= 0:
            continue
        owner = proc_name(pid_of(h)).lower()
        if "tooltip" in c.lower() or (owner in ("msedgewebview2.exe", os.path.basename(EXE).lower()) and c.startswith("Chrome_WidgetWin") and h_ <= 90 and w_ <= 600):
            res.append((c, owner, title(h), r))
    return res


def tip_watch(el, tag, secs=3.2):
    """（查問題用）滑鼠停在元素上，每隔一下看提示文字出來了沒"""
    p = bar_pt(el)
    if not p:
        log(f"  [{tag}] 找不到位置")
        return
    hover(*C(*p), settle=0.0)
    seen = []
    t0 = time.time()
    while time.time() - t0 < secs:
        seen.append((round(time.time() - t0, 1), len(tips_now())))
        time.sleep(0.3)
    log(f"  [{tag}] 停在 {C(*p)}：{seen}")
    tip_diag(el, tag)
    shot("diag_" + tag)
    move(*PG(620, 330))
    time.sleep(0.7)


class GUITHREADINFO(ctypes.Structure):
    _fields_ = [("cbSize", wt.DWORD), ("flags", wt.DWORD), ("hwndActive", wt.HWND), ("hwndFocus", wt.HWND), ("hwndCapture", wt.HWND),
                ("hwndMenuOwner", wt.HWND), ("hwndMoveSize", wt.HWND), ("hwndCaret", wt.HWND), ("rcCaret", wt.RECT)]


def post_move_here():
    pt = wt.POINT()
    user32.GetCursorPos(ctypes.byref(pt))
    h = user32.WindowFromPoint(pt)
    for _ in range(6):
        if not h:
            return False
        if cls(h) == "Chrome_WidgetWin_1":
            c = wt.POINT(pt.x, pt.y)
            user32.ScreenToClient(h, ctypes.byref(c))
            user32.PostMessageW(h, 0x0200, 0, ((c.y & 0xffff) << 16) | (c.x & 0xffff))
            return True
        h = user32.GetParent(h)
    return False


def s_login():
    """登入時另外跳出來的小視窗／新分頁：跟原本的網頁連不連得上、登入完會不會自己關"""
    home()
    ensure_web_fg()
    chord([VK_CTRL], 0x54, wait=1.5)  # Ctrl+T
    type_text(f"127.0.0.1:{PORT}/login.html")
    key(VK_ENTER)
    time.sleep(3.0)
    if "登入測試" not in title(web):
        check("登入測試頁打得開", False, repr(title(web)))
        return
    base = set(tab_ids())
    log("  瀏覽器報給網站的身分：", [l.split(" ", 1)[1] for l in page_log().splitlines() if "[login] load ua=" in l or "[hdr]" in l][-2:])

    def since(t0):
        return "\n".join(l.split(" ", 1)[1] for l in page_log_since(t0))

    def tail(L):
        return " | ".join(L.splitlines()[-12:])

    def press(i, wait=4.5, win=None):
        if win is None and not select_title("登入測試", 1.0):
            return False
        n = None
        for _ in range(6):
            n = find_id(page_tree(win), i)
            if n is not None:
                break
            time.sleep(0.5)
        if n is None:
            log(f"  找不到按鈕 {i}")
            return False
        click(*n.center, wait=wait)
        return True

    def extra(w0):
        return [h for h in app_windows() if h not in w0 and user32.IsWindowVisible(h) and title(h) != MENU_TITLE]

    def cleanup(w0):
        for h in extra(w0):
            user32.PostMessageW(h, 0x0010, None, None)
        time.sleep(0.8)
        ensure_web_fg()
        restore_tabs(base)
        select_title("登入測試", 1.0)

    # ---- 指定大小的小視窗 ----
    for b, name in (("b1", "登入小視窗"), ("b2", "登入小視窗（先到別的網站再轉回來）"), ("b6", "登入小視窗（先開空白再前往）")):
        t0, w0 = time.time(), set(app_windows())
        press(b, 5.5)
        L, left = since(t0), extra(w0)
        check(f"{name}：跟開它的網頁連得上", f"[pop {b}] loaded opener=true" in L and (b != "b2" or "[provider] loaded origin=http://localhost" in L and "opener=true" in L.split("[provider]")[1].splitlines()[0]), tail(L))
        check(f"{name}：登入結果送得回原本的網頁", "[login] message from" in L and f'"tag":"{b}"' in L, tail(L))
        check(f"{name}：登入完自己關掉，原本的網頁也知道", f"{b} open=window" in L and f"{b} closed=true" in L and not left, f"留下的視窗={[title(h) for h in left]} " + tail(L))
        check(f"{name}：裡面記住的登入狀態，原本的網頁讀得到", f"login_{b}=1" in L, tail(L))
        cleanup(w0)

    # ---- 小視窗開著：打字、裡面的連結、由原本的網頁關掉 ----
    t0, w0, n_tabs = time.time(), set(app_windows()), tab_count()
    press("b8", 3.0)
    pops = extra(w0)
    shot("login_popup")
    check("登入小視窗：開成另一個視窗、在最前面", len(pops) == 1 and fg() == pops[0], f"{[(title(h), wrect(h)) for h in pops]} fg={fg()}")
    if pops:
        pop = pops[0]
        type_text("ab")
        time.sleep(0.6)
        check("登入小視窗：可以直接打字", "[pop b8] input:ab" in since(t0), tail(since(t0)))
        n = find_id(page_tree(pop), "help")
        if n is not None:
            click(*n.center, wait=3.5)
        w1 = extra(w0)
        tabs_now = tab_count()
        check("登入小視窗裡會開新視窗的連結（說明、忘記密碼這類）：點了有打開", tabs_now > n_tabs or len(w1) > len(pops), f"分頁 {n_tabs}→{tabs_now}，視窗 {len(pops)}→{len(w1)}")
        check("登入小視窗裡的連結：開在原本那個瀏覽器視窗的新分頁，那個視窗到最前面、小視窗還在", tabs_now == n_tabs + 1 and fg() == web and bool(user32.IsWindow(pop)) and "短影片" in title(web),
              f"分頁 {n_tabs}→{tabs_now} fg={fg()} web={web} 目前={title(web)!r}")
        ensure_web_fg()
        press("c8", 2.5)
        L = since(t0)
        gone = not (user32.IsWindow(pop) and user32.IsWindowVisible(pop))
        check("原本的網頁可以把登入小視窗關掉", "c8 called close" in L and "b8 closed=true" in L and gone, f"關了={gone} " + tail(L))
    cleanup(w0)

    # ---- 沒指定大小：開成新分頁 ----
    for b, name in (("b3", "開成新分頁的登入頁"), ("b4", "開成新分頁的登入頁（先開空白再前往）")):
        t0, w0, n0 = time.time(), set(app_windows()), tab_count()
        press(b, 5.5)
        L = since(t0)
        check(f"{name}：網站拿得到開出來的分頁（不會以為被擋掉）", f"{b} open=window" in L, tail(L))
        check(f"{name}：跟開它的網頁連得上、登入結果送得回來", f"[pop {b}] loaded opener=true" in L and f'"tag":"{b}"' in L, tail(L))
        n1 = tab_count()
        check(f"{name}：登入完分頁自己關掉、回到原本的分頁", f"{b} closed=true" in L and n1 == n0 and "登入測試" in title(web), f"分頁 {n0}→{n1} 目前={title(web)!r} " + tail(L))
        cleanup(w0)

    t0, w0, n0 = time.time(), set(app_windows()), tab_count()
    press("b5", 4.5)
    L = since(t0)
    check("送表單到新分頁：資料有送到", "[post] method=POST body=v=hello" in L and tab_count() == n0 + 1, f"分頁 {n0}→{tab_count()} " + tail(L))
    cleanup(w0)

    t0, w0, n0 = time.time(), set(app_windows()), tab_count()
    press("b7", 6.0)
    L = since(t0)
    n1 = tab_count()
    check("同一個名字的視窗開兩次：用同一個分頁（不會開兩個）", "same=true" in L and n1 == n0 + 1 and "[pop b7b] loaded" in L, f"分頁 {n0}→{n1} " + tail(L))
    cleanup(w0)

    for a, want, name in (("a1", "false", "一般的「開新分頁」連結：開新分頁，新分頁碰不到原本的網頁"), ("a2", "true", "指定要連著的連結（rel=opener）：新分頁拿得到原本的網頁")):
        t0, w0, n0 = time.time(), set(app_windows()), tab_count()
        press(a, 4.0)
        L = since(t0)
        check(name, f"[pop {a}] loaded opener={want}" in L and tab_count() == n0 + 1, f"分頁 {n0}→{tab_count()} " + tail(L))
        cleanup(w0)

    # ---- 無痕式視窗 ----
    before = set(app_windows())
    chord([VK_CTRL, VK_SHIFT], 0x4E, wait=4.0)  # Ctrl+Shift+N
    new = [h for h in app_windows() if h not in before and user32.IsWindowVisible(h) and title(h) not in KNOWN]
    if len(new) == 1:
        inc = new[0]
        r = wrect(inc)
        click(r[0] + 400 * scale, r[1] + 71 * scale, wait=0.7)
        type_text(f"127.0.0.1:{PORT}/login.html")
        key(VK_ENTER)
        time.sleep(3.0)
        t0, w0 = time.time(), set(app_windows())
        press("b1", 5.5, win=inc)
        L, left = since(t0), extra(w0)
        shot("login_incognito")
        check("無痕式視窗裡的登入小視窗：連得上、結果送得回來、自己關掉", "[pop b1] loaded opener=true" in L and '"tag":"b1"' in L and "b1 closed=true" in L and not left, f"留下的視窗={[title(h) for h in left]} " + tail(L))
        line = next((l for l in L.splitlines() if "b1 closed=true" in l), "")
        check("無痕式視窗裡的登入小視窗：用無痕的資料（讀得到自己存的、讀不到一般視窗的登入狀態）", "login_b1=1" in line and "login_b2=1" not in line, line)
        for h in left:
            user32.PostMessageW(h, 0x0010, None, None)
        time.sleep(0.8)
        t0, w0 = time.time(), set(app_windows())
        press("b3", 5.5, win=inc)
        L = since(t0)
        check("無痕式視窗裡開成新分頁的登入頁：連得上、結果送得回來、自己關掉", "b3 open=window" in L and "[pop b3] loaded opener=true" in L and '"tag":"b3"' in L and "b3 closed=true" in L, tail(L))
        for h in extra(w0):
            user32.PostMessageW(h, 0x0010, None, None)
        r = wrect(inc)
        click(r[2] - 23 * scale, r[1] + 16 * scale, wait=1.5)
        if user32.IsWindow(inc) and user32.IsWindowVisible(inc):
            user32.PostMessageW(inc, 0x0010, None, None)
            time.sleep(1.0)
    else:
        check("無痕式視窗打得開（測登入小視窗用）", False, f"{[(title(h), wrect(h)) for h in new]}")
    ensure_web_fg()
    restore_tabs(base)
    # 登入測試那個分頁也關掉，回到原本的樣子
    if select_title("登入測試", 1.0):
        chord([VK_CTRL], 0x57, wait=1.2)  # Ctrl+W
    home()


def focus_info(tag):
    """（查問題用）瀏覽器視窗那條執行緒現在鍵盤在哪個視窗上。回傳：鍵盤是不是在某個網頁（或工具列）裡"""
    try:
        tid = user32.GetWindowThreadProcessId(web, None)
        gi = GUITHREADINFO()
        gi.cbSize = ctypes.sizeof(GUITHREADINFO)
        ok = user32.GetGUIThreadInfo(tid, ctypes.byref(gi))
        d = lambda h: (H(h), cls(h), wrect(h), proc_name(pid_of(h))) if h else None
        log(f"  [{tag}] 前景={fg()}（{title(fg())!r}） web={web} ok={ok} active={d(gi.hwndActive)} focus={d(gi.hwndFocus)}")
        return bool(gi.hwndFocus) and H(gi.hwndFocus) != web
    except Exception as e:
        log(f"  [{tag}] 讀不到鍵盤在哪：{e}")
        return None


def s_refocus():
    """別的視窗（無痕式視窗、登入小視窗、開發人員工具）關掉、回到瀏覽器視窗之後：不用先點一下，快捷鍵馬上能用"""
    home()
    ensure_web_fg()
    click_blank(0.6)
    keep = tab_ids()
    focus_info("一開始（剛點過網頁）")

    def keys_work(name):
        """不點任何東西，直接按 Ctrl+T：有沒有多一個分頁"""
        time.sleep(1.2)
        in_page = focus_info(name)
        was_fg = fg() == web
        n0 = tab_count()
        chord([VK_CTRL], 0x54, wait=1.6)  # Ctrl+T
        n1 = tab_count()
        check(f"{name}：回到瀏覽器視窗後不用先點一下，快捷鍵（Ctrl+T）馬上有用", was_fg and n1 == n0 + 1, f"瀏覽器視窗在最前面={was_fg} 鍵盤在網頁或工具列裡={in_page} 分頁 {n0}→{n1}")
        ensure_web_fg()
        restore_tabs(keep)
        home()
        click_blank(0.6)

    # (1) 無痕式視窗：開、在裡面逛一個網頁、按右上角的 × 關掉
    before = set(app_windows())
    chord([VK_CTRL, VK_SHIFT], 0x4E, wait=4.0)  # Ctrl+Shift+N
    new = [h for h in app_windows() if h not in before and user32.IsWindowVisible(h) and title(h) not in KNOWN]
    if len(new) == 1:
        inc = new[0]
        r = wrect(inc)
        click(r[0] + 400 * scale, r[1] + 71 * scale, wait=0.7)
        type_text(f"127.0.0.1:{PORT}/snap.html?refocus")
        key(VK_ENTER)
        time.sleep(2.5)
        r = wrect(inc)
        click(r[2] - 23 * scale, r[1] + 16 * scale, wait=1.5)
        gone = not (user32.IsWindow(inc) and user32.IsWindowVisible(inc))
        log("  無痕式視窗關掉了：", gone)
        if not gone:
            user32.PostMessageW(inc, 0x0010, None, None)
            time.sleep(1.0)
        keys_work("無痕式視窗關掉")
    else:
        check("無痕式視窗打得開（測鍵盤用）", False, f"{[(title(h), wrect(h)) for h in new]}")

    # (2) 登入小視窗：登入完自己關掉
    chord([VK_CTRL], 0x54, wait=1.5)  # Ctrl+T
    type_text(f"127.0.0.1:{PORT}/login.html")
    key(VK_ENTER)
    time.sleep(3.0)
    keep2 = tab_ids()
    n = None
    for _ in range(6):
        n = find_id(page_tree(), "b1")
        if n is not None:
            break
        time.sleep(0.5)
    if n is not None:
        w0 = set(app_windows())
        click(*n.center, wait=5.5)
        left = [h for h in app_windows() if h not in w0 and user32.IsWindowVisible(h) and title(h) != MENU_TITLE]
        log("  登入小視窗留下的視窗：", [title(h) for h in left])
        time.sleep(1.2)
        in_page = focus_info("登入小視窗自己關掉")
        was_fg = fg() == web
        n0 = tab_count()
        chord([VK_CTRL], 0x54, wait=1.6)
        n1 = tab_count()
        check("登入小視窗自己關掉：回到瀏覽器視窗後不用先點一下，快捷鍵（Ctrl+T）馬上有用", was_fg and n1 == n0 + 1, f"瀏覽器視窗在最前面={was_fg} 鍵盤在網頁或工具列裡={in_page} 分頁 {n0}→{n1}")
    else:
        check("登入測試頁打得開（測鍵盤用）", False)
    ensure_web_fg()
    restore_tabs(keep)
    home()
    click_blank(0.6)

    # (3) 開發人員工具：開了再關
    base_dt = set(dt_windows())
    key(0x7B)
    ws = wait_for(lambda: [h for h in dt_windows() if h not in base_dt], 15, 0.3)
    time.sleep(1.0)
    if ws:
        for h in ws:
            user32.PostMessageW(h, 0x0010, None, None)
        wait_for(lambda: not [h for h in dt_windows() if h not in base_dt], 8, 0.3)
        keys_work("開發人員工具關掉")
    else:
        check("開發人員工具打得開（測鍵盤用）", False)

    # (4) 切到別的程式（假的遊戲視窗）再切回來
    if game:
        user32.keybd_event(VK_ALT, 0, 0, 0)
        user32.keybd_event(VK_ALT, 0, 2, 0)
        user32.SetForegroundWindow(game)
        time.sleep(1.0)
        log("  切到遊戲：", fg() == game)
        user32.keybd_event(VK_ALT, 0, 0, 0)
        user32.keybd_event(VK_ALT, 0, 2, 0)
        user32.SetForegroundWindow(web)
        keys_work("切到別的程式再切回來")
    ensure_web_fg()
    restore_tabs(keep)
    home()


class CDP:
    """（查問題用）接到瀏覽器元件的偵錯埠，看一個網頁的要求、錯誤訊息"""

    def __init__(self, ws_url):
        self.ws = websocket.create_connection(ws_url, timeout=8, suppress_origin=True)
        self.n = 0
        self.events = []

    def _recv(self):
        try:
            self.ws.settimeout(0.5)
            return json.loads(self.ws.recv())
        except websocket.WebSocketTimeoutException:
            return None

    def call(self, method, params=None, wait=8):
        self.n += 1
        i = self.n
        self.ws.send(json.dumps({"id": i, "method": method, "params": params or {}}))
        end = time.time() + wait
        while time.time() < end:
            m = self._recv()
            if m is None:
                continue
            if m.get("id") == i:
                return m
            self.events.append((time.time(), m))
        return None

    def pump(self, secs):
        end = time.time() + secs
        while time.time() < end:
            m = self._recv()
            if m is not None:
                self.events.append((time.time(), m))

    def js(self, expr, wait=8):
        r = self.call("Runtime.evaluate", {"expression": expr, "returnByValue": True, "awaitPromise": True}, wait)
        try:
            return r["result"]["result"].get("value")
        except Exception:
            return r

    def close(self):
        try:
            self.ws.close()
        except Exception:
            pass


def cdp_targets(port):
    try:
        with urllib.request.urlopen(f"http://127.0.0.1:{port}/json/list", timeout=5) as r:
            return json.loads(r.read().decode("utf-8", "replace"))
    except Exception as e:
        log(f"  偵錯埠 {port} 讀不到：{e}")
        return []


def app_cdp_ports():
    """程式的瀏覽器元件開的偵錯埠（每個資料夾各一個）"""
    import glob
    res = []
    roots = [data_dir, os.path.join(os.environ.get("LOCALAPPDATA", ""), "com.toolbox.desktop")]
    for root in roots:
        for f in glob.glob(os.path.join(root, "**", "DevToolsActivePort"), recursive=True):
            try:
                res.append((f, int(open(f).read().split()[0])))
            except Exception as e:
                log("  讀不到", f, e)
    return res


STATE_JS = """(async () => {
  const q = (s) => document.querySelector(s);
  let sw = '?';
  try { sw = (await navigator.serviceWorker.getRegistrations()).map(r => r.scope + ':' + (r.active ? r.active.state : 'none')).join(','); } catch (e) { sw = 'err ' + e; }
  const root = q('#react-root');
  return {
    href: location.href, ready: document.readyState, title: document.title,
    text: (document.body ? document.body.innerText : '').replace(/\\s+/g, ' ').slice(0, 260),
    placeholder: !!q('#placeholder'), rootKids: root ? root.querySelectorAll('*').length : -1,
    scripts: document.scripts.length, scriptsLoaded: JSON.stringify(window.__SCRIPTS_LOADED__ || null),
    ua: navigator.userAgent, webdriver: navigator.webdriver, cookieEnabled: navigator.cookieEnabled,
    cookieLen: document.cookie.length, cookieNames: document.cookie.split(';').map(c => c.split('=')[0].trim()).filter(Boolean).join(','),
    webview: !!(window.chrome && window.chrome.webview), tauri: Object.keys(window).filter(k => /TAURI|^__mb/i.test(k)).join(','),
    sw,
  };
})()"""


def cdp_watch(tag, c, url, secs=28):
    """用偵錯埠前往一個網址，記下每個要求的結果、錯誤訊息，最後回報網頁的狀態"""
    for d in ("Page", "Runtime", "Log", "Network"):
        c.call(d + ".enable")
    c.events.clear()
    t0 = time.time()
    c.call("Page.navigate", {"url": url})
    c.pump(10)
    shot(f"xcom_{tag}_10s")
    st10 = c.js(STATE_JS)
    log(f"  [{tag}] 10 秒時：", json.dumps(st10, ensure_ascii=False)[:900])
    c.pump(max(1, secs - (time.time() - t0)))
    shot(f"xcom_{tag}_end")
    reqs, order, errs, pages = {}, [], [], []
    for t, m in c.events:
        me, pr = m.get("method"), m.get("params", {})
        if me == "Network.requestWillBeSent":
            i = pr["requestId"]
            if i not in reqs:
                order.append(i)
            reqs[i] = {"url": pr["request"]["url"], "type": pr.get("type"), "t": t - t0, "method": pr["request"].get("method"), "done": None}
        elif me == "Network.responseReceived" and pr["requestId"] in reqs:
            r = pr["response"]
            reqs[pr["requestId"]].update(status=r.get("status"), sw=r.get("fromServiceWorker"), proto=r.get("protocol"), mime=r.get("mimeType"))
        elif me == "Network.loadingFinished" and pr["requestId"] in reqs:
            reqs[pr["requestId"]]["done"] = "ok"
        elif me == "Network.loadingFailed" and pr["requestId"] in reqs:
            reqs[pr["requestId"]]["done"] = "FAILED " + str(pr.get("errorText")) + (" blocked=" + str(pr.get("blockedReason")) if pr.get("blockedReason") else "") + (" canceled" if pr.get("canceled") else "")
        elif me == "Runtime.exceptionThrown":
            d = pr.get("exceptionDetails", {})
            errs.append(f"exception {d.get('text')} {(d.get('exception') or {}).get('description', '')[:300]} @{d.get('url')}:{d.get('lineNumber')}")
        elif me == "Runtime.consoleAPICalled" and pr.get("type") in ("error", "warning", "assert"):
            errs.append(f"console.{pr.get('type')} " + " ".join(str(a.get("value", a.get("description", "")))[:200] for a in pr.get("args", [])[:4]))
        elif me == "Log.entryAdded":
            e = pr.get("entry", {})
            if e.get("level") in ("error", "warning"):
                errs.append(f"log.{e.get('level')} [{e.get('source')}] {str(e.get('text'))[:260]} {str(e.get('url') or '')[:120]}")
        elif me in ("Page.loadEventFired", "Page.domContentEventFired"):
            pages.append(f"{me.split('.')[1]}@{t - t0:.1f}s")
        elif me == "Page.frameNavigated" and not pr.get("frame", {}).get("parentId"):
            pages.append(f"navigated {pr['frame'].get('url')}@{t - t0:.1f}s")
    short = lambda u: u if len(u) <= 150 else u[:150] + "…"
    done = [r for r in reqs.values() if r["done"] == "ok"]
    failed = [r for r in reqs.values() if r["done"] and r["done"] != "ok"]
    pending = [r for r in reqs.values() if not r["done"]]
    bad = [r for r in reqs.values() if isinstance(r.get("status"), int) and r["status"] >= 400]
    log(f"  [{tag}] 頁面事件：{pages}")
    log(f"  [{tag}] 要求 {len(reqs)} 個：完成 {len(done)}、失敗 {len(failed)}、還沒回來 {len(pending)}、4xx/5xx {len(bad)}")
    for r in failed[:25]:
        log(f"    失敗 {r['type']} {r['done']} {short(r['url'])}")
    for r in pending[:25]:
        log(f"    還沒回來（{r['t']:.1f}s 送出） {r['type']} {r.get('status')} {short(r['url'])}")
    for r in bad[:25]:
        log(f"    {r['status']} {r['type']} {short(r['url'])}")
    scripts = [r for r in reqs.values() if r["type"] == "Script"]
    log(f"  [{tag}] 程式檔 {len(scripts)} 個：", [(r.get("status"), r["done"], r["url"].split("/")[-1][:40]) for r in scripts][:40])
    for e in errs[:40]:
        log(f"    {e}")
    st = c.js(STATE_JS)
    log(f"  [{tag}] 最後：", json.dumps(st, ensure_ascii=False)[:900])
    return st if isinstance(st, dict) else {}


def listening_ports():
    """程式的瀏覽器元件（msedgewebview2.exe）在這台電腦上開著的埠（偵錯埠）"""
    out = subprocess.run(["netstat", "-ano", "-p", "tcp"], capture_output=True, text=True).stdout
    res = []
    for l in out.splitlines():
        q = l.split()
        if len(q) >= 5 and q[0] == "TCP" and q[3] == "LISTENING" and q[1].startswith("127.0.0.1:"):
            try:
                pid, port = int(q[4]), int(q[1].rsplit(":", 1)[1])
            except Exception:
                continue
            if proc_name(pid).lower() == "msedgewebview2.exe":
                res.append((pid, port))
    return res


XCOM_URLS = [("root", "https://x.com/"), ("profile", "https://x.com/X"), ("explore", "https://x.com/explore"), ("home", "https://x.com/home"), ("search", "https://x.com/search?q=MapleStory&src=typed_query")]


def s_xcom():
    """（查問題用）程式裡的瀏覽器開 x.com（不登入），看有沒有卡在 X 的標誌；跟同一台機器上的 Edge 對照"""
    home()
    ensure_web_fg()
    click_blank(0.5)
    keep = tab_ids()
    try:
        ps = ("Get-CimInstance Win32_Process | Where-Object { $_.Name -eq 'msedgewebview2.exe' -and $_.CommandLine -notmatch '--type=' } | ForEach-Object { \"$($_.ProcessId) :: $($_.CommandLine)\" }")
        out = subprocess.run(["powershell", "-NoProfile", "-Command", ps], capture_output=True, text=True, timeout=60).stdout
        log("  瀏覽器元件的啟動參數：\n" + "\n".join("    " + l for l in out.splitlines() if l.strip()))
    except Exception as e:
        log("  讀不到啟動參數", e)
    chord([VK_CTRL], 0x54, wait=1.5)  # Ctrl+T
    type_text("x.com")
    key(VK_ENTER)
    time.sleep(12)
    shot("xcom_typed")
    log("  打網址前往 x.com 12 秒後：標題", repr(title(web)), bar_state())
    txt = page_text()
    log("  網頁上的文字：", " | ".join(x.strip() for x in txt.splitlines() if x.strip())[:300])
    ports = [pt for _, pt in app_cdp_ports()] + [pt for _, pt in listening_ports()]
    log("  偵錯埠：", app_cdp_ports(), listening_ports())
    res = {}
    c = None
    for port in dict.fromkeys(ports):
        ts = cdp_targets(port)
        log(f"  {port} 的網頁：", [(t.get("type"), t.get("url", "")[:70]) for t in ts])
        tg = next((t for t in ts if t.get("type") == "page" and "x.com" in t.get("url", "")), None)
        if tg is not None:
            c = CDP(tg["webSocketDebuggerUrl"])
            break
    if c is not None:
        try:
            for tag, url in XCOM_URLS:
                res[tag] = cdp_watch("app_" + tag, c, url, secs=24)
        finally:
            c.close()
    else:
        log("  接不上程式瀏覽器的偵錯埠：改用打網址、讀畫面的方式")
        for tag, url in XCOM_URLS:
            go(url.replace("https://", ""), wait=18)
            shot("xcom_app_" + tag)
            t2 = page_text()
            st = bar_state()
            words = " | ".join(x.strip() for x in t2.splitlines() if x.strip())
            log(f"  [app_{tag}] 標題={title(web)!r} 網址列={st.get('shown')!r} 文字（{len(words)} 字）：{words[:300]}")
            res[tag] = {"title": title(web), "placeholder": len(words) < 20, "rootKids": len(words)}
    # ---- 對照：同一台機器上的 Edge（全新的使用者資料） ----
    edge = next((p for p in (r"C:\Program Files (x86)\Microsoft\Edge\Application\msedge.exe", r"C:\Program Files\Microsoft\Edge\Application\msedge.exe") if os.path.exists(p)), None)
    ctl = {}
    if edge:
        udd = os.path.join(os.environ.get("TEMP", "."), "edge-xcom")
        shutil.rmtree(udd, ignore_errors=True)
        ep = subprocess.Popen([edge, "--remote-debugging-port=9444", "--remote-allow-origins=*", f"--user-data-dir={udd}", "--no-first-run", "--no-default-browser-check", "--window-size=1000,700", "about:blank"])
        time.sleep(6)
        ts = cdp_targets(9444)
        tg = next((t for t in ts if t.get("type") == "page"), None)
        if tg is not None:
            c2 = CDP(tg["webSocketDebuggerUrl"])
            try:
                for tag, url in XCOM_URLS:
                    ctl[tag] = cdp_watch("edge_" + tag, c2, url, secs=20)
            finally:
                c2.close()
        else:
            log("  Edge 的偵錯埠接不上", ts)
        subprocess.run(["taskkill", "/PID", str(ep.pid), "/T", "/F"], capture_output=True)
        time.sleep(1.0)
    else:
        log("  這台機器沒有 Edge")
    stuck = lambda st: bool(st) and bool(st.get("placeholder")) and st.get("rootKids", 0) < 20
    brief = lambda d: {k: ("卡住" if stuck(v) else "有內容", v.get("title"), v.get("rootKids")) for k, v in d.items()}
    log("  結果：程式裡", brief(res))
    log("  結果：Edge", brief(ctl))
    for tag, _ in XCOM_URLS:
        if tag in res:
            check(f"x.com（沒登入，{tag}）在程式的瀏覽器裡：不會卡在 X 的標誌", not stuck(res[tag]), f"程式={res[tag].get('title')!r}/{res[tag].get('rootKids')} | Edge={ctl.get(tag, {}).get('title')!r}/{ctl.get(tag, {}).get('rootKids')}")
    ensure_web_fg()
    restore_tabs(keep)
    home()


def s_xprobe():
    """（查問題用）x.com 的首頁：網頁和它的 Worker 在等哪些瀏覽器功能；一般視窗、無痕式視窗、加上使用者的整合包各看一次"""
    home()
    ensure_web_fg()
    click_blank(0.5)
    keep = tab_ids()

    def read_overlay(win=None):
        root = page_tree(win)
        if root is None:
            return ""
        parts = sorted(n.name for n in find_all(root, lambda n: n.type == "TextControl" and n.name[:2] == "MB" and n.name[4:5] == ":"))
        return "".join(x[5:] for x in parts)

    def report(tag, ov, win=None):
        try:
            st = json.loads(ov[ov.index("{"):])
        except Exception:
            log(f"  [{tag}] 讀不到（或不完整，共 {len(ov)} 字）：", ov[:12000])
            return None
        stuck = st.get("textLen", 0) < 40
        log(f"  [{tag}] {'停在標誌' if stuck else '有內容'} 標題={st.get('title')!r} 文字={st.get('textLen')} raw={st.get('raw')} raf={st.get('raf')} 還在等={st.get('pending')}")
        log(f"  [{tag}] 環境：", json.dumps(st.get("env"), ensure_ascii=False))
        log(f"  [{tag}] 動態效果：", json.dumps(st.get("motion"), ensure_ascii=False), "畫面：", json.dumps(st.get("view"), ensure_ascii=False), "vis", st.get("vis"))
        for k, w in enumerate(st.get("workers") or []):
            log(f"  [{tag}] Worker {k}：", json.dumps({x: w.get(x) for x in ("u", "len", "inst", "posts", "got", "term", "beat", "wt")}, ensure_ascii=False))
            log(f"      誰建立的：{w.get('stack')}")
            log(f"      開頭：{w.get('head')!r}")
            log(f"      裡面：{json.dumps(w.get('w'), ensure_ascii=False)}")
        log(f"  [{tag}] 呼叫：", json.dumps(st.get("calls"), ensure_ascii=False)[:5000])
        return st

    def to_game():
        # 遊戲上沒有被 web 視窗蓋住的點：點一下，前景就是遊戲（跟邊玩邊看網頁一樣）
        g, r = wrect(game), wrect(web)
        sh = user32.GetSystemMetrics(1) - 60
        cands = [(g[0] + 40, g[1] + 60), (g[2] - 40, g[1] + 60), (g[0] + 40, min(g[3], sh) - 30), (g[2] - 40, min(g[3], sh) - 30)]
        gx, gy = next(((x, y) for x, y in cands if not (r[0] <= x < r[2] and r[1] <= y < r[3])), cands[0])
        click(gx, gy, wait=0.3)
        return fg() == game

    def app_pass(tag, url, game_fg=False):
        ensure_web_fg()
        chord([VK_CTRL], 0x54, wait=1.5)  # Ctrl+T
        type_text(url)
        key(0x2E)  # Delete：網址列自動補上的字不要
        key(VK_ENTER)
        if game_fg:
            log(f"  [{tag}] 按 Enter 後馬上點遊戲：遊戲在前景", to_game(), "fg", repr(title(fg())))
        time.sleep(6)
        shot("xprobe_" + tag + "_6s")
        time.sleep(13)
        if game_fg:
            log(f"  [{tag}] 19 秒時的前景：", repr(title(fg())))
        shot("xprobe_" + tag)
        log(f"  [{tag}] 19 秒後：標題", repr(title(web)), bar_state().get("shown"))
        st = report(tag, read_overlay())
        txt = page_text()
        log(f"  [{tag}] 網頁上的文字：", " | ".join(x.strip() for x in txt.splitlines() if x.strip() and not x.startswith("MB"))[:300])
        return st

    # ---- 程式裡的瀏覽器：一般視窗 ----
    app_pass("app_raw", "https://x.com/#mbraw")
    st = app_pass("app", "https://x.com/")
    if st is not None:
        time.sleep(10)
        report("app 再過 10 秒", read_overlay())
    app_pass("app_bundle", "https://x.com/#mbb")
    app_pass("app_game", "https://x.com/", game_fg=True)
    ensure_web_fg()
    restore_tabs(keep)
    home()
    # ---- 程式裡的瀏覽器：無痕式視窗 ----
    click_blank(0.5)
    before = set(app_windows())
    chord([VK_CTRL, VK_SHIFT], 0x4E, wait=4.0)  # Ctrl+Shift+N
    new = [h for h in app_windows() if h not in before and user32.IsWindowVisible(h) and title(h) not in KNOWN]
    if len(new) == 1 and "無痕" in title(new[0]):
        inc = new[0]
        r = wrect(inc)
        click(r[0] + 400 * scale, r[1] + 71 * scale, wait=0.7)
        type_text("https://x.com/")
        key(0x2E)
        key(VK_ENTER)
        time.sleep(19)
        shot("xprobe_incognito")
        log("  [無痕] 19 秒後：標題", repr(title(inc)))
        report("無痕", read_overlay(inc), inc)
        txt = page_text(inc)
        log("  [無痕] 網頁上的文字：", " | ".join(x.strip() for x in txt.splitlines() if x.strip() and not x.startswith("MB"))[:300])
        r = wrect(inc)
        click(r[2] - 23 * scale, r[1] + 16 * scale, wait=1.5)
        log("  [無痕] 視窗關掉了：", not user32.IsWindow(inc) or not user32.IsWindowVisible(inc))
    else:
        log("  無痕式視窗打不開：", [(title(h), wrect(h)) for h in new])
    ensure_web_fg()
    home()
    # ---- 同一台機器上的 Edge（對照）----
    edge = next((p for p in (r"C:\Program Files (x86)\Microsoft\Edge\Application\msedge.exe", r"C:\Program Files\Microsoft\Edge\Application\msedge.exe") if os.path.exists(p)), None)
    if not edge:
        log("  這台機器沒有 Edge")
        return
    diag_js = open(os.path.join(HERE, "xdiag.js"), encoding="utf-8").read()
    udd = os.path.join(os.environ.get("TEMP", "."), "edge-xprobe")
    shutil.rmtree(udd, ignore_errors=True)
    ep = subprocess.Popen([edge, "--remote-debugging-port=9444", "--remote-allow-origins=*", f"--user-data-dir={udd}", "--no-first-run", "--no-default-browser-check", "--lang=zh-TW", "--window-size=1000,700", "about:blank"])
    time.sleep(6)
    tg = next((t for t in cdp_targets(9444) if t.get("type") == "page"), None)
    if tg is None:
        log("  Edge 的偵錯埠接不上")
        subprocess.run(["taskkill", "/PID", str(ep.pid), "/T", "/F"], capture_output=True)
        return
    c = CDP(tg["webSocketDebuggerUrl"])
    try:
        for d in ("Page", "Runtime", "Network"):
            c.call(d + ".enable")
        c.call("Network.setExtraHTTPHeaders", {"headers": {"Accept-Language": "zh-TW,zh;q=0.9,en-US;q=0.8,en;q=0.7"}})
        last = None
        for tag, pre, motion in (("edge", "", "no-preference"), ("edge_raw", "window.__mbRaw=true;", "no-preference"), ("edge_reduce", "", "reduce")):
            c.call("Emulation.setEmulatedMedia", {"features": [{"name": "prefers-color-scheme", "value": "dark"}, {"name": "prefers-reduced-motion", "value": motion}]})
            if last:
                c.call("Page.removeScriptToEvaluateOnNewDocument", {"identifier": last})
            c.call("Network.clearBrowserCookies")
            c.call("Storage.clearDataForOrigin", {"origin": "https://x.com", "storageTypes": "all"})
            r = c.call("Page.addScriptToEvaluateOnNewDocument", {"source": pre + diag_js})
            last = (r or {}).get("result", {}).get("identifier")
            c.events.clear()
            c.call("Page.navigate", {"url": "https://x.com/"})
            c.pump(19)
            raw = c.js("window.__mbSummary ? window.__mbSummary() : 'no summary'")
            report(tag, "URL " + str(raw))
            shot("xprobe_" + tag)
            if tag == "edge":
                # Worker 的原始碼、網頁載入了哪些檔案：存下來看
                try:
                    blobs = json.loads(c.js("JSON.stringify(window.__mbBlobs || {})", wait=20) or "{}")
                    for k, (u, text) in enumerate(blobs.items()):
                        with open(os.path.join(OUT, f"xworker_{k}.js.txt"), "w", encoding="utf-8") as f:
                            f.write("// " + u + "\n" + text)
                        log(f"  Edge：blob {k} {u[-14:]} 長度 {len(text)}")
                    res = c.js("JSON.stringify(performance.getEntriesByType('resource').map(e => e.initiatorType + ' ' + Math.round(e.startTime) + '-' + Math.round(e.responseEnd) + ' ' + e.name.slice(0, 150)))", wait=20)
                    with open(os.path.join(OUT, "xresources.txt"), "w", encoding="utf-8") as f:
                        f.write("\n".join(json.loads(res or "[]")))
                    html = c.js("document.documentElement.outerHTML.slice(0, 60000)", wait=20)
                    with open(os.path.join(OUT, "xpage.html.txt"), "w", encoding="utf-8") as f:
                        f.write(str(html))
                except Exception as e:
                    log("  Edge：存檔失敗", repr(e))
    finally:
        c.close()
        subprocess.run(["taskkill", "/PID", str(ep.pid), "/T", "/F"], capture_output=True)
        time.sleep(1.0)
    ensure_web_fg()
    home()


def s_xuser():
    """（查問題用）要給使用者的診斷腳本：在程式的瀏覽器裡開 x.com，報告會不會自己存到「下載」、網頁還正不正常"""
    import glob
    dl_dir = os.path.join(os.path.expanduser("~"), "Downloads")
    os.makedirs(dl_dir, exist_ok=True)
    home()
    ensure_web_fg()
    click_blank(0.5)
    keep = tab_ids()

    def reports():
        return set(glob.glob(os.path.join(dl_dir, "x-diag-report-*")))

    def collect(tag, before, win=None):
        new = sorted(reports() - before)
        log(f"  [{tag}] 「下載」裡多出來的報告：", [os.path.basename(x) for x in new])
        for k, f in enumerate(new):
            try:
                shutil.copyfile(f, os.path.join(OUT, f"xuser_{tag}_{k}.txt"))
                rep = json.loads(open(f, encoding="utf-8").read())
                log(f"  [{tag}] 報告 {os.path.getsize(f)} 位元組：執行的地方={rep.get('執行的地方')!r} 心跳={json.dumps(rep.get('心跳'), ensure_ascii=False)[:200]}")
                log(f"  [{tag}] 小測試：", json.dumps(rep.get("小測試"), ensure_ascii=False)[:1500])
                log(f"  [{tag}] 畫面：", json.dumps(rep.get("畫面"), ensure_ascii=False)[:1200])
            except Exception as e:
                log(f"  [{tag}] 報告讀不了：", repr(e))
        txt = page_text(win)
        lines = [x.strip() for x in txt.splitlines() if x.strip()]
        log(f"  [{tag}] 網頁上的文字：", " | ".join(lines)[:700])
        check(f"診斷腳本（{tag}）：報告自己存到「下載」", len(new) >= 1, f"{[os.path.basename(x) for x in new]}")
        check(f"診斷腳本（{tag}）：x.com 的登入畫面照常出現", any("繼續" in x for x in lines), " | ".join(lines)[:200])
        check(f"診斷腳本（{tag}）：右下角的小視窗寫著報告已存好", any("報告已存到" in x for x in lines), " | ".join(x for x in lines if "報告" in x or "診斷" in x)[:300])
        return new

    # ---- 一般視窗 ----
    before = reports()
    chord([VK_CTRL], 0x54, wait=1.5)  # Ctrl+T
    type_text("https://x.com/")
    key(0x2E)
    key(VK_ENTER)
    time.sleep(3)
    shot("xuser_3s")
    early = page_text()
    check("診斷腳本：一開始右下角就有小視窗", "工具箱：X 診斷" in early, " | ".join(x.strip() for x in early.splitlines() if x.strip())[:200])
    time.sleep(24)
    shot("xuser_normal")
    collect("一般視窗", before)
    # ---- 重新整理一次（同一個分頁第二次自動存檔）----
    before = reports()
    ensure_web_fg()
    key(VK_F5)
    time.sleep(27)
    shot("xuser_reload")
    collect("重新整理", before)
    # ---- 故意把網頁卡死（跟使用者的情況一樣：網頁整個不動）----
    before = reports()
    ensure_web_fg()
    chord([VK_CTRL], 0x54, wait=1.5)
    type_text("https://x.com/#mbfreeze")
    key(0x2E)
    key(VK_ENTER)
    time.sleep(12)
    shot("xuser_frozen_12s")
    txt = page_text()
    log("  [卡死] 12 秒時網頁上的文字：", " | ".join(x.strip() for x in txt.splitlines() if x.strip())[:300])
    time.sleep(18)
    shot("xuser_frozen_30s")
    new = sorted(reports() - before)
    check("診斷腳本（卡死）：卡死的那一頁存不了報告（預期中）", len(new) == 0, f"{[os.path.basename(x) for x in new]}")
    # 卡死的分頁還開著：開新分頁到 x.com/robots.txt，把上一次卡住的記錄拿出來
    before = reports()
    ensure_web_fg()
    chord([VK_CTRL], 0x54, wait=1.5)
    type_text("x.com/robots.txt")
    key(0x2E)
    key(VK_ENTER)
    time.sleep(7)
    shot("xuser_robots")
    txt = page_text()
    lines = [x.strip() for x in txt.splitlines() if x.strip()]
    log("  [robots] 網頁上的文字：", " | ".join(lines)[:600])
    early = sorted(reports() - before)
    log("  [robots]「下載」裡多出來的：", [os.path.basename(x) for x in early])
    prev_ok = False
    for f in early:
        try:
            shutil.copyfile(f, os.path.join(OUT, "xuser_prev.txt"))
            rep = json.loads(open(f, encoding="utf-8").read())
            pv = (rep.get("上次沒跑完") or [{}])[0]
            log("  [robots] 看門狗的記錄：", rep.get("看門狗的記錄"), "→", json.dumps(pv.get("看門狗"), ensure_ascii=False))
            log("  [robots] 上一次的記錄：", json.dumps(pv, ensure_ascii=False)[:900])
            prev_ok = "webgl.getParameter" in str(pv.get("最後正在做")) and bool(pv.get("看門狗"))
        except Exception as e:
            log("  [robots] 讀不了：", repr(e))
    check("診斷腳本（卡死之後開 x.com/robots.txt）：上一次停在哪裡有記下來、有自動存檔", prev_ok, f"{[os.path.basename(x) for x in early]}")
    check("診斷腳本（x.com/robots.txt）：小視窗寫著記錄已存好", any("上一次卡住的記錄已存到" in x for x in lines), " | ".join(x for x in lines if "記錄" in x)[:300])
    restore_tabs(keep)
    home()
    # ---- 無痕式視窗 ----
    click_blank(0.5)
    before = reports()
    wins0 = set(app_windows())
    chord([VK_CTRL, VK_SHIFT], 0x4E, wait=4.0)
    new = [h for h in app_windows() if h not in wins0 and user32.IsWindowVisible(h) and title(h) not in KNOWN]
    if len(new) == 1 and "無痕" in title(new[0]):
        inc = new[0]
        r = wrect(inc)
        click(r[0] + 400 * scale, r[1] + 71 * scale, wait=0.7)
        type_text("https://x.com/")
        key(0x2E)
        key(VK_ENTER)
        time.sleep(27)
        shot("xuser_incognito")
        collect("無痕視窗", before, inc)
        r = wrect(inc)
        click(r[2] - 23 * scale, r[1] + 16 * scale, wait=1.5)
    else:
        check("診斷腳本：無痕式視窗打得開", False, f"{[(title(h), wrect(h)) for h in new]}")
    ensure_web_fg()
    home()


def dt_windows():
    """畫面上 WebView2 自己開的大視窗（開發人員工具是另一個視窗）"""
    res = []
    for h in top_windows():
        if not user32.IsWindowVisible(h) or proc_name(pid_of(h)).lower() != "msedgewebview2.exe":
            continue
        r = wrect(h)
        if r[2] - r[0] >= 300 and r[3] - r[1] >= 200:
            res.append(h)
    return res


def s_devtools():
    """開發人員工具（F12）：一般網頁打得開；程式自己的頁面（設定、新分頁…）不行"""
    home()
    ensure_web_fg()
    click_blank(0.5)
    keep = tab_ids()
    base_dt = set(dt_windows())
    log("  原本就有的 WebView2 視窗：", [(title(h), cls(h), wrect(h)) for h in base_dt])

    def new_dt():
        return [h for h in dt_windows() if h not in base_dt]

    def info(ws):
        return [(title(h), cls(h), wrect(h)) for h in ws]

    def opened(wait=15):
        ws = wait_for(new_dt, wait, 0.3)
        time.sleep(1.0)
        return new_dt() if ws else []

    def close_dt():
        for h in new_dt():
            user32.PostMessageW(h, 0x0010, None, None)
        wait_for(lambda: not new_dt(), 8, 0.3)
        time.sleep(0.5)
        ensure_web_fg()
        return not new_dt()

    def none_opens(secs=3.5):
        time.sleep(secs)
        return not new_dt()

    # 1. 一般網頁：F12
    key(0x7B)
    ws = opened()
    shot("devtools_f12")
    check("一般網頁按 F12：打開開發人員工具（另一個視窗）", len(ws) == 1, f"{info(ws)}")
    check("開發人員工具開著時：瀏覽器視窗還在、有回應", bool(user32.IsWindow(web)) and responsive(web))
    check("開發人員工具：關得掉", close_dt(), f"{info(new_dt())}")

    # 2. 開過的分頁要前往程式自己的頁面：改開在新分頁
    n0 = tab_count()
    go("chrome://settings", wait=3.0)
    ts = tabs_info()
    check("開過開發人員工具的分頁前往「設定」：改開在新分頁，原本的分頁留在網頁上",
          len(ts) == n0 + 1 and any(t["active"] and "設定" in t["title"] for t in ts) and any("測試頁" in t["title"] for t in ts), f"{[(t['title'], t['active']) for t in ts]}")

    # 3. 程式自己的頁面：打不開
    if any(t["active"] and "設定" in t["title"] for t in ts):
        # 點頁面上的標題文字（不會動到任何設定），鍵盤交給這個頁面
        hd = next((n for n in find_all(page_tree(), lambda n: n.type == "TextControl" and n.name.strip() == "設定" and n.w > 0)), None)
        log("  設定頁面的標題：", hd)
        # 鍵盤還在網址列上時（工具列接到按鍵）
        key(0x7B)
        ok0 = none_opens()
        if hd is not None:
            click(*hd.center, wait=0.5)
        key(0x7B)
        ok1 = none_opens()
        chord([VK_CTRL, VK_SHIFT], 0x49, wait=0.5)  # Ctrl+Shift+I
        ok2 = none_opens()
        check("「設定」（程式自己的頁面）按 F12、Ctrl+Shift+I：不會打開開發人員工具", ok0 and ok1 and ok2, f"網址列上按 F12={ok0} 頁面上按 F12={ok1} Ctrl+Shift+I={ok2} {info(new_dt())}")
        close_dt()
        if hd is not None:
            ms = ctx_at(*hd.center)
            names = [it["t"] for _, m in ms for it in m["items"]]
            check("「設定」按右鍵：沒有「檢查」", "檢查" not in names, f"{names}")
            close_menus()
        ms = open_menu(MENU_BTN, "⋮（設定頁面上）")
        if ms:
            menu_hover("更多工具")
            wait_for(lambda: len(menu_wins()) >= 2, 3, 0.2)
            time.sleep(0.5)
            f = menu_find("開發人員工具")
            check("「設定」頁面上：⋮ → 更多工具 →「開發人員工具」不能按", bool(f) and f[1]["off"], f"{menu_titles()}")
        close_menus()
    restore_tabs(keep)
    home()
    click_blank(0.5)

    # 4. 右鍵 → 檢查
    ms = ctx_at(*PG(300, 330))
    names = [it["t"] for _, m in ms for it in m["items"]]
    check("一般網頁按右鍵：最後一項是「檢查」", bool(names) and names[-1] == "檢查", f"{names}")
    if "檢查" in names:
        menu_click("檢查", wait=1.0)
        ws = opened()
        check("右鍵 → 檢查：打開開發人員工具", len(ws) == 1 and not menu_wins(), f"{info(ws)}")
        close_dt()
    close_menus()

    # 5. ⋮ → 更多工具 → 開發人員工具
    ms = open_menu(MENU_BTN, "⋮（要開開發人員工具）")
    if ms:
        menu_hover("更多工具")
        wait_for(lambda: len(menu_wins()) >= 2, 3, 0.2)
        time.sleep(0.5)
        f = menu_find("開發人員工具")
        check("⋮ → 更多工具：有「開發人員工具」，一般網頁上可以按", bool(f) and not f[1]["off"], f"{menu_titles()}")
        if f:
            menu_click("開發人員工具", wait=1.0)
            ws = opened()
            check("⋮ → 更多工具 → 開發人員工具：打開", len(ws) == 1, f"{info(ws)}")
            close_dt()
    close_menus()

    # 6. 新分頁（程式自己的頁面）打不開；前往一般網頁之後才打得開；再按上一頁回新分頁時，改開在另一個分頁
    click_blank(0.5)
    chord([VK_CTRL], 0x54, wait=1.8)  # Ctrl+T
    key(0x7B)
    check("新分頁（程式自己的頁面）按 F12：不會打開開發人員工具", none_opens(), f"{info(new_dt())}")
    close_dt()
    type_text(f"127.0.0.1:{PORT}/snap.html?devtools")
    key(VK_ENTER)
    time.sleep(3.5)
    click_blank(0.5)
    key(0x7B)
    ws = opened()
    check("從新分頁前往一般網頁之後按 F12：打得開", len(ws) == 1, f"{info(ws)}")
    # 開發人員工具還開著：按上一頁（要回到新分頁那個程式自己的頁面）
    ensure_web_fg()
    click_blank(0.5)
    n1 = tab_count()
    chord([VK_ALT], VK_LEFT, wait=3.0)
    ts = tabs_info()
    check("開發人員工具開著時按上一頁要回到新分頁：原本的分頁留在網頁上，新分頁另外開",
          len(ts) == n1 + 1 and any("短影片" in t["title"] for t in ts) and len(new_dt()) == len(ws), f"{[(t['title'], t['active']) for t in ts]} 工具視窗={info(new_dt())}")
    shot("devtools_back")
    close_dt()
    restore_tabs(keep)
    home()
    check("開發人員工具這一步結束：沒有留下工具視窗、分頁回到原本的樣子", not new_dt() and tab_count() == len(keep), f"{info(new_dt())} 分頁 {tab_count()}/{len(keep)}")


def s_maindiag():
    """（查問題用）主畫面上有提示文字的元素"""
    main = next((h for h in app_windows() if title(h) == "工具箱"), 0)
    if not main:
        log("找不到主畫面")
        return
    user32.keybd_event(VK_ALT, 0, 0, 0)
    user32.keybd_event(VK_ALT, 0, 2, 0)
    user32.SetForegroundWindow(main)
    time.sleep(1.0)
    shot("main_window")
    log("主畫面", wrect(main), "fg", fg() == main)
    rhs = render_hwnds(main)
    root = snap(rhs[0], limit=900) if rhs else None
    n_all = 0
    shown = 0
    if root is not None:
        for n in root.walk():
            n_all += 1
            if n.type not in ("ButtonControl", "HyperlinkControl", "GroupControl", "CheckBoxControl", "EditControl", "ComboBoxControl", "ListItemControl", "TextControl", "ImageControl"):
                continue
            if n.w <= 0 or n.h <= 0:
                continue
            try:
                ht = n.ctrl.HelpText
            except Exception:
                ht = ""
            if ht or n.type == "ButtonControl":
                shown += 1
                if shown <= 80:
                    log(f"  {n.type} #{n.id} .{n.cls} name={n.name!r} help={ht!r} rect={(n.l, n.t, n.r, n.b)} kids={[(k.type, k.name[:12]) for k in n.kids][:4]}")
    log("主畫面元素數", n_all)
    try:
        main_tips(rhs, root)
    except Exception:
        log(traceback.format_exc())
    ensure_web_fg()


def find_ui(start, want, skip=None, limit=1500):
    """照樹狀一層一層找第一個符合的元素。skip(c) = 不用往裡面找的；最多看 limit 個（整棵樹很大，不能全部走）"""
    q, n = [start], 0
    while q and n < limit:
        c = q.pop(0)
        n += 1
        try:
            if want(c):
                return c
            if skip is not None and skip(c):
                continue
            q.extend(c.GetChildren())
        except Exception:
            pass
    return None


def main_tips(rhs, root):
    """主畫面的提示文字：有圖示的按鈕，滑鼠停在圖示的線條上也要跳得出來（美容美髮裡的「角色動作」）"""
    if not rhs or root is None:
        return
    t_start = time.time()
    tile = next((n for n in root.walk() if n.type == "ButtonControl" and n.has("tile") and n.name.startswith("美容美髮") and n.w > 0), None)
    back = next((n for n in root.walk() if n.type == "ButtonControl" and n.has("back") and n.w > 0), None)
    if tile is None:
        log("（主畫面）找不到美容美髮")
        return
    pane = auto.ControlFromHandle(rhs[0])
    docs = [c for c in pane.GetChildren() if c.ControlTypeName == "DocumentControl"]
    if not docs:
        log("（主畫面）讀不到內容")
        return
    doc = docs[0]
    cls_of = lambda c: (c.ClassName or "").split()
    # 每個功能的頁面（class 是 view）很大：只往要看的那一頁裡面找
    other_view = lambda c: "view" in cls_of(c) and c.AutomationId != "view-beauty"
    # 測試版開起來會跳「發現新版本」：按「稍後」關掉，不然點不到後面的東西
    later = find_ui(doc, lambda c: c.ControlTypeName == "ButtonControl" and c.Name.strip() == "稍後", skip=lambda c: "view" in cls_of(c), limit=400)
    if later is not None:
        r = later.BoundingRectangle
        log("  主畫面有「發現新版本」的視窗：按稍後", (r.left, r.top, r.right, r.bottom))
        click((r.left + r.right) / 2, (r.top + r.bottom) / 2, wait=1.0)
    click(*tile.center, wait=3.0)
    shot("main_beauty")
    sec = find_ui(doc, lambda c: c.AutomationId == "view-beauty", skip=other_view, limit=400)
    sec_cls = sec.ClassName if sec is not None else "?"
    act = find_ui(sec, lambda c: c.AutomationId == "bAct", limit=1200) if sec is not None and "active" in sec_cls.split() else None
    log(f"  美容美髮頁面={sec_cls!r} 找到角色動作按鈕={act is not None} 花了 {time.time() - t_start:.1f} 秒")
    if act is not None:
        r = act.BoundingRectangle
        kids = act.GetChildren()
        imgs = [k for k in kids if k.ControlTypeName == "ImageControl"]
        log(f"  角色動作按鈕 rect={(r.left, r.top, r.right, r.bottom)} help={act.HelpText!r} 裡面={[(k.ControlTypeName, k.Name) for k in kids]}")
        if r.width() > 0 and imgs:
            ir = imgs[0].BoundingRectangle
            res = {}
            # 箭頭圖示（24×24 的格子裡從 (7,10) 畫到 (12,15) 再到 (17,10)）：兩邊的線上、尖端、空白處
            for tag, vx, vy in (("左邊的線", 9.5, 12.5), ("右邊的線", 14.5, 12.5), ("尖端", 12, 15), ("圖示空白處", 12, 8)):
                x = ir.left + ir.width() * vx / 24
                y = ir.top + ir.height() * vy / 24
                move(r.left - 140, r.top - 90)
                time.sleep(0.7)
                hover(x, y, settle=0.0)
                n, t0 = 0, time.time()
                while time.time() - t0 < 2.4 and not n:
                    time.sleep(0.25)
                    n = len(tips_now())
                res[tag] = bool(n)
                if tag == "左邊的線":
                    shot("main_tip")
            move(r.left - 140, r.top - 90)
            time.sleep(0.5)
            check("主畫面有圖示的按鈕（美容美髮的「角色動作」）：滑鼠停在圖示的線條上，提示文字也會跳出來", all(res.values()), f"{res} 圖示={(ir.left, ir.top, ir.right, ir.bottom)}")
        else:
            log("（主畫面）角色動作按鈕沒有顯示出來，這一項不檢查")
    else:
        log("（主畫面）找不到角色動作按鈕，這一項不檢查")
    # 回到首頁
    if back is not None:
        click(*back.center, wait=1.2)


def s_tipdiag():
    """（查問題用）哪些地方停上去會跳提示文字"""
    home()

    def gui_info(tag):
        try:
            bh = bar_hwnd()
            par = user32.GetParent(bh)
            tid = user32.GetWindowThreadProcessId(par or bh, None)
            gi = GUITHREADINFO()
            gi.cbSize = ctypes.sizeof(GUITHREADINFO)
            ok = user32.GetGUIThreadInfo(tid, ctypes.byref(gi))
            d = lambda h: (cls(h), wrect(h)) if h else None
            log(f"  [{tag}] thread={tid} ok={ok} flags={gi.flags:#x} active={d(gi.hwndActive)} focus={d(gi.hwndFocus)} capture={d(gi.hwndCapture)} fg={cls(user32.GetForegroundWindow())!r}")
        except Exception as e:
            log(f"  [{tag}] gui_info 失敗 {e}")

    def all_tops(tag):
        res = []
        for h in top_windows():
            if proc_name(pid_of(h)).lower() != "msedgewebview2.exe":
                continue
            ex = user32.GetWindowLongW(h, -20) & 0xffffffff
            res.append((cls(h), bool(user32.IsWindowVisible(h)), wrect(h), hex(ex), title(h)[:16]))
        log(f"  [{tag}] webview 的頂層視窗：{res}")

    def watch_at(tag, x, y, secs=2.4, info=False):
        hover(x, y, settle=0.0)
        seen = []
        t0 = time.time()
        while time.time() - t0 < secs:
            seen.append(len(tips_now()))
            time.sleep(0.3)
        log(f"  [{tag}] 停在 {(round(x), round(y))}：{''.join(str(n) for n in seen)}  {tips_now()}")
        if info:
            gui_info(tag)
            all_tops(tag)
        move(*PG(620, 330))
        time.sleep(0.6)

    def bar_c(el):
        p = bar_pt(el)
        return C(*p) if p else None

    def shows(x, y):
        """滑鼠移過去停住，最後還左右各動 1px（一般人的手就是這樣），看提示文字有沒有出來"""
        hover(x, y, settle=0.0)
        for dx in (1, 0, -1, 0, 1, 0):
            move(x + dx, y)
            time.sleep(0.03)
        t0 = time.time()
        n = 0
        while time.time() - t0 < 2.0 and not n:
            time.sleep(0.25)
            n = len(tips_now())
        move(*PG(620, 330))
        time.sleep(0.6)
        return bool(n)

    # 工具列：圓鈕的不同位置
    c = bar_c(SITE)
    if c:
        r1 = {}
        for dx, dy in ((0, 0), (-6, -6), (6, 6), (-8, 0), (8, 0), (0, -9), (0, 9), (3, 0), (-1, 2), (1, -4)):
            r1[(dx, dy)] = shows(c[0] + dx * scale, c[1] + dy * scale)
        check("網址列左邊的圓鈕：不管停在哪裡（正中間的線條上也一樣）都會跳出提示文字", all(r1.values()), str(r1))
    # 工具列：每個按鈕正中間
    r2 = {}
    for el, tag in ((BACK, "上一頁"), (FWD, "下一頁"), (RELOAD, "重新載入"), (E("home"), "首頁"), (STAR, "星星"), (EXT_BTN, "擴充功能"), (PROFILE, "頭像"), (MENU_BTN, "⋮"),
                    (TABSEARCH, "搜尋分頁"), (NEWTAB, "新分頁"), (BTN_MIN, "最小化"), (BTN_MAX, "最大化"), (BTN_CLOSE, "關閉"), (BM("Example"), "書籤"), (OTHER, "所有書籤"), (TAB(1), "分頁"), (TABX(1), "分頁的 ×")):
        c2 = bar_c(el)
        if c2:
            r2[tag] = shows(c2[0], c2[1])
        else:
            log(f"  [{tag}] 找不到")
    check("工具列每個按鈕、分頁、書籤：停在正中間都會跳出提示文字", bool(r2) and all(r2.values()), str(r2))
    c2 = bar_c(ADDR)
    if c2:
        check("網址列的文字上：沒有提示文字", not shows(c2[0], c2[1]))
    # 換到別種網頁再試圓鈕
    for part in ("Example", "短影片"):
        if select_title(part, 1.5):
            log("  目前分頁：", active_tab(), bar_state())
            tip_diag(SITE, "site@" + part)
            c3 = bar_c(SITE)
            if c3:
                watch_at("site@" + part, c3[0], c3[1])
                r = rect_of(SITE)
                if r and r[2] > 40:
                    # 有文字的圓鈕：停在文字上
                    watch_at("site_label@" + part, *C(r[0] + r[2] - 14, r[1] + r[3] / 2))
    home()
    # 網頁裡照著做一個一樣的
    go(f"http://127.0.0.1:{PORT}/tip.html", wait=3.0)
    shot("tipdiag_page")
    root = page_tree()
    res = {}

    def at(i):
        n = find_id(root, i)
        return ((n.l + n.r) / 2, (n.t + n.b) / 2) if n is not None else None

    def probe(i, dx=0):
        c4 = at(i)
        if not c4:
            log(f"  [page {i}] 找不到")
            return None
        hover(c4[0] + dx, c4[1], settle=0.0)
        seen = []
        t0 = time.time()
        while time.time() - t0 < 2.1:
            seen.append(len(tips_now()))
            time.sleep(0.3)
        ok_ = bool(seen[-1])
        log(f"  [page {i}{'+%d' % dx if dx else ''}] {''.join(str(n) for n in seen)}")
        move(*PG(620, 330))
        time.sleep(0.6)
        return ok_

    for i in ("a1", "a2", "a3", "a4", "a5", "a6"):
        res[i] = probe(i)
    for i in ("b1", "b2", "b3", "b4", "b5"):
        res[i] = probe(i)
    for i in ("a1", "a4", "a5"):
        c6 = at(i)
        if c6:
            hover(c6[0], c6[1], settle=1.6)
            n0 = len(tips_now())
            sent = post_move_here()
            time.sleep(1.6)
            n1 = len(tips_now())
            log(f"  [page {i} 補一個滑鼠訊息] 補之前={n0} 有送出={sent} 補之後={n1}")
            move(*PG(620, 330))
            time.sleep(0.6)
    # 只是記錄 WebView2 本身的行為（不是我們的程式）：滑鼠底下的元素一換，提示文字就不出現
    log("  WebView2 本身：", res)
    check("（對照）網頁裡照做的按鈕：圖示不單獨接滑鼠的那兩個，停在線條上提示文字會出現", bool(res.get("a2")) and bool(res.get("a6")), str(res))
    c5 = at("go")
    if c5:
        click(c5[0], c5[1], wait=0.8)
        move(*PG(620, 330))
        time.sleep(0.6)
        res["c1"] = probe("c1")
        res["c2"] = probe("c2")
        log("  一直換圖示：", {k: res[k] for k in ("c1", "c2")})
    go(f"http://127.0.0.1:{PORT}/test.html", wait=2.5)
    # 回到測試頁之後圓鈕再試一次
    c = bar_c(SITE)
    if c:
        watch_at("site_end", c[0], c[1])


def s_bubbles():
    home()
    for el, key_, name, side in ((PROFILE, "profile", "頭像（Chrome 使用者）", "right"), (SITE, "site", "網址列左邊的圓鈕（網站資訊）", "left"), (TABSEARCH, "tabs", "左上角的搜尋分頁", "left"), (EXT_BTN, "ext", "擴充功能（拼圖）", "right")):
        ms = open_menu(el, name)
        shot("bubble_" + key_)
        if ms:
            log(name, menu_titles(ms))
            ok, d = below(rect_of(el), ms[0][0], side=side)
            check(f"{name}：開在按鈕下面", ok, d)
            n = el(bar_tree())
            check(f"{name}：選單開著時按鈕是按下去的樣子", n is not None and n.has("open"), n.cls if n is not None else "找不到按鈕")
        close_menus()
        check(f"{name}：點外面關掉", not menu_wins())
        n = el(bar_tree())
        check(f"{name}：選單關了，按鈕恢復原樣", n is not None and not n.has("open"), n.cls if n is not None else "找不到按鈕")
    # 提示文字（滑鼠停在按鈕上跳出來的小字）：打開選單後不能蓋在選單上
    def tips():
        """畫面上看得到的提示文字視窗（瀏覽器核心自己開的小視窗：沒有標題、很矮）"""
        res = []
        for h in top_windows():
            if not user32.IsWindowVisible(h) or title(h) == MENU_TITLE:
                continue
            c = cls(h)
            r = wrect(h)
            w_, h_ = r[2] - r[0], r[3] - r[1]
            if w_ <= 0 or h_ <= 0:
                continue
            owner = proc_name(pid_of(h)).lower()
            if "tooltip" in c.lower() or (owner in ("msedgewebview2.exe", os.path.basename(EXE).lower()) and c.startswith("Chrome_WidgetWin") and h_ <= 90 and w_ <= 600):
                res.append((c, owner, title(h), r))
        return res

    for el, key_, name in ((SITE, "site", "網址列左邊的圓鈕"), (MENU_BTN, "menu", "⋮"), (TABSEARCH, "tabs", "搜尋分頁")):
        p = bar_pt(el)
        if not p:
            continue
        # 滑鼠先停著，等提示文字出來再按（慢慢按）
        hover(*C(*p), settle=1.8)
        t1 = tips()
        if key_ == "site":
            shot("tooltip_before_click")
        log(f"（{name}）滑鼠停著時的提示文字視窗：", t1)
        click(*C(*p), wait=1.8)
        t2 = tips()
        shot("tooltip_after_click_" + key_)
        check(f"{name}：滑鼠停著時會跳出提示文字（確認抓得到）", bool(t1), f"{t1}")
        check(f"{name}：提示文字出來之後才按，選單打開時提示文字收起來", bool(menu_wins()) and not t2, f"按之前={t1} 按之後={t2}")
        close_menus()
        move(*PG(620, 330))
        time.sleep(0.6)
        # 滑鼠一到就馬上按（快快按）：選單打開後過一下，提示文字也不能跳出來
        click(*C(*p), wait=2.2)
        t3 = tips()
        if key_ == "tabs":
            shot("tooltip_fast_click")
        check(f"{name}：一移過去就按，選單打開後提示文字不會跳出來", bool(menu_wins()) and not t3, f"{t3}")
        # 選單開著時滑鼠移開再移回按鈕：提示文字也不會跳出來蓋住選單
        move(*PG(620, 330))
        time.sleep(0.4)
        if menu_wins():
            hover(*C(*p), settle=1.8)
            t4 = tips()
            if key_ == "site":
                shot("tooltip_back_while_open")
            check(f"{name}：選單開著時滑鼠移開再移回按鈕，提示文字不會跳出來", bool(menu_wins()) and not t4, f"{t4}")
        close_menus()
        move(*PG(620, 330))
        time.sleep(0.6)
        # 選單關了、滑鼠移開之後再停上去：提示文字照常出現
        hover(*C(*p), settle=1.8)
        t5 = tips()
        check(f"{name}：選單關了之後再停上去，提示文字照常出現", bool(t5), f"{t5}")
        move(*PG(620, 330))
        time.sleep(0.6)
    # 搜尋分頁：點清單裡的分頁會切過去
    ms = open_menu(TABSEARCH, "搜尋分頁")
    if ms:
        menu_click("短影片測試", exact=False, wait=1.5)
        check("搜尋分頁 → 點清單裡的分頁：切過去", "短影片" in title(web), title(web))
    close_menus()
    home()


def s_download():
    home()
    dl = os.path.join(os.path.expanduser("~"), "Downloads", "ui-check.bin")
    try:
        os.remove(dl)
    except Exception:
        pass
    go(f"127.0.0.1:{PORT}/dl.bin", wait=4.0)
    st = bar_state()
    shot("download")
    check("下載檔案：存到「下載」資料夾", os.path.exists(dl) and os.path.getsize(dl) == 300000, f"{dl} exists={os.path.exists(dl)}")
    check("下載檔案：工具列出現下載按鈕、分頁留在原本的網頁", st.get("dl") is True and "測試頁" in title(web), f"{st} {title(web)}")
    log("下載後的視窗", [(title(h), wrect(h)) for h in app_windows() if user32.IsWindowVisible(h)])
    click_blank(0.8)
    keep = tab_ids()
    click_blank(0.5)
    chord([VK_CTRL], 0x4A, wait=2.5)
    txt = page_text()
    shot("downloads_page")
    if txt:
        check("chrome://downloads：列出剛下載的檔案", "ui-check.bin" in txt, txt[:300].replace("\n", " "))
    else:
        log("（讀不到下載頁面的內容）")
    restore_tabs(keep)
    home()


def s_incognito():
    home()
    click_blank(0.5)
    before = set(app_windows())
    chord([VK_CTRL, VK_SHIFT], 0x4E, wait=4.0)  # Ctrl+Shift+N
    new = [h for h in app_windows() if h not in before and user32.IsWindowVisible(h) and title(h) not in KNOWN]
    shot("incognito")
    check("Ctrl+Shift+N：開無痕式視窗", len(new) == 1 and "無痕" in title(new[0]), f"{[(title(h), wrect(h)) for h in new]}")
    if not new:
        return
    inc = new[0]
    r = wrect(inc)
    iroot = bar_tree(inc)
    chip = find_id(iroot, "inco")
    check("無痕式視窗：分頁列上有「無痕模式」的標示、沒有頭像", chip is not None and chip.w > 0 and find_id(iroot, "profile") is None, f"{chip} {chip.texts() if chip else ''}")
    # 在無痕式視窗逛一個網頁，之後歷史記錄裡不能有
    click(r[0] + 400 * scale, r[1] + 71 * scale, wait=0.7)
    type_text(f"127.0.0.1:{PORT}/snap.html?incognito")
    key(VK_ENTER)
    time.sleep(2.5)
    check("無痕式視窗：可以逛網頁", "短影片" in title(inc), title(inc))
    shot("incognito_page")
    time.sleep(1.5)
    try:
        saved = json.load(open(os.path.join(data_dir, "web.json"), encoding="utf-8"))
        check("無痕式視窗：不會存進設定檔", len(saved.get("wins", [])) == 1, f"{[w.get('id') for w in saved.get('wins', [])]}")
    except Exception as e:
        log("讀 web.json 失敗", e)
    click(r[2] - 23 * scale, r[1] + 16 * scale, wait=1.5)
    check("無痕式視窗：按 × 關掉", not user32.IsWindow(inc) or not user32.IsWindowVisible(inc))
    time.sleep(1.5)
    try:
        hist = open(os.path.join(data_dir, "web-history.json"), encoding="utf-8").read()
        check("無痕式視窗逛的網頁：歷史記錄裡沒有", "?incognito" not in hist, "")
    except Exception as e:
        log("讀 web-history.json 失敗（可能還沒存）", e)
    click_bar(TAB(0), wait=1.0)


def s_new_window():
    home()
    click_blank(0.5)
    before = set(app_windows())
    chord([VK_CTRL], 0x4E, wait=4.0)  # Ctrl+N
    new = [h for h in app_windows() if h not in before and user32.IsWindowVisible(h) and title(h) not in KNOWN]
    shot("new_window")
    check("Ctrl+N：開新視窗（新分頁）", len(new) == 1 and title(new[0]) == "新分頁", f"{[(title(h), wrect(h)) for h in new]}")
    if new:
        r, r0 = wrect(new[0]), wrect(web)
        check("新視窗：跟原本的一樣大、位置錯開一點", (r[2] - r[0], r[3] - r[1]) == (r0[2] - r0[0], r0[3] - r0[1]) and (r[0], r[1]) != (r0[0], r0[1]), f"{r} {r0}")
        w2 = new[0]
        a2, a1 = find_id(bar_tree(w2), "addr"), find_id(bar_tree(), "addr")
        check("新視窗：游標在它自己的網址列，原本那個視窗的網址列沒有被動到", bool(a2 and a2.foc) and not (a1 and a1.foc) and fg() == w2, f"新={a2 and a2.foc} 舊={a1 and a1.foc} fg={fg()} w2={w2}")
        # 在新視窗按 F10（打開 ⋮ 選單）：只有新視窗開選單，而且貼在新視窗的 ⋮ 下面
        key(0x79)
        time.sleep(1.2)
        mw = menu_wins()
        b2 = find_id(bar_tree(w2), "menu")
        shot("two_windows_menu")
        ok = len(mw) == 1 and b2 is not None and abs(wrect(mw[0])[2] - b2.r) <= 12 * scale and abs(wrect(mw[0])[1] - b2.b) <= 12 * scale
        check("兩個視窗時按 F10：選單開在按的那個視窗", ok, f"選單={[wrect(h) for h in mw]} 新視窗的⋮={b2}")
        key(VK_ESC)
        time.sleep(0.6)
        if menu_wins():
            click(r[0] + 500 * scale, r[1] + 400 * scale, wait=0.8)
        check("按 Esc：選單關掉", not menu_wins(), f"{len(menu_wins())}")
        # Ctrl+L：只有新視窗的網址列有游標
        click(r[0] + 500 * scale, r[1] + 400 * scale, wait=0.6)
        chord([VK_CTRL], 0x4C, wait=0.8)
        a2, a1 = find_id(bar_tree(w2), "addr"), find_id(bar_tree(), "addr")
        check("兩個視窗時按 Ctrl+L：只有按的那個視窗的網址列有游標", bool(a2 and a2.foc) and not (a1 and a1.foc) and fg() == w2, f"新={a2 and a2.foc} 舊={a1 and a1.foc} fg={fg()}")
        key(VK_ESC)
        time.sleep(0.3)
        key(VK_ESC)
        time.sleep(0.3)
        click(r[2] - 23 * scale, r[1] + 16 * scale, wait=1.5)
        check("新視窗：按 × 關掉", not user32.IsWindow(new[0]) or not user32.IsWindowVisible(new[0]))
    click_bar(TAB(0), wait=1.0)


# =====================================================================================
# 邊玩遊戲邊用（鍵盤留在遊戲，打字時才交給瀏覽器）
# =====================================================================================
def s_game():
    home(1.5)
    # 遊戲上沒有被 web 視窗蓋住、也不在工作列上的點
    g, r = wrect(game), wrect(web)
    sh = user32.GetSystemMetrics(1) - 60
    cands = [(g[0] + 40, g[1] + 60), (g[2] - 40, g[1] + 60), (g[0] + 40, min(g[3], sh) - 30), (g[2] - 40, min(g[3], sh) - 30)]
    gx, gy = next(((x, y) for x, y in cands if not (r[0] <= x < r[2] and r[1] <= y < r[3])), cands[0])
    log("遊戲要點的位置", (gx, gy), "game", g, "web", r)
    click(gx, gy, wait=1.0)
    check("點遊戲：遊戲在前景", fg() == game, f"fg={fg()} game={game}")
    check("遊戲在前景：web 視窗在遊戲上面", above(web, game), f"z={zorder()[:6]}")
    click(*PG(200, 320), wait=0.8)
    check("點網頁空白處：鍵盤還在遊戲", fg() == game, f"fg={fg()}")
    check("點網頁空白處：web 視窗還在遊戲上面", above(web, game))
    n0 = tab_count()
    click_bar(NEWTAB, wait=1.2)
    n1 = tab_count()
    check("遊戲在前景時點 web 的按鈕：第一下就有作用", n1 == n0 + 1, f"{n0} → {n1}")
    st = bar_state()
    check("遊戲在前景時按 +：新分頁可以直接打網址（鍵盤先借給網址列）", fg() == web and st.get("focus") == "addr", f"fg={fg()} {st}")
    move(gx, gy)
    time.sleep(1.3)
    check("按 + 之後滑鼠移出 web：鍵盤還給遊戲", fg() == game, f"fg={fg()}")
    # 中鍵關掉剛開的分頁，回到測試頁
    if n1 == n0 + 1:
        click_bar(TAB(n1 - 1), button="middle", wait=1.0)
        home(1.0)
        check("中鍵關掉分頁", tab_count() == n0, f"{tab_count()}")
    click(gx, gy, wait=1.0)
    check("再點遊戲：遊戲在前景", fg() == game, f"fg={fg()}")
    key(0x41)
    time.sleep(0.4)
    check("按 A：遊戲收到", "key:a" in game_keys(), game_keys()[-80:])
    click(*PG(200, 40), wait=1.0)
    check("點網頁輸入框：鍵盤交給 web", fg() == web, f"fg={fg()}")
    type_text("hello")
    time.sleep(0.5)
    check("在輸入框打字：網頁收到", "input:hello" in page_log(), page_log()[-200:])
    key(VK_ENTER)
    time.sleep(1.0)
    check("按 Enter：鍵盤還給遊戲", fg() == game, f"fg={fg()}")
    key(0x42)
    time.sleep(0.4)
    check("按 B：遊戲收到", "key:b" in game_keys())
    click(*PG(200, 40), wait=1.0)
    type_text("xy")
    move(gx, gy)
    time.sleep(1.2)
    check("打字中滑鼠移出 web：鍵盤還給遊戲", fg() == game, f"fg={fg()}")
    click(*PG(182, 120), wait=1.0)
    check("點內嵌框裡的輸入框：鍵盤交給 web", fg() == web, f"fg={fg()}")
    type_text("fr")
    time.sleep(0.5)
    check("內嵌框打字：收到", "[frame] input:fr" in page_log())
    move(gx, gy)
    time.sleep(1.2)
    # 打字中自己點遊戲（不是打完）：之後從遊戲再點輸入框，還是要把鍵盤交給 web
    click(*PG(200, 40), wait=1.0)
    check("再點網頁輸入框：鍵盤交給 web", fg() == web, f"fg={fg()}")
    click(gx, gy, wait=1.0)
    check("打字中點遊戲：遊戲在前景", fg() == game, f"fg={fg()}")
    click(*PG(200, 40), wait=1.0)
    check("點遊戲後再點輸入框：鍵盤又交給 web", fg() == web, f"fg={fg()}")
    type_text("w")
    time.sleep(0.5)
    check("再打字：網頁收到", "input:helloxyw" in page_log(), page_log()[-160:])
    key(VK_ENTER)
    time.sleep(1.0)
    check("再按 Enter：鍵盤還給遊戲", fg() == game, f"fg={fg()}")
    move(gx, gy)
    time.sleep(0.8)

    # ---- 網址列 ----
    keys0 = game_keys()
    click_bar(ADDR, wait=1.0)
    st = bar_state()
    check("遊戲在前景時點網址列：鍵盤交給 web、游標在網址列", fg() == web and st.get("focus") == "addr", f"fg={fg()} {st}")
    type_text(f"127.0.0.1:{PORT}/snap.html?game")
    time.sleep(0.8)
    shot("game_omnibox")
    st = bar_state()
    check("在網址列打字：字進網址列、遊戲沒收到", "snap.html?game" in (st.get("addr") or "") and game_keys() == keys0, f"{st.get('addr')!r} keys={game_keys()[len(keys0):]!r}")
    t0 = time.time()
    key(VK_ENTER)
    time.sleep(2.5)
    check("網址列按 Enter：前往網頁", "短影片" in title(web) and any("[snap] load" in l for l in page_log_since(t0)), f"{title(web)}")
    check("網址列按 Enter：鍵盤還給遊戲", fg() == game, f"fg={fg()}")
    key(0x43)
    time.sleep(0.4)
    check("按 C：遊戲收到", "key:c" in game_keys(), game_keys()[-60:])
    # 點網址列之後不打字、按 Esc：鍵盤也要還給遊戲
    click_bar(ADDR, wait=1.0)
    check("再點網址列：鍵盤交給 web", fg() == web, f"fg={fg()}")
    key(VK_ESC)
    time.sleep(0.5)
    key(VK_ESC)
    time.sleep(1.0)
    check("網址列按 Esc：鍵盤還給遊戲", fg() == game, f"fg={fg()}")
    # 點網址列之後改點網頁：鍵盤還給遊戲
    click_bar(ADDR, wait=1.0)
    click_blank(1.2)
    check("點網址列後改點網頁：鍵盤還給遊戲", fg() == game, f"fg={fg()}")
    # 上一頁（滑鼠）
    click_bar(BACK, wait=2.0)
    check("遊戲在前景時按上一頁：回到測試頁、鍵盤還在遊戲", "測試頁" in title(web) and fg() == game, f"{title(web)} fg={fg()}")

    # ---- 新分頁頁面中間的搜尋框 ----
    keep = tab_ids()
    click_bar(NEWTAB, wait=1.5)
    move(gx, gy)
    time.sleep(1.3)
    box = next((n for n in find_all(page_tree(), lambda n: n.type == "EditControl")), None)
    if box is not None and fg() == game:
        click(*box.center, wait=1.0)
        check("遊戲在前景時點新分頁中間的搜尋框：鍵盤交給 web", fg() == web, f"fg={fg()}")
        keys1 = game_keys()
        type_text(f"127.0.0.1:{PORT}/snap.html?ntpgame")
        key(VK_ENTER)
        time.sleep(2.5)
        check("在搜尋框打網址按 Enter：前往、遊戲沒收到那些字", "短影片" in title(web) and game_keys() == keys1, f"{title(web)} keys={game_keys()[len(keys1):]!r}")
        check("搜尋框按 Enter 之後：鍵盤還給遊戲", fg() == game, f"fg={fg()}")
    else:
        check("遊戲在前景時開新分頁：找得到搜尋框、鍵盤在遊戲", False, f"box={box} fg={fg()}")
    restore_tabs(keep)
    home()
    click(gx, gy, wait=0.8)

    # ---- 選單 ----
    ms = open_menu(MENU_BTN, "遊戲在前景時按 ⋮")
    shot("game_menu")
    check("遊戲在前景時開選單：鍵盤還在遊戲", fg() == game, f"fg={fg()}")
    if ms:
        check("遊戲在前景時：選單在遊戲上面", above(ms[0][0], game), f"z={zorder()[:6]}")
        menu_hover("更多工具")
        check("遊戲在前景時滑到有下一層的項目：打開下一層", bool(wait_for(lambda: len(menu_wins()) >= 2, 3, 0.2)), f"{len(menu_wins())}")
        n0, keep = tab_count(), tab_ids()
        menu_click("新分頁", wait=1.5)
        check("遊戲在前景時 ⋮ → 新分頁：有開、選單關掉、可以直接打網址", tab_count() == n0 + 1 and not menu_wins() and fg() == web, f"{tab_count()} fg={fg()}")
        move(gx, gy)
        time.sleep(1.3)
        check("⋮ → 新分頁之後滑鼠移出 web：鍵盤還給遊戲", fg() == game, f"fg={fg()}")
        key(0x44)
        time.sleep(0.4)
        check("按 D：遊戲收到", "key:d" in game_keys(), game_keys()[-60:])
        restore_tabs(keep)
        home()
    close_menus()
    # 右鍵選單
    t0 = time.time()
    ms = ctx_at(*PG(300, 330))
    shot("game_context")
    check("遊戲在前景時網頁按右鍵：出現選單、鍵盤還在遊戲", bool(ms) and fg() == game, f"fg={fg()}")
    if ms:
        menu_click("重新載入", exact=False, wait=2.0)
        check("遊戲在前景時右鍵 → 重新載入：有作用、鍵盤還在遊戲", any("[test] load" in l for l in page_log_since(t0)) and fg() == game, f"fg={fg()}")
    close_menus()
    # 書籤資料夾
    ms = open_menu(BM("資料夾"), "遊戲在前景時點書籤資料夾")
    check("遊戲在前景時開書籤資料夾：鍵盤還在遊戲", fg() == game, f"fg={fg()}")
    click_blank(1.0)
    check("遊戲在前景時點網頁關選單：關掉、鍵盤還在遊戲", not menu_wins() and fg() == game, f"fg={fg()}")
    # 點遊戲也會把選單關掉
    ms = open_menu(MENU_BTN, "再開一次選單")
    click(gx, gy, wait=1.0)
    check("選單開著時點遊戲：選單關掉、遊戲在前景", not menu_wins() and fg() == game, f"{len(menu_wins())} fg={fg()}")

    # ---- 登入小視窗 ----
    before = set(app_windows())
    click(*PG(100, 200), wait=2.5)
    new = [h for h in app_windows() if h not in before and user32.IsWindowVisible(h)]
    shot("popup")
    check("登入小視窗：開成另一個視窗", len(new) >= 1, f"{[(title(h), wrect(h)) for h in new]}")
    if new:
        type_text("pp")
        time.sleep(0.5)
        check("登入小視窗：可以打字", "[popup] input:pp" in page_log(), page_log()[-200:])
        user32.PostMessageW(new[0], 0x0010, None, None)
        time.sleep(1)


def s_notitle():
    """一開始沒有 <title>、載入時一直往 <head> 加東西的網頁（X 的舊版網頁就是這樣）：
    程式注入的「有沒有在打字」小腳本不能讓網頁卡死；之後才出現的標題、打字偵測都照常"""
    import re
    home()
    ensure_web_fg()
    click_blank(0.5)
    keep = tab_ids()
    t0 = time.time()
    chord([VK_CTRL], 0x54, wait=1.5)  # Ctrl+T
    type_text("http://127.0.0.1:8765/notitle.html")
    key(0x2E)
    key(VK_ENTER)
    done = wait_for(lambda: next((l for l in page_log_since(t0) if "[notitle] done" in l), None), 15)
    lines = [l.split(" ", 1)[1] for l in page_log_since(t0) if "[notitle]" in l]
    log("  網頁回報：", lines)
    check("沒有標題又一直往 <head> 加東西的網頁：沒有卡死（40 次都做完）", bool(done), " | ".join(lines[-3:])[:300])
    m = re.search(r"max=(\d+)ms", done or "")
    check("沒有標題的網頁：每一步都不慢", bool(m) and int(m.group(1)) < 1000, (done or "")[-70:])
    shot("notitle_loaded")
    ts = wait_for(lambda: [t["title"] for t in tabs_info() if "晚到的標題" in t["title"]], 5)
    check("沒有標題的網頁：之後才設的標題出現在分頁上", bool(ts), f"{[t['title'] for t in tabs_info()]}")
    txt = page_text()
    check("沒有標題的網頁：畫面有更新到最後", "done" in txt, " | ".join(x.strip() for x in txt.splitlines() if x.strip())[:200])
    # 從遊戲點這一頁的輸入框：鍵盤要交給 web（靠標題上的記號，而這一頁的標題是晚到的）
    g, r = wrect(game), wrect(web)
    sh = user32.GetSystemMetrics(1) - 60
    cands = [(g[0] + 40, g[1] + 60), (g[2] - 40, g[1] + 60), (g[0] + 40, min(g[3], sh) - 30), (g[2] - 40, min(g[3], sh) - 30)]
    gx, gy = next(((x, y) for x, y in cands if not (r[0] <= x < r[2] and r[1] <= y < r[3])), cands[0])
    click(gx, gy, wait=1.0)
    check("標題晚到的網頁：點遊戲，遊戲在前景", fg() == game, f"fg={fg()} game={game}")
    click(*PG(200, 40), wait=1.0)
    check("標題晚到的網頁：點輸入框，鍵盤交給 web", fg() == web, f"fg={fg()}")
    type_text("nt")
    time.sleep(0.5)
    got = [l.split(" ", 1)[1] for l in page_log_since(t0) if "[notitle] input" in l]
    check("標題晚到的網頁：打字網頁收到", "[notitle] input:nt" in got, f"{got}")
    key(VK_ENTER)
    time.sleep(1.0)
    check("標題晚到的網頁：按 Enter 鍵盤還給遊戲", fg() == game, f"fg={fg()}")
    move(gx, gy)
    time.sleep(0.8)
    ensure_web_fg()
    restore_tabs(keep)
    home()


class CDPB:
    """（查問題用）接到整個瀏覽器的偵錯埠：可以看所有的分頁、擴充功能的背景程式"""

    def __init__(self, ws_url):
        self.ws = websocket.create_connection(ws_url, timeout=8, suppress_origin=True)
        self.n = 0
        self.events = []
        self.sessions = {}

    def _recv(self):
        try:
            self.ws.settimeout(0.5)
            return json.loads(self.ws.recv())
        except websocket.WebSocketTimeoutException:
            return None

    def call(self, method, params=None, session=None, wait=12):
        self.n += 1
        i = self.n
        msg = {"id": i, "method": method, "params": params or {}}
        if session:
            msg["sessionId"] = session
        self.ws.send(json.dumps(msg))
        end = time.time() + wait
        while time.time() < end:
            m = self._recv()
            if m is None:
                continue
            if m.get("id") == i:
                return m
            self.events.append((time.time(), m))
        return None

    def pump(self, secs):
        end = time.time() + secs
        while time.time() < end:
            m = self._recv()
            if m is not None:
                self.events.append((time.time(), m))

    def targets(self):
        r = self.call("Target.getTargets")
        try:
            return r["result"]["targetInfos"]
        except Exception:
            return []

    def attach(self, target_id):
        if target_id in self.sessions:
            return self.sessions[target_id]
        r = self.call("Target.attachToTarget", {"targetId": target_id, "flatten": True})
        try:
            sid = r["result"]["sessionId"]
        except Exception:
            log("  接不上", target_id, r)
            return None
        self.sessions[target_id] = sid
        return sid

    def ev(self, session, expr, wait=15):
        """執行一段程式（可以是 Promise），結果轉成 Python 的值；出錯回傳 'ERR …'"""
        code = "(async () => { try { const v = await (" + expr + "); return JSON.stringify(v === undefined ? '__undefined__' : v); } catch (e) { return JSON.stringify('ERR ' + ((e && (e.message || String(e))) || e)); } })()"
        r = self.call("Runtime.evaluate", {"expression": code, "returnByValue": True, "awaitPromise": True}, session, wait)
        try:
            res = r["result"]
            if "exceptionDetails" in res:
                d = res["exceptionDetails"]
                return "EXC " + str(d.get("text")) + " " + str((d.get("exception") or {}).get("description", ""))[:300]
            v = res["result"].get("value")
            return json.loads(v) if isinstance(v, str) else res["result"]
        except Exception:
            return "NORESULT " + str(r)[:300]

    def drain(self, session=None):
        """到目前為止收到的錯誤訊息（console.error、例外）"""
        out = []
        for t, m in self.events:
            if session and m.get("sessionId") != session:
                continue
            me, pr = m.get("method"), m.get("params", {})
            if me == "Runtime.exceptionThrown":
                d = pr.get("exceptionDetails", {})
                out.append(f"exception {d.get('text')} {(d.get('exception') or {}).get('description', '')[:300]} @{str(d.get('url'))[-60:]}:{d.get('lineNumber')}")
            elif me == "Runtime.consoleAPICalled" and pr.get("type") in ("error", "warning", "assert"):
                out.append(f"console.{pr.get('type')} " + " ".join(str(a.get("value", a.get("description", "")))[:240] for a in pr.get("args", [])[:5]))
            elif me == "Log.entryAdded":
                e = pr.get("entry", {})
                if e.get("level") in ("error", "warning"):
                    out.append(f"log.{e.get('level')} [{e.get('source')}] {str(e.get('text'))[:300]} {str(e.get('url') or '')[-80:]}")
        self.events = [(t, m) for t, m in self.events if session and m.get("sessionId") != session]
        return out

    def close(self):
        try:
            self.ws.close()
        except Exception:
            pass


def popup_hwnd():
    """擴充功能的小視窗（蓋在網頁上的那一塊）"""
    bh, ph = bar_hwnd(), page_hwnd()
    hs = [h for h in render_hwnds(web) if h not in (bh, ph)]
    return hs[0] if hs else 0


def tree_dump(root, name):
    """把畫面上讀到的元素寫成檔案（之後好對照）"""
    lines = []

    def rec(n, d):
        if n.name or n.id or n.type in ("ButtonControl", "HyperlinkControl", "EditControl", "CheckBoxControl", "ImageControl", "TabItemControl", "MenuItemControl", "ListItemControl", "ComboBoxControl"):
            lines.append(f"{'  ' * min(d, 14)}{n.type} {n.name[:80]!r} #{n.id} .{n.cls[:50]} ({n.l},{n.t},{n.r},{n.b})")
        for k in n.kids:
            rec(k, d + 1)

    if root is not None:
        rec(root, 0)
    with open(os.path.join(OUT, name), "w", encoding="utf-8") as f:
        f.write("\n".join(lines))
    return lines


def words(root, n=500):
    if root is None:
        return "（讀不到）"
    return " | ".join(x.name.strip() for x in root.walk() if x.name.strip() and x.type in ("TextControl", "ButtonControl", "HyperlinkControl", "CheckBoxControl", "MenuItemControl", "ListItemControl"))[:n]


def extprobe_ui():
    ensure_web_fg()
    # 擴充功能剛裝好會開一堆歡迎頁：先關掉，只留原本三個分頁（分頁太多時標題會被擠掉，後面認不出來）
    log("  一開始的分頁編號：", tab_ids(), [t["title"] for t in tabs_info()])
    restore_tabs(["t1", "t2", "t3"])
    select_tab(0, 1.2)
    home()
    click_blank(0.5)
    keep = tab_ids()
    user32.GetAsyncKeyState(0x04)

    def titles():
        return [t["title"] for t in tabs_info()]

    def pick(root, labels):
        """小視窗裡名字含這些字的元素，挑最小的那個（才點得準）"""
        c = [x for x in (root.walk() if root else []) if x.name and any(l in x.name for l in labels) and x.w > 0 and x.h > 0]
        return min(c, key=lambda x: x.w * x.h) if c else None

    def press(tag, root, labels, zh):
        n = pick(root, labels)
        if n is None:
            log(f"  [{tag}] 小視窗裡找不到 {labels}")
            check(f"extprobe：{zh}（找得到那一項）", False, words(root, 300))
            return
        before = titles()
        wins0 = [(title(h), cls(h), proc_name(pid_of(h))) for h in top_windows() if user32.IsWindowVisible(h)]
        click(*n.center, wait=4.5)
        after = titles()
        st = bar_state()
        wins1 = [(title(h), cls(h), proc_name(pid_of(h))) for h in top_windows() if user32.IsWindowVisible(h)]
        extra = [w for w in wins1 if w not in wins0]
        if extra:
            log(f"  [{tag}] 多出來的視窗：", extra)
        check(f"extprobe：{zh}（沒有另外跳出瀏覽器自己的視窗）", not any(w[1] == "Chrome_WidgetWin_1" and w[0] for w in extra), f"{extra}")
        log(f"  [{tag}] 按了 {n.name!r}（{n.type} {n.w}x{n.h}）之後：分頁 {len(before)} → {len(after)} {after[-3:]}；小視窗還在：{bool(popup_hwnd())}；網址列：{st.get('shown')!r}")
        check(f"extprobe：{zh}（開了分頁）", len(after) > len(before), f"{len(before)} → {len(after)}；網址列 {st.get('shown')!r}")
        check(f"extprobe：{zh}（小視窗收起來）", not popup_hwnd())
        shot("extprobe_" + tag)

    # ---- 篡改猴的小視窗 ----
    def open_tm():
        home()
        click_blank(0.4)
        click_bar(EXTICON(0), wait=3.0)
        h = popup_hwnd()
        return snap(h, tries=8, limit=900) if h else None

    root = open_tm()
    shot("extprobe_tm_popup")
    lines = tree_dump(root, "tm_popup_tree.txt")
    log("  篡改猴小視窗：", len(lines), "個元素；", words(root))
    check("extprobe：點工具列上的篡改猴圖示會出現小視窗", bool(popup_hwnd()), f"{[wrect(c) for c in vis_kids()]}")
    first = True
    for tag, labels, zh in (("tm_Dashboard", ("Dashboard", "控制台"), "篡改猴小視窗按「控制台」"), ("tm_Create", ("Create a new script", "新增腳本"), "篡改猴小視窗按「新增腳本」"),
                            ("tm_Find", ("Find new scripts", "取得新腳本"), "篡改猴小視窗按「取得新腳本」"), ("tm_Help", ("Help", "說明"), "篡改猴小視窗按「說明」")):
        if not first:
            root = open_tm()
        first = False
        press(tag, root, labels, zh)
    if popup_hwnd():
        click_blank(0.8)
    # ---- 其他擴充功能：小視窗裡按一個會開分頁的東西 ----
    for nm, labels, zh in (("AdGuard", ("Open AdGuard settings", "開啟 AdGuard 設定", "設定"), "AdGuard 小視窗按「設定」"),
                           ("SponsorBlock", ("Options", "選項"), "SponsorBlock 小視窗按「選項」"),
                           ("Return YouTube Dislike", ("Website", "網站"), "Return YouTube Dislike 小視窗按「網站」"),
                           ("iCloud", ("Mail", "郵件"), "iCloud 儀表板小視窗按「郵件」")):
        home()
        click_blank(0.5)
        ms = open_menu(EXT_BTN, f"擴充功能按鈕（{nm}）")
        if not ms or not menu_click(nm, exact=False, wait=4.5):
            close_menus()
            continue
        tag = "pop_" + nm.split()[0]
        h = popup_hwnd()
        root = snap(h, tries=8, limit=1200) if h else None
        tree_dump(root, f"{tag}_tree.txt")
        shot("extprobe_" + tag)
        log(f"  [{nm}] 小視窗：{'有' if h else '沒有'}；文字：{words(root, 300) if h else ''}")
        press(tag + "_click", root, labels, zh)
        if popup_hwnd():
            click_blank(0.8)
    # ---- 一般網頁（與內嵌框架）：程式跟使用者腳本之間的通道，網頁自己看不到 ----
    t0 = time.time()
    ensure_web_fg()
    home()
    chord([VK_CTRL], 0x54, wait=1.5)
    type_text("http://127.0.0.1:8765/gm.html")
    key(0x2E)
    key(VK_ENTER)
    time.sleep(6)
    gl = [l.split(" ", 1)[1] for l in page_log_since(t0) if "[gm " in l]
    for l in gl:
        log("  ", l[:300])
    for who in ("top", "frame", "nested"):
        check(f"extprobe：網頁自己的程式看不到 chrome.webview（{who}）", any(l.startswith(f"[gm {who}] webview=undefined") for l in gl), f"{[l[:80] for l in gl if l.startswith('[gm ' + who)]}")
    shot("extprobe_gm")
    ensure_web_fg()
    restore_tabs(keep)
    home()
    user32.GetAsyncKeyState(0x04)  # 剛才用中鍵關分頁：把「剛按過中鍵」的記號清掉，不然下一個開分頁的要求會被當成中鍵點的


def s_extprobe():
    """（查問題用）跟使用者一樣的擴充功能在程式的瀏覽器裡：哪些功能能用、哪些不能"""
    TM = TM_ID
    home()
    ensure_web_fg()
    click_blank(0.5)
    time.sleep(6)
    try:
        log("  web-ext.json：", json.dumps(json.load(open(os.path.join(data_dir, "web-ext.json"), encoding="utf-8")), ensure_ascii=False)[:3000])
    except Exception as e:
        log("  web-ext.json 讀不到", e)
    log("  一開始的分頁：", [t["title"] for t in tabs_info()])
    log("  一開始的視窗：", [(title(h), cls(h), proc_name(pid_of(h)), wrect(h)) for h in top_windows() if user32.IsWindowVisible(h)][:20])
    shot("extprobe_start")
    try:
        ps = ("Get-CimInstance Win32_Process | Where-Object { $_.Name -eq 'msedgewebview2.exe' -and $_.CommandLine -notmatch '--type=' } | ForEach-Object { \"$($_.ProcessId) :: $($_.CommandLine)\" }")
        out = subprocess.run(["powershell", "-NoProfile", "-Command", ps], capture_output=True, text=True, timeout=60).stdout
        log("  瀏覽器元件的啟動參數：\n" + "\n".join("    " + l for l in out.splitlines() if l.strip()))
    except Exception as e:
        log("  讀不到啟動參數", e)
    try:
        extprobe_ui()
    except Exception:
        log(traceback.format_exc())
        check("extprobe 畫面操作 (沒有錯誤)", False, "exception")
    port = None
    for f, pt in app_cdp_ports():
        log("  偵錯埠檔案：", f, pt)
        if "web-profile" in f.replace("\\", "/"):
            port = pt
    if port is None:
        log("  （接不上瀏覽器的偵錯埠，偵錯埠那一段不做）", app_cdp_ports(), listening_ports())
        return
    ver = json.loads(urllib.request.urlopen(f"http://127.0.0.1:{port}/json/version", timeout=5).read().decode("utf-8", "replace"))
    log("  瀏覽器：", json.dumps(ver, ensure_ascii=False)[:500])
    b = CDPB(ver["webSocketDebuggerUrl"])

    def tlist(tag):
        ts = b.targets()
        log(f"  [{tag}] 偵錯目標 {len(ts)} 個：")
        for t in ts:
            log(f"      {t.get('type'):16} {t.get('targetId')[:8]} att={t.get('attached')} {t.get('url', '')[:110]} | {t.get('title', '')[:40]}")
        return ts

    def snap_state(tag):
        ts = b.targets()
        tabs = [t["title"] for t in tabs_info()]
        wins = [(title(h), cls(h), proc_name(pid_of(h))) for h in top_windows() if user32.IsWindowVisible(h)]
        log(f"  [{tag}] 分頁：{tabs}")
        log(f"  [{tag}] 視窗：{wins[:14]}")
        return {"targets": {t["targetId"]: t for t in ts}, "tabs": tabs, "wins": wins}

    def diff(tag, before, after):
        new_t = [t for i, t in after["targets"].items() if i not in before["targets"]]
        gone_t = [t for i, t in before["targets"].items() if i not in after["targets"]]
        log(f"  [{tag}] 多出來的偵錯目標：", [(t.get("type"), t.get("url", "")[:100]) for t in new_t], "不見的：", [(t.get("type"), t.get("url", "")[:80]) for t in gone_t])
        log(f"  [{tag}] 分頁 {len(before['tabs'])} → {len(after['tabs'])}：{after['tabs']}")
        nw = [w for w in after["wins"] if w not in before["wins"]]
        if nw:
            log(f"  [{tag}] 多出來的視窗：", nw)
        return new_t

    def sw_of(eid, wake=True):
        """擴充功能的背景程式（service worker 或背景頁）"""
        for _ in range(2):
            for t in b.targets():
                if t.get("url", "").startswith(f"chrome-extension://{eid}/") and t.get("type") in ("service_worker", "background_page"):
                    return t
            if not wake:
                break
            time.sleep(1.5)
        return None

    ts = tlist("一開始")
    json.dump(ts, open(os.path.join(OUT, "ext_targets.json"), "w", encoding="utf-8"), ensure_ascii=False, indent=1)
    # ---- 每個擴充功能的背景程式：在不在、能用哪些功能 ----
    for eid in REAL_EXTS:
        info = REAL_INFO.get(eid, {})
        t = sw_of(eid, wake=False)
        if t is None:
            log(f"  [{info.get('name')}] {eid}：沒有背景程式在跑")
            continue
        sid = b.attach(t["targetId"])
        if not sid:
            continue
        r = b.ev(sid, "({type: typeof window === 'undefined' ? 'worker' : 'page', apis: Object.keys(chrome).sort().join(','), ver: chrome.runtime.getManifest().version, name: chrome.runtime.getManifest().name, us: typeof chrome.userScripts, tabs: typeof chrome.tabs, incog: chrome.extension && chrome.extension.inIncognitoContext})")
        log(f"  [{info.get('name')}] {eid} {t.get('type')}：", json.dumps(r, ensure_ascii=False)[:900])
    # ---- 篡改猴的背景程式 ----
    t = sw_of(TM)
    if t is None:
        check("extprobe：篡改猴的背景程式在跑", False, "找不到")
        b.close()
        return
    sw = b.attach(t["targetId"])
    for d in ("Runtime", "Log"):
        b.call(d + ".enable", session=sw)
    b.pump(1.5)
    log("  篡改猴背景程式的錯誤訊息：", b.drain(sw)[:12])
    Q = "chrome.tabs.query({}).then(ts => ts.map(t => ({id: t.id, w: t.windowId, a: t.active, i: t.index, st: t.status, u: (t.url || '').slice(0, 70), ti: (t.title || '').slice(0, 24)})))"
    log("  tabs.query({})：", json.dumps(b.ev(sw, Q), ensure_ascii=False)[:2500])
    log("  tabs.query({active:true})：", json.dumps(b.ev(sw, "chrome.tabs.query({active: true}).then(ts => ts.map(t => [t.id, t.windowId, (t.url || '').slice(0, 60)]))"), ensure_ascii=False)[:1200])
    log("  tabs.query({active:true,currentWindow:true})：", json.dumps(b.ev(sw, "chrome.tabs.query({active: true, currentWindow: true}).then(ts => ts.map(t => [t.id, t.windowId, (t.url || '').slice(0, 60)]))"), ensure_ascii=False)[:1200])
    log("  tabs.query({active:true,lastFocusedWindow:true})：", json.dumps(b.ev(sw, "chrome.tabs.query({active: true, lastFocusedWindow: true}).then(ts => ts.map(t => [t.id, t.windowId, (t.url || '').slice(0, 60)]))"), ensure_ascii=False)[:1200])
    log("  windows.getAll：", json.dumps(b.ev(sw, "chrome.windows.getAll({populate: false}).then(ws => ws.map(w => ({id: w.id, f: w.focused, t: w.type, s: w.state, x: w.left, y: w.top, w: w.width, h: w.height})))"), ensure_ascii=False)[:1500])
    log("  windows.getLastFocused：", json.dumps(b.ev(sw, "chrome.windows.getLastFocused().then(w => ({id: w.id, f: w.focused, t: w.type}))"), ensure_ascii=False)[:300])
    log("  userScripts（一開始）：", json.dumps(b.ev(sw, "({type: typeof chrome.userScripts, get: chrome.userScripts ? await chrome.userScripts.getScripts().then(x => 'ok ' + x.length, e => 'ERR ' + e.message) : 'undefined'})"), ensure_ascii=False))
    log("  permissions：", json.dumps(b.ev(sw, "chrome.permissions.getAll()"), ensure_ascii=False)[:900])

    # ---- 擴充功能要開分頁、視窗時會怎樣 ----
    def act(tag, expr, settle=3.5):
        before = snap_state(tag + " 之前")
        r = b.ev(sw, expr)
        log(f"  [{tag}] 回傳：", json.dumps(r, ensure_ascii=False)[:500])
        time.sleep(settle)
        b.pump(0.5)
        after = snap_state(tag + " 之後")
        new_t = diff(tag, before, after)
        errs = b.drain(sw)
        if errs:
            log(f"  [{tag}] 背景程式的錯誤訊息：", errs[:8])
        shot("extprobe_" + tag)
        return r, before, after, new_t

    T2 = "(t => ({id: t.id, w: t.windowId, a: t.active, u: t.url, pu: t.pendingUrl, st: t.status}))"
    act("tabs_create_http", f"chrome.tabs.create({{url: 'http://127.0.0.1:8765/snap.html?extcreate=1'}}).then({T2})")
    act("tabs_create_http_bg", f"chrome.tabs.create({{url: 'http://127.0.0.1:8765/snap.html?extcreate=bg', active: false}}).then({T2})")
    _r, bf, af, _n = act("tabs_create_ext", f"chrome.tabs.create({{url: chrome.runtime.getURL('options.html') + '#nav=dashboard'}}).then({T2})", settle=5)
    check("extprobe：擴充功能的背景程式要開它自己的頁面 → 開成分頁", len(af["tabs"]) == len(bf["tabs"]) + 1 and "chrome-extension://" in str(bar_state().get("shown")), f"{len(bf['tabs'])} → {len(af['tabs'])} {bar_state().get('shown')!r}")
    act("windows_create", "chrome.windows.create({url: 'http://127.0.0.1:8765/snap.html?wincreate=1', type: 'popup', width: 420, height: 320}).then(w => ({id: w && w.id, t: w && w.type}))")
    _r, bf, af, _n = act("openOptionsPage", "chrome.runtime.openOptionsPage().then(() => 'ok')", settle=6)
    stray = [w for w in af["wins"] if w[1] == "Chrome_WidgetWin_1" and w[0]]
    check("extprobe：擴充功能的背景程式開選項頁 → 開成分頁，不是另外跳出一個視窗", len(af["tabs"]) == len(bf["tabs"]) + 1 and not stray, f"{len(bf['tabs'])} → {len(af['tabs'])}；視窗 {stray}；網址列 {bar_state().get('shown')!r}")
    # AdGuard：背景程式自己開設定頁會怎樣
    AD = "bgnkhhnnamicmpeenaelnjfhikgbkllg"
    ta = sw_of(AD)
    if ta is not None:
        sa = b.attach(ta["targetId"])
        for d in ("Runtime", "Log"):
            b.call(d + ".enable", session=sa)
        log("  AdGuard tabs.query(options)：", json.dumps(b.ev(sa, "chrome.tabs.query({url: chrome.runtime.getURL('pages/options.html') + '*'}).then(ts => ts.map(t => [t.id, t.windowId, t.url]))"), ensure_ascii=False)[:400])
        log("  AdGuard tabs.query({})：", json.dumps(b.ev(sa, "chrome.tabs.query({}).then(ts => ts.map(t => [t.id, t.windowId, t.active, (t.url || '').slice(0, 60)]))"), ensure_ascii=False)[:1500])
        home()
        click_blank(0.5)
        ms = open_menu(EXT_BTN, "擴充功能按鈕（AdGuard，接著偵錯）")
        if ms and menu_click("AdGuard", exact=False, wait=4.5):
            h = popup_hwnd()
            root = snap(h, tries=8, limit=1200) if h else None
            n = next((x for x in (root.walk() if root else []) if x.name == "Open AdGuard settings"), None)
            if n is not None:
                bf = snap_state("AdGuard 設定 之前")
                b.events.clear()
                click(*n.center, wait=0.2)
                b.pump(5)
                af = snap_state("AdGuard 設定 之後")
                diff("AdGuard 設定", bf, af)
                log("  AdGuard 背景程式的訊息：", b.drain(sa)[:15])
                log("  其他事件：", [m.get("method") for _, m in b.events][:30])
        bf = snap_state("AdGuard 自己開設定頁 之前")
        log("  AdGuard tabs.create(options)：", json.dumps(b.ev(sa, "chrome.tabs.create({url: chrome.runtime.getURL('pages/options.html')}).then(t => ({id: t && t.id}))"), ensure_ascii=False)[:300])
        time.sleep(5)
        af = snap_state("AdGuard 自己開設定頁 之後")
        diff("AdGuard 自己開設定頁", bf, af)
        log("  AdGuard 背景程式的訊息：", b.drain(sa)[:15])
    # 擴充功能各自讓網頁存取哪些檔案
    for eid in REAL_EXTS:
        try:
            base_ = os.path.join(chrome_root, "Default", "Extensions", eid)
            ver_ = sorted(os.listdir(base_))[-1]
            man_ = json.load(open(os.path.join(base_, ver_, "manifest.json"), encoding="utf-8"))
            log(f"  {eid} 讓網頁存取的檔案：", json.dumps(man_.get("web_accessible_resources"), ensure_ascii=False)[:600], "；選項頁：", man_.get("options_page") or (man_.get("options_ui") or {}).get("page"))
        except Exception as e:
            log("  讀不到 manifest", eid, repr(e)[:100])
    # 一般網頁硬要開別人的擴充功能頁面（沒有開放給網頁的）：不可以真的開出來
    SB = "mnjggcdmjocbbbhaepdhchncahnbgone"
    home()
    time.sleep(1.0)
    pg = next((t for t in b.targets() if t.get("type") == "page" and t.get("url", "").endswith("/test.html")), None)
    if pg is not None:
        ps = b.attach(pg["targetId"])
        for tag_, url_, words_ in (("SponsorBlock 選項頁", f"chrome-extension://{SB}/options/options.html", ("SponsorBlock", "Options", "選項")),
                                   ("AdGuard 篩選記錄頁", "chrome-extension://bgnkhhnnamicmpeenaelnjfhikgbkllg/pages/filtering-log.html", ("Filtering log", "AdGuard"))):
            bf = snap_state(f"網頁開 {tag_} 之前")
            r = b.call("Runtime.evaluate", {"expression": f"(() => {{ const w = window.open('{url_}'); return String(w); }})()", "returnByValue": True, "userGesture": True}, ps)
            log(f"  網頁 window.open({tag_})：", json.dumps(r, ensure_ascii=False)[:300])
            time.sleep(4)
            af = snap_state(f"網頁開 {tag_} 之後")
            diff(f"網頁開 {tag_}", bf, af)
            st = bar_state()
            txt = page_text()
            log("  之後網址列：", st.get("shown"), "；分頁：", [t["title"] for t in tabs_info()], "；畫面文字：", " | ".join(x.strip() for x in txt.splitlines() if x.strip())[:300])
            shot("extprobe_page_opens_ext")
            shown_ = (st.get("shown") or "")
            check(f"extprobe：一般網頁要開擴充功能的頁面（{tag_}）→ 看不到那個頁面的內容", not (url_.split("//")[1][:40] in shown_ and any(w_ in txt for w_ in words_)), f"{shown_!r} {txt[:200]}")
            home()
            time.sleep(0.8)
    # chrome.webview.postMessage 是不是瀏覽器內建的（不經過網頁能動手腳的東西）：在程式自己的工具列頁面上看
    bar_t = next((t for t in b.targets() if t.get("type") == "page" and "tauri.localhost/web.html" in t.get("url", "")), None)
    if bar_t is not None:
        bs = b.attach(bar_t["targetId"])
        r = b.ev(bs, """(() => {
          const wv = window.chrome && window.chrome.webview; if (!wv) return 'no webview';
          const ts = Function.prototype.toString;
          const seen = [];
          const oj = JSON.stringify, ok = Object.keys;
          JSON.stringify = function (...a) { seen.push('JSON.stringify'); return oj.apply(this, a); };
          Object.keys = function (o) { seen.push('Object.keys'); return ok(o); };
          Object.prototype.toJSON = function () { seen.push('toJSON'); return this; };
          let err = '';
          try { wv.postMessage('{"cmd":"mb","callback":0,"error":0,"payload":null,"__TAURI_INVOKE_KEY__":"","mb":"probe","d":"{}"}'); } catch (e) { err = String(e); }
          delete Object.prototype.toJSON; JSON.stringify = oj; Object.keys = ok;
          return { post: ts.call(wv.postMessage), add: ts.call(wv.addEventListener), proto: Object.prototype.toString.call(wv), hooked: seen, err };
        })()""")
        log("  chrome.webview：", json.dumps(r, ensure_ascii=False)[:600])
        check("extprobe：chrome.webview.postMessage 是瀏覽器內建的，送文字不會經過網頁換得掉的功能", isinstance(r, dict) and "[native code]" in r.get("post", "") and not r.get("hooked"), json.dumps(r, ensure_ascii=False)[:300])
    log("  之後的 tabs.query({})：", json.dumps(b.ev(sw, Q), ensure_ascii=False)[:3000])

    b.pump(0.5)
    log("  最後的偵錯目標：")
    tlist("最後")
    snap_state("最後")
    b.close()
    shot("extprobe_end")
    try:
        APP_ERR.flush()
        log("  程式的訊息：\n" + "\n".join("    " + l for l in open(os.path.join(OUT, "app_stderr.txt"), encoding="utf-8", errors="replace").read().splitlines()[-120:]))
    except Exception as e:
        log("  讀不到程式的訊息", e)


def s_notepad():
    np_proc = subprocess.Popen(["notepad.exe"])
    np = wait_for(lambda: next((h for h in top_windows() if pid_of(h) == np_proc.pid and user32.IsWindowVisible(h)), 0), 10)
    time.sleep(1)
    if np:
        user32.ShowWindow(np, 1)
        # 記事本放在 web 視窗的右下（蓋住一部分網頁，不蓋到分頁）
        r = wrect(web)
        user32.MoveWindow(np, r[0] + 300, r[1] + 250, 500, 320, True)
        time.sleep(0.5)
        click(wrect(np)[0] + 250, wrect(np)[1] + 200, wait=1.0)
        check("記事本在前景時可以蓋住 web 視窗", above(np, web), f"fg={fg()} np={np}")
        shot("notepad_over_web")
        i = active_tab() or 0
        click_bar(TAB(i), wait=1.0)
        check("點 web 視窗：變成前景（一般視窗）", fg() == web and above(web, np), f"fg={fg()}")
    else:
        log("（記事本沒有開起來，略過）")
    np_proc.kill()
    subprocess.run(["taskkill", "/IM", "notepad.exe", "/F"], capture_output=True)


def s_dpi():
    global scale
    user32.SetProcessDPIAware()
    info = dpi_info()
    log("縮放比例資訊", info)
    if user32.GetSystemMetrics(0) < 1600:
        if not set_resolution(1920, 1080):
            log("沒辦法改解析度，略過縮放比例測試")
            return
        time.sleep(3)
        info = dpi_info()
        log("改解析度後的縮放比例資訊", info, "螢幕", user32.GetSystemMetrics(0), user32.GetSystemMetrics(1), dump(web, "改解析度後"))
    if not info or info[3] <= info[2]:
        log("這台測試機不能調高縮放比例，略過")
        return
    src, lo, cur, hi = info
    # 切到短影片分頁（捲到某一張了），看縮放比例改變後還是不是同一張
    select_title("短影片", 1.5)
    t0 = time.time()
    d0 = user32.GetDpiForWindow(web)
    r = set_dpi_rel(src, cur + 1)
    time.sleep(4)
    d1 = user32.GetDpiForWindow(web)
    log("改縮放比例", r, "DPI", d0, "→", d1)
    shot("dpi_up")
    dump(web, "dpi up")
    scale = d1 / 96
    ok, vis = layout_ok(d1 / 96)
    check("縮放比例改變後：視窗還看得到", user32.IsWindowVisible(web) and not user32.IsIconic(web))
    check("縮放比例改變後：工具列和網頁位置正確", ok, f"dpi={d1} {vis}")
    after = slides_since(t0)
    check("縮放比例改變後：短影片還是同一張", all(x == SNAP["cur"] for x in after), f"{after} 原本={SNAP['cur']}")
    check("縮放比例改變後：程式有回應", responsive(web))
    # 放大的比例下選單位置也要對
    ms = open_menu(MENU_BTN, f"縮放比例 {int(scale * 100)}% 時按 ⋮")
    shot("dpi_menu")
    if ms:
        ok, d = below(rect_of(MENU_BTN), ms[0][0], side="right")
        check(f"縮放比例 {int(scale * 100)}% 時：選單還是貼在按鈕下面", ok, d)
        cw = client(ms[0][0])[2]
        check(f"縮放比例 {int(scale * 100)}% 時：選單跟著放大", MAIN["w"] > 0 and abs(cw - MAIN["w"] * scale) <= 3, f"現在寬={cw} 100% 時寬={MAIN['w']}")
    close_menus()
    t0 = time.time()
    set_dpi_rel(src, cur)
    time.sleep(4)
    d2 = user32.GetDpiForWindow(web)
    scale = d2 / 96
    log("縮放比例改回來 DPI", d2)
    shot("dpi_back")
    dump(web, "dpi back")
    ok, vis = layout_ok(d2 / 96)
    check("縮放比例改回來：工具列和網頁位置正確", ok, f"dpi={d2} {vis}")
    after = slides_since(t0)
    check("縮放比例改回來：短影片還是同一張", all(x == SNAP["cur"] for x in after), f"{after}")
    home(1.0)


def s_fullscreen():
    home(1.0)
    ensure_web_fg()
    r0 = wrect(web)
    n0 = len(vis_kids())
    click(*PG(100, 420), wait=2.0)
    shot("fullscreen")
    r = wrect(web)
    sw, sh = user32.GetSystemMetrics(0), user32.GetSystemMetrics(1)
    vis = [wrect(c) for c in vis_kids()]
    check("影片全螢幕：視窗蓋滿整個螢幕", r == (0, 0, sw, sh), f"{r} screen={sw}x{sh}")
    check("影片全螢幕：工具列藏起來、網頁蓋滿", len(vis) == 1 and vis[0] == (0, 0, sw, sh), f"{vis}")
    check("影片全螢幕：網頁收到", "fullscreen on" in page_log() or "fullscreenchange true" in page_log(), page_log()[-200:])
    key(VK_ESC)
    time.sleep(2.0)
    shot("fullscreen_off")
    r = wrect(web)
    vis = [wrect(c) for c in vis_kids()]
    check("按 Esc：回到原本的大小", r == r0, f"{r} before={r0}")
    check("按 Esc：工具列回來", len(vis) == n0 and any(x[1] == r[1] for x in vis), f"{vis}")
    # F11：整個視窗全螢幕，滑鼠移到最上面工具列會出來
    click_blank(0.6)
    key(VK_F11)
    time.sleep(2.0)
    r = wrect(web)
    vis = [wrect(c) for c in vis_kids()]
    shot("f11")
    check("F11：視窗蓋滿整個螢幕、工具列藏起來", r == (0, 0, sw, sh) and len(vis) == 1, f"{r} {vis}")
    hover(sw // 2, 0, settle=1.2)
    move(sw // 2, 1)
    time.sleep(1.0)
    vis = [wrect(c) for c in vis_kids()]
    shot("f11_reveal")
    check("F11 時滑鼠移到螢幕最上面：工具列滑出來", len(vis) == 2, f"{vis}")
    hover(sw // 2, sh // 2, settle=1.5)
    vis = [wrect(c) for c in vis_kids()]
    check("F11 時滑鼠移開：工具列又藏起來", len(vis) == 1, f"{vis}")
    key(VK_F11)
    time.sleep(2.0)
    r = wrect(web)
    vis = [wrect(c) for c in vis_kids()]
    check("再按 F11：回到原本的大小、工具列回來", r == r0 and len(vis) == n0, f"{r} before={r0} {vis}")


def s_ext():
    home()
    ensure_web_fg()
    before = vis_kids()
    ms = open_menu(EXT_BTN, "按擴充功能按鈕")
    shot("ext_menu")
    if ms:
        log("擴充功能選單", menu_titles(ms))
        check("擴充功能選單：列出 Chrome 同步來的擴充功能", any("測試擴充功能" in it["t"] for it in ms[0][1]["items"]), f"{menu_titles(ms)}")
        menu_click("測試擴充功能", exact=False, wait=3.0)
    shot("ext_popup")
    after = vis_kids()
    check("點擴充功能：小視窗蓋在網頁上", len(after) == len(before) + 1, f"{[wrect(c) for c in after]}")
    log("ext log:", [l for l in page_log().splitlines() if "[ext]" in l][-6:])
    act = next((l for l in page_log().splitlines() if "[ext] active:" in l), "")
    check("擴充功能看得到目前的網頁（active tab）", "test.html" in act and "popup.html" not in act, act[-200:])
    lf = next((l for l in page_log().splitlines() if "[ext] lastFocused:" in l), "")
    check("擴充功能看得到目前的網頁（lastFocused）", "test.html" in lf and "popup.html" not in lf, lf[-200:])
    click_blank(1.0)
    after2 = vis_kids()
    check("點外面：小視窗關掉", len(after2) == len(before), f"{len(after2)}")
    # 固定在工具列上
    ms = open_menu(EXT_BTN, "再按擴充功能按鈕")
    if ms:
        row = next((it for it in ms[0][1]["items"] if "測試擴充功能" in it["t"]), None)
        pin = row["btns"].get("固定在工具列上") if row else None
        log("固定按鈕", pin, row and list(row["btns"]))
        if pin:
            click(*pin.center, wait=1.2)
            n = len([k for k in (find_id(bar_tree(), "exts") or N_EMPTY).kids if k.w > 0])
            shot("ext_pinned")
            check("擴充功能按圖釘：固定到工具列上", n == 1, f"{n}")
            close_menus()
            if n == 1:
                click_bar(EXTICON(0), wait=2.5)
                check("點工具列上固定的擴充功能：開它的小視窗", len(vis_kids()) == len(before) + 1, f"{[wrect(c) for c in vis_kids()]}")
                click_blank(1.0)
                ms = open_menu(EXTICON(0), "固定的擴充功能按右鍵", button="right")
                if ms:
                    log("固定的擴充功能右鍵選單", menu_titles(ms))
                    menu_click("取消固定", exact=False, wait=1.0)
                check("取消固定：工具列上的圖示拿掉", rect_of(EXTICON(0)) is None)
        else:
            check("擴充功能選單：每一列有圖釘按鈕", False, f"{row}")
    close_menus()


def s_scripts():
    lines = [l for l in page_log().splitlines() if "[us]" in l]
    log("腳本記錄：", lines[-8:])
    check("使用者腳本：載入後跑的有執行", any("end ran" in l and "test.html" in l for l in lines))
    ns = [int(m.group(1)) for l in lines for m in [re.search(r"end ran n=(\d+)", l)] if m]
    check("使用者腳本：GM_setValue 有記住（每載入一次加一）", len(ns) >= 2 and max(ns) >= 2 and ns == sorted(ns), f"{ns}")
    check("使用者腳本：document-start 的有執行", any("start ran /test.html" in l for l in lines))
    check("使用者腳本：@exclude 有效（內嵌框不跑）", not any("start ran /frame.html" in l for l in lines))
    check("使用者腳本：不符合的網站不跑", not any("WRONG" in l for l in lines))
    # 檢視網頁原始碼：那個頁面不跑腳本
    home()
    keep = tab_ids()
    t0 = time.time()
    ms = ctx_at(*PG(620, 330))
    if ms and menu_click("檢視網頁原始碼", wait=3.0):
        st = bar_state()
        shot("view_source")
        check("右鍵 → 檢視網頁原始碼：在新分頁打開，網址列是 view-source: 開頭", "view-source:" in (st.get("shown") or ""), f"{st.get('shown')!r} {title(web)!r} {st}")
        check("檢視原始碼的分頁：沒有星星（不能加書籤）", st.get("star") is False, f"{st}")
        ran = [l for l in page_log_since(t0) if "[us]" in l]
        check("檢視原始碼的頁面：不跑使用者腳本", not ran, f"{ran}")
        time.sleep(1.5)
        tabs = (saved().get("wins") or [{}])[0].get("tabs", [])
        check("檢視原始碼的分頁：記的網址是 view-source: 開頭", any(str(t.get("url", "")).startswith("view-source:http://127.0.0.1") for t in tabs), f"{[t.get('url') for t in tabs]}")
        restore_tabs(keep)
    close_menus()
    home()


def open_tab(url, wait=3.5):
    """開新分頁前往某個網址"""
    ensure_web_fg()
    chord([VK_CTRL], 0x54, wait=1.5)
    type_text(url)
    key(0x2E)
    key(VK_ENTER)
    time.sleep(wait)


US_BTN = E("us")


def us_lines(t0, tag="[us4]"):
    return [l.split(" ", 1)[1] for l in page_log_since(t0) if tag in l]


def s_userscripts():
    ensure_web_fg()
    if "extprobe" in ONLY_EARLY:
        # 擴充功能剛裝好會開一堆歡迎頁：先關掉（分頁太多時標題會被擠掉，後面認不出來）
        restore_tabs(["t1", "t2", "t3"])
        select_tab(0, 1.2)
    home()
    click_blank(0.5)
    keep = tab_ids()
    user32.GetAsyncKeyState(0x04)
    base = f"http://127.0.0.1:{PORT}"
    # 讓 localhost 先有一個 Cookie（要最上層的網頁才設得了）
    open_tab(f"http://localhost:{PORT}/setck", 2.5)
    t0 = time.time()
    go(f"{base}/us.html", 8.0)
    L, P = us_lines(t0), us_lines(t0, "[uspage]")
    for l in L + P:
        log("  ", l[:400])
    shot("us_page")
    top = next((l for l in L if l.startswith("[us4] top - start")), "")
    check("腳本（document-start）在網頁自己的程式之前跑", "pageRan=false ready=loading" in top, top)
    check("腳本拿得到自己的資訊；有寫 @grant 的功能才有", "handler=MAOBOO ver=1.0 types=function,undefined,undefined" in top, top)
    check("內嵌框架（別的網站）裡也會跑，而且也在框架自己的程式之前", any(l.startswith("[us4] frame - start pageRan=false") for l in L), f"{[l[:80] for l in L if 'frame' in l]}")
    check("網頁自己的程式看不到程式跟腳本之間的通道（最上層、框架都是）", any("[uspage] top - webview=undefined extra=0" in l for l in P) and any("[uspage] frame webview=undefined extra=0" in l for l in P), f"{P}")
    check("腳本自己也拿不到那個通道（只能用 GM 功能）", "webview=undefined" in top, top)
    g = next((l for l in L if "xhr get" in l), "")
    check("GM_xmlhttpRequest：跨網站連得到（狀態、最後的網址、回應標頭）", "status=200 method=GET" in g and f"final=http://localhost:{PORT}/xhr-echo?auto=1" in g and "hdr=true" in g, g)
    check("GM_xmlhttpRequest：腳本給的標頭（Referer、自訂的）有帶到，瀏覽器名稱跟網頁一樣", "referer=https://ref.example/ xtest=abc" in g and "ua=true" in g, g)
    check("GM_xmlhttpRequest：@connect 有寫的網域會帶這個瀏覽器的 Cookie", "cookie=mbck=abc" in g, g)
    po = next((l for l in L if "xhr post" in l), "")
    check("GM_xmlhttpRequest：POST 的內容送得到、二進位回應拿得到", "status=200 bytes=true" in po and 'body={"a":"中文"} ctype=application/json' in po, po)
    sm = next((l for l in L if "xhr stream" in l), "")
    mm = re.search(r"chunks=(\d+)", sm)
    check("GM_xmlhttpRequest：串流一段一段收到", "text=data: 1|data: 2|data: 3|" in sm and bool(mm) and int(mm.group(1)) >= 2, sm)
    check("GM_xmlhttpRequest：腳本沒有允許的網域連不了", any("xhr denied ok" in l for l in L) and not any("denied??" in l for l in L), f"{[l for l in L if 'denied' in l]}")
    check("對照：網頁自己連同一個網址會被擋（所以上面是程式代連的）", any("pagefetch blocked" in l for l in L), f"{[l for l in L if 'pagefetch' in l]}")
    b = next((l for l in P if "blank webview" in l), "")
    check("還沒載入的空白框架：通道也拿掉了，而且留了記號", "blank webview=undefined marker=1" in b, b)
    r = next((l for l in P if "reuse reused" in l), "")
    log("  空白框架接著載入同網站的網頁：", r)
    check("空白框架的視窗被沿用的話腳本不跑（沒被沿用就照常跑）", ("reused=false innerRan=true" in r) or ("reused=true innerRan=false" in r), r)

    # ---- 工具列的「使用者腳本」按鈕 ----
    check("有裝腳本時，工具列有「使用者腳本」按鈕", rect_of(US_BTN) is not None)
    ms = open_menu(US_BTN, "使用者腳本按鈕")
    names = names_of(ms[0][1]) if ms else []
    log("  選單：", menu_titles(ms), [it["texts"] for it in ms[0][1]["items"]] if ms else [])
    shot("us_menu")
    check("選單列出這個網頁的腳本和它登記的項目", all(any(x in n for n in names) for x in ("測試腳本四", "測試：設值", "【關】開關", "測試：開分頁", "管理使用者腳本")), f"{names}")
    row = next((it for it in ms[0][1]["items"] if "測試腳本四" in it["t"]), None) if ms else None
    check("選單上標出腳本正在執行", bool(row) and any("執行中" in x for x in row["texts"]), f"{row and row['texts']}")
    t1 = time.time()
    menu_click("測試：設值", wait=2.0)
    L = us_lines(t1)
    val = next((m.group(1) for l in L for m in [re.search(r"menu set (v\d+)", l)] if m), "")
    check("點腳本的選單項目：腳本的函式有執行、選單關掉", bool(val) and not menu_wins(), f"{L}")
    check("腳本改了設定值：同一頁的別的框架收到通知（remote=true），自己這份是 remote=false",
          any(l.startswith("[us4] frame - change") and val in l and "remote=true" in l for l in L) and any(l.startswith("[us4] top - change") and val in l and "remote=false" in l for l in L), f"{L}")
    ms = open_menu(US_BTN, "使用者腳本按鈕（第二次）")
    t2 = time.time()
    menu_click("【關】開關", wait=1.8)
    names2 = [n for _, m in menus() for n in names_of(m)]
    shot("us_menu_toggle")
    check("開關類的項目（按了不關選單）：選單還在、標題跟著變", bool(menu_wins()) and any("【開】開關" in n for n in names2) and any("menu toggle on=true" in l for l in us_lines(t2)), f"{names2} {us_lines(t2)}")
    close_menus()

    # ---- 另一個分頁 ----
    t3 = time.time()
    open_tab(f"{base}/us.html?two=1&noxhr", 5.0)
    L = us_lines(t3)
    check("另一個分頁開同一個網站：一開始就拿到剛才設的值", any(l.startswith("[us4] top ?two=1&noxhr start") and f'shared="{val}"' in l for l in L), f"{[l[:160] for l in L]}")
    ms = open_menu(US_BTN, "使用者腳本按鈕（另一個分頁）")
    t4 = time.time()
    menu_click("測試：設值", wait=2.5)
    L = us_lines(t4)
    val2 = next((m.group(1) for l in L for m in [re.search(r"menu set (v\d+)", l)] if m), "")
    check("在這個分頁改值：另一個分頁（沒在顯示）也收到（remote=true）", bool(val2) and any(l.startswith("[us4] top - change") and val2 in l and "remote=true" in l for l in L), f"{L}")
    # GM_openInTab
    n0 = tab_count()
    ms = open_menu(US_BTN, "使用者腳本按鈕（開分頁）")
    menu_click("測試：開分頁", wait=3.5)
    st = bar_state()
    check("GM_openInTab：開了新分頁並切過去", tab_count() == n0 + 1 and "gmtab=1" in (st.get("shown") or ""), f"{n0} → {tab_count()} {st.get('shown')!r}")

    # ---- 在選單開關腳本 ----
    select_title("腳本測試")
    ms = open_menu(US_BTN, "使用者腳本按鈕（開關腳本）")
    menu_click("測試腳本四", exact=False, wait=1.5)
    row = next((it for _, m in menus() for it in m["items"] if "測試腳本四" in it["t"]), None)
    check("在選單點腳本的名稱：關掉（選單上標「已關閉」）", bool(row) and any("已關閉" in x for x in row["texts"]), f"{row and row['texts']}")
    close_menus()
    click_blank(0.5)
    t5 = time.time()
    key(VK_F5)
    time.sleep(4.5)
    check("腳本關掉後重新整理：這支不跑了，其他的照跑", not us_lines(t5) and any("[us] start ran /us.html" in l for l in page_log_since(t5)), f"{us_lines(t5)[:3]} {[l for l in page_log_since(t5) if '[us]' in l][:3]}")
    ms = open_menu(US_BTN, "使用者腳本按鈕（再打開腳本）")
    menu_click("測試腳本四", exact=False, wait=1.5)
    close_menus()
    click_blank(0.5)
    t6 = time.time()
    key(VK_F5)
    time.sleep(4.5)
    check("再打開、重新整理：又跑了", any("top" in l and "start" in l for l in us_lines(t6)), f"{us_lines(t6)[:3]}")

    # ---- 從網址安裝 ----
    go(f"{base}/inst.user.js", 5.0)
    st, txt = bar_state(), page_text()
    shot("us_install")
    check("開腳本的網址（.user.js）：出現安裝畫面，不是直接顯示程式碼", "chrome://extensions/install" in (st.get("shown") or "") and "安裝使用者腳本" in txt and "安裝測試腳本" in txt and "1.0" in txt, f"{st.get('shown')!r} {txt[:300]}")
    btn = next((n for n in find_all(page_tree(), lambda n: n.type == "ButtonControl" and n.name.strip() == "安裝")), None)
    if btn is not None:
        click(*btn.center, wait=3.5)
    txt = page_text()
    inst_file = os.path.join(data_dir, "web-scripts", "安裝測試腳本.user.js")
    check("按「安裝」：裝好了", "安裝好了" in txt and os.path.exists(inst_file), f"{txt[:300]}")
    t7 = time.time()
    go(f"{base}/us.html?inst=1&noxhr", 5.0)
    check("裝好的腳本在網頁上執行", any("[inst] ran v=1.0 ?inst=1" in l for l in page_log_since(t7)), f"{[l for l in page_log_since(t7) if '[inst]' in l]}")
    # 更新
    INST["ver"] = "2.0"
    go("chrome://extensions", 3.5)
    upd = next((n for n in find_all(page_tree(), lambda n: n.type == "ButtonControl" and "檢查更新" in n.name)), None)
    check("擴充功能頁：有「檢查更新」「從網址安裝」「自動更新腳本」", upd is not None and "從網址安裝" in page_text() and "自動更新腳本" in page_text(), page_text()[-400:])
    if upd is not None:
        click(*upd.center, wait=5.0)
    shot("us_update")
    t8 = time.time()
    go(f"{base}/us.html?inst=2&noxhr", 5.0)
    check("檢查更新：換成新版，重新整理後跑的是新版", any("[inst] ran v=2.0 ?inst=2" in l for l in page_log_since(t8)), f"{[l for l in page_log_since(t8) if '[inst]' in l]}")
    # 網頁上的連結點下去（不是打網址）：安裝畫面開在新分頁
    n0 = tab_count()
    link = next((n for n in find_all(page_tree(), lambda n: n.type == "HyperlinkControl" and "安裝連結" in n.name)), None)
    if link is not None:
        click(*link.center, wait=4.0)
    st = bar_state()
    check("點網頁上的腳本連結：安裝畫面開在新分頁，原本的網頁還在", link is not None and tab_count() == n0 + 1 and "chrome://extensions/install" in (st.get("shown") or ""), f"{link} {n0} → {tab_count()} {st.get('shown')!r}")
    try:
        vals = json.load(open(os.path.join(data_dir, "web-scripts", "values", "t4.user.js.json"), encoding="utf-8"))
    except Exception as e:
        vals = {"讀不到": repr(e)}
    check("設定值有存進檔案（關掉程式再開還在）", vals.get("shared") == json.dumps(val2), f"{vals}")
    ensure_web_fg()
    restore_tabs(keep)
    home()
    user32.GetAsyncKeyState(0x04)


def s_minimize():
    home()
    click_bar(BTN_MIN, (W() - 115, 16), wait=1.2)
    check("按縮小：縮到工作列", user32.IsIconic(web))
    user32.ShowWindow(web, 9)
    time.sleep(1.2)
    check("從工作列還原", not user32.IsIconic(web) and user32.IsWindowVisible(web))
    r, c = wrect(web), client(web)
    check("還原後：大小沒變", (c[2], c[3]) == (round(SIZE0[0] * scale), round(SIZE0[1] * scale)), f"rect={r} client={c}")


def s_close():
    click_bar(BTN_CLOSE, (W() - 23, 16), wait=2.0)
    check("按關閉：視窗關掉", not user32.IsWindow(web) or not user32.IsWindowVisible(web))
    time.sleep(1.5)
    try:
        saved = json.load(open(os.path.join(data_dir, "web.json"), encoding="utf-8"))
        w = (saved.get("wins") or [{}])[0]
        check("關閉後：記住是關著的、分頁都還在", w.get("open") is False and len(w.get("tabs", [])) >= 3, f"open={w.get('open')} tabs={[t.get('title') for t in w.get('tabs', [])]}")
    except Exception as e:
        log("讀 web.json 失敗", e)
    main = next((h for h in app_windows() if title(h) == "工具箱"), 0)
    check("關掉瀏覽器視窗後：主程式還在、有回應", bool(main) and responsive(main), f"main={main}")


STEPS = [
    ("start", s_start),
    ("chrome_sync", s_chrome_sync),
    ("max_button", s_max_button),
    ("restore_button", s_restore_button),
    ("snap_resize", s_snap_resize),
    ("double_click", s_double_click),
    ("drag_snap", s_drag_snap),
    ("tabs", s_tabs),
    ("bm_menu", s_bm_menu),
    ("new_tab", s_new_tab),
    ("omnibox", s_omnibox),
    ("main_menu", s_main_menu),
    ("context", s_context),
    ("internal_pages", s_internal_pages),
    ("shortcuts", s_shortcuts),
    ("bubbles", s_bubbles),
    ("download", s_download),
    ("incognito", s_incognito),
    ("new_window", s_new_window),
    ("game", s_game),
    ("notitle", s_notitle),
    ("notepad", s_notepad),
    ("dpi", s_dpi),
    ("fullscreen", s_fullscreen),
    ("ext", s_ext),
    ("scripts", s_scripts),
    ("userscripts", s_userscripts),
    ("login", s_login),
    ("devtools", s_devtools),
    ("refocus", s_refocus),
    ("tips", s_tipdiag),
    ("minimize", s_minimize),
    ("close", s_close),
]
# 只跑其中幾步：寫在命令列，或旁邊的 only.txt（空白分隔）；start 和 close 一定會跑
ONLY = [a for a in sys.argv[2:] if a]
try:
    ONLY += open(os.path.join(HERE, "only.txt"), encoding="utf-8").read().split()
except Exception:
    pass
# 主畫面那一步：讀主畫面的元素非常慢（會讓整輪超過時間），只有指名要跑時才跑
if "maindiag" in ONLY:
    STEPS.insert(len(STEPS) - 2, ("maindiag", s_maindiag))
# 查 x.com 卡住：要連到外面的網站，平常不跑
if "xcom" in ONLY:
    STEPS.insert(len(STEPS) - 2, ("xcom", s_xcom))
if "xprobe" in ONLY:
    STEPS.insert(len(STEPS) - 2, ("xprobe", s_xprobe))
if "xuser" in ONLY:
    STEPS.insert(len(STEPS) - 2, ("xuser", s_xuser))
if "extprobe" in ONLY:
    STEPS.insert(len(STEPS) - 2, ("extprobe", s_extprobe))
for name, fn in STEPS:
    if ONLY and name not in ONLY and name not in ("start", "close"):
        continue
    step(name, fn)

try:
    with open(os.path.join(OUT, "app_stderr_copy.txt"), "w", encoding="utf-8") as f:
        f.write(stderr_text())
except Exception:
    pass
log("\n===== 結果 =====")
for name, ok, detail in RESULTS:
    log(("PASS" if ok else "FAIL"), name, detail)
log(f"{sum(1 for r in RESULTS if r[1])}/{len(RESULTS)} passed")
fails = [r for r in RESULTS if not r[1]]
with open(os.path.join(OUT, "summary.txt"), "w", encoding="utf-8") as f:
    f.write(f"{sum(1 for r in RESULTS if r[1])}/{len(RESULTS)} passed\n")
    for name, ok, detail in fails:
        f.write(f"FAIL {name} :: {detail}\n")
log("\n----- page log -----\n" + page_log())
log("\n----- game keys -----\n" + game_keys())
for fn in ("web.json", "web-history.json", "web-downloads.json", "web-ext.json"):
    try:
        shutil.copy(os.path.join(data_dir, fn), os.path.join(OUT, "data-" + fn))
    except Exception:
        pass
subprocess.run(["taskkill", "/IM", os.path.basename(EXE), "/F"], capture_output=True)
game_proc.kill()
os._exit(0)
