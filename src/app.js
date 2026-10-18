"use strict";

/* ---------- 後端橋接 ---------- */
// 在 Tauri 中呼叫 Rust 指令；直接用瀏覽器開啟時使用模擬後端，方便預覽介面。
const TAURI = window.__TAURI__;
const invoke = TAURI ? TAURI.core.invoke : null;
const listen = TAURI ? TAURI.event.listen : () => {};

const api = TAURI ? {
  getState: () => invoke("get_state"),
  extract: (raw) => invoke("extract", { raw }),
  setIndex: (index) => invoke("set_index", { index }),
  setActive: (active) => invoke("set_active", { active }),
  copy: (text) => invoke("copy_text", { text }),
  clear: () => invoke("clear_list"),
  saveSettings: (settings) => invoke("save_settings", { settings }),
  onChange: (cb) => listen("state-changed", (e) => cb(e.payload)),
  setWindowTheme: (t) => { try { TAURI.window.getCurrentWindow().setTheme(t); } catch (_) {} },
} : createMockApi();

function createMockApi() {
  const KEY = "toolbox-mock";
  let st;
  try { st = JSON.parse(localStorage.getItem(KEY)); } catch (_) {}
  st = st || { raw: "", serials: [], index: 0, active: false, settings: { theme: "system", font: "m" } };
  st.active = false;
  const save = () => { try { localStorage.setItem(KEY, JSON.stringify(st)); } catch (_) {} return { ...st }; };
  const extract = (text) => {
    const out = [];
    for (const s of text.match(/[A-Za-z0-9]+/g) || []) {
      if (s.length >= 15 && s.length <= 25 && !/^\d+$/.test(s) && !out.includes(s)) out.push(s);
    }
    return out;
  };
  return {
    getState: async () => ({ ...st }),
    extract: async (raw) => { st.raw = raw; st.serials = extract(raw); st.index = 0; st.active = st.serials.length > 0; return save(); },
    setIndex: async (i) => { st.index = Math.min(i, st.serials.length); return save(); },
    setActive: async (a) => { st.active = a && st.serials.length > 0; return save(); },
    copy: async (t) => { await navigator.clipboard?.writeText(t); },
    clear: async () => { Object.assign(st, { raw: "", serials: [], index: 0, active: false }); return save(); },
    saveSettings: async (s) => { st.settings = s; save(); },
    onChange: () => {},
    setWindowTheme: () => {},
  };
}

/* ---------- 共用工具 ---------- */
const $ = (sel, root = document) => root.querySelector(sel);
const $$ = (sel, root = document) => [...root.querySelectorAll(sel)];
const root = document.documentElement;
const darkQuery = window.matchMedia("(prefers-color-scheme: dark)");

let toastTimer;
function toast(msg) {
  const el = $("#toast");
  el.textContent = msg;
  el.classList.add("show");
  clearTimeout(toastTimer);
  toastTimer = setTimeout(() => el.classList.remove("show"), 1800);
}

async function run(fn) {
  try {
    return await fn();
  } catch (e) {
    toast(String(e));
  }
}

/** 確認對話框（倉儲管理器樣式：是／否），回傳 Promise<boolean> */
function confirmBox(message) {
  const back = $("#alertBackdrop");
  $("#alertMsg").textContent = message;
  const okBtn = $("#alertOk");
  const cancelBtn = $("#alertCancel");
  back.classList.add("show");
  return new Promise((resolve) => {
    const done = (v) => {
      back.classList.remove("show");
      okBtn.onclick = cancelBtn.onclick = back.onclick = null;
      document.removeEventListener("keydown", onKey, true);
      resolve(v);
    };
    const onKey = (e) => {
      if (e.key === "Escape") { e.preventDefault(); done(false); }
      if (e.key === "Enter") { e.preventDefault(); done(true); }
    };
    okBtn.onclick = () => done(true);
    cancelBtn.onclick = () => done(false);
    back.onclick = (e) => { if (e.target === back) done(false); };
    document.addEventListener("keydown", onKey, true);
    setTimeout(() => okBtn.focus(), 30);
  });
}

const store = {
  get(k, d) { try { const v = localStorage.getItem(k); return v === null ? d : JSON.parse(v); } catch (_) { return d; } },
  set(k, v) { try { localStorage.setItem(k, JSON.stringify(v)); } catch (_) {} },
};

// 懸停說明：靠近視窗底部時改顯示在上面，不會被切掉
document.addEventListener("mouseover", (e) => {
  const el = e.target.closest?.("[data-tip]");
  if (el) el.classList.toggle("tip-up", el.getBoundingClientRect().bottom > window.innerHeight - 120);
});

// 桌面程式不需要重新整理頁面，避免誤按 F5 / Ctrl+R 讓畫面重載
document.addEventListener("keydown", (e) => {
  if (e.key === "F5" || ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === "r")) e.preventDefault();
});

/* ---------- 滾輪：每格捲動距離縮小 ---------- */
const WHEEL_STEP = 48; // 每一格滾輪捲動的像素
function scrollableParent(el) {
  for (; el && el !== document.body; el = el.parentElement) {
    const oy = getComputedStyle(el).overflowY;
    if ((oy === "auto" || oy === "scroll") && el.scrollHeight > el.clientHeight + 1) return el;
  }
  return null;
}
document.addEventListener("wheel", (e) => {
  if (e.ctrlKey || e.deltaY === 0 || e.defaultPrevented) return;
  // 數字輸入框、下拉選單保留原本的滾輪行為
  if (e.target.closest("select")) return;
  const box = scrollableParent(e.target);
  if (!box) return;
  e.preventDefault();
  // 觸控板的細微捲動照原比例，滑鼠滾輪每格固定 WHEEL_STEP
  const px = e.deltaMode === 1 ? e.deltaY * 16 : e.deltaY;
  const step = Math.abs(px) >= 50 ? Math.sign(px) * WHEEL_STEP : px * 0.6;
  box.scrollTop += step;
}, { passive: false });

/* ---------- 外觀設定（齒輪） ---------- */
let settings = { theme: "system", font: "m" };
// 啟動時讀完設定才決定要不要自動檢查更新
let settingsLoaded;
const settingsReady = new Promise((r) => (settingsLoaded = r));

const isCharTheme = () => String(settings.theme).startsWith("char");
function effectiveTheme() {
  const t = settings.theme;
  if (t === "light" || t === "char-light") return "light";
  if (t === "dark" || t === "char-dark") return "dark";
  return darkQuery.matches ? "dark" : "light";
}

/* ---------- 主題「角色色彩」：從側欄紙娃娃取主色，套用到程式與視窗外框 ---------- */
const TINT_VARS = ["--bg", "--side", "--card", "--card-2", "--text-2", "--sep", "--blue", "--blue-soft", "--glass"];
function rgbToHsl(r, g, b) {
  r /= 255; g /= 255; b /= 255;
  const max = Math.max(r, g, b), min = Math.min(r, g, b), l = (max + min) / 2;
  if (max === min) return [0, 0, l];
  const d = max - min, s = l > 0.5 ? d / (2 - max - min) : d / (max + min);
  const h = max === r ? (g - b) / d + (g < b ? 6 : 0) : max === g ? (b - r) / d + 2 : (r - g) / d + 4;
  return [h * 60, s, l];
}
/* OKLCH：亮度一致時色相看起來比較準，深色背景不會把粉色壓成咖啡色 */
const _lin = (c) => { c /= 255; return c <= 0.04045 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4; };
const _gam = (c) => 255 * (c <= 0.0031308 ? 12.92 * c : 1.055 * c ** (1 / 2.4) - 0.055);
function rgbToOklch(r, g, b) {
  r = _lin(r); g = _lin(g); b = _lin(b);
  const l = Math.cbrt(0.4122214708 * r + 0.5363325363 * g + 0.0514459929 * b);
  const m = Math.cbrt(0.2119034982 * r + 0.6806995451 * g + 0.1073969566 * b);
  const s = Math.cbrt(0.0883024619 * r + 0.2817188376 * g + 0.6299787005 * b);
  const L = 0.2104542553 * l + 0.793617785 * m - 0.0040720468 * s;
  const A = 1.9779984951 * l - 2.428592205 * m + 0.4505937099 * s;
  const B = 0.0259040371 * l + 0.7827717662 * m - 0.808675766 * s;
  return [L, Math.hypot(A, B), ((Math.atan2(B, A) * 180) / Math.PI + 360) % 360];
}
function oklchToRgb(L, C, H) {
  // 超出螢幕色域時慢慢降低彩度
  for (let c = C; ; c *= 0.92) {
    const A = c * Math.cos((H * Math.PI) / 180), B = c * Math.sin((H * Math.PI) / 180);
    const l = (L + 0.3963377774 * A + 0.2158037573 * B) ** 3;
    const m = (L - 0.1055613458 * A - 0.0638541728 * B) ** 3;
    const s = (L - 0.0894841775 * A - 1.291485548 * B) ** 3;
    const rgb = [
      4.0767416621 * l - 3.3077085802 * m + 0.2309699292 * s,
      -1.2684380046 * l + 2.6097574011 * m - 0.3413193965 * s,
      -0.0041960863 * l - 0.7034186147 * m + 1.707614701 * s,
    ];
    if (c < 0.002 || rgb.every((v) => v >= -0.001 && v <= 1.001)) return rgb.map((v) => Math.round(Math.max(0, Math.min(255, _gam(Math.max(0, v))))));
  }
}
const oklchHex = (L, C, H) => "#" + oklchToRgb(L, C, H).map((x) => x.toString(16).padStart(2, "0")).join("");
const oklchA = (L, C, H, a) => `rgba(${oklchToRgb(L, C, H).join(", ")}, ${a})`;

