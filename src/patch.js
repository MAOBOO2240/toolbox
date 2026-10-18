"use strict";

/* ==========================================================
 * 版本更新
 * - 每個大版本（V282、V281…）一行，連到官網的「改版頁面」（例如【楓之谷學院】那種活動頁，
 *   上方有楓之谷學院、變身燃燒、技能改善、系統改善…選單的那一頁）
 * - 版本與日期：「更新」「重要」公告裡的「V282 完整程式與手動更新下載開放通知」這類公告
 * - 改版頁面：改版當天前後的「活動」公告連到的活動頁，選單第一項就是改版主頁
 * - 很舊的版本官網已經沒有活動頁，改連當時的版本公告
 * - 依年份篩選；每次打開會向官網補上新的版本
 * - PatchCore：分組與挑選（可獨立測試）
 * ========================================================== */

const PatchCore = (() => {
  // 不是版本更新內容的公告（已知問題、制裁、補償、時間調整…）
  const BAD = /失敗|異常|制裁|問題|補償|延後|維護時間|檢測|錯誤|暫停|測試|說明與|LAG|不穩定|已知/i;
  /** 公告的代表性：改版內容 > 更新公告 > 完整程式 > 手動更新檔 > 其他 */
  function rank(t) {
    if (/改版內容|版內容說明|改版說明|內容說明/.test(t)) return 0;
    if (/版本更新公告|更新公告|更新為|改版開機|改版維護開機|更新開機|版本更新開機/.test(t)) return 1;
    if (/完整程式|完整主程式|完整安裝/.test(t)) return 2;
    if (/手動更新/.test(t)) return 3;
    return 4;
  }
  /** 標題裡的版本號：[大版本, 是否為小版本] */
  function versionOf(title) {
    const m = String(title).match(/[Vv][\s_.]?0*(\d{1,3})(?:\.(\d+))?/);
    if (!m || !+m[1]) return null;
    return [+m[1], !!m[2]];
  }
  const addDays = (d, n) => {
    const t = new Date(`${String(d).replace(/\//g, "-")}T00:00:00Z`);
    t.setUTCDate(t.getUTCDate() + n);
    return t.toISOString().slice(0, 10).replace(/-/g, "/");
  };
  /**
   * 每個大版本挑一則公告，並找出改版當天前後（前 1 天～後 3 天）連到活動頁的活動公告；新版本在前
   */
  function group(items) {
    const best = new Map();
    const events = [];
    for (const it of items || []) {
      if (!it || String(it.date).startsWith("19")) continue;
      if (it.kind === "72") { if (it.ev) events.push(it); continue; }
      if (BAD.test(it.title)) continue;
      const v = versionOf(it.title);
      if (!v) continue;
      const r = rank(it.title) + (v[1] ? 10 : 0);
      const cur = best.get(v[0]);
      if (!cur || r < cur.r || (r === cur.r && it.date < cur.it.date)) best.set(v[0], { r, it });
    }
    return [...best.entries()]
      .sort((a, b) => b[0] - a[0])
      .map(([v, { it }]) => {
        const from = addDays(it.date, -1), to = addDays(it.date, 3);
        const cands = [...new Set(events.filter((e) => e.date >= from && e.date <= to).map((e) => e.ev))].sort((a, b) => a - b);
        return { v, date: it.date, year: String(it.date).slice(0, 4), title: it.title, url: it.url, cands };
      });
  }
  /**
   * 依查到的改版主頁（活動編號 → 主頁）決定每個版本連到哪裡：
   * 候選活動頁的主頁中，選單最多的那個；同一個主頁只給一個版本（新的優先）
   */
  function attach(list, hubs) {
    const used = new Set();
    return list.map((x) => {
      const opts = x.cands.map((id) => hubs[id]).filter(Boolean).filter((h, i, a) => a.findIndex((y) => y.id === h.id) === i);
      opts.sort((a, b) => b.sections.length - a.sections.length || a.id - b.id);
      const hub = opts.find((h) => !used.has(h.id));
      if (hub) used.add(hub.id);
      return { ...x, hub: hub || null };
    });
  }
  return { rank, versionOf, group, attach, addDays };
})();

if (typeof module !== "undefined") module.exports = PatchCore;

/* ==========================================================
 * 介面
 * ========================================================== */
