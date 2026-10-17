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
  if (e.ctrlKey || e.deltaY === 0) return;
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

function effectiveTheme() {
  const t = settings.theme;
  if (t === "light" || t === "dark") return t;
  return darkQuery.matches ? "dark" : "light";
}

function applySettings() {
  const theme = effectiveTheme();
  root.dataset.theme = theme;
  root.dataset.font = settings.font;
  $$("#fontSeg button").forEach((b) => b.classList.toggle("on", b.dataset.font === settings.font));
  $$("#themeSeg button").forEach((b) => b.classList.toggle("on", b.dataset.theme === theme));
  $$("#closeSeg button").forEach((b) => b.classList.toggle("on", b.dataset.v === (settings.close_tray ? "tray" : "exit")));
  api.setWindowTheme(theme);
  // 視窗外框（標題列與邊框）換成和程式內容有區別的顏色
  if (TAURI) invoke("set_frame", { dark: theme === "dark" }).catch(() => {});
}

function saveSettings() {
  applySettings();
  api.saveSettings({ theme: settings.theme, font: settings.font, close_tray: !!settings.close_tray });
}

$$("#fontSeg button").forEach((b) => b.addEventListener("click", () => { settings.font = b.dataset.font; saveSettings(); }));
$$("#themeSeg button").forEach((b) => b.addEventListener("click", () => { settings.theme = b.dataset.theme; saveSettings(); }));
darkQuery.addEventListener("change", () => { if (settings.theme === "system") applySettings(); });

$$("#closeSeg button").forEach((b) => b.addEventListener("click", () => { settings.close_tray = b.dataset.v === "tray"; saveSettings(); }));

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
  copy: '<svg viewBox="0 0 24 24"><rect x="8" y="8" width="12" height="12" rx="3"/><path d="M16 8V6a2 2 0 0 0-2-2H6a2 2 0 0 0-2 2v8a2 2 0 0 0 2 2h2"/></svg>',
};

// 滑過磁磚時的光暈顏色（與圖示漸層同色系）
const TILE_ACCENT = { red: "#ff5a5f", purple: "#9b5cf2", pink: "#ff5fa8", gold: "#ffb000", comet: "#4fb8ff", star: "#a78bfa", blue: "#3d7bff", green: "#30c26a", orange: "#ff7a2f" };

const CATS = {
  maple: {
    title: "楓之谷工具",
    eyebrow: "MapleStory",
    tools: [
      { id: "beauty", name: "美容美髮", desc: "載入角色造型，模擬皇家美髮／整形", color: "pink", icon: "scissors", item: "皇家美髮券(選擇型)" },
      { id: "gacha", name: "黃金蘋果", desc: "獎池解析、模擬抽獎與期望值分析", color: "gold", icon: "apple", item: "黃金蘋果" },
      { id: "comet", name: "閃亮彗星", desc: "模擬抽獎、分解碎片與兌換璀璨彗星", color: "comet", icon: "comet", item: "閃亮彗星" },
      { id: "star", name: "星光錦囊", desc: "套娃抽獎：玲瓏星光合成、層層開啟與期望值", color: "star", icon: "star", item: "星光錦囊" },
    ],
  },
  tools: {
    title: "工具箱",
    eyebrow: "Toolbox",
    tools: [
      { id: "timer", name: "計時器", desc: "道具／技能倒數與浮動視窗", color: "purple", icon: "clock" },
      { id: "serial", name: "序號複製", desc: "擷取序號，Ctrl+V 後自動換下一組", color: "blue", icon: "copy" },
      { id: "watch", name: "斷線通知", desc: "新楓之谷斷線、回到登入畫面或關閉時通知", color: "red", icon: "plug" },
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

/** 工具圖示：有對應的遊戲道具時換成道具圖示（讀不到就保留原本的圖示） */
function toolIcon(t) {
  const box = document.createElement("span");
  box.className = `cat-icon ${t.color}`;
  box.style.setProperty("--accent", TILE_ACCENT[t.color] || "#4c8dff");
  box.innerHTML = ICONS[t.icon];
  if (t.item) ItemIcons.get(t.item).then((png) => {
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

/* 一排幾個：空白 = 依視窗寬度自動 */
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
$("#perRow").addEventListener("input", (e) => {
  const v = Math.floor(Number(e.target.value));
  perRow = v >= 1 ? Math.min(12, v) : 0;
  store.set("toolbox-perrow", perRow);
  applyPerRow();
});
$("#perRow").addEventListener("blur", applyPerRow);
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
  if (id === "watch" && window.Watch) window.Watch.onShow();
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
  // 文字大小改版：原本的「中」變成「小」、「大」變成「中」，新增更大的「大」；預設為「中」
  if (!store.get("toolbox-font-v2", false)) {
    if (settings.font === "l") settings.font = "m";
    store.set("toolbox-font-v2", true);
    api.saveSettings({ theme: settings.theme, font: settings.font, close_tray: !!settings.close_tray });
  }
  applySettings();
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
 * 程式開啟時檢查一次；按「稍後」之後這次就不再提醒，下次開啟程式才會再跳出。
 * 設定裡的「檢查更新」可以手動再檢查。 */
(() => {
  if (!TAURI) return;
  let info = null, snoozed = false;
  const status = (t) => { $("#updStatus").textContent = t || "程式開啟時會自動檢查一次"; };

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

  refresh().then(() => check(false));
})();