let tintCache = { src: null, hsl: null };
/**
 * 紙娃娃最有代表性的顏色，回傳 OKLCH [亮度, 彩度, 色相]
 * - 淡色（粉彩）也要算進來，不然淡粉色角色只會取到深色描邊
 * - 找出最多的色相後，把附近 ±35° 的像素平均，取得角色真正看起來的顏色
 */
function dollAccent() {
  const img = document.getElementById("sideDollImg");
  if (!img || img.hidden || !img.complete || !img.naturalWidth) return null;
  if (tintCache.src === img.src) return tintCache.hsl;
  let hsl = null;
  try {
    const scale = Math.min(1, 128 / Math.max(img.naturalWidth, img.naturalHeight));
    const w = Math.max(1, Math.round(img.naturalWidth * scale)), h = Math.max(1, Math.round(img.naturalHeight * scale));
    const cv = document.createElement("canvas");
    cv.width = w; cv.height = h;
    const ctx = cv.getContext("2d", { willReadFrequently: true });
    ctx.imageSmoothingEnabled = false;
    ctx.drawImage(img, 0, 0, w, h);
    const px = ctx.getImageData(0, 0, w, h).data;
    const bins = new Float64Array(36);
    const pts = [];
    let total = 0;
    for (let i = 0; i < px.length; i += 4) {
      if (px[i + 3] < 200) continue;
      const [hh, ss, ll] = rgbToHsl(px[i], px[i + 1], px[i + 2]);
      const chroma = ss * (1 - Math.abs(2 * ll - 1));
      if (chroma < 0.05 || ll > 0.97 || ll < 0.08) continue;
      const wgt = Math.sqrt(chroma); // 開根號：淡色的面積也算數
      bins[Math.floor(hh / 10) % 36] += wgt;
      pts.push(hh, wgt, i);
      total += wgt;
    }
    // 相鄰色相（±20°）合在一起比，挑最多的
    let best = -1, bestW = 0;
    for (let i = 0; i < 36; i++) {
      const s5 = bins[(i + 34) % 36] * 0.25 + bins[(i + 35) % 36] * 0.5 + bins[i] + bins[(i + 1) % 36] * 0.5 + bins[(i + 2) % 36] * 0.25;
      if (s5 > bestW) { bestW = s5; best = i; }
    }
    if (best >= 0 && total > 4) {
      const ctr = best * 10 + 5;
      let sw = 0, r = 0, g = 0, b = 0;
      for (let k = 0; k < pts.length; k += 3) {
        const d = ((pts[k] - ctr + 540) % 360) - 180;
        if (Math.abs(d) > 35) continue;
        const wgt = pts[k + 1], i = pts[k + 2];
        sw += wgt; r += _lin(px[i]) * wgt; g += _lin(px[i + 1]) * wgt; b += _lin(px[i + 2]) * wgt;
      }
      if (sw > 0) hsl = rgbToOklch(_gam(r / sw), _gam(g / sw), _gam(b / sw));
    }
  } catch (_) { hsl = null; } // 官方角色圖（外部網址）無法讀取像素
  tintCache = { src: img.src, hsl };
  return hsl;
}
/** 套用角色色彩；回傳視窗外框要用的顏色 */
function applyTint(dark) {
  const acc = isCharTheme() ? dollAccent() : null;
  if (!acc) { TINT_VARS.forEach((v) => root.style.removeProperty(v)); return null; }
  const [, ac, ah] = acc;
  // 深色底時紅、粉色會看起來像咖啡色，底色稍微往紫紅偏一點
  let h = ah;
  if (dark && ((h - 345 + 360) % 360) < 55) h = (h - 12 + 360) % 360; // 345°～40°
  // 底色的彩度跟著角色：粉彩角色淡淡的，鮮豔角色濃一點
  const bc = Math.min(0.045, 0.02 + ac * 0.25);
  const c = (k) => bc * k;
  const accL = dark ? 0.7 : 0.56; // 按鈕底色要能放白字
  const accC = Math.min(0.17, Math.max(0.09, ac * 1.4)); // 粉彩角色的強調色也要看得出顏色
  const vars = dark
    ? { "--bg": oklchHex(0.215, c(0.9), h), "--side": oklchHex(0.245, c(1), h), "--card": oklchHex(0.28, c(0.9), h), "--card-2": oklchHex(0.32, c(0.85), h), "--text-2": oklchHex(0.76, c(0.8), h), "--sep": oklchA(0.85, c(1.4), h, 0.14), "--glass": oklchA(0.215, c(0.9), h, 0.86) }
    : { "--bg": oklchHex(0.965, c(0.35), h), "--side": oklchHex(0.978, c(0.25), h), "--card": "#ffffff", "--card-2": oklchHex(0.945, c(0.4), h), "--text-2": oklchHex(0.5, c(0.8), h), "--sep": oklchA(0.4, c(1), h, 0.14), "--glass": oklchA(0.965, c(0.35), h, 0.8) };
  vars["--blue"] = oklchHex(accL, accC, ah);
  vars["--blue-soft"] = oklchA(accL, accC, ah, dark ? 0.2 : 0.14);
  for (const [k, v] of Object.entries(vars)) root.style.setProperty(k, v);
  return { caption: dark ? oklchHex(0.245, c(1), h) : oklchHex(0.93, c(0.6), h), border: oklchHex(accL, accC, ah) };
}

function applySettings() {
  const theme = effectiveTheme();
  root.dataset.theme = theme;
  root.dataset.font = settings.font;
  $$("#fontSeg button").forEach((b) => b.classList.toggle("on", b.dataset.font === settings.font));
  $$("#themeSeg button").forEach((b) => b.classList.toggle("on", b.dataset.theme === (isCharTheme() ? settings.theme : theme)));
  $$("#closeSeg button").forEach((b) => b.classList.toggle("on", b.dataset.v === (settings.close_tray ? "tray" : "exit")));
  $$("#updSeg button").forEach((b) => b.classList.toggle("on", b.dataset.v === (settings.upd_manual ? "manual" : "auto")));
  api.setWindowTheme(theme);
  const tint = applyTint(theme === "dark");
  // 視窗外框（標題列與邊框）換成和程式內容有區別的顏色；角色色彩時跟著角色
  if (TAURI) invoke("set_frame", { dark: theme === "dark", caption: tint?.caption ?? null, border: tint?.border ?? null }).catch(() => {});
}

function saveSettings() {
  applySettings();
  api.saveSettings({ theme: settings.theme, font: settings.font, close_tray: !!settings.close_tray, close_asked: !!settings.close_asked, upd_manual: !!settings.upd_manual });
}

$$("#fontSeg button").forEach((b) => b.addEventListener("click", () => { settings.font = b.dataset.font; saveSettings(); }));
$$("#themeSeg button").forEach((b) => b.addEventListener("click", () => { settings.theme = b.dataset.theme; saveSettings(); }));
darkQuery.addEventListener("change", () => { if (settings.theme === "system") applySettings(); });

$$("#closeSeg button").forEach((b) => b.addEventListener("click", () => { settings.close_tray = b.dataset.v === "tray"; settings.close_asked = true; saveSettings(); }));
$$("#updSeg button").forEach((b) => b.addEventListener("click", () => { settings.upd_manual = b.dataset.v === "manual"; saveSettings(); window.Updater?.mode(); }));

/* 第一次按右上角關閉：問要直接關閉還是縮小到右下角（只問一次，選了就照做） */
{
  const box = $("#closeAsk");
  const hide = () => box.classList.remove("show");
  const pick = (tray) => {
    hide();
    settings.close_tray = tray;
    settings.close_asked = true;
    applySettings();
    if (TAURI) invoke("close_choice", { tray }).catch(() => {});
  };
  $("#closeAskTray").addEventListener("click", () => pick(true));
  $("#closeAskExit").addEventListener("click", () => pick(false));
  box.addEventListener("click", (e) => { if (e.target === box) hide(); });
  document.addEventListener("keydown", (e) => { if (e.key === "Escape" && box.classList.contains("show")) hide(); });
  listen("ask-close", () => { box.classList.add("show"); setTimeout(() => $("#closeAskTray").focus(), 30); });
}

/* 開機自動啟動（寫在 Windows 的開機啟動清單，以實際狀態為準） */
{
  const box = $("#autoStart");
  if (!TAURI) box.disabled = true;
  else invoke("autostart_get").then((on) => (box.checked = !!on)).catch(() => {});
  box.addEventListener("change", async () => {
    if (!TAURI) return;
    const want = box.checked;
    try {
      box.checked = await invoke("autostart_set", { on: want });
      toast(box.checked ? "已開啟開機自動啟動" : "已關閉開機自動啟動");
    } catch (e) {
      box.checked = !want;
      toast(String(e));
    }
  });
}

