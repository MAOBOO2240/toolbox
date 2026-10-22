"use strict";

/* ==========================================================
 * 官網獎池：黃金蘋果、美容美髮、閃亮彗星、星光錦囊
 * - 官網的機率頁是固定的活動頁（換獎池時直接更新同一頁）
 * - 自動檢查只在每週三（官網通常週三換獎池）：
 *   · 00:00～18:00 每 1 小時檢查一次；當天有找到新獎池就不再檢查
 *   · 上週三有找到新獎池時，這週三只在 12:00～18:00 檢查
 *   · 其他時間都不自動檢查；按「從官網更新」隨時可以手動檢查
 * - 頁面轉成文字的方式和「全選複製貼上」一樣，再用各工具原本的解析
 * - 內容和上次套用的不同才換獎池；自動檢查時機率加總不是 100% 就保留原本的獎池並提醒
 *   （按「從官網更新」一定套用）
 * ========================================================== */

const OfficialPools = (() => {
  const SITE = "https://maplestory-event.beanfun.com";
  // words：官網「活動」公告標題裡要找的名稱
  const PAGES = {
    gacha: { id: 8369, name: "黃金蘋果", words: ["黃金蘋果"] },
    beauty: { id: 8372, name: "皇家美容院", words: ["皇家美容院", "皇家美髮", "皇家整形"] },
    comet: { id: 8368, name: "閃亮彗星", words: ["閃亮彗星", "璀璨彗星"] },
    star: { id: 8370, name: "星光錦囊", words: ["星光錦囊"] },
  };
  const KEY = "official-pools-v1";
  const HOUR = 3600 * 1000;
  let st = (() => { try { return JSON.parse(localStorage.getItem(KEY)) || {}; } catch (_) { return {}; } })();
  const save = () => { try { localStorage.setItem(KEY, JSON.stringify(st)); } catch (_) {} };
  const rec = (tool) => (st[tool] ||= { auto: true });
  const tools = {};
  const busy = {};

  /** 簡單的雜湊，用來比較獎池內容有沒有變 */
  function hashOf(obj) {
    const s = JSON.stringify(obj);
    let h = 2166136261;
    for (let i = 0; i < s.length; i++) h = Math.imul(h ^ s.charCodeAt(i), 16777619);
    return `${(h >>> 0).toString(16)}-${s.length}`;
  }

  /**
   * HTML 轉成文字：放進看不見的框裡排版後取 innerText（和全選複製一樣，表格每格之間是 Tab）
   * 不執行官網的程式、不載入圖片；withCss = false 時連樣式也不載入（有些內容要靠程式切換才看得到）
   */
  function htmlText(html, withCss) {
    return new Promise((resolve) => {
      let clean = String(html)
        .replace(/<script\b[\s\S]*?<\/script>/gi, "")
        .replace(/<noscript\b[\s\S]*?<\/noscript>/gi, "")
        .replace(/<(iframe|video|audio|object)\b[\s\S]*?<\/\1>/gi, "")
        .replace(/<(img|source|embed|iframe|video|audio)\b[^>]*>/gi, "");
      if (!withCss) clean = clean.replace(/<style\b[\s\S]*?<\/style>/gi, "").replace(/<link\b[^>]*>/gi, "");
      clean = /<head[^>]*>/i.test(clean) ? clean.replace(/<head([^>]*)>/i, `<head$1><base href="${SITE}/">`) : `<base href="${SITE}/">${clean}`;
      const f = document.createElement("iframe");
      f.setAttribute("sandbox", "allow-same-origin");
      f.setAttribute("aria-hidden", "true");
      f.tabIndex = -1;
      f.style.cssText = "position:fixed;left:-30000px;top:0;width:1280px;height:900px;opacity:0;pointer-events:none;border:0";
      let done = false;
      const finish = () => {
        if (done) return;
        done = true;
        let t = "";
        try { t = f.contentDocument?.body?.innerText || ""; } catch (_) {}
        f.remove();
        resolve(t);
      };
      f.addEventListener("load", () => setTimeout(finish, 60));
      setTimeout(finish, 15000);
      f.srcdoc = clean;
      document.body.appendChild(f);
    });
  }

  /** 活動時間：2026/10/07 09:00 ~ 2026/10/20 23:59 */
  function periodOf(text) {
    const m = /活動時間[^\d]{0,40}?(\d{4}\/\d{1,2}\/\d{1,2})(?:\s+\d{1,2}:\d{2})?\s*[~～\-－至]\s*(\d{4}\/\d{1,2}\/\d{1,2})(?:\s+(\d{1,2}:\d{2}))?/.exec(text || "");
    return m ? { start: m[1], end: m[2], endTime: m[3] || "23:59" } : null;
  }
  const count = (parsed) => Object.values(parsed || {}).reduce((a, l) => a + (Array.isArray(l) ? l.length : 0), 0);

  /** 有樣式的版本為主；樣式藏起來的獎池（例如要切換分頁才看得到）用沒有樣式的版本補上 */
  function merge(a, b) {
    const out = { ...a };
    for (const [k, v] of Object.entries(b || {})) if (Array.isArray(v) && v.length && !(out[k] && out[k].length)) out[k] = v;
    return out;
  }

  const fmtTime = (ms) => { const d = new Date(ms); return `${d.getMonth() + 1}/${d.getDate()} ${String(d.getHours()).padStart(2, "0")}:${String(d.getMinutes()).padStart(2, "0")}`; };

  function renderStatus(tool, extra) {
    const t = tools[tool];
    if (!t) return;
    const r = rec(tool);
    t.auto.checked = r.auto !== false;
    const parts = [];
    if (extra) parts.push(extra);
    else {
      if (r.period) parts.push(`活動時間 ${r.period.start} ~ ${r.period.end}`);
      if (r.summary) parts.push(`官網獎池：${r.summary}`);
      if (r.checked) parts.push(`${fmtTime(r.checked)} 檢查`);
      if (r.held) parts.push(`官網機率加總 ${r.held.sum}%，沒有自動套用（按「從官網更新」可以套用）`);
      parts.push(r.auto !== false ? `每週三 ${weekStartHour()}:00～18:00 自動檢查` : "自動更新已關閉，可以按「從官網更新」");
    }
    t.status.textContent = parts.join("・");
    t.btn.disabled = !!busy[tool];
  }

  /** 每個獎池的機率加總；有不是 100% 的就回傳那個加總（四捨五入到小數 2 位），都正常回傳 null */
  function sumsOff(parsed) {
    for (const v of Object.values(parsed || {})) {
      if (!Array.isArray(v) || !v.length) continue;
      const sum = v.reduce((a, it) => a + (Number(it.prob) || 0), 0);
      if (Math.abs(sum - 100) > 0.1) return +sum.toFixed(2);
    }
    return null;
  }

  /** 回傳這次有沒有找到新的獎池（自動檢查用來決定當天要不要停） */
  async function sync(tool, manual) {
    const t = tools[tool];
    if (!t || busy[tool]) return false;
    let found = false;
    if (!TAURI) { if (manual) toast("預覽模式無法讀取官網"); return false; }
    busy[tool] = true;
    renderStatus(tool, "從官網讀取中…");
    const r = rec(tool);
    let note = "";
    try {
      const html = await invoke("pool_page", { id: PAGES[tool].id });
      const text = await htmlText(html, true);
      let parsed = t.parse(text);
      const plain = await htmlText(html, false);
      parsed = merge(parsed, t.parse(plain));
      if (!count(parsed)) throw new Error("官網頁面讀不到機率（可能改版了），可以先用下面的貼上解析");
      r.period = periodOf(text) || periodOf(plain) || r.period || null;
      r.checked = Date.now();
      const h = hashOf(parsed);
      if (manual || h !== r.hash) {
        // 跟之前套用過的獎池不一樣 = 官網換獎池了（第一次讀取不算）
        found = !manual && !!r.hash;
        const off = sumsOff(parsed);
        if (!manual && off) {
          // 機率加總不是 100%：自動檢查時保留原本的獎池並提醒
          if (!r.held || r.held.hash !== h) toast(`${PAGES[tool].name}：官網機率加總 ${off}%，不是 100%，先保留原本的獎池（要套用請到解析頁按「從官網更新」）`);
          r.held = { hash: h, sum: off };
        } else {
          const msg = t.apply(parsed);
          if (!msg) throw new Error("官網頁面讀不到機率（可能改版了），可以先用下面的貼上解析");
          r.held = null;
          r.hash = h;
          r.summary = msg;
          toast(`${PAGES[tool].name}已依官網更新獎池：${msg}${off ? `（官網機率加總 ${off}%，不是 100%）` : ""}`);
        }
      }
    } catch (e) {
      note = String(e?.message || e);
      if (manual) toast(note);
    } finally {
      busy[tool] = false;
      save();
      renderStatus(tool, note || null);
    }
    return found;
  }

  /* ---------- 自動檢查：每週三 ---------- */
  /** 台灣時間的日期（YYYY-MM-DD）、星期、小時 */
  const tw = (ms) => { const d = new Date(ms + 8 * HOUR); return { ymd: d.toISOString().slice(0, 10), day: d.getUTCDay(), hour: d.getUTCHours() }; };
  const sched = () => (st._sched ||= {});
  /** 這週三（今天是週三就是今天，不然是下一個週三）從幾點開始查：前一個週三有找到新獎池就從 12 點，否則從 0 點 */
  function weekStartHour(now = Date.now()) {
    const ahead = (3 - tw(now).day + 7) % 7;
    const prevWed = tw(now + (ahead - 7) * 24 * HOUR).ymd;
    return sched().found === prevWed ? 12 : 0;
  }
  /** 現在要不要自動檢查 */
  function due(now = Date.now()) {
    const t = tw(now), s = sched();
    if (t.day !== 3 || t.hour >= 18 || t.hour < weekStartHour(now)) return false;
    if (s.found === t.ymd) return false; // 今天已經找到了
    return !s.last || now - s.last >= HOUR;
  }
  async function autoCheck() {
    if (!TAURI || !due()) return;
    const s = sched();
    s.last = Date.now();
    save();
    let found = false;
    for (const tool of Object.keys(tools)) {
      if (rec(tool).auto === false) continue;
      if (await sync(tool, false)) found = true;
    }
    if (found) { sched().found = tw(Date.now()).ymd; save(); }
  }
  // 開著的時候每分鐘看一次是不是該檢查了（不是週三的檢查時間就什麼都不做）
  setTimeout(autoCheck, 5000);
  setInterval(autoCheck, 60 * 1000);
  /** 舊版打開工具時會檢查；現在只在週三的檢查時間自動檢查 */
  function onOpen() {}

  /**
   * 工具註冊：在解析頁的「文字解析」卡片上方加一列「官網獎池」
   * opts = { card, parse(text) → 各獎池, apply(parsed) → 套用說明文字或 null }
   */
  function register(tool, opts) {
    const bar = document.createElement("div");
    bar.className = "off-bar";
    bar.innerHTML = `<div class="off-main"><b>官網獎池</b><small class="muted"></small></div>` +
      `<label class="off-auto" title="每週三自動檢查官網，有換獎池就自動換成新的"><span>自動更新</span><span class="switch sm"><input type="checkbox" /><span class="slider"></span></span></label>` +
      `<button class="cbtn c-teal small solid">從官網更新</button>`;
    const h3 = opts.card.querySelector("h3");
    h3 ? h3.after(bar) : opts.card.prepend(bar);
    const t = { ...opts, status: bar.querySelector("small"), auto: bar.querySelector("input"), btn: bar.querySelector("button") };
    tools[tool] = t;
    t.auto.addEventListener("change", () => {
      rec(tool).auto = t.auto.checked;
      save();
      renderStatus(tool);
      if (t.auto.checked) autoCheck();
    });
    t.btn.addEventListener("click", () => sync(tool, true));
    renderStatus(tool);
  }

  return { register, onOpen, sync, periodOf, due, weekStartHour, PAGES };
})();

if (typeof module !== "undefined") module.exports = OfficialPools;
