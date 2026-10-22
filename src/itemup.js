"use strict";

/* ==========================================================
 * 更新道具
 * - 程式記下遊戲裡所有道具的編號，遊戲改版後列出新增了哪些道具（例如 v282 → v283）
 * - 大改版之後的小更新新增的道具也算在這次大改版裡，下一次大改版才整個換掉
 * - 程式一開啟就在背景檢查（遊戲檔沒變就不會重新讀）
 * - 點道具圖示：顯示遊戲裡的名稱與說明
 * ========================================================== */
if (typeof document !== "undefined") (() => {
  const CATS = [
    { id: "", name: "全部", color: "c-silver" },
    { id: "eqp", name: "裝備", color: "c-blue" },
    { id: "outfit", name: "點裝", color: "c-pink" },
    { id: "use", name: "消耗", color: "c-red" },
    { id: "setup", name: "裝飾", color: "c-green" },
    { id: "etc", name: "其他", color: "c-amber" },
    { id: "cash", name: "特殊", color: "c-purple" },
  ];
  const catOf = (it) => {
    if (it.folder.startsWith("Character/")) return it.cash ? "outfit" : "eqp";
    if (it.folder === "Item/Consume") return "use";
    if (it.folder === "Item/Install") return "setup";
    if (it.folder === "Item/Etc") return "etc";
    return "cash"; // Item/Cash、Item/Pet
  };
  const catName = (c) => (CATS.find((x) => x.id === c) || CATS[0]).name;
  const nf = new Intl.NumberFormat("zh-TW");
  const gameDir = () => { try { return JSON.parse(localStorage.getItem("beauty-v1"))?.gamePath || null; } catch (_) { return null; } };
  const fmtDay = (ms) => { const d = new Date(ms); return `${d.getFullYear()}/${d.getMonth() + 1}/${d.getDate()}`; };
  const verText = (r) => (r.from === r.to ? `v${r.to}` : `v${r.from} → v${r.to}`);
  const recSig = (d) => (d?.records || []).map((r) => `${r.from}-${r.to}-${r.items.length}`).join(",");

  let data = null; // { version, total, since, records }
  let error = "";
  let busy = false;
  let sel = 0; // 第幾筆改版紀錄
  let cat = "";
  let q = "";

  /* ---------- 圖示：看到才讀，一次讀一批 ---------- */
  const icons = new Map(); // "資料夾|編號" → data URL 或 null
  const failedAt = new Map(); // 讀不到的圖示 → 時間（過 5 分鐘再讀一次）
  const iconOf = (key) => {
    const t = failedAt.get(key);
    if (t !== undefined && Date.now() - t >= 5 * 60 * 1000) { icons.delete(key); failedAt.delete(key); }
    return icons.has(key);
  };
  const wantIcon = new Map(); // key → [元素]
  let iconTimer = null, iconBusy = false, iconGen = 0;
  // 重新檢查時呼叫：丟掉讀到的圖示與正在讀的結果，下次重新向遊戲檔要
  function resetIcons() {
    iconGen++;
    icons.clear();
    failedAt.clear();
    wantIcon.clear();
  }
  function fillIcon(el, png) {
    el.innerHTML = "";
    if (png) { const img = new Image(); img.src = png; img.alt = ""; img.draggable = false; el.append(img); el.classList.remove("none"); }
    else el.classList.add("none");
  }
  async function flushIcons() {
    iconTimer = null;
    if (iconBusy || !TAURI) return;
    const keys = [...wantIcon.keys()].slice(0, 60);
    if (!keys.length) return;
    iconBusy = true;
    const gen = iconGen;
    let pngs = [];
    try { pngs = await invoke("item_icons_by_id", { gameDir: gameDir(), items: keys.map((k) => { const [f, id] = k.split("|"); return [f, Number(id)]; }) }); } catch (_) {}
    iconBusy = false;
    // 讀的過程中按了重新檢查：這批結果已經過期，丟掉
    if (gen !== iconGen) { if (wantIcon.size) flushIcons(); return; }
    keys.forEach((k, i) => {
      icons.set(k, pngs[i] || null);
      if (pngs[i]) failedAt.delete(k); else failedAt.set(k, Date.now());
      for (const el of wantIcon.get(k) || []) if (el.isConnected) fillIcon(el, pngs[i] || null);
      wantIcon.delete(k);
    });
    if (wantIcon.size) flushIcons();
  }
  const io = typeof IntersectionObserver !== "undefined" ? new IntersectionObserver((ents) => {
    for (const e of ents) {
      if (!e.isIntersecting) continue;
      io.unobserve(e.target);
      const key = e.target.dataset.key;
      const box = e.target.querySelector(".iu-ic");
      if (iconOf(key)) { fillIcon(box, icons.get(key)); continue; }
      if (!wantIcon.has(key)) wantIcon.set(key, []);
      wantIcon.get(key).push(box);
      if (!iconTimer) iconTimer = setTimeout(flushIcons, 30);
    }
  }, { root: null, rootMargin: "200px" }) : null;

  /* ---------- 畫面 ---------- */
  function renderVers() {
    const box = $("#iuVers");
    box.innerHTML = "";
    const recs = data?.records || [];
    recs.forEach((r, i) => {
      const b = document.createElement("button");
      b.className = "cbtn small c-orange" + (i === sel ? " on" : "");
      b.textContent = verText(r);
      b.title = `新增 ${nf.format(r.items.length)} 個道具（最後更新 ${fmtDay(r.at)}）`;
      b.addEventListener("click", () => { sel = i; render(); $("#iuPage").scrollTop = 0; });
      box.append(b);
    });
    box.hidden = !recs.length;
  }

  function renderCats(items) {
    const box = $("#iuCats");
    box.innerHTML = "";
    const count = {};
    for (const it of items) count[it.cat] = (count[it.cat] || 0) + 1;
    for (const c of CATS) {
      const n = c.id ? count[c.id] || 0 : items.length;
      if (c.id && !n) continue;
      const b = document.createElement("button");
      b.className = `cbtn small ${c.color}` + (c.id === cat ? " on" : "");
      b.innerHTML = `${c.name}<small>${nf.format(n)}</small>`;
      b.addEventListener("click", () => { cat = c.id; render(); $("#iuPage").scrollTop = 0; });
      box.append(b);
    }
  }

  function tile(it) {
    const b = document.createElement("button");
    b.className = "iu-item";
    b.title = `${it.name}（${it.id}）`;
    b.dataset.key = `${it.folder}|${it.id}`;
    const ic = document.createElement("span");
    ic.className = "iu-ic";
    b.append(ic);
    if (iconOf(b.dataset.key)) fillIcon(ic, icons.get(b.dataset.key));
    else if (io) io.observe(b);
    b.addEventListener("click", () => openDetail(it));
    return b;
  }

  function render() {
    const grid = $("#iuGrid");
    grid.innerHTML = "";
    const status = $("#iuStatus");
    const tools = $("#iuTools");
    if (!TAURI) { status.textContent = ""; tools.hidden = true; $("#iuVers").hidden = true; grid.innerHTML = '<div class="g-empty">預覽模式無法讀取遊戲資料</div>'; return; }
    if (!data) {
      tools.hidden = true;
      $("#iuVers").hidden = true;
      status.textContent = "";
      grid.innerHTML = `<div class="g-empty">${busy ? "讀取遊戲資料中…（第一次比較久）" : error || ""}</div>`;
      return;
    }
    const recs = data.records || [];
    if (sel >= recs.length) sel = 0;
    renderVers();
    const head = `目前遊戲版本 v${data.version}・共 ${nf.format(data.total)} 個道具`;
    if (!recs.length) {
      tools.hidden = true;
      status.textContent = head;
      grid.innerHTML = `<div class="g-empty iu-first"><b>已記錄 v${data.version} 的道具清單</b><span>遊戲大改版後打開這裡，就會列出新增了哪些道具。<br />程式開啟時也會自動檢查一次。</span></div>`;
      return;
    }
    const r = recs[sel];
    status.textContent = `${verText(r)}：新增 ${nf.format(r.items.length)} 個（包含之後的小更新，最後更新 ${fmtDay(r.at)}）・${head}`;
    tools.hidden = false;
    const all = r.items.map((it) => ({ ...it, cat: catOf(it) }));
    const needle = q.toLowerCase();
    const found = needle ? all.filter((it) => it.name.toLowerCase().includes(needle) || String(it.id).includes(needle)) : all;
    if (cat && !found.some((it) => it.cat === cat)) cat = "";
    renderCats(found);
    const list = cat ? found.filter((it) => it.cat === cat) : found;
    if (!list.length) { grid.innerHTML = '<div class="g-empty">沒有符合的道具</div>'; return; }
    const frag = document.createDocumentFragment();
    // 「全部」依分類分段
    for (const c of CATS.filter((x) => x.id && (!cat || x.id === cat))) {
      const part = list.filter((it) => it.cat === c.id);
      if (!part.length) continue;
      if (!cat) {
        const h = document.createElement("div");
        h.className = "iu-sec";
        h.innerHTML = `<span class="${c.color}">${c.name}</span><small class="muted">${nf.format(part.length)}</small>`;
        frag.append(h);
      }
      const wrap = document.createElement("div");
      wrap.className = "iu-items";
      part.forEach((it) => wrap.append(tile(it)));
      frag.append(wrap);
    }
    grid.append(frag);
  }

  /* ---------- 道具說明 ---------- */
  /** 遊戲的說明文字：\n 換行、#c…# 橘字（#b 藍、#r 紅、#g 綠、#e 粗體、#k #n 回復） */
  function descNodes(text) {
    const out = document.createDocumentFragment();
    const s = String(text || "").replace(/\\r\\n|\\n|\\r/g, "\n");
    let cls = "";
    let buf = "";
    const flush = () => {
      if (!buf) return;
      if (cls) { const sp = document.createElement("span"); sp.className = cls; sp.textContent = buf; out.append(sp); }
      else out.append(document.createTextNode(buf));
      buf = "";
    };
    for (let i = 0; i < s.length; i++) {
      const ch = s[i];
      if (ch !== "#") { buf += ch; continue; }
      const code = s[i + 1];
      const map = { c: "d-orange", b: "d-blue", r: "d-red", g: "d-green", e: "d-bold" };
      flush();
      if (code && map[code]) { cls = map[code]; i++; }
      else if (code === "k" || code === "n") { cls = ""; i++; }
      else cls = "";
    }
    flush();
    return out;
  }

  async function openDetail(it) {
    const m = $("#iuModal");
    $("#iuDetName").textContent = it.name || `道具 ${it.id}`;
    $("#iuDetMeta").textContent = `${catName(catOf(it))}・編號 ${it.id}`;
    const ic = $("#iuDetIc");
    const key = `${it.folder}|${it.id}`;
    fillIcon(ic, icons.get(key) || null);
    // 大圖示：放大兩倍（像素圖用整數倍才清楚）
    const big = ic.querySelector("img");
    if (big) { const two = () => { big.style.width = `${big.naturalWidth * 2}px`; }; if (big.complete) two(); else big.addEventListener("load", two, { once: true }); }
    const desc = $("#iuDetDesc");
    desc.textContent = "讀取中…";
    m.hidden = false;
    try {
      const t = await invoke("item_text", { gameDir: gameDir(), folder: it.folder, id: it.id });
      if (t.name) $("#iuDetName").textContent = t.name;
      desc.innerHTML = "";
      if (t.desc) desc.append(descNodes(t.desc));
      else desc.innerHTML = '<span class="muted">這個道具沒有說明</span>';
    } catch (e) {
      desc.textContent = String(e);
    }
  }
  const closeDetail = () => { $("#iuModal").hidden = true; };
  $("#iuDetClose").addEventListener("click", closeDetail);
  $("#iuModal").addEventListener("pointerdown", (e) => { if (e.target === $("#iuModal")) closeDetail(); });
  document.addEventListener("keydown", (e) => { if (e.key === "Escape" && !$("#iuModal").hidden) closeDetail(); });
  // 離開這個工具時收起說明
  document.addEventListener("viewchange", (e) => { if (e.detail !== "itemup") closeDetail(); });

  /* ---------- 讀取 ---------- */
  async function load(force, quiet) {
    if (busy || !TAURI) { render(); return; }
    busy = true;
    $("#iuRefresh").disabled = true;
    if (!quiet) render();
    try {
      const before = recSig(data);
      data = await invoke("item_updates", { gameDir: gameDir(), force: !!force });
      error = "";
      const changed = recSig(data) !== before;
      if (changed) sel = 0;
      // 遊戲檔更新或手動重新讀取後，圖示也重新讀（之前讀不到的可能已經有了）
      if (changed || force) resetIcons();
      if (force) toast(changed && data.records.length ? `道具有更新：${verText(data.records[0])} 共 ${nf.format(data.records[0].items.length)} 個` : "道具沒有變化");
    } catch (e) {
      error = String(e);
      if (force) toast(error);
    } finally {
      busy = false;
      $("#iuRefresh").disabled = false;
      render();
    }
  }

  let qTimer;
  const syncX = () => { $("#iuSearchX").hidden = !$("#iuSearch").value; };
  $("#iuSearch").addEventListener("input", (e) => {
    syncX();
    clearTimeout(qTimer);
    qTimer = setTimeout(() => { q = e.target.value.trim(); render(); }, 150);
  });
  $("#iuSearchX").addEventListener("click", () => { $("#iuSearch").value = ""; q = ""; syncX(); render(); $("#iuSearch").focus(); });
  $("#iuRefresh").addEventListener("click", () => load(true));

  render();
  window.ItemUpdates = { onShow: () => load(false) };
  // 程式一打開就在背景檢查（遊戲改版後第一次開程式時記下新增的道具）；主頁的道具圖示先讀完
  if (TAURI) setTimeout(() => (window.ItemIcons?.idle?.() || Promise.resolve()).then(() => load(false, true)), 2500);
})();