/* 設定：在右邊內容區顯示整頁 */
$("#gearBtn").addEventListener("click", () => {
  $$(".cat").forEach((b) => b.classList.remove("on"));
  showView("settings");
  $("#app").classList.remove("collapsed");
});

/* 主內容區不應該被捲動（切換頁面的滑入動畫中點擊或聚焦輸入框時，瀏覽器會把它捲過去，造成整頁偏移） */
{
  const main = document.querySelector("main.content");
  if (main) main.addEventListener("scroll", () => { if (main.scrollLeft || main.scrollTop) main.scrollLeft = main.scrollTop = 0; });
}

/* ---------- 道具圖示（依名稱從遊戲檔找，黃金蘋果、閃亮彗星共用） ---------- */
/** BOSS 結晶圖示：每週、每月用內建的圖；每日王的結晶讀遊戲資料（強烈的力量結晶） */
const CRYSTAL = { week: "assets/crystal/weekly.png", month: "assets/crystal/monthly.png", dayItem: "強烈的力量結晶" };

const ItemIcons = (() => {
  const cache = new Map(); // 名稱 → data URL 或 null
  const want = new Set();
  const live = new Set(); // 等待圖示的元素
  const waiters = new Map(); // 名稱 → 等待結果的 Promise resolve
  let timer = null, busy = false;
  const gameDir = () => { try { return JSON.parse(localStorage.getItem("beauty-v1"))?.gamePath || null; } catch (_) { return null; } };

  function fill(el) {
    const name = el.dataset.name;
    if (!cache.has(name)) return false;
    const png = cache.get(name);
    el.innerHTML = "";
    if (png) { const img = new Image(); img.src = png; img.alt = ""; img.draggable = false; el.append(img); el.classList.remove("none"); }
    else el.classList.add("none");
    return true;
  }
  async function flush() {
    timer = null;
    if (busy || !TAURI) return;
    const names = [...want].filter((n) => !cache.has(n)).slice(0, 60);
    names.forEach((n) => want.delete(n));
    if (!names.length) return;
    busy = true;
    try {
      const pngs = await invoke("item_icons", { gameDir: gameDir(), names });
      names.forEach((n, i) => cache.set(n, pngs[i] || null));
    } catch (_) { names.forEach((n) => cache.set(n, null)); }
    busy = false;
    for (const n of names) { for (const r of waiters.get(n) || []) r(cache.get(n)); waiters.delete(n); }
    for (const el of [...live]) if (!el.isConnected || fill(el)) live.delete(el);
    if (want.size) flush();
  }
  /** 回傳一個圖示格子，圖示讀到後自動填入 */
  function el(name) {
    const span = document.createElement("span");
    span.className = "item-ic";
    span.dataset.name = name;
    if (!fill(span)) {
      live.add(span);
      want.add(name);
      if (!timer) timer = setTimeout(flush, 30);
    }
    return span;
  }
  /** 取得圖示的 data URL（沒有遊戲資料或找不到時為 null） */
  function get(name) {
    if (cache.has(name)) return Promise.resolve(cache.get(name));
    if (!TAURI) return Promise.resolve(null);
    return new Promise((resolve) => {
      if (!waiters.has(name)) waiters.set(name, []);
      waiters.get(name).push(resolve);
      want.add(name);
      if (!timer) timer = setTimeout(flush, 30);
    });
  }
  return { el, get };
})();
window.ItemIcons = ItemIcons;

/* ---------- 分類與導覽 ---------- */
const ICONS = {
  clock: '<svg viewBox="0 0 24 24"><circle cx="12" cy="13" r="8"/><path d="M12 9v4l2.5 2.5M9 2h6"/></svg>',
  scissors: '<svg viewBox="0 0 24 24"><circle cx="6" cy="6" r="3"/><circle cx="6" cy="18" r="3"/><path d="M20 4L8.1 15.9M14.5 14.5L20 20M8.1 8.1L12 12"/></svg>',
  apple: '<svg viewBox="0 0 24 24"><path d="M12 7c-1.5-1.3-3.8-1.6-5.4-.4C4.3 8.3 4.6 12.6 6.5 16c1.2 2.2 2.7 4 4.3 3.6.6-.1.9-.4 1.2-.4s.6.3 1.2.4c1.6.4 3.1-1.4 4.3-3.6 1.9-3.4 2.2-7.7-.1-9.4C15.8 5.4 13.5 5.7 12 7z"/><path d="M12 7c0-2 1-3.5 3-4"/></svg>',
  comet: '<svg viewBox="0 0 24 24"><path d="M16.5 4.5l1.2 2.6 2.8.4-2 2 .5 2.8-2.5-1.3-2.5 1.3.5-2.8-2-2 2.8-.4z"/><path d="M11 10.5 4 17.5M13 13.5 7.5 19M8.5 9 3.5 14"/></svg>',
  plug: '<svg viewBox="0 0 24 24"><path d="M2 8.8a15 15 0 0 1 20 0"/><path d="M5.5 12.5a10 10 0 0 1 13 0"/><path d="M9 16.2a5 5 0 0 1 6 0"/><circle cx="12" cy="19.5" r="1"/><path d="M3 3l18 18"/></svg>',
  star: '<svg viewBox="0 0 24 24"><path d="M12 3.5l2.4 4.9 5.4.8-3.9 3.8.9 5.4-4.8-2.5-4.8 2.5.9-5.4-3.9-3.8 5.4-.8z"/><path d="M19 3v3M17.5 4.5h3"/></svg>',
  gift: '<svg viewBox="0 0 24 24"><rect x="3.5" y="8" width="17" height="4" rx="1"/><path d="M5 12v7.5a1 1 0 0 0 1 1h12a1 1 0 0 0 1-1V12M12 8v12.5M12 8c-1.5-3.5-6-4-6-1.5S10 8 12 8zm0 0c1.5-3.5 6-4 6-1.5S14 8 12 8z"/></svg>',
  copy: '<svg viewBox="0 0 24 24"><rect x="8" y="8" width="12" height="12" rx="3"/><path d="M16 8V6a2 2 0 0 0-2-2H6a2 2 0 0 0-2 2v8a2 2 0 0 0 2 2h2"/></svg>',
  crystal: '<svg viewBox="0 0 24 24"><path d="M12 2.5 18.5 9 12 21.5 5.5 9z"/><path d="M5.5 9h13M9 9l3 12.5L15 9M9 9l3-6.5L15 9"/></svg>',
  trend: '<svg viewBox="0 0 24 24"><path d="M3 17l6-6 4 4 8-8"/><path d="M15 7h6v6"/></svg>',
  news: '<svg viewBox="0 0 24 24"><path fill-rule="evenodd" d="M6.5 3h11A2.5 2.5 0 0 1 20 5.5v13a2.5 2.5 0 0 1-2.5 2.5h-11A2.5 2.5 0 0 1 4 18.5v-13A2.5 2.5 0 0 1 6.5 3zM8 7.2v1.7h8V7.2zm0 4v1.7h8v-1.7zm0 4v1.7h5v-1.7z"/></svg>',
};

// 滑過磁磚時的光暈顏色（與圖示漸層同色系）
const TILE_ACCENT = { red: "#ff5a5f", purple: "#9b5cf2", pink: "#ff5fa8", gold: "#ffb000", comet: "#4fb8ff", star: "#a78bfa", blue: "#3d7bff", green: "#30c26a", orange: "#ff7a2f", crystal: "#7c5cff", teal: "#14b8a6", indigo: "#5865f2" };

const CATS = {
  maple: {
    title: "楓之谷工具",
    eyebrow: "MapleStory",
    tools: [
      { id: "gacha", name: "黃金蘋果", desc: "獎池解析、模擬抽獎與期望值分析", color: "gold", icon: "apple", item: "黃金蘋果" },
      { id: "comet", name: "閃亮彗星", desc: "模擬抽獎、分解碎片與兌換璀璨彗星", color: "comet", icon: "comet", item: "閃亮彗星" },
      { id: "beauty", name: "美容美髮", desc: "載入角色造型，模擬皇家美髮／整形", color: "pink", icon: "scissors", item: "皇家美髮券(選擇型)" },
      { id: "star", name: "星光錦囊", desc: "套娃抽獎：玲瓏星光合成、層層開啟與期望值", color: "star", icon: "star", item: "星光錦囊" },
      { id: "boss", name: "BOSS 結晶", desc: "勾選每隻角色打的 BOSS，算出結晶每週能賣多少楓幣", color: "crystal", icon: "crystal", img: "assets/crystal/monthly.png" },
    ],
  },
  tools: {
    title: "工具箱",
    eyebrow: "Toolbox",
    tools: [
      { id: "timer", name: "計時器", desc: "道具／技能倒數與浮動視窗", color: "purple", icon: "clock" },
      { id: "serial", name: "序號複製", desc: "擷取序號，Ctrl+V 後自動換下一組", color: "blue", icon: "copy" },
      { id: "watch", name: "斷線通知", desc: "新楓之谷斷線、回到登入畫面或關閉時通知", color: "red", icon: "plug" },
      { id: "gift", name: "送禮買賣", desc: "買賣送禮試算、儲值成本與 VIP 回饋；楓幣台幣換算", color: "green", icon: "gift" },
      { id: "track", name: "經驗追蹤", desc: "每隻角色今天練了多少、近 30 天平均與預計升級日", color: "indigo", icon: "trend" },
      { id: "patch", name: "版本更新", desc: "新楓之谷歷次大版本更新的官網公告，依年份查看", color: "orange", icon: "news" },
    ],
  },
  // 我的最愛：內容由使用者收藏決定
  fav: { title: "我的最愛", eyebrow: "Favorites", tools: [] },
};

