"use strict";

/* ==========================================================
 * 官網獎池：黃金蘋果、美容美髮、閃亮彗星、星光錦囊
 * - 官網的機率頁是固定的活動頁（換獎池時直接更新同一頁）
 * - 打開工具時決定要不要檢查：
 *   · 已經知道活動結束時間：結束當天起才查（3 小時內查過就不再查），讀到新的活動時間就停
 *   · 官網「活動」公告出現新的同名公告（例如新的黃金蘋果公告）：馬上查
 *   · 還不知道活動時間：3 小時內查過就不再查
 * - 頁面轉成文字的方式和「全選複製貼上」一樣，再用各工具原本的解析
 * - 內容和上次套用的不同才換獎池；按「從官網更新」一定重新套用
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
  const GAP = 3 * 3600 * 1000;
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
  /** 某天 00:00（台灣時間） */
  const dayStart = (ymd) => {
    const [y, m, d] = String(ymd).split("/").map(Number);
    return Date.UTC(y, (m || 1) - 1, d || 1) - 8 * 3600 * 1000;
  };
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
      if (r.period && r.auto !== false) parts.push(Date.now() >= dayStart(r.period.end) ? "活動已到結束日，打開工具時會檢查新獎池" : `${r.period.end.slice(5)} 活動結束當天起自動檢查`);
      if (!parts.length) parts.push(r.auto !== false ? "打開工具時會自動檢查官網有沒有換獎池" : "自動更新已關閉，可以按「從官網更新」");
    }
    t.status.textContent = parts.join("・");
    t.btn.disabled = !!busy[tool];
  }

  async function sync(tool, manual) {
    const t = tools[tool];
    if (!t || busy[tool]) return;
    if (!TAURI) { if (manual) toast("預覽模式無法讀取官網"); return; }
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
        const msg = t.apply(parsed);
        if (!msg) throw new Error("官網頁面讀不到機率（可能改版了），可以先用下面的貼上解析");
        r.hash = h;
        r.summary = msg;
        toast(`${PAGES[tool].name}已依官網更新獎池：${msg}`);
      }
    } catch (e) {
      note = String(e?.message || e);
      if (manual) toast(note);
    } finally {
      busy[tool] = false;
      save();
      renderStatus(tool, note || null);
    }
  }

  /** 官網「活動」公告裡最新一則同名公告（用版本更新工具同一份公告資料，6 小時內不重查官網） */
  async function latestBulletin(tool) {
    try {
      const out = await invoke("version_list", { force: false });
      const p = PAGES[tool];
      const hits = (out?.items || []).filter((it) => it.kind === "72" && (it.ev === p.id || p.words.some((w) => String(it.title).includes(w))));
      hits.sort((a, b) => (a.date === b.date ? Number(b.id) - Number(a.id) : a.date < b.date ? 1 : -1));
      return hits[0] || null;
    } catch (_) {
      return null;
    }
  }

  /** 打開工具時：依活動結束時間與官網公告決定要不要查 */
  async function onOpen(tool) {
    if (!tools[tool] || !TAURI) return;
    const r = rec(tool);
    if (r.auto === false || busy[tool]) return;
    const now = Date.now();
    const fresh = r.checked && now - r.checked < GAP;
    // 已經知道活動時間：結束當天起才查；還不知道：3 小時查一次
    let due = r.period ? now >= dayStart(r.period.end) && !fresh : !fresh;
    const top = await latestBulletin(tool);
    if (top && top.id !== r.bul) {
      // 出現新的同名活動公告（第一次只記下來）
      if (r.bul) due = true;
      r.bul = top.id;
      save();
    }
    if (due) sync(tool, false);
  }

  /**
   * 工具註冊：在解析頁的「文字解析」卡片上方加一列「官網獎池」
   * opts = { card, parse(text) → 各獎池, apply(parsed) → 套用說明文字或 null }
   */
  function register(tool, opts) {
    const bar = document.createElement("div");
    bar.className = "off-bar";
    bar.innerHTML = `<div class="off-main"><b>官網獎池</b><small class="muted"></small></div>` +
      `<label class="off-auto" title="打開工具時自動檢查官網，有換獎池就自動換成新的"><span>自動更新</span><span class="switch sm"><input type="checkbox" /><span class="slider"></span></span></label>` +
      `<button class="cbtn c-teal small solid">從官網更新</button>`;
    const h3 = opts.card.querySelector("h3");
    h3 ? h3.after(bar) : opts.card.prepend(bar);
    const t = { ...opts, status: bar.querySelector("small"), auto: bar.querySelector("input"), btn: bar.querySelector("button") };
    tools[tool] = t;
    t.auto.addEventListener("change", () => {
      rec(tool).auto = t.auto.checked;
      save();
      renderStatus(tool);
      if (t.auto.checked) onOpen(tool);
    });
    t.btn.addEventListener("click", () => sync(tool, true));
    renderStatus(tool);
  }

  return { register, onOpen, sync, periodOf, PAGES };
})();

if (typeof module !== "undefined") module.exports = OfficialPools;
