"use strict";

/* ==========================================================
 * 活動預測
 * - 推測「下一次大改版」什麼時候，以及那次改版大概會有哪些活動
 * - 資料來自官網公告（版本更新工具已經抓下來的那一份）：
 *   · 改版日期：更新／重要公告裡的版本號（V282、V283…）
 *   · 改版活動：改版前後幾天內的活動公告
 * - 商城輪替（黃金蘋果、閃亮彗星、皇家美容院、星光錦囊…）不算改版活動，
 *   那些有自己的工具，也沒有和改版綁在一起
 * - 只顯示未來：不列過去的日期
 * - 只是照過去的規律推測，官方隨時可能改
 * - PredictCore：分組與推算（可獨立測試）
 * ========================================================== */

const PredictCore = (() => {
  const DAY = 86400000;

  /** 已經知道會重複出現的活動：用關鍵字認出是同一個活動（排在前面的先比對） */
  const FAMILIES = [
    // shop：商城輪替，不算改版活動
    { key: "apple", name: "黃金蘋果", cat: "shop", re: /黃金蘋果/ },
    { key: "comet", name: "閃亮彗星", cat: "shop", re: /閃亮彗星|璀璨彗星/ },
    { key: "beauty", name: "皇家美容院", cat: "shop", re: /皇家美容院|皇家美髮|皇家整形/ },
    { key: "star", name: "星光錦囊", cat: "shop", re: /星光錦囊/ },
    { key: "jump", name: "跳框洗洗樂", cat: "shop", re: /跳框洗洗樂|洗洗樂/ },
    { key: "pandora", name: "潘朵拉箱子", cat: "shop", re: /潘朵拉/ },
    { key: "cart", name: "魔法馬車", cat: "shop", re: /魔法馬車/ },
    { key: "cards", name: "萌獸卡牌包", cat: "shop", re: /萌獸卡牌|卡牌包/ },
    { key: "petbox", name: "寵物隨機箱", cat: "shop", re: /寵物隨機箱|寵物箱/ },
    { key: "lottery", name: "幸運彩券", cat: "shop", re: /幸運彩券/ },
    { key: "royal", name: "皇家禮盒", cat: "shop", re: /皇家禮盒/ },
    // 改版常見的活動
    { key: "burning", name: "變身燃燒", cat: "growth", re: /變身燃燒|燃燒成長|燃燒計畫|燃燒事件/ },
    { key: "double", name: "經驗加倍", cat: "growth", re: /經驗加倍|雙倍經驗|加倍活動|經驗值加成/ },
    { key: "attend", name: "出席活動", cat: "growth", re: /出席|簽到/ },
    { key: "hyper", name: "衝等活動", cat: "growth", re: /飛躍|衝級|衝等|升級活動/ },
    { key: "back", name: "回歸玩家活動", cat: "growth", re: /回歸玩家|歸來|回鍋/ },
    { key: "newyear", name: "新年／春節", cat: "season", re: /新年|春節|過年|農曆/ },
    { key: "summer", name: "夏日活動", cat: "season", re: /盛夏|夏日|暑假|清涼/ },
    { key: "moon", name: "中秋", cat: "season", re: /中秋/ },
    { key: "halloween", name: "萬聖節", cat: "season", re: /萬聖/ },
    { key: "xmas", name: "聖誕", cat: "season", re: /聖誕/ },
    { key: "anniv", name: "週年慶", cat: "season", re: /週年慶|周年慶/ },
    { key: "childrens", name: "兒童節", cat: "season", re: /兒童節/ },
    { key: "valentine", name: "情人節", cat: "season", re: /情人節/ },
  ];

  /** 看得到的分類（商城輪替不列） */
  const CATS = [
    { id: "growth", name: "成長", color: "c-green" },
    { id: "season", name: "節日", color: "c-pink" },
    { id: "other", name: "其他", color: "c-teal" },
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
  /** 移到最近的星期三（台版大多在週三維護時改版） */
  function snapWed(ms) {
    const d = weekday(ms);
    let diff = 3 - d;
    if (diff > 3) diff -= 7;
    if (diff < -3) diff += 7;
    return ms + diff * DAY;
  }
  /** 一年當中的第幾天 */
  const dayOfYear = (ms) => Math.round((ms - Date.UTC(new Date(ms).getUTCFullYear(), 0, 1)) / DAY);
  /** 兩個日子在一年當中差幾天（跨年頭年尾也算得對） */
  function yearGap(a, b) {
    const d = Math.abs(dayOfYear(a) - dayOfYear(b));
    return Math.min(d, 365 - d);
  }

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

  /** 這則活動公告屬於哪個活動 */
  function familyOf(title) {
    const t = String(title || "");
    for (const f of FAMILIES) {
      if (f.re.test(t)) return { key: f.key, name: f.name, cat: f.cat };
    }
    const c = cleanTitle(t);
    if (c.length < 2) return null;
    return { key: `x:${c}`, name: c, cat: "other" };
  }

  /** 標題裡的大版本號；回傳 null 表示不是版本公告 */
  function versionOf(title) {
    const m = String(title || "").match(/[Vv][\s_.]?0*(\d{1,3})(?:\.(\d+))?/);
    if (!m || !+m[1]) return null;
    return { ver: +m[1], minor: !!m[2] };
  }

  /** 每個大版本第一則公告的日期，由舊到新 */
  function majors(items) {
    const byVer = new Map();
    for (const it of items || []) {
      const k = String(it.kind);
      if (k !== "67" && k !== "68") continue;
      const v = versionOf(it.title);
      const day = parseDay(it.date);
      if (!v || v.minor || day === null) continue;
      if (!byVer.has(v.ver) || day < byVer.get(v.ver)) byVer.set(v.ver, day);
    }
    return [...byVer.entries()].map(([ver, day]) => ({ ver, day })).sort((a, b) => a.day - b.day);
  }

  /** 間隔的中位數與離散程度 */
  function gapStats(days) {
    const gaps = [];
    for (let i = 1; i < days.length; i++) gaps.push((days[i] - days[i - 1]) / DAY);
    if (!gaps.length) return null;
    const sorted = [...gaps].sort((a, b) => a - b);
    const median = sorted.length % 2 ? sorted[(sorted.length - 1) / 2] : (sorted[sorted.length / 2 - 1] + sorted[sorted.length / 2]) / 2;
    const mean = gaps.reduce((a, b) => a + b, 0) / gaps.length;
    const sd = Math.sqrt(gaps.reduce((a, b) => a + (b - mean) ** 2, 0) / gaps.length);
    return { gaps, median, mean, sd, cv: mean > 0 ? sd / mean : 1 };
  }

  /** 下一次大改版：版本號與預計日期 */
  function nextMajor(ms, now, look = 8) {
    if (!ms || ms.length < 3) return null;
    const recent = ms.slice(-look);
    const s = gapStats(recent.map((x) => x.day));
    if (!s || !(s.median > 0)) return null;
    const last = ms[ms.length - 1];
    const step = Math.max(1, Math.round(s.median)) * DAY;
    let day = last.day + step;
    let ver = last.ver + 1;
    let guard = 0;
    while (day < now && guard++ < 60) { day += step; ver += 1; }
    const wed = recent.filter((x) => weekday(x.day) === 3).length / recent.length;
    if (wed >= 0.6) day = snapWed(day);
    return { ver, day, range: Math.max(3, Math.min(30, Math.round(s.sd))), median: Math.round(s.median), n: recent.length, lastVer: last.ver };
  }

  /** 每則活動公告的日期，依活動分組（商城輪替不算） */
  function eventDays(items) {
    const map = new Map();
    for (const it of items || []) {
      if (String(it.kind) !== "72") continue;
      const day = parseDay(it.date);
      const fam = familyOf(it.title);
      if (day === null || !fam || fam.cat === "shop") continue;
      if (!map.has(fam.key)) map.set(fam.key, { key: fam.key, name: fam.name, cat: fam.cat, days: [] });
      map.get(fam.key).days.push(day);
    }
    for (const g of map.values()) g.days = [...new Set(g.days)].sort((a, b) => a - b);
    return map;
  }

  /** 每次改版前後出現過哪些活動：版本 → 活動代號的集合 */
  function patchEvents(groups, ms, before = 3, after = 10) {
    const per = new Map(ms.map((m) => [m.ver, new Set()]));
    for (const g of groups.values()) {
      for (const d of g.days) {
        for (const m of ms) {
          if (d >= m.day - before * DAY && d <= m.day + after * DAY) { per.get(m.ver).add(g.key); break; }
        }
      }
    }
    return per;
  }

  /**
   * 推測下一次大改版與那次會有的活動。
   * 回傳 { next, events, all, majors }；資料不夠時 next 為 null
   */
  function predict(items, now, opts = {}) {
    const look = opts.look || 8;
    const ms = majors(items);
    const next = nextMajor(ms, now, look);
    if (!next) return { next: null, events: [], all: [], majors: ms.length };
    const recent = ms.slice(-look);
    const groups = eventDays(items);
    const per = patchEvents(groups, recent, opts.before, opts.after);
    const total = recent.length;
    const hits = new Map();
    for (const m of recent) for (const k of per.get(m.ver) || []) hits.set(k, (hits.get(k) || 0) + 1);

    const year = new Date(next.day).getUTCFullYear();
    const all = [];
    for (const g of groups.values()) {
      const n = hits.get(g.key) || 0;
      const rate = total ? n / total : 0;
      // 每年這個時候：以前（不同年）在預計改版日前後三週內出現過
      const season = g.days.some((d) => new Date(d).getUTCFullYear() < year && yearGap(d, next.day) <= 21);
      if (rate < 0.3 && !season) continue;
      let conf = "low";
      if (rate >= 0.7) conf = "high";
      else if (rate >= 0.45 || (season && (rate >= 0.25 || g.cat === "season"))) conf = "mid";
      let reason;
      if (rate >= 0.9) reason = "幾乎每次改版都有";
      else if (rate >= 0.7) reason = "大部分改版都有";
      else if (season && rate < 0.45) reason = "每年這個時候通常會有";
      else if (rate >= 0.45) reason = "改版時常常有";
      else reason = "偶爾會有";
      all.push({ key: g.key, name: g.name, cat: g.cat, rate, conf, reason, season });
    }
    const order = { high: 0, mid: 1, low: 2 };
    all.sort((a, b) => order[a.conf] - order[b.conf] || b.rate - a.rate || a.name.localeCompare(b.name, "zh-TW"));
    const events = opts.cats && opts.cats.length ? all.filter((p) => opts.cats.includes(p.cat)) : all;
    return { next, events, all, majors: ms.length };
  }

  return { DAY, FAMILIES, CATS, parseDay, dayStr, weekday, snapWed, yearGap, cleanTitle, familyOf, versionOf, majors, gapStats, nextMajor, eventDays, patchEvents, predict };
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
    const y = d.getUTCFullYear() !== new Date().getFullYear() ? `${d.getUTCFullYear()}年` : "";
    return `${y}${d.getUTCMonth() + 1}月${d.getUTCDate()}日（週${WD[d.getUTCDay()]}）`;
  }
  function inDays(ms, now) {
    const n = Math.round((ms - now) / C.DAY);
    if (n <= 0) return "就是這幾天";
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

  /** 上面那張大卡片：下一次大改版 */
  function headCard(next, now) {
    const el = document.createElement("div");
    el.className = "card pd-next";
    const ver = document.createElement("div");
    ver.className = "pd-ver";
    ver.innerHTML = `<small class="muted">下一次大改版</small><b></b>`;
    ver.children[1].textContent = `V${next.ver}`;
    const when = document.createElement("div");
    when.className = "pd-when";
    when.innerHTML = `<b></b><small class="muted"></small>`;
    when.children[0].textContent = dateText(next.day);
    when.children[1].textContent = `${inDays(next.day, now)}・誤差約 ${next.range} 天・大約每 ${next.median} 天改版一次`;
    el.append(ver, when);
    return el;
  }

  function row(p) {
    const el = document.createElement("div");
    el.className = "card pd-card";
    const [confText, confCls] = CONF[p.conf] || CONF.low;
    const cat = C.CATS.find((c) => c.id === p.cat) || C.CATS[C.CATS.length - 1];
    const name = document.createElement("div");
    name.className = "pd-name";
    name.innerHTML = `<b></b><small class="muted"></small>`;
    name.children[0].textContent = p.name;
    name.children[1].textContent = p.reason;
    const tags = document.createElement("div");
    tags.className = "pd-tags";
    const ct = document.createElement("span");
    ct.className = `cbtn small ${cat.color} on pd-tag`;
    ct.textContent = cat.name;
    const cf = document.createElement("span");
    cf.className = `pd-conf ${confCls}`;
    cf.textContent = confText;
    tags.append(ct, cf);
    el.append(name, tags);
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
    const n = new Date();
    const now = Date.UTC(n.getFullYear(), n.getMonth(), n.getDate());
    const out = C.predict(items, now, { cats: st.cats });
    status.textContent = `依官網 ${nf.format(out.majors)} 次改版的紀錄推測`
      + (fetchedAt ? `・${new Date(fetchedAt * 1000).toLocaleDateString("zh-TW")} 更新` : "")
      + (stale ? "（這次沒連上官網，用之前存的）" : "");
    if (!out.next) {
      $("#pdCats").innerHTML = "";
      list.innerHTML = '<div class="card g-empty">官網公告的紀錄還不夠，至少要有三次大改版才推測得出來</div>';
      return;
    }
    list.append(headCard(out.next, now));
    renderCats(out.all);
    const head = document.createElement("div");
    head.className = "pd-sec";
    head.textContent = "這次改版預計會有的活動";
    list.append(head);
    if (!out.events.length) {
      const empty = document.createElement("div");
      empty.className = "card g-empty";
      empty.textContent = "沒有推測得出來的活動（官網公告的紀錄還不夠）";
      list.append(empty);
    } else {
      const frag = document.createDocumentFragment();
      for (const p of out.events) frag.append(row(p));
      list.append(frag);
    }
    const note = document.createElement("p");
    note.className = "hint pd-note";
    note.textContent = "只是照過去的規律推測，官方隨時可能改，日期與內容僅供參考。商城輪替（黃金蘋果、閃亮彗星等）不算改版活動，不列在這裡。";
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