const TOOL = {};
for (const [cat, c] of Object.entries(CATS)) c.tools.forEach((t, i) => (TOOL[t.id] = { ...t, cat, def: i }));

let currentCat = store.get("toolbox-cat", "maple");
if (!CATS[currentCat]) currentCat = "maple";

/* 自訂排序（每個分類各自一份）與我的最愛 */
let tileOrder = store.get("toolbox-order", {}) || {};
let favs = (store.get("toolbox-favs", []) || []).filter((id) => TOOL[id]);
const saveFavs = () => store.set("toolbox-favs", favs);
const isFav = (id) => favs.includes(id);

/** 依自訂順序排列；沒排過的（例如新加入的工具）放回它預設的位置 */
function orderedTools(cat) {
  const ids = CATS[cat].tools.map((t) => t.id);
  const out = (tileOrder[cat] || []).filter((id) => ids.includes(id));
  for (const id of ids) if (!out.includes(id)) out.splice(Math.min(TOOL[id].def, out.length), 0, id);
  return out.map((id) => TOOL[id]);
}

/** 工具圖示：有對應的遊戲道具時換成道具圖示（讀不到就保留原本的圖示）；item 可以是好幾個名稱，依序找 */
async function firstIcon(names) {
  for (const n of [].concat(names)) { const png = await ItemIcons.get(n); if (png) return png; }
  return null;
}
function toolIcon(t) {
  const box = document.createElement("span");
  box.className = `cat-icon ${t.color}`;
  box.style.setProperty("--accent", TILE_ACCENT[t.color] || "#4c8dff");
  box.innerHTML = ICONS[t.icon];
  // 內建的道具圖（例如 BOSS 結晶）
  if (t.img) {
    box.classList.add("item");
    box.innerHTML = "";
    const img = new Image();
    img.src = t.img;
    img.alt = "";
    img.draggable = false;
    box.append(img);
    return box;
  }
  if (t.item) firstIcon(t.item).then((png) => {
    if (!png) return;
    box.classList.add("item");
    box.innerHTML = "";
    const img = new Image();
    img.src = png;
    img.alt = "";
    img.draggable = false;
    box.append(img);
  });
  return box;
}

function addFav(id) {
  if (isFav(id)) return;
  favs.push(id);
  saveFavs();
  renderFavCount();
  if (currentCat === "fav") renderTiles();
  toast(`已加入我的最愛：${TOOL[id].name}`);
}
function removeFav(id) {
  favs = favs.filter((x) => x !== id);
  saveFavs();
  renderFavCount();
  if (currentCat === "fav") renderTiles();
  toast(`已從我的最愛移除：${TOOL[id].name}`);
}
function renderFavCount() {
  const c = $("#favCount");
  c.textContent = favs.length || "";
  c.hidden = !favs.length;
}

function renderTiles() {
  const tiles = $("#tiles");
  tiles.innerHTML = "";
  const isFavPage = currentCat === "fav";
  const list = isFavPage ? favs.map((id) => TOOL[id]) : orderedTools(currentCat);
  $("#favEmpty").hidden = !(isFavPage && !list.length);
  // 沒有收藏時，空白提示緊接在標題下面
  tiles.style.display = isFavPage && !list.length ? "none" : "";
  $("#catHint").hidden = !isFavPage;
  for (const t of list) {
    const btn = document.createElement("button");
    btn.className = "tile";
    btn.dataset.id = t.id;
    btn.style.setProperty("--accent", TILE_ACCENT[t.color] || "#4c8dff");
    const strong = document.createElement("strong");
    strong.textContent = t.name;
    const small = document.createElement("small");
    small.textContent = t.desc;
    const inner = document.createElement("span");
    inner.className = "tile-in";
    inner.append(toolIcon(t), strong, small);
    btn.append(inner);
    btn.addEventListener("click", () => openTool(t.id));
    btn.addEventListener("contextmenu", (e) => {
      e.preventDefault();
      const opts = [
        isFav(t.id) ? ["移除我的最愛", () => removeFav(t.id)] : ["加入我的最愛", () => addFav(t.id)],
        ["開啟", () => openTool(t.id)],
      ];
      if (!isFavPage && tileOrder[currentCat]) opts.push(["恢復預設排序", () => { delete tileOrder[currentCat]; store.set("toolbox-order", tileOrder); renderTiles(); }]);
      showCtx(e.clientX, e.clientY, t.name, opts);
    });
    dragTool(btn, t.id);
    tiles.appendChild(btn);
  }
}

function showCat(cat) {
  currentCat = cat;
  store.set("toolbox-cat", cat);
  const c = CATS[cat];
  $$(".cat").forEach((b) => b.classList.toggle("on", b.dataset.cat === cat));
  $("#catTitle").textContent = c.title;
  $("#catEyebrow").textContent = c.eyebrow;
  renderTiles();
  showView("cat");
  $("#app").classList.remove("collapsed");
}

/* ---------- 右鍵選單 ---------- */
const ctxMenu = document.createElement("div");
ctxMenu.className = "b-ctx";
ctxMenu.hidden = true;
document.body.append(ctxMenu);
function showCtx(x, y, title, opts) {
  ctxMenu.innerHTML = "";
  const h = document.createElement("div");
  h.className = "b-ctx-title";
  h.textContent = title;
  ctxMenu.append(h);
  for (const [label, fn] of opts) {
    const b = document.createElement("button");
    b.textContent = label;
    b.addEventListener("click", () => { ctxMenu.hidden = true; fn(); });
    ctxMenu.append(b);
  }
  ctxMenu.hidden = false;
  const r = ctxMenu.getBoundingClientRect();
  ctxMenu.style.left = `${Math.min(x, innerWidth - r.width - 8)}px`;
  ctxMenu.style.top = `${Math.min(y, innerHeight - r.height - 8)}px`;
}
document.addEventListener("pointerdown", (e) => { if (!ctxMenu.contains(e.target)) ctxMenu.hidden = true; });
document.addEventListener("keydown", (e) => { if (e.key === "Escape") ctxMenu.hidden = true; });
window.addEventListener("blur", () => (ctxMenu.hidden = true));

/* ---------- 拖曳：磁磚排序；拖到左邊的「我的最愛」= 收藏 ---------- */
/** 移動 DOM 後，讓其他項目滑到新位置 */
function flip(container, mutate) {
  const kids = [...container.children];
  const before = new Map(kids.map((k) => [k, k.getBoundingClientRect()]));
  mutate();
  for (const k of kids) {
    const a = before.get(k), b = k.getBoundingClientRect();
    const dx = a.left - b.left, dy = a.top - b.top;
    if (dx || dy) k.animate([{ transform: `translate(${dx}px, ${dy}px)` }, { transform: "none" }], { duration: 200, easing: "cubic-bezier(0.2, 0.8, 0.2, 1)" });
  }
}

