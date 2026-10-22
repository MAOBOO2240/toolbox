"use strict";

/* ==========================================================
 * 活動預測
 * - 用官網「活動」公告的歷史（版本更新工具已經抓下來的那一份）推測下一次活動
 * - 兩種推法：
 *   · 週期型：同一個活動每隔固定天數出現一次（例如黃金蘋果），用過去的間隔推下一次
 *   · 年度型：每年同一個時候出現（例如中秋、聖誕），用去年的日期推今年
 * - 台版大多在週三維護時開始，所以過去多半落在週三時，預測也會對到週三
 * - 只是依照過去的規律推測，官方隨時可能改，僅供參考
 * - PredictCore：分組與推算（可獨立測試）
 * ========================================================== */

const PredictCore = (() => {
  const DAY = 86400000;

  /** 已經知道會重複出現的活動：用關鍵字認出是同一個活動（排在前面的先比對） */
  const FAMILIES = [
    { key: "apple", name: "黃金蘋果", cat: "gacha", re: /黃金蘋果/ },
    { key: "comet", name: "閃亮彗星", cat: "gacha", re: /閃亮彗星|璀璨彗星/ },
    { key: "beauty", name: "皇家美容院", cat: "gacha", re: /皇家美容院|皇家美髮|皇家整形/ },
    { key: "star", name: "星光錦囊", cat: "gacha", re: /星光錦囊/ },
    { key: "jump", name: "跳框洗洗樂", cat: "box", re: /跳框洗洗樂|洗洗樂/ },
    { key: "pandora", name: "潘朵拉箱子", cat: "box", re: /潘朵拉/ },
    { key: "cart", name: "魔法馬車", cat: "box", re: /魔法馬車/ },
    { key: "cards", name: "萌獸卡牌包", cat: "box", re: /萌獸卡牌|卡牌包/ },
    { key: "petbox", name: "寵物隨機箱", cat: "box", re: /寵物隨機箱|寵物箱/ },
    { key: "lottery", name: "幸運彩券", cat: "box", re: /幸運彩券/ },
    { key: "royal", name: "皇家禮盒", cat: "box", re: /皇家禮盒/ },
    { key: "burning", name: "變身燃燒", cat: "growth", re: /變身燃燒|燃燒成長|燃燒計畫/ },
    { key: "double", name: "經驗加倍", cat: "growth", re: /經驗加倍|雙倍經驗|加倍活動/ },
    { key: "attend", name: "出席活動", cat: "growth", re: /出席|簽到/ },
    { key: "newyear", name: "新年／春節", cat: "season", re: /新年|春節|過年|農曆/ },
    { key: "summer", name: "夏日活動", cat: "season", re: /盛夏|夏日|暑假|清涼/ },
    { key: "moon", name: "中秋", cat: "season", re: /中秋/ },
    { key: "halloween", name: "萬聖節", cat: "season", re: /萬聖/ },
    { key: "xmas", name: "聖誕", cat: "season", re: /聖誕/ },
    { key: "anniv", name: "週年慶", cat: "season", re: /週年慶|周年慶/ },
    { key: "childrens", name: "兒童節", cat: "season", re: /兒童節/ },
    { key: "valentine", name: "情人節", cat: "season", re: /情人節/ },
  ];

  const CATS = [
    { id: "gacha", name: "轉蛋", color: "c-amber" },
    { id: "box", name: "箱子", color: "c-teal" },
    { id: "growth", name: "成長", color: "c-green" },
    { id: "season", name: "節日", color: "c-pink" },
    { id: "other", name: "其他", color: "c-silver" },
  ];

  /** "YYYY/MM/DD" → 毫秒（當天 00:00）；格式不對回傳 null */
  function parseDay(s) {
    const m = /^(\d{4})[/-](\d{1,2})[/-](\d{1,2})/.exec(String(s || "").trim());
    if (!m) return null;
    const t = Date.UTC(+m[1], +m[2] - 1, +m[3]);
    return Number.isFinite(t) ? t : null;
  }
  const dayStr = (ms) => new Date(ms).toISOString().slice(0, 10).replace(/-/g, "/");
  /** 星期幾（0 日 ～ 6 六） */
  const weekday = (ms) => new Date(ms).getUTCDay();

  /** 標題去掉裝飾與日期，用來當作沒列在 FAMILIES 裡的活動的代號 */
  function cleanTitle(title) {
    return String(title || "")
      .replace(/[【】「」『』《》（）()\[\]]/g, " ")
      .replace(/\d{2,4}\s*[/年.-]\s*\d{1,2}\s*[/月.-]\s*\d{1,2}\s*日?/g, " ")
      .replace(/第\s*\d+\s*[彈波回期]/g, " ")
      .replace(/[Vv]\s*\d{2,3}/g, " ")
      .replace(/活動|公告|開跑|開始|登場|回歸|新增|限時|販售|開賣|上市|預告|延長|加碼/g, " ")
      .replace(/\s+/g, " ")
      .trim();
  }

  /** 這則公告屬於哪個活動 */
  function familyOf(title) {
    const t = String(title || "");
    for (const f of FAMILIES) {
      if (f.re.test(t)) return { key: f.key, name: f.name, cat: f.cat };
    }
    const c = cleanTitle(t);
    if (c.length < 2) return null;
    return { key: `x:${c}`, name: c, cat: "other" };
  }

  /**
   * 把公告分成一個個活動；同一個活動在同一天的多則公告只算一次，
   * 間隔太近（預設 10 天內）的也併成同一次（同一檔活動常有好幾則公告）
   */
  function collect(items, minGap = 10) {
    const map = new Map();
    for (const it of items || []) {
      if (String(it.kind) !== "72") continue;
      const day = parseDay(it.date);
      const fam = familyOf(it.title);
      if (day === null || !fam) continue;
      if (!map.has(fam.key)) map.set(fam.key, { key: fam.key, name: fam.name, cat: fam.cat, days: [], titles: [] });
      const g = map.get(fam.key);
      g.days.push(day);
      if (g.titles.length < 3) g.titles.push(it.title);
    }
    for (const g of map.values()) {
      const sorted = [...new Set(g.days)].sort((a, b) => a - b);
      // 同一檔活動的多則公告併成一次
      const runs = [];
      for (const d of sorted) {
        if (!runs.length || d - runs[runs.length - 1] >= minGap * DAY) runs.push(d);
      }
      g.days = runs;
    }
    return [...map.values()];
  }

  /** 間隔的中位數與離散程度 */
  function gapStats(days) {
    const gaps = [];
    for (let i = 1; i < days.length; i++) gaps.push((days[i] - days[i - 1]) / DAY);
    if (!gaps.length) return null;
    const sorted = [...gaps].sort((a, b) => a - b);
    const mid = sorted.length % 2 ? sorted[(sorted.length - 1) / 2] : (sorted[sorted.length / 2 - 1] + sorted[sorted.length / 2]) / 2;
    const mean = gaps.reduce((a, b) => a + b, 0) / gaps.length;
    const sd = Math.sqrt(gaps.reduce((a, b) => a + (b - mean) ** 2, 0) / gaps.length);
    return { gaps, median: mid, mean, sd, cv: mean > 0 ? sd / mean : 1 };
  }

  /** 把日期移到最近的星期三（台版大多在週三維護時開始） */
  function snapWed(ms) {
    const d = weekday(ms);
    let diff = 3 - d;
    if (diff > 3) diff -= 7;
    if (diff < -3) diff += 7;
    return ms + diff * DAY;
  }

  /**
   * 推測一個活動的下一次：
   * - 間隔接近一年 → 年度型，用最後一次的日期加一年
   * - 其他 → 週期型，用間隔的中位數
   * 回傳 null 表示資料不足以推測
   */
  function predictOne(g, now) {
    if (!g.days || g.days.length < 3) return null;
    const s = gapStats(g.days);
    if (!s || !(s.median > 0)) return null;
    const last = g.days[g.days.length - 1];
    const annual = s.median >= 300 && s.median <= 430;
    let next = annual ? last + Math.round(s.median) * DAY : last + Math.round(s.median) * DAY;
    // 已經過了就一直往後推，推到今天以後（最多推 3 年，避免資料很舊時算不完）
    const step = Math.max(1, Math.round(s.median)) * DAY;
    let guard = 0;
    while (next < now && guard++ < Math.ceil((3 * 365 * DAY) / step) + 2) next += step;
    // 過去多半在週三 → 預測也對到週三
    const wed = g.days.filter((d) => weekday(d) === 3).length / g.days.length;
    if (wed >= 0.6) next = snapWed(next);
    // 信心：次數越多、間隔越固定越高
    const n = g.days.length;
    let conf = "low";
    if (n >= 5 && s.cv <= 0.25) conf = "high";
    else if ((n >= 4 && s.cv <= 0.5) || (n >= 3 && s.cv <= 0.3)) conf = "mid";
    const range = Math.max(3, Math.min(30, Math.round(s.sd)));
    return { key: g.key, name: g.name, cat: g.cat, kind: annual ? "annual" : "cycle", next, range, conf, median: Math.round(s.median), n, last, days: g.days, titles: g.titles };
  }

  /** 全部活動的預測，照預測日期由近到遠；only 有值時只留這些分類 */
  function predict(items, now, opts = {}) {
    const within = (opts.withinDays || 180) * DAY;
    const out = [];
    for (const g of collect(items, opts.minGap)) {
      const p = predictOne(g, now);
      if (!p) continue;
      if (p.next - now > within) continue;
      if (opts.cats && opts.cats.length && !opts.cats.includes(p.cat)) continue;
      out.push(p);
    }
    out.sort((a, b) => a.next - b.next || a.name.localeCompare(b.name, "zh-TW"));
    return out;
  }

  return { DAY, FAMILIES, CATS, parseDay, dayStr, weekday, cleanTitle, familyOf, collect, gapStats, snapWed, predictOne, predict };
})();

