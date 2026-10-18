"use strict";

/* ==========================================================
 * 版本更新
 * - 從官網公告（更新、重要）找出每個大版本（V282、V281…）的公告，一個版本一行
 * - 小版本（V256.2 這種）併到大版本裡，只在沒有大版本公告時才用
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
  /** 每個大版本挑一則公告；新版本在前 */
  function group(items) {
    const best = new Map();
    for (const it of items || []) {
      if (!it || BAD.test(it.title) || String(it.date).startsWith("19")) continue;
      const v = versionOf(it.title);
      if (!v) continue;
      const r = rank(it.title) + (v[1] ? 10 : 0);
      const cur = best.get(v[0]);
      if (!cur || r < cur.r || (r === cur.r && it.date < cur.it.date)) best.set(v[0], { r, it });
    }
    return [...best.entries()]
      .sort((a, b) => b[0] - a[0])
      .map(([v, { it }]) => ({ v, date: it.date, year: String(it.date).slice(0, 4), title: it.title, url: it.url }));
  }
  return { rank, versionOf, group };
})();

if (typeof module !== "undefined") module.exports = PatchCore;

/* ==========================================================
 * 介面
 * ========================================================== */
if (typeof document !== "undefined") (() => {
  const C = PatchCore;
  const KEY = "patch-v1";
  let year = (() => { try { return localStorage.getItem(KEY) || "all"; } catch (_) { return "all"; } })();
  let list = [], busy = false, loaded = false;

  /** 公告標題去掉「新楓之谷」與書名號，比較好讀 */
  const clean = (t) => String(t).replace(/^新楓之谷\s*/, "").replace(/^《|》$/g, "").replace(/^新楓之谷\s*/, "").trim();

  function renderYears() {
    const sel = $("#ptYear");
    const years = [...new Set(list.map((x) => x.year))].sort((a, b) => b - a);
    if (year !== "all" && !years.includes(year)) year = "all";
    sel.innerHTML = `<option value="all">全部年份（${list.length} 個版本）</option>` +
      years.map((y) => `<option value="${y}">${y} 年（${list.filter((x) => x.year === y).length}）</option>`).join("");
    sel.value = year;
  }

  function render() {
    renderYears();
    const box = $("#ptList");
    box.innerHTML = "";
    const rows = year === "all" ? list : list.filter((x) => x.year === year);
    if (!rows.length) {
      box.innerHTML = `<div class="card g-empty pt-empty">${loaded ? "沒有找到版本更新公告" : "讀取中…"}</div>`;
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
      const row = document.createElement("button");
      row.className = "card pt-row";
      row.title = `開啟官網公告：${r.title}`;
      row.innerHTML = `<span class="pt-ver"></span><span class="pt-date muted"></span><span class="pt-title"></span><span class="pt-go">開啟</span>`;
      row.querySelector(".pt-ver").textContent = `V${r.v}`;
      row.querySelector(".pt-date").textContent = r.date;
      row.querySelector(".pt-title").textContent = clean(r.title);
      row.addEventListener("click", () => {
        if (!TAURI) return toast("預覽模式無法開啟網址");
        invoke("open_bulletin", { url: r.url }).catch((e) => toast(String(e)));
      });
      box.appendChild(row);
    }
  }

  async function refresh(force) {
    if (busy) return;
    if (!TAURI) { loaded = true; $("#ptStatus").textContent = "預覽模式無法讀取官網"; render(); return; }
    busy = true;
    $("#ptRefresh").disabled = true;
    $("#ptStatus").textContent = loaded ? "更新中…" : "從官網讀取中…";
    try {
      const out = await invoke("version_list", { force: !!force });
      const before = new Set(list.map((x) => x.v));
      list = C.group(out.items);
      const added = loaded ? list.filter((x) => !before.has(x.v)) : [];
      loaded = true;
      const t = new Date(out.fetched_at * 1000);
      const when = `${t.getMonth() + 1}/${t.getDate()} ${String(t.getHours()).padStart(2, "0")}:${String(t.getMinutes()).padStart(2, "0")}`;
      $("#ptStatus").textContent = out.stale ? `連不上官網，顯示 ${when} 的資料` : `官網資料更新於 ${when}`;
      if (added.length) toast(`新增 ${added.map((x) => `V${x.v}`).join("、")}`);
    } catch (e) {
      loaded = true;
      $("#ptStatus").textContent = String(e);
    } finally {
      busy = false;
      $("#ptRefresh").disabled = false;
      render();
    }
  }

  $("#ptYear").addEventListener("change", (e) => {
    year = e.target.value;
    try { localStorage.setItem(KEY, year); } catch (_) {}
    render();
    $("#ptList").scrollTop = 0;
  });
  $("#ptRefresh").addEventListener("click", () => refresh(true));

  render();
  window.Patches = { onShow: () => refresh(false) };
})();