/* ---------- 名牌、對話框：依遊戲圖片的原點拼起來（和遊戲相同的畫法） ---------- */
const UiArt = (() => {
  const imgs = new Map();
  const load = (p) => {
    if (!imgs.has(p.png)) {
      imgs.set(p.png, new Promise((res) => { const im = new Image(); im.onload = () => res(im); im.onerror = () => res(null); im.src = p.png; }));
    }
    return imgs.get(p.png);
  };
  const done = new Map();
  /** 先把所有圖片載入（之後畫圖不用等） */
  async function ready(skin) {
    if (!skin?.pieces) return false;
    const list = [...Object.values(skin.pieces), ...(skin.ani || [])];
    const got = await Promise.all(list.map(load));
    list.forEach((p, i) => done.set(p.png, got[i]));
    return got.some(Boolean);
  }
  /**
   * 依序畫出每一塊：塊的原點對齊 (x, y)；clip 是可選的裁切範圍 [左, 上, 右, 下]。
   * 回傳畫布與畫布左上角的位置（和 x、y 同一個座標系）；trim = 去掉四周透明的部分
   */
  function paint(ops, trim) {
    let x0 = Infinity, y0 = Infinity, x1 = -Infinity, y1 = -Infinity;
    const rects = ops.map(({ p, x, y, clip }) => {
      let l = x - p.ox, t = y - p.oy, r = l + p.w, b = t + p.h;
      if (clip) { l = Math.max(l, clip[0]); t = Math.max(t, clip[1]); r = Math.min(r, clip[2]); b = Math.min(b, clip[3]); }
      if (r > l && b > t) { x0 = Math.min(x0, l); y0 = Math.min(y0, t); x1 = Math.max(x1, r); y1 = Math.max(y1, b); }
      return r > l && b > t;
    });
    if (!isFinite(x0)) return null;
    const cv = document.createElement("canvas");
    cv.width = x1 - x0;
    cv.height = y1 - y0;
    const g = cv.getContext("2d");
    g.imageSmoothingEnabled = false;
    ops.forEach(({ p, x, y, clip }, i) => {
      const im = done.get(p.png);
      if (!rects[i] || !im) return;
      g.save();
      if (clip) { g.beginPath(); g.rect(clip[0] - x0, clip[1] - y0, clip[2] - clip[0], clip[3] - clip[1]); g.clip(); }
      g.drawImage(im, x - p.ox - x0, y - p.oy - y0);
      g.restore();
    });
    if (!trim) return { canvas: cv, x: x0, y: y0 };
    const px = g.getImageData(0, 0, cv.width, cv.height).data;
    let tx0 = cv.width, ty0 = cv.height, tx1 = 0, ty1 = 0;
    for (let yy = 0; yy < cv.height; yy++) for (let xx = 0; xx < cv.width; xx++) {
      if (px[(yy * cv.width + xx) * 4 + 3] > 8) { tx0 = Math.min(tx0, xx); ty0 = Math.min(ty0, yy); tx1 = Math.max(tx1, xx + 1); ty1 = Math.max(ty1, yy + 1); }
    }
    if (tx1 <= tx0) return { canvas: cv, x: x0, y: y0 };
    const out = document.createElement("canvas");
    out.width = tx1 - tx0;
    out.height = ty1 - ty0;
    out.getContext("2d").drawImage(cv, -tx0, -ty0);
    return { canvas: out, x: x0 + tx0, y: y0 + ty0 };
  }
  /** 好幾組要疊在一起的圖（動畫的每一格）畫成同樣大小、同樣位置的畫布，再一起去掉四周透明的部分 */
  function paintFrames(list) {
    let x0 = Infinity, y0 = Infinity, x1 = -Infinity, y1 = -Infinity;
    for (const ops of list) for (const { p, x, y, clip } of ops) {
      let l = x - p.ox, t = y - p.oy, r = l + p.w, b = t + p.h;
      if (clip) { l = Math.max(l, clip[0]); t = Math.max(t, clip[1]); r = Math.min(r, clip[2]); b = Math.min(b, clip[3]); }
      if (r > l && b > t) { x0 = Math.min(x0, l); y0 = Math.min(y0, t); x1 = Math.max(x1, r); y1 = Math.max(y1, b); }
    }
    if (!isFinite(x0)) return null;
    const W = x1 - x0, H = y1 - y0;
    const canvases = list.map((ops) => {
      const cv = document.createElement("canvas");
      cv.width = W; cv.height = H;
      const g = cv.getContext("2d");
      g.imageSmoothingEnabled = false;
      for (const { p, x, y, clip } of ops) {
        const im = done.get(p.png);
        if (!im) continue;
        g.save();
        if (clip) { g.beginPath(); g.rect(clip[0] - x0, clip[1] - y0, clip[2] - clip[0], clip[3] - clip[1]); g.clip(); }
        g.drawImage(im, x - p.ox - x0, y - p.oy - y0);
        g.restore();
      }
      return cv;
    });
    // 所有格合起來的不透明範圍
    let tx0 = W, ty0 = H, tx1 = 0, ty1 = 0;
    for (const cv of canvases) {
      const px = cv.getContext("2d").getImageData(0, 0, W, H).data;
      for (let yy = 0; yy < H; yy++) for (let xx = 0; xx < W; xx++) {
        if (px[(yy * W + xx) * 4 + 3] > 8) { if (xx < tx0) tx0 = xx; if (yy < ty0) ty0 = yy; if (xx + 1 > tx1) tx1 = xx + 1; if (yy + 1 > ty1) ty1 = yy + 1; }
      }
    }
    if (tx1 <= tx0) return { canvases, x: x0, y: y0 };
    const out = canvases.map((cv) => {
      const o = document.createElement("canvas");
      o.width = tx1 - tx0; o.height = ty1 - ty0;
      o.getContext("2d").drawImage(cv, -tx0, -ty0);
      return o;
    });
    return { canvases: out, x: x0 + tx0, y: y0 + ty0 };
  }
  /**
   * 名牌：文字寬 tw；原點 (0, 0) 是文字範圍的上緣中間。
   * - 一般名牌：中間 c 鋪滿文字範圍，左右接 w、e
   * - 會動的名牌（和 WzComparerR2 相同）：c 是整塊底圖（不鋪），動畫貼在 c 的左上角；
   *   aniNameTag 的名牌只有動畫；c 只有 1×1 的名牌，動畫貼在文字左上角
   * 回傳的 frames 是每一格動畫的畫布（一般名牌只有一格）
   */
  function nameTag(skin, tw) {
    const P = skin.pieces || {};
    const A = skin.ani || [];
    let left = -Math.ceil(tw / 2), right = left + tw;
    let list, cy = P.c ? P.c.h / 2 - P.c.oy : 7;
    let textRoom = tw; // 名牌中間可以放字的寬度（固定大小的名牌比文字寬）
    if (!A.length) {
      const ops = [];
      if (P.c) for (let x = left; x < right; x += Math.max(1, P.c.w)) ops.push({ p: P.c, x, y: 0, clip: [left, -1e4, right, 1e4] });
      if (P.w) ops.push({ p: P.w, x: left, y: 0 });
      if (P.e) ops.push({ p: P.e, x: right, y: 0 });
      list = [ops];
    } else {
      const mixed = skin.ani_tag || (P.c && (P.c.w > 1 || P.c.h > 1));
      const bgW = mixed ? (P.c ? P.c.w : 0) : tw; // 只有動畫時，動畫原點對齊名牌中間
      const bgLeft = -Math.floor(bgW / 2);
      const sx = skin.ani_tag || !P.c ? 0 : P.c.ox, sy = skin.ani_tag || !P.c ? 0 : P.c.oy;
      const base = [];
      if (mixed && !skin.ani_tag && P.c) base.push({ p: P.c, x: bgLeft, y: 0 });
      const ax = mixed ? bgLeft - sx : left, ay = mixed ? -sy : 0;
      list = A.map((a) => [...base, { p: a, x: ax, y: ay }]);
      if (mixed) { left -= sx; right -= sx; textRoom = Math.max(tw, P.c && P.c.w > 1 ? P.c.w : Math.round(A[0].w * 0.6)); }
      if (!P.c || P.c.w <= 1) {
        // 沒有中間那塊：文字放在動畫第一格的中間高度
        const a = A[0];
        cy = Math.round(ay - a.oy + a.h / 2);
      }
    }
    const art = paintFrames(list);
    if (!art) return null;
    const frames = art.canvases.map((canvas, i) => ({ canvas, delay: A[i]?.delay || 100 }));
    return { canvas: frames[0].canvas, frames, x: art.x, y: art.y, left, right, cy, textRoom };
  }
  /** 名牌動畫：依每一格的時間換圖；同一個 key 重新播放時會停掉上一次的 */
  const players = new Map();
  function play(key, frames, show) {
    clearTimeout(players.get(key));
    players.delete(key);
    if (!frames?.length) return;
    show(frames[0], 0);
    if (frames.length < 2) return;
    let i = 0;
    const next = () => {
      i = (i + 1) % frames.length;
      show(frames[i], i);
      players.set(key, setTimeout(next, Math.max(30, frames[i].delay || 100)));
    };
    players.set(key, setTimeout(next, Math.max(30, frames[0].delay || 100)));
  }
  const stop = (key) => { clearTimeout(players.get(key)); players.delete(key); };
  /**
   * 對話框（九宮格）：文字 tw × th；原點 (0, 0) 是箭頭的位置（對話框下緣的正中間）。
   * 畫法和遊戲相同：四邊依圖片大小重複，角落、箭頭依原點貼上
   */
  function balloon(skin, tw, th) {
    const P = skin.pieces || {};
    const xt = Math.max(1, P.n?.w || P.s?.w || 1), yt = Math.max(1, P.w?.h || P.e?.h || 1);
    const width = (Math.ceil(tw / xt) + 2) * xt, height = Math.max(1, Math.ceil(th / yt)) * yt;
    const left = -Math.floor(width / 2), right = left + width, top = -height, bottom = 0;
    const ops = [];
    const at = (p, x, y, clip) => { if (p) ops.push({ p, x, y, clip }); };
    at(P.nw, left, top);
    at(P.sw, left, bottom);
    for (let y = top; y < bottom; y += yt) { at(P.w, left, y); at(P.e, right, y); }
    if (P.c) for (let y = top; y < bottom; y += Math.max(1, P.c.h)) for (let x = left; x < right; x += Math.max(1, P.c.w)) at(P.c, x, y, [left, top, right, bottom]);
    for (let x = left; x < right; x += xt) { at(P.n, x, top); at(P.s, x, bottom); }
    at(P.ne, right, top);
    at(P.se, right, bottom);
    at(P.arrow, 0, bottom);
    at(P.head, 0, top);
    const art = paint(ops, false);
    if (!art) return null;
    return { ...art, left: -tw / 2, top: top + (height - th) / 2 };
  }
  return { ready, nameTag, balloon, play, stop };
})();