if (typeof document !== "undefined") (() => {
  const C = PatchCore;
  const KEY = "patch-v1";
  let year = (() => { try { return localStorage.getItem(KEY) || "all"; } catch (_) { return "all"; } })();
  let q = "";
  let base = [], list = [], hubs = {}, busy = false, loaded = false, gen = 0;
  // 年份按鈕各自的顏色（輪流）
  const YEAR_COLORS = ["c-blue", "c-teal", "c-green", "c-amber", "c-orange", "c-pink", "c-purple", "c-indigo", "c-red", "c-silver"];
  const EV_URL = "https://maplestory-event.beanfun.com/eventad/eventad?eventadid=";

  /** 公告標題去掉「新楓之谷」與書名號，比較好讀 */
  const clean = (t) => String(t).replace(/^新楓之谷\s*/, "").replace(/^《|》$/g, "").replace(/^新楓之谷\s*/, "").trim();
  const esc = (t) => String(t).replace(/[&<>"]/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;" }[c]));
  const pending = (x) => x.cands.some((id) => !(id in hubs));
  /** 只列更新內容：有改版頁面的版本；沒有改版頁面時只留改版內容／更新公告，不列完整程式、手動更新這類下載檔案 */
  const content = (x) => !!x.hub || C.rank(x.title) <= 1 || pending(x);
  const words = () => q.toLowerCase().split(/\s+/).filter(Boolean);
  function hay(x) {
    const parts = [`v${x.v}`, `v${String(x.v).padStart(3, "0")}`, x.date];
    if (x.hub) parts.push(x.hub.title, ...x.hub.sections.map((s) => s[1]));
    else parts.push(clean(x.title));
    return parts.join(" ").toLowerCase();
  }
  const matches = (x) => { const w = words(); if (!w.length) return true; const h = hay(x); return w.every((t) => h.includes(t.replace(/^v0*/, "v"))) || w.every((t) => h.includes(t)); };
  /** 搜尋到的字標出來 */
  function hl(text) {
    const w = words();
    let html = esc(text);
    if (!w.length) return html;
    const re = new RegExp(`(${w.map((t) => esc(t).replace(/[.*+?^${}()|[\]\\]/g, "\\$&")).join("|")})`, "gi");
    return html.replace(re, "<mark>$1</mark>");
  }

  function renderYears(rows) {
    const box = $("#ptYears");
    const years = [...new Set(rows.map((x) => x.year))].sort((a, b) => b - a);
    if (year !== "all" && loaded && !years.includes(year)) year = "all";
    box.innerHTML = "";
    const chip = (label, val, color) => {
      const b = document.createElement("button");
      b.className = `cbtn small ${color}` + (year === val ? " on" : "");
      b.textContent = label;
      b.addEventListener("click", () => {
        year = val;
        try { localStorage.setItem(KEY, year); } catch (_) {}
        render();
        $("#ptList").parentElement.scrollTop = 0;
      });
      box.appendChild(b);
      return b;
    };
    chip("全部", "all", "c-silver");
    years.forEach((y, i) => chip(y, y, YEAR_COLORS[i % YEAR_COLORS.length]));
    box.querySelector(".on")?.scrollIntoView({ block: "nearest", inline: "nearest" });
  }

  function open(url) {
    if (!TAURI) return toast("預覽模式無法開啟網址");
    invoke("open_bulletin", { url }).catch((e) => toast(String(e)));
  }

  function render() {
    list = C.attach(base, hubs);
    const shown = list.filter(content);
    renderYears(shown);
    const box = $("#ptList");
    const top = box.parentElement.scrollTop;
    box.innerHTML = "";
    const rows = shown.filter((x) => (year === "all" || x.year === year) && matches(x));
    if (!rows.length) {
      box.innerHTML = `<div class="card g-empty pt-empty">${!loaded ? "讀取中…" : q ? "沒有符合搜尋的版本" : "沒有找到版本更新"}</div>`;
      return;
    }
    let lastYear = null;
    for (const r of rows) {
      if (year === "all" && r.year !== lastYear) {
        lastYear = r.year;
        const h = document.createElement("div");
        h.className = "pt-year";
        h.textContent = `${r.year} 年`;
        box.appendChild(h);
      }
      const row = document.createElement("div");
      row.className = "card pt-row" + (r.hub ? " hub" : "");
      row.tabIndex = 0;
      const url = r.hub ? r.hub.url : r.url;
      row.title = r.hub ? `開啟改版頁面：${r.hub.title}` : `開啟官網公告：${clean(r.title)}`;
      const sub = r.hub ? "" : pending(r) ? "改版頁面讀取中…" : "官網已經沒有這個版本的改版頁面，改開當時的版本公告";
      row.innerHTML = `<span class="pt-ver">${hl(`V${r.v}`)}</span><span class="pt-date muted">${esc(r.date)}</span>` +
        `<span class="pt-main"><span class="pt-title">${hl(r.hub ? r.hub.title : clean(r.title))}</span>` +
        (r.hub ? `<span class="pt-secs"></span>` : `<small class="pt-sub muted">${esc(sub)}</small>`) +
        `</span><span class="pt-go">${r.hub ? "改版頁面" : "版本公告"}</span>`;
      // 改版頁面裡的每個項目：點一下直接開那一頁
      if (r.hub) {
        const secs = row.querySelector(".pt-secs");
        for (const [id, name] of r.hub.sections.slice(1)) {
          const b = document.createElement("button");
          b.className = "pt-sec";
          b.innerHTML = hl(name);
          b.title = `開啟「${name}」`;
          b.addEventListener("click", (e) => { e.stopPropagation(); open(EV_URL + id); });
          secs.appendChild(b);
        }
      }
      row.addEventListener("click", () => open(url));
      row.addEventListener("keydown", (e) => { if (e.key === "Enter") open(url); });
      box.appendChild(row);
    }
    box.parentElement.scrollTop = top;
  }

  /** 分批查改版主頁（查過的官網資料會記住，第一次比較久） */
  async function loadHubs(my) {
    for (let round = 0; round < 20 && my === gen; round++) {
      const need = [...new Set(base.flatMap((x) => x.cands).filter((id) => !(id in hubs)))];
      if (!need.length) break;
      $("#ptStatus").textContent = `讀取改版頁面中…（還有 ${need.length} 個）`;
      let got;
      try { got = await invoke("version_hubs", { ids: need.slice(0, 40) }); } catch (e) { $("#ptStatus").textContent = String(e); return false; }
      if (my !== gen) return false;
      let n = 0;
      for (const [k, h] of Object.entries(got || {})) { hubs[k] = h; n++; }
      // 這一批沒有任何進展（例如官網連不上）就停下來，避免一直重試
      if (!n) break;
      render();
    }
    return true;
  }

  async function refresh(force) {
    if (busy) return;
    if (!TAURI) { loaded = true; $("#ptStatus").textContent = "預覽模式無法讀取官網"; render(); return; }
    busy = true;
    const my = ++gen;
    $("#ptRefresh").disabled = true;
    $("#ptStatus").textContent = loaded ? "更新中…" : "從官網讀取中…（第一次比較久）";
    let status = "";
    try {
      const out = await invoke("version_list", { force: !!force });
      const before = new Set(base.map((x) => x.v));
      base = C.group(out.items);
      const added = loaded ? base.filter((x) => !before.has(x.v)) : [];
      loaded = true;
      render();
      // 改版主頁：查過的直接拿（不會連線），沒查過的才向官網查
      const known = [...new Set(base.flatMap((x) => x.cands))];
      await loadHubs(my);
      const t = new Date(out.fetched_at * 1000);
      const when = `${t.getMonth() + 1}/${t.getDate()} ${String(t.getHours()).padStart(2, "0")}:${String(t.getMinutes()).padStart(2, "0")}`;
      status = out.stale ? `連不上官網，顯示 ${when} 的資料` : `官網資料更新於 ${when}`;
      if (known.some((id) => !(id in hubs))) status += "・有些改版頁面還沒讀到，按重新整理再試";
      if (added.length) toast(`新增 ${added.map((x) => `V${x.v}`).join("、")}`);
    } catch (e) {
      loaded = true;
      status = String(e);
    } finally {
      busy = false;
      $("#ptRefresh").disabled = false;
      render();
      if (status) $("#ptStatus").textContent = status;
    }
  }

  let qTimer;
  const syncX = () => { $("#ptSearchX").hidden = !$("#ptSearch").value; };
  $("#ptSearch").addEventListener("input", (e) => {
    syncX();
    clearTimeout(qTimer);
    qTimer = setTimeout(() => { q = e.target.value.trim(); render(); $("#ptList").parentElement.scrollTop = 0; }, 150);
  });
  $("#ptSearchX").addEventListener("click", () => { $("#ptSearch").value = ""; q = ""; syncX(); render(); $("#ptSearch").focus(); });
  // 年份列：滑鼠滾輪左右捲動
  $("#ptYears").addEventListener("wheel", (e) => {
    const el = e.currentTarget;
    if (el.scrollWidth <= el.clientWidth || Math.abs(e.deltaX) > Math.abs(e.deltaY)) return;
    e.preventDefault();
    el.scrollLeft += e.deltaY;
  }, { passive: false });
  $("#ptRefresh").addEventListener("click", () => refresh(true));

  render();
  window.Patches = { onShow: () => refresh(false) };
})();
