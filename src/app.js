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
  api.setWindowTheme(theme);
}

function saveSettings() {
  applySettings();
  api.saveSettings({ theme: settings.theme, font: settings.font });
}

$$("#fontSeg button").forEach((b) => b.addEventListener("click", () => { settings.font = b.dataset.font; saveSettings(); }));
$$("#themeSeg button").forEach((b) => b.addEventListener("click", () => { settings.theme = b.dataset.theme; saveSettings(); }));
darkQuery.addEventListener("change", () => { if (settings.theme === "system") applySettings(); });

const pop = $("#settingsPop");
$("#gearBtn").addEventListener("click", (e) => {
  e.stopPropagation();
  pop.classList.toggle("show");
  $("#gearBtn").classList.toggle("on", pop.classList.contains("show"));
});
document.addEventListener("click", (e) => {
  if (!pop.contains(e.target)) { pop.classList.remove("show"); $("#gearBtn").classList.remove("on"); }
});

/* ---------- 分類與導覽 ---------- */
const ICONS = {
  clock: '<svg viewBox="0 0 24 24"><circle cx="12" cy="13" r="8"/><path d="M12 9v4l2.5 2.5M9 2h6"/></svg>',
  scissors: '<svg viewBox="0 0 24 24"><circle cx="6" cy="6" r="3"/><circle cx="6" cy="18" r="3"/><path d="M20 4L8.1 15.9M14.5 14.5L20 20M8.1 8.1L12 12"/></svg>',
  apple: '<svg viewBox="0 0 24 24"><path d="M12 7c-1.5-1.3-3.8-1.6-5.4-.4C4.3 8.3 4.6 12.6 6.5 16c1.2 2.2 2.7 4 4.3 3.6.6-.1.9-.4 1.2-.4s.6.3 1.2.4c1.6.4 3.1-1.4 4.3-3.6 1.9-3.4 2.2-7.7-.1-9.4C15.8 5.4 13.5 5.7 12 7z"/><path d="M12 7c0-2 1-3.5 3-4"/></svg>',
  copy: '<svg viewBox="0 0 24 24"><rect x="8" y="8" width="12" height="12" rx="3"/><path d="M16 8V6a2 2 0 0 0-2-2H6a2 2 0 0 0-2 2v8a2 2 0 0 0 2 2h2"/></svg>',
};

const CATS = {
  maple: {
    title: "楓之谷工具",
    eyebrow: "MapleStory",
    tools: [
      { id: "timer", name: "計時器", desc: "道具／技能倒數與浮動視窗", color: "purple", icon: "clock" },
      { id: "beauty", name: "美容美髮模擬器", desc: "載入角色造型，模擬皇家美髮／整形", color: "pink", icon: "scissors" },
      { id: "gacha", name: "黃金蘋果轉蛋模擬器", desc: "獎池解析、模擬抽獎與期望值分析", color: "gold", icon: "apple" },
    ],
  },
  tools: {
    title: "工具箱",
    eyebrow: "Toolbox",
    tools: [{ id: "serial", name: "序號複製", desc: "擷取序號，Ctrl+V 後自動換下一組", color: "blue", icon: "copy" }],
  },
};

let currentCat = store.get("toolbox-cat", "maple");
if (!CATS[currentCat]) currentCat = "maple";

function showCat(cat) {
  currentCat = cat;
  store.set("toolbox-cat", cat);
  const c = CATS[cat];
  $$(".cat").forEach((b) => b.classList.toggle("on", b.dataset.cat === cat));
  $("#catTitle").textContent = c.title;
  $("#catEyebrow").textContent = c.eyebrow;
  const tiles = $("#tiles");
  tiles.innerHTML = "";
  for (const t of c.tools) {
    const btn = document.createElement("button");
    btn.className = "tile";
    btn.innerHTML = `<span class="cat-icon ${t.color}">${ICONS[t.icon]}</span><strong></strong><small></small>`;
    btn.querySelector("strong").textContent = t.name;
    btn.querySelector("small").textContent = t.desc;
    btn.addEventListener("click", () => openTool(t.id));
    tiles.appendChild(btn);
  }
  showView("cat");
  $("#app").classList.remove("collapsed");
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
  refreshBack();
  $("#app").classList.add("collapsed");
  showView(id);
  if (id === "timer" && window.Timers) window.Timers.onShow();
  if (id === "gacha" && window.Gacha) window.Gacha.onShow();
  if (id === "beauty" && window.Beauty) window.Beauty.onShow();
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
  applySettings();
  state = st;
  $("#rawInput").value = st.raw || "";
  render();
  showCat(currentCat);
  api.onChange((next) => {
    const wasDone = state.index >= state.serials.length;
    update(next);
    if (!wasDone && state.index >= state.serials.length) toast("已全部使用完畢");
  });
})();

window.App = { invoke, listen, toast, run, store, confirmBox, $, $$, TAURI };