/* ---------- 側欄紙娃娃：輸入角色名稱，或同步美容美髮的造型 ---------- */
const SideDoll = (() => {
  let cfg = { name: "", sync: false, ...(store.get("side-doll", {}) || {}) };
  let doll = null, gen = 0;
  const saveCfg = () => store.set("side-doll", cfg);
  const img = $("#sideDollImg"), empty = $("#sideDollEmpty"), box = $("#sideDollPic"), inp = $("#sideName");

  /**
   * 找出角色本體的位置：在不透明像素最密集的地方放一個「角色大小」的框
   * （橫向、縱向各自找總量最大的區段）。大型武器、披風雖然延伸得遠，但像素密度比不上身體，
   * 所以不會把中心拉走；放大倍率也以這個角色大小為準。
   */
  const BODY_W = 44, BODY_H = 74;
  function bodyBox(el) {
    try {
      const w = el.naturalWidth, h = el.naturalHeight;
      const cv = document.createElement("canvas");
      cv.width = w; cv.height = h;
      const ctx = cv.getContext("2d", { willReadFrequently: true });
      ctx.drawImage(el, 0, 0);
      const px = ctx.getImageData(0, 0, w, h).data;
      const cols = new Float64Array(w), rows = new Float64Array(h);
      for (let y = 0; y < h; y++) for (let x = 0; x < w; x++) {
        if (px[(y * w + x) * 4 + 3] > 128) { cols[x]++; rows[y]++; }
      }
      const densest = (arr, size) => {
        const n = arr.length, k = Math.min(size, n);
        let sum = 0;
        for (let i = 0; i < k; i++) sum += arr[i];
        let best = sum, at = 0;
        for (let i = k; i < n; i++) { sum += arr[i] - arr[i - k]; if (sum > best) { best = sum; at = i - k + 1; } }
        return { at, k, mass: best };
      };
      const cx = densest(cols, BODY_W), cy = densest(rows, BODY_H);
      if (!cx.mass) return null;
      return { cx: cx.at + cx.k / 2, cy: cy.at + cy.k / 2 };
    } catch (_) { return null; }
  }
  function fit() {
    const body = !!(doll && !doll.official && doll.w);
    img.classList.toggle("body", body);
    if (!body || img.hidden) { img.style.width = img.style.left = img.style.top = ""; return; }
    const bw = box.clientWidth, bh = box.clientHeight;
    const room = Math.min((bw * 0.82) / BODY_W, (bh * 0.8) / BODY_H, 5);
    const scale = room >= 1 ? Math.floor(room) : Math.max(0.4, room);
    img.style.width = `${Math.round(doll.w * scale)}px`;
    // 左右以角色本體置中；上下讓腳踩在框的下方（像站在地上），不是整個置中
    const c = doll.box || { cx: doll.cx ?? doll.w / 2, cy: doll.cy ?? doll.h / 2 };
    const feet = doll.fy ?? c.cy + BODY_H / 2;
    const ground = bh - Math.max(10, Math.round(bh * 0.1));
    img.style.left = `${Math.round(bw / 2 - c.cx * scale)}px`;
    img.style.top = `${Math.round(ground - feet * scale)}px`;
  }
  /** 名牌戒指：輸入框直接變成名牌的樣子（依名字長度拼出名牌，像素放大兩倍） */
  let nameSkin = null, paintGen = 0;
  const measure = document.createElement("canvas").getContext("2d");
  function styleName(tags) {
    nameSkin = tags?.name_tag || null;
    paintName();
  }
  async function paintName() {
    const my = ++paintGen;
    const t = nameSkin;
    const wrap = inp.parentElement;
    if (!t || !(await UiArt.ready(t))) {
      if (my !== paintGen) return;
      UiArt.stop("side-name");
      wrap.classList.remove("tagged");
      inp.removeAttribute("style");
      return;
    }
    if (my !== paintGen) return;
    const family = getComputedStyle(inp).fontFamily;
    const room = wrap.clientWidth || 240;
    const text = inp.value || inp.placeholder || "";
    measure.font = `700 11px ${family}`;
    const tw = Math.ceil(measure.measureText(text).width) + 4;
    const art = UiArt.nameTag(t, tw);
    if (art) {
      const W = art.canvas.width, H = art.canvas.height;
      // 放大倍率：最多兩倍，名牌太寬（裝飾多）時縮到剛好放得下，不會直接掉回一倍讓字變很小
      const k = Math.max(1, Math.min(2, (room - 4) / W));
      // 字的大小跟著名牌放大；固定大小的名牌中間比較寬，字可以再大一點（最大 16px）
      let fs = 11 * k;
      if (art.textRoom > tw) fs = Math.max(fs, Math.min(16, (11 * k * (art.textRoom - 6)) / Math.max(1, tw - 4)));
      const font = `700 ${fs.toFixed(1)}px ${family}`;
      // 文字放在名牌中間那塊的正中央（左右、上下都用 padding 把中心對過去）
      const midY = (art.cy - art.y) * k, full = H * k;
      const midX = ((art.left + art.right) / 2 - art.x) * k, wide = W * k;
      wrap.classList.add("tagged");
      Object.assign(inp.style, {
        font,
        color: t.clr || "#fff",
        width: `${wide}px`,
        height: `${full}px`,
        paddingLeft: `${Math.max(0, 2 * midX - wide)}px`,
        paddingRight: `${Math.max(0, wide - 2 * midX)}px`,
        paddingTop: `${Math.max(0, 2 * midY - full)}px`,
        paddingBottom: `${Math.max(0, full - 2 * midY)}px`,
        backgroundSize: `${wide}px ${full}px`,
        backgroundRepeat: "no-repeat",
        backgroundColor: "transparent",
      });
      // 會動的名牌：每一格先轉成圖片，輪流換背景
      const urls = art.frames.map((f) => ({ url: `url("${f.canvas.toDataURL()}")`, delay: f.delay }));
      UiArt.play("side-name", urls, (f) => (inp.style.backgroundImage = f.url));
      return;
    }
    UiArt.stop("side-name");
    wrap.classList.remove("tagged");
    inp.removeAttribute("style");
  }
  inp.addEventListener("input", () => { if (nameSkin) paintName(); });
  function show(d) {
    doll = d && d.png ? d : null;
    if (!doll) {
      img.hidden = true;
      empty.removeAttribute("hidden"); // SVG 沒有 hidden 屬性，要用 attribute
      img.removeAttribute("src");
      if (isCharTheme()) applySettings();
      return;
    }
    img.classList.toggle("official", !!doll.official);
    if (!doll.official) img.style.maxWidth = img.style.maxHeight = "none";
    else img.style.maxWidth = img.style.maxHeight = "";
    img.onload = () => {
      if (doll && !doll.official && !doll.box) doll.box = bodyBox(img);
      fit();
      if (isCharTheme()) applySettings();
    };
    if (img.getAttribute("src") !== doll.png) img.src = doll.png;
    else img.onload();
    img.hidden = false;
    empty.setAttribute("hidden", "");
  }
  async function load(manual) {
    $("#sideSync").checked = !!cfg.sync;
    inp.placeholder = cfg.sync ? "美容美髮的角色" : "角色名稱";
    if (cfg.sync) {
      const cur = window.Beauty?.current?.();
      if (document.activeElement !== inp) inp.value = cur?.name || "";
      show(cur?.doll || null);
      styleName(cur?.name ? cur?.tags : null);
      return;
    }
    if (document.activeElement !== inp) inp.value = cfg.name;
    if (!cfg.name || !TAURI || !window.Beauty?.charLook) { show(null); styleName(null); return; }
    const my = ++gen;
    try {
      const d = await window.Beauty.charLook(cfg.name);
      if (my === gen) { show(d); styleName(d?.tags); }
    } catch (e) {
      if (my !== gen) return;
      show(null);
      styleName(null);
      if (manual) toast(String(e));
    }
  }
  function submit() {
    const name = inp.value.trim();
    if (cfg.sync) {
      if (window.Beauty?.setName && name !== (window.Beauty.current?.().name || "")) window.Beauty.setName(name);
      return;
    }
    if (name === cfg.name && doll) return;
    cfg.name = name;
    saveCfg();
    load(true);
  }
  inp.addEventListener("keydown", (e) => { if (e.key === "Enter") { e.preventDefault(); submit(); inp.blur(); } });
  inp.addEventListener("change", submit);
  $("#sideSync").addEventListener("change", (e) => { cfg.sync = e.target.checked; saveCfg(); load(true); });
  document.addEventListener("beauty-doll", (e) => {
    if (!cfg.sync) return;
    if (document.activeElement !== inp) inp.value = window.Beauty?.current?.().name || "";
    show(e.detail);
  });
  document.addEventListener("beauty-tags", (e) => { if (cfg.sync) styleName(window.Beauty?.current?.().name ? e.detail : null); });
  document.addEventListener("beauty-ready", () => load(false));
  if (typeof ResizeObserver !== "undefined") new ResizeObserver(fit).observe(box);
  return { load };
})();

