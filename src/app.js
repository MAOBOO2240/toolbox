"use strict";

/* ---------- 後端橋接 ---------- */
// 在 Tauri 中呼叫 Rust 指令；直接用瀏覽器開啟時使用模擬後端，方便預覽介面。
const TAURI = window.__TAURI__;
const api = TAURI ? createTauriApi() : createMockApi();

function createTauriApi() {
  const invoke = TAURI.core.invoke;
  return {
    getState: () => invoke("get_state"),
    extract: (raw) => invoke("extract", { raw }),
    setIndex: (index) => invoke("set_index", { index }),
    setActive: (active) => invoke("set_active", { active }),
    copy: (text) => invoke("copy_text", { text }),
    clear: () => invoke("clear_list"),
    saveSettings: (settings) => invoke("save_settings", { settings }),
    onChange: (cb) => TAURI.event.listen("state-changed", (e) => cb(e.payload)),
    setWindowTheme: (t) => {
      try { TAURI.window.getCurrentWindow().setTheme(t); } catch (_) {}
    },
  };
}

function createMockApi() {
  const KEY = "toolbox-mock";
  let st;
  try { st = JSON.parse(localStorage.getItem(KEY)); } catch (_) {}
  st = st || { raw: "", serials: [], index: 0, active: false, settings: { theme: "system", font: "m" } };
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
    extract: async (raw) => { st.raw = raw; st.serials = extract(raw); st.index = 0; if (!st.serials.length) st.active = false; return save(); },
    setIndex: async (i) => { st.index = Math.min(i, st.serials.length); return save(); },
    setActive: async (a) => { st.active = a && st.serials.length > 0; return save(); },
    copy: async (t) => { await navigator.clipboard?.writeText(t); },
    clear: async () => { Object.assign(st, { raw: "", serials: [], index: 0, active: false }); return save(); },
    saveSettings: async (s) => { st.settings = s; save(); },
    onChange: () => {},
    setWindowTheme: () => {},
  };
}

/* ---------- 工具 ---------- */
const $ = (sel) => document.querySelector(sel);
const root = document.documentElement;
const darkQuery = window.matchMedia("(prefers-color-scheme: dark)");

let state = { raw: "", serials: [], index: 0, active: false, settings: { theme: "system", font: "m" } };

let toastTimer;
function toast(msg) {
  const el = $("#toast");
  el.textContent = msg;
  el.classList.add("show");
  clearTimeout(toastTimer);
  toastTimer = setTimeout(() => el.classList.remove("show"), 1600);
}

async function run(fn) {
  try {
    return await fn();
  } catch (e) {
    toast(String(e));
  }
}

/* ---------- 外觀設定 ---------- */
function effectiveTheme() {
  const t = state.settings.theme;
  if (t === "light" || t === "dark") return t;
  return darkQuery.matches ? "dark" : "light";
}

function applySettings() {
  const theme = effectiveTheme();
  root.dataset.theme = theme;
  root.dataset.font = state.settings.font;
  $("#themeIcon").textContent = theme === "dark" ? "☾" : "☀";
  $("#themeLabel").textContent = theme === "dark" ? "深色" : "淺色";
  document.querySelectorAll(".seg-btn").forEach((b) => b.classList.toggle("on", b.dataset.font === state.settings.font));
  api.setWindowTheme(theme);
}

function saveSettings() {
  applySettings();
  api.saveSettings(state.settings);
}

$("#themeBtn").addEventListener("click", () => {
  state.settings.theme = effectiveTheme() === "dark" ? "light" : "dark";
  saveSettings();
});

document.querySelectorAll(".seg-btn").forEach((btn) =>
  btn.addEventListener("click", () => {
    state.settings.font = btn.dataset.font;
    saveSettings();
  })
);

darkQuery.addEventListener("change", () => {
  if (state.settings.theme === "system") applySettings();
});

/* ---------- 頁面導覽 ---------- */
function go(page) {
  document.querySelectorAll(".page").forEach((p) => p.classList.toggle("active", p.id === `page-${page}`));
}
document.querySelectorAll("[data-go]").forEach((el) => el.addEventListener("click", () => go(el.dataset.go)));

/* ---------- 序號工具畫面 ---------- */
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
      li.innerHTML = `<span class="num">${i + 1}</span><span class="code"></span><span class="tag"></span>`;
      li.querySelector(".code").textContent = s;
      list.appendChild(li);
    });
  }
  list.querySelectorAll(".item").forEach((li) => {
    const i = Number(li.dataset.i);
    li.classList.toggle("used", i < index);
    li.classList.toggle("cur", i === index);
    li.querySelector(".num").textContent = i < index ? "✓" : i + 1;
    li.querySelector(".tag").textContent = i === index ? "目前" : "";
  });

  const curItem = list.querySelector(".item.cur");
  if (curItem && prev && prev.index !== index) curItem.scrollIntoView({ block: "nearest", behavior: "smooth" });
}

function update(next) {
  if (!next) return;
  const prev = state;
  state = { ...next, settings: state.settings };
  render(prev);
}

$("#extractBtn").addEventListener("click", () =>
  run(async () => {
    const raw = $("#rawInput").value;
    const next = await api.extract(raw);
    update(next);
    toast(next.serials.length ? `已擷取 ${next.serials.length} 組序號` : "沒有找到符合規則的序號");
  })
);

$("#clearBtn").addEventListener("click", () =>
  run(async () => {
    if (state.serials.length && !confirm("確定要清除序號清單與進度嗎？")) return;
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
  state = st;
  state.settings = { theme: "system", font: "m", ...(st.settings || {}) };
  $("#rawInput").value = st.raw || "";
  applySettings();
  render();
  api.onChange((next) => {
    const wasDone = state.index >= state.serials.length;
    update(next);
    if (!wasDone && state.index >= state.serials.length) toast("已全部使用完畢");
  });
})();