if (typeof module !== "undefined") module.exports = PredictCore;

/* ==========================================================
 * 介面
 * ========================================================== */
if (typeof document !== "undefined") (() => {
  const C = PredictCore;
  const KEY = "predict-v1";
  let st = (() => { try { return { cats: [], ...(JSON.parse(localStorage.getItem(KEY)) || {}) }; } catch (_) { return { cats: [] }; } })();
  if (!Array.isArray(st.cats)) st.cats = [];
  const save = () => { try { localStorage.setItem(KEY, JSON.stringify(st)); } catch (_) {} };

  let items = null, fetchedAt = 0, stale = false, error = "", busy = false;

  const CONF = { high: ["很可能", "pd-high"], mid: ["有機會", "pd-mid"], low: ["參考看看", "pd-low"] };
  const nf = new Intl.NumberFormat("zh-TW");
  const WD = "日一二三四五六";
  function dateText(ms) {
    const d = new Date(ms);
    return `${d.getUTCMonth() + 1}月${d.getUTCDate()}日（週${WD[d.getUTCDay()]}）`;
  }
  function yearText(ms) {
    const d = new Date(ms);
    return d.getUTCFullYear() !== new Date().getFullYear() ? `${d.getUTCFullYear()}年` : "";
  }
  /** 還有幾天 */
  function inDays(ms, now) {
    const n = Math.round((ms - now) / C.DAY);
    if (n < 0) return `已經過了 ${-n} 天`;
    if (n === 0) return "就是今天";
    if (n === 1) return "明天";
    return `還有 ${n} 天`;
  }

  function renderCats(all) {
    const box = $("#pdCats");
    box.innerHTML = "";
    const count = {};
    for (const p of all) count[p.cat] = (count[p.cat] || 0) + 1;
    for (const c of C.CATS) {
      const n = count[c.id] || 0;
      if (!n) continue;
      const b = document.createElement("button");
      b.className = `cbtn small ${c.color}` + (st.cats.includes(c.id) ? " on" : "");
      b.innerHTML = `${c.name}<small>${nf.format(n)}</small>`;
      b.addEventListener("click", () => {
        st.cats = st.cats.includes(c.id) ? st.cats.filter((x) => x !== c.id) : [...st.cats, c.id];
        save(); render();
      });
      box.append(b);
    }
  }

  function card(p, now) {
    const el = document.createElement("div");
    el.className = "card pd-card";
    const [confText, confCls] = CONF[p.conf] || CONF.low;
    const cat = C.CATS.find((c) => c.id === p.cat) || C.CATS[C.CATS.length - 1];

    const head = document.createElement("div");
    head.className = "pd-head";
    const when = document.createElement("div");
    when.className = "pd-when";
    when.innerHTML = `<b></b><small class="muted"></small>`;
    when.children[0].textContent = `${yearText(p.next)}${dateText(p.next)}`;
    when.children[1].textContent = `${inDays(p.next, now)}・誤差約 ${p.range} 天`;
    const name = document.createElement("div");
    name.className = "pd-name";
    name.innerHTML = `<b></b><small class="muted"></small>`;
    name.children[0].textContent = p.name;
    name.children[1].textContent = p.kind === "annual" ? `每年這個時候（過去 ${p.n} 次）` : `大約每 ${p.median} 天一次（過去 ${p.n} 次）`;
    const tags = document.createElement("div");
    tags.className = "pd-tags";
    const ct = document.createElement("span");
    ct.className = `cbtn small ${cat.color} on pd-tag`;
    ct.textContent = cat.name;
    const cf = document.createElement("span");
    cf.className = `pd-conf ${confCls}`;
    cf.textContent = confText;
    tags.append(ct, cf);
    head.append(when, name, tags);

    const past = document.createElement("div");
    past.className = "pd-past";
    const lbl = document.createElement("span");
    lbl.className = "muted";
    lbl.textContent = "過去";
    past.append(lbl);
    for (const d of p.days.slice(-6)) {
      const x = document.createElement("span");
      x.className = "pd-day";
      x.textContent = C.dayStr(d).slice(2);
      past.append(x);
    }
    if (p.titles && p.titles.length) past.title = `公告標題例如：${p.titles.join("、")}`;
    el.append(head, past);
    return el;
  }

  function render() {
    const list = $("#pdList");
    const status = $("#pdStatus");
    list.innerHTML = "";
    if (!TAURI) { status.textContent = ""; $("#pdCats").innerHTML = ""; list.innerHTML = '<div class="card g-empty">預覽模式無法讀取官網公告</div>'; return; }
    if (!items) {
      $("#pdCats").innerHTML = "";
      status.textContent = "";
      list.innerHTML = `<div class="card g-empty">${busy ? "讀取官網公告中…" : error || ""}</div>`;
      return;
    }
    const now = Date.UTC(new Date().getFullYear(), new Date().getMonth(), new Date().getDate());
    const all = C.predict(items, now, { withinDays: 180 });
    renderCats(all);
    const shown = st.cats.length ? all.filter((p) => st.cats.includes(p.cat)) : all;
    const events = items.filter((i) => String(i.kind) === "72").length;
    status.textContent = `依官網 ${nf.format(events)} 則活動公告推測・共 ${nf.format(all.length)} 個活動`
      + (fetchedAt ? `・${new Date(fetchedAt * 1000).toLocaleDateString("zh-TW")} 更新` : "")
      + (stale ? "（這次沒連上官網，用之前存的）" : "");
    if (!shown.length) {
      list.innerHTML = '<div class="card g-empty">沒有可以推測的活動（官網公告的歷史還不夠，同一個活動要出現三次以上才推測）</div>';
      return;
    }
    const frag = document.createDocumentFragment();
    for (const p of shown) frag.append(card(p, now));
    list.append(frag);
    const note = document.createElement("p");
    note.className = "hint pd-note";
    note.textContent = "只是照過去的規律推測，官方隨時可能改，日期僅供參考。";
    list.append(note);
  }

  async function load(force) {
    if (busy || !TAURI) { render(); return; }
    busy = true;
    $("#pdRefresh").disabled = true;
    render();
    try {
      const out = await invoke("version_list", { force: !!force });
      items = out.items || [];
      fetchedAt = out.fetched_at || 0;
      stale = !!out.stale;
      error = out.error || "";
    } catch (e) {
      error = String(e);
      if (force) toast(error);
    } finally {
      busy = false;
      $("#pdRefresh").disabled = false;
      render();
    }
  }

  $("#pdRefresh").addEventListener("click", () => load(true));
  render();
  window.Predict = { onShow: () => { if (!items) load(false); else render(); } };
})();