/* 左上角圖示：點一下播放動畫，再點一下停止 */
{
  const img = $("#brandIcon");
  let playing = false;
  img.addEventListener("click", () => {
    playing = !playing;
    // 每次播放都從第一格開始
    img.src = playing ? `assets/brand.gif?t=${Date.now()}` : "assets/brand.png";
  });
}

/* 並排數量：空白 = 依視窗寬度自動 */
let perRow = Number(store.get("toolbox-perrow", 0)) || 0;
function applyPerRow() {
  // 欄寬要是固定值（不能是 minmax），方框的高度才算得出來；放不下時平均縮小
  const tiles = $("#tiles");
  if (perRow) {
    // 分類頁寬度扣掉左右留白與間距後平均分；比預設大小大時維持預設大小
    tiles.style.setProperty("--col", `min(var(--tile), calc((100cqw - 4rem - ${perRow - 1} * 1.1rem) / ${perRow}))`);
    tiles.style.gridTemplateColumns = `repeat(${perRow}, var(--col))`;
  } else {
    tiles.style.removeProperty("--col");
    tiles.style.gridTemplateColumns = "";
  }
  const inp = $("#perRow");
  if (document.activeElement !== inp) inp.value = perRow || "";
}
function setPerRow(v) {
  perRow = v >= 1 ? Math.min(12, Math.floor(v)) : 0;
  store.set("toolbox-perrow", perRow);
  applyPerRow();
}
$("#perRow").addEventListener("input", (e) => {
  // 只留數字
  const digits = e.target.value.replace(/\D/g, "");
  if (digits !== e.target.value) e.target.value = digits;
  setPerRow(Number(digits));
});
$("#perRow").addEventListener("blur", applyPerRow);
// 滾輪直接加減（自動時從目前實際的數量開始）
$("#perRow").addEventListener("wheel", (e) => {
  if (!e.deltaY) return;
  e.preventDefault();
  const cols = getComputedStyle($("#tiles")).gridTemplateColumns;
  const shown = cols && cols !== "none" ? cols.split(" ").filter(Boolean).length : 0;
  const cur = perRow || shown || 4;
  setPerRow(Math.max(1, Math.min(12, cur + (e.deltaY < 0 ? 1 : -1))));
  e.target.value = perRow;
}, { passive: false });
applyPerRow();

/* 圖釘：固定後所有工具都不能拖曳移動 */
let pinned = !!store.get("toolbox-pinned", false);
function renderPin() {
  const b = $("#pinBtn");
  b.classList.toggle("on", pinned);
  b.dataset.tip = pinned ? "已固定：所有工具都不會移動，再按一次解除" : "固定：開啟後所有工具都不再移動";
  document.body.classList.toggle("tiles-pinned", pinned);
}
$("#pinBtn").addEventListener("click", () => {
  pinned = !pinned;
  store.set("toolbox-pinned", pinned);
  renderPin();
  toast(pinned ? "已固定所有工具的位置" : "已解除固定，可以拖曳工具");
});
renderPin();

let dragJustEnded = false;
function dragTool(el, id) {
  el.addEventListener("pointerdown", (e) => {
    if (e.button !== 0 || pinned) return;
    const sx = e.clientX, sy = e.clientY;
    const list = el.parentElement;
    const startOrder = [...list.children];
    let ghost = null, zone = null, ox = 0, oy = 0, frame = null, box = null;
    const onFavPage = currentCat === "fav";
    const setZone = (z) => {
      if (z === zone) return;
      zone = z;
      $("#favCat").classList.toggle("drop", z === "fav");
      // 拖到「我的最愛」上方時縮小，比較好對準
      if (ghost) ghost.style.scale = z === "fav" ? "0.45" : "";
      ghost?.classList.toggle("del", z === "unfav");
      frame?.classList.toggle("out", z === "unfav");
    };
    const move = (ev) => {
      if (!ghost) {
        if (Math.hypot(ev.clientX - sx, ev.clientY - sy) < 6) return;
        const r = el.getBoundingClientRect();
        ox = sx - r.left;
        oy = sy - r.top;
        ghost = el.cloneNode(true);
        ghost.classList.add("drag-ghost");
        Object.assign(ghost.style, { width: `${r.width}px`, height: `${r.height}px`, transformOrigin: `${ox}px ${oy}px` });
        document.body.append(ghost);
        // 我的最愛頁：用虛線框出範圍，拖到框外放開就移除
        if (onFavPage) {
          const rs = [...list.children].map((k) => k.getBoundingClientRect());
          const pad = 12;
          box = {
            left: Math.min(...rs.map((q) => q.left)) - pad, top: Math.min(...rs.map((q) => q.top)) - pad,
            right: Math.max(...rs.map((q) => q.right)) + pad, bottom: Math.max(...rs.map((q) => q.bottom)) + pad,
          };
          frame = document.createElement("div");
          frame.className = "fav-frame";
          Object.assign(frame.style, { left: `${box.left}px`, top: `${box.top}px`, width: `${box.right - box.left}px`, height: `${box.bottom - box.top}px` });
          document.body.append(frame);
        }
        el.classList.add("drag-src");
        document.body.classList.add("tool-dragging");
        ctxMenu.hidden = true;
      }
      ghost.style.transform = `translate(${ev.clientX - ox}px, ${ev.clientY - oy}px) rotate(-2deg)`;
      const t = document.elementFromPoint(ev.clientX, ev.clientY);
      if (!onFavPage && t?.closest("#favCat")) { setZone("fav"); return; }
      if (onFavPage && box && !t?.closest("#favCat")) {
        const inside = ev.clientX >= box.left && ev.clientX <= box.right && ev.clientY >= box.top && ev.clientY <= box.bottom;
        if (!inside) { setZone("unfav"); return; }
      }
      setZone(null);
      const over = t?.closest(".tile");
      if (over && over !== el && over.parentElement === list && !over.getAnimations().length) {
        const kids = [...list.children];
        flip(list, () => list.insertBefore(el, kids.indexOf(el) < kids.indexOf(over) ? over.nextSibling : over));
      }
    };
    const up = () => {
      window.removeEventListener("pointermove", move);
      window.removeEventListener("pointerup", up);
      if (!ghost) return;
      dragJustEnded = true;
      setTimeout(() => (dragJustEnded = false), 0);
      ghost.remove();
      frame?.remove();
      el.classList.remove("drag-src");
      document.body.classList.remove("tool-dragging");
      const z = zone;
      setZone(null);
      // 拖到我的最愛／拖出我的最愛：經過其他工具時造成的位置變動，瞬間還原
      if (z === "fav" || z === "unfav") {
        for (const k of startOrder) { k.getAnimations().forEach((a) => a.cancel()); list.appendChild(k); }
        if (z === "fav") addFav(id); else removeFav(id);
        return;
      }
      const order = [...list.children].map((x) => x.dataset.id);
      if (currentCat === "fav") {
        favs = order;
        saveFavs();
      } else {
        tileOrder[currentCat] = order;
        store.set("toolbox-order", tileOrder);
      }
    };
    window.addEventListener("pointermove", move);
    window.addEventListener("pointerup", up);
  });
  // 拖曳結束時不要當成點擊
  el.addEventListener("click", (e) => { if (dragJustEnded) { e.stopImmediatePropagation(); e.preventDefault(); } }, true);
}

let currentView = "cat";
function showView(name) {
  // 離開序號複製功能時，關閉自動切換
  if (currentView === "serial" && name !== "serial" && state.active) {
    run(async () => update(await api.setActive(false)));
  }
  // 打開序號複製時，若還有未使用的序號就自動開啟自動切換
  if (name === "serial" && currentView !== "serial" && !state.active && state.index < state.serials.length) {
    run(async () => {
      update(await api.setActive(true));
      toast("自動切換已開啟，目前序號已放入剪貼簿");
    });
  }
  currentView = name;
  $$(".view").forEach((v) => v.classList.toggle("active", v.id === `view-${name}`));
  $("#gearBtn").classList.toggle("on", name === "settings");
  // 讓各工具知道換頁了（例如離開美容美髮時收起浮動視窗）
  document.dispatchEvent(new CustomEvent("viewchange", { detail: name }));
}

/** 各工具可以接管返回按鈕：在子頁面（例如解析）時顯示「上一頁」並回到抽獎模擬 */
const BackHooks = {};
function refreshBack() {
  $$(".view").forEach((v) => {
    const label = v.querySelector(".back-label");
    if (!label) return;
    const h = BackHooks[v.id];
    label.textContent = (h && h.label()) || (CATS[currentCat] ? CATS[currentCat].title : "");
  });
}
window.BackHooks = BackHooks;
window.refreshBack = refreshBack;

function openTool(id) {
  // 從其他地方開啟時，返回鍵回到這個工具所在的分類（從我的最愛開啟就回到我的最愛）
  if (TOOL[id] && currentCat !== "fav" && TOOL[id].cat !== currentCat) {
    currentCat = TOOL[id].cat;
    store.set("toolbox-cat", currentCat);
    $$(".cat").forEach((b) => b.classList.toggle("on", b.dataset.cat === currentCat));
    $("#catTitle").textContent = CATS[currentCat].title;
    $("#catEyebrow").textContent = CATS[currentCat].eyebrow;
    renderTiles();
  }
  ctxMenu.hidden = true;
  refreshBack();
  $("#app").classList.add("collapsed");
  showView(id);
  if (id === "timer" && window.Timers) window.Timers.onShow();
  if (id === "gacha" && window.Gacha) window.Gacha.onShow();
  if (id === "beauty" && window.Beauty) window.Beauty.onShow();
  if (id === "comet" && window.Comet) window.Comet.onShow();
  if (id === "star" && window.Starlight) window.Starlight.onShow();
  if (id === "gift" && window.Gift) window.Gift.onShow();
  if (id === "watch" && window.Watch) window.Watch.onShow();
  if (id === "boss" && window.Boss) window.Boss.onShow();
  if (id === "track" && window.Track) window.Track.onShow();
  if (id === "patch" && window.Patches) window.Patches.onShow();
  // 抽獎模擬器：檢查官網有沒有換獎池
  if (typeof OfficialPools !== "undefined") OfficialPools.onOpen(id);
}

$$(".cat").forEach((b) => b.addEventListener("click", () => showCat(b.dataset.cat)));
$$("[data-back]").forEach((b) => b.addEventListener("click", () => {
  const view = b.closest(".view");
  const h = view && BackHooks[view.id];
  if (h && h.back()) { refreshBack(); return; }
  showCat(currentCat);
}));

/* ---------- 序號複製 ---------- */
let state = { raw: "", serials: [], index: 0, active: false };

function render(prev) {
  const { serials, index, active } = state;
  const total = serials.length;
  const done = total > 0 && index >= total;

  $("#progressText").textContent = total === 0 ? "0 / 0" : done ? `${total} / ${total}` : `${index + 1} / ${total}`;
  $("#barFill").style.width = total ? `${(Math.min(index, total) / total) * 100}%` : "0";
  $("#statusCard").classList.toggle("done", done);

  const cur = $("#currentSerial");
  cur.textContent = total === 0 ? "—" : serials[index] ?? "—";
  if (prev && prev.index !== index) {
    cur.classList.remove("flash");
    void cur.offsetWidth;
    cur.classList.add("flash");
  }

  const toggle = $("#activeToggle");
  toggle.checked = active;
  toggle.disabled = total === 0;
  $("#activeLabel").textContent = active ? "自動切換中" : "自動切換";

  $("#prevBtn").disabled = total === 0 || index <= 0;
  $("#nextBtn").disabled = total === 0 || index >= total;
  $("#countText").textContent = `${total} 組`;
  $("#emptyText").style.display = total ? "none" : "block";

  // 清單：只有內容改變時才重建，避免閃爍
  const list = $("#serialList");
  const sig = serials.join("\n");
  if (list.dataset.sig !== sig) {
    list.dataset.sig = sig;
    list.innerHTML = "";
    serials.forEach((s, i) => {
      const li = document.createElement("li");
      li.className = "item";
      li.dataset.i = i;
      li.innerHTML = `<span class="num"></span><span class="code"></span><span class="tag"></span>`;
      li.querySelector(".code").textContent = s;
      list.appendChild(li);
    });
  }
  $$(".item", list).forEach((li) => {
    const i = Number(li.dataset.i);
    li.classList.toggle("used", i < index);
    li.classList.toggle("cur", i === index);
    const num = li.querySelector(".num");
    if (i < index) num.innerHTML = '<svg viewBox="0 0 24 24" class="check"><path d="M5 12.5l4.5 4.5L19 7.5"/></svg>';
    else num.textContent = i + 1;
    li.querySelector(".tag").textContent = i === index ? "目前" : "";
  });

  // 只捲動清單本身，不帶動整個視窗
  const curItem = $(".item.cur", list);
  if (curItem && prev && prev.index !== index) {
    const box = $(".list-scroll");
    const top = curItem.offsetTop - box.clientHeight / 2 + curItem.clientHeight / 2;
    box.scrollTo({ top, behavior: "smooth" });
  }
}

function update(next) {
  if (!next) return;
  const prev = state;
  state = next;
  render(prev);
}

$("#extractBtn").addEventListener("click", () =>
  run(async () => {
    const next = await api.extract($("#rawInput").value);
    update(next);
    toast(next.serials.length ? `已擷取 ${next.serials.length} 組序號，自動切換已開啟` : "沒有找到符合規則的序號");
  })
);

$("#clearBtn").addEventListener("click", () =>
  run(async () => {
    if (state.serials.length && !(await confirmBox("確定要清除序號清單與目前進度嗎？"))) return;
    $("#rawInput").value = "";
    update(await api.clear());
  })
);

$("#activeToggle").addEventListener("change", (e) =>
  run(async () => {
    update(await api.setActive(e.target.checked));
    if (state.active) toast("已啟用，目前序號已放入剪貼簿");
  })
);

$("#prevBtn").addEventListener("click", () => run(async () => update(await api.setIndex(Math.max(0, state.index - 1)))));
$("#nextBtn").addEventListener("click", () => run(async () => update(await api.setIndex(state.index + 1))));

$("#serialList").addEventListener("click", (e) => {
  const li = e.target.closest(".item");
  if (!li) return;
  run(async () => {
    update(await api.setIndex(Number(li.dataset.i)));
    if (state.active) toast(`已跳到第 ${state.index + 1} 組並放入剪貼簿`);
  });
});

$("#currentSerial").addEventListener("click", () =>
  run(async () => {
    const s = state.serials[state.index];
    if (!s) return;
    await api.copy(s);
    toast("已複製到剪貼簿");
  })
);

/* ---------- 啟動 ---------- */
(async () => {
  const st = await api.getState();
  settings = { theme: "system", font: "m", ...(st.settings || {}) };
  // 舊版只有「角色色彩」：依目前 Windows 的明暗換成淺色或深色
  if (settings.theme === "char") settings.theme = darkQuery.matches ? "char-dark" : "char-light";
  // 文字大小改版：原本的「中」變成「小」、「大」變成「中」，新增更大的「大」；預設為「中」
  if (!store.get("toolbox-font-v2", false)) {
    if (settings.font === "l") settings.font = "m";
    store.set("toolbox-font-v2", true);
    api.saveSettings({ theme: settings.theme, font: settings.font, close_tray: !!settings.close_tray, close_asked: !!settings.close_asked, upd_manual: !!settings.upd_manual });
  }
  applySettings();
  settingsLoaded();
  state = st;
  $("#rawInput").value = st.raw || "";
  render();
  showCat(currentCat);
  renderFavCount();
  api.onChange((next) => {
    const wasDone = state.index >= state.serials.length;
    update(next);
    if (!wasDone && state.index >= state.serials.length) toast("已全部使用完畢");
  });
})();

window.App = { invoke, listen, toast, run, store, confirmBox, $, $$, TAURI };

/* ---------- 自動更新 ----------
 * 更新方式（設定）：
 * - 每次開啟時檢查：程式開啟時檢查一次；按「稍後」之後這次就不再提醒，下次開啟程式才會再跳出
 * - 手動檢查：開啟時不檢查，想更新時按設定裡的「檢查更新」 */
(() => {
  const idle = () => (settings.upd_manual ? "不會自動檢查，想更新時按右邊的「檢查更新」" : "程式開啟時會自動檢查一次");
  let shown = "";
  const status = (t) => { shown = t || ""; $("#updStatus").textContent = shown || idle(); };
  window.Updater = { mode: () => { if (!shown) status(""); } };
  settingsReady.then(() => status(shown));
  if (!TAURI) return;
  let info = null, snoozed = false;

  async function refresh() {
    try { info = await invoke("update_info"); } catch (_) { return; }
    $("#verText").textContent = "1.0.0";
    $("#verText").title = info.current ? `編譯 #${info.run}（${info.current}）` : "自行編譯版";
  }

  const closeDialog = () => $("#updBackdrop").classList.remove("show");

  async function install() {
    closeDialog();
    status("下載新版本中…");
    toast("正在下載新版本，完成後會自動重新開啟");
    try { await invoke("update_install"); }
    catch (e) { status(String(e)); toast(String(e)); }
  }

  function offer(r) {
    $("#updMsg").textContent = "發現新版本";
    $("#updSub").textContent = `目前 ${r.current}，最新 ${r.latest}。更新後程式會重新開啟。`;
    $("#updBackdrop").classList.add("show");
  }

  async function check(manual) {
    if (!info) await refresh();
    if (!info?.token_set) { if (manual) status("這個版本沒有內建更新權限"); return; }
    if (manual) status("檢查中…");
    try {
      const r = await invoke("update_check");
      if (r.has_update) {
        status(`有新版本（${r.latest}）`);
        if (manual || !snoozed) offer(r);
      } else status(manual ? (r.current ? "已經是最新版本" : "自行編譯版不檢查更新") : "");
    } catch (e) { status(String(e)); }
  }

  $("#updCheck").addEventListener("click", () => check(true));
  $("#updNow").addEventListener("click", install);
  $("#updLater").addEventListener("click", () => { snoozed = true; closeDialog(); });

  refresh();
  // 選「手動檢查」就不在開啟時檢查
  settingsReady.then(() => { if (!settings.upd_manual) check(false); });
})();
