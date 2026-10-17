"use strict";

/* ==========================================================
 * BOSS 結晶收入計算
 * - 每週王：每隻角色每週最多賣 12 顆（價值高的優先）
 * - 每日王、每月王：不算在角色的 12 顆裡
 * - 整個帳號每週最多賣 90 顆（每日王、每週王、每月王都算）
 * - 結晶價值依組隊人數平分
 * - BossCore：價格表與計算（可獨立測試）
 * ========================================================== */

const BossCore = (() => {
  const CHAR_CAP = 12;
  const WORLD_CAP = 90;
  /**
   * [類型, BOSS, [[難度, 價格], …]]；價格是一顆強烈的力量結晶賣給收藏家的楓幣（一個人打）。
   * 分類與順序和遊戲裡的 BOSS 清單相同
   */
  const LIST = [
    ["day", "炎魔", [["簡易", 115800], ["普通", 354800]]],
    ["day", "梅格耐斯", [["簡單", 418300], ["普通", 1501700]]],
    ["day", "希拉", [["普通", 463500]]],
    ["day", "卡翁", [["普通", 724200]]],
    ["day", "拉圖斯", [["簡單", 396500], ["普通", 1543700]]],
    ["day", "比艾樂", [["普通", 560800]]],
    ["day", "斑斑", [["普通", 560800]]],
    ["day", "血腥皇后", [["普通", 560800]]],
    ["day", "貝倫", [["普通", 560800]]],
    ["day", "凡雷恩", [["簡單", 612900], ["普通", 844700], ["困難", 1419500]]],
    ["day", "闇黑龍王", [["簡單", 511000], ["普通", 586600], ["混沌", 783300]]],
    ["day", "阿卡伊農", [["簡單", 667400], ["普通", 1460300]]],
    ["day", "粉豆", [["普通", 813700]]],
    ["day", "森蘭丸", [["一般", 375500], ["困難", 1543700]]],

    ["week", "炎魔", [["混沌", 7059750]]],
    ["week", "梅格耐斯", [["困難", 8819007]]],
    ["week", "希拉", [["困難", 6677700]]],
    ["week", "拉圖斯", [["混沌", 20088150]]],
    ["week", "比艾樂", [["混沌", 7313306]]],
    ["week", "斑斑", [["混沌", 7693781]]],
    ["week", "血腥皇后", [["混沌", 7682035]]],
    ["week", "貝倫", [["混沌", 9070003]]],
    ["week", "粉豆", [["混沌", 7630700]]],
    ["week", "西格諾斯", [["簡單", 5307400], ["普通", 8709400]]],
    ["week", "史烏", [["普通", 27207040], ["困難", 91900000], ["終極", 323500000]]],
    ["week", "戴米安", [["普通", 28843452], ["困難", 85700000]]],
    ["week", "守護天使綠水靈", [["普通", 43000000], ["混沌", 126500000]]],
    ["week", "露希妲", [["簡單", 53800000], ["普通", 64300000], ["困難", 102400000]]],
    ["week", "威爾", [["簡單", 57400000], ["一般", 74200000], ["困難", 127400000]]],
    ["week", "戴斯克", [["一般", 79500000], ["混沌", 111200000]]],
    ["week", "真·希拉", [["普通", 124400000], ["困難", 145200000]]],
    ["week", "頓凱爾", [["一般", 84700000], ["困難", 126200000]]],
    ["week", "受選的賽蓮", [["普通", 149800000], ["困難", 271600000], ["終極", 724200000]]],
    ["week", "監視者卡洛斯", [["簡單", 236900000], ["普通", 309000000], ["混沌", 618800000], ["終極", 1237100000]]],
    ["week", "最初的敵對者", [["簡單", 252700000], ["普通", 371000000], ["困難", 682000000], ["終極", 1344000000]]],
    ["week", "咖凌", [["簡單", 258300000], ["普通", 361700000], ["困難", 721100000], ["終極", 1443300000]]],
    ["week", "燦爛的凶星", [["普通", 355320000], ["困難", 817510000]]],
    ["week", "林波", [["普通", 420000000], ["困難", 749000000]]],
    ["week", "巴德利斯", [["普通", 560000000], ["困難", 840000000]]],
    ["week", "尤比太", [["普通", 705000000], ["困難", 1368000000]]],
    ["week", "濃姬", [["普通", 18500000]]],
    ["week", "瑪麗西亞", [["普通", 150000000], ["終極", 1500000000]]],

    ["month", "黑魔法師", [["困難", 990841916], ["終極", 3000000000]]],
  ].map(([type, name, diffs]) => ({ type, name, key: `${type}:${name}`, diffs }));
  // 結晶價格高的排在前面（難的在上面）
  const top = (b) => Math.max(...b.diffs.map((d) => d[1]));
  LIST.sort((a, b) => top(b) - top(a));
  const BY_KEY = Object.fromEntries(LIST.map((b) => [b.key, b]));

  /**
   * 組隊人數上限：大部分 6 人；最新的幾隻只能 3 人，終極史烏只能 2 人
   * （最初的敵對者、燦爛的凶星、林波、巴德利斯、尤比太、瑪麗西亞：所有難度 3 人）
   */
  const PARTY3 = new Set(["最初的敵對者", "燦爛的凶星", "林波", "巴德利斯", "尤比太", "瑪麗西亞"]);
  function maxParty(name, diff) {
    if (PARTY3.has(name)) return 3;
    if (name === "史烏" && diff === "終極") return 2;
    return 6;
  }

  /** 一顆結晶分給每個人的價值（組隊平分，無條件捨去） */
  const share = (price, party, max = 6) => Math.floor(price / Math.max(1, Math.min(max, party || 1)));

  /**
   * 一隻角色這週的結晶：
   * - ch.sel = { key: { d: 難度索引, p: 組隊人數, n: 每週天數（每日王） } }
   * - ch.rs = { key: { d, p } }：用 BOSS重製卷多打一次的每週王（難度、人數可以和第一次不同）
   * 回傳 { week: 每週王（依價值排序，只有前 12 顆會賣），weekOver: 超過 12 顆的，day: 每日王（每天一顆），month: 每月王 }
   * 重製卷多打的一樣是每週王結晶，算在角色 12 顆、帳號 90 顆裡
   */
  function crystals(ch, owner) {
    const week = [], day = [], month = [];
    for (const [key, s] of Object.entries(ch?.sel || {})) {
      const b = BY_KEY[key];
      if (!b || !b.diffs[s.d]) continue;
      const [diff, price] = b.diffs[s.d];
      const item = { key, name: b.name, diff, value: share(price, s.p, maxParty(b.name, diff)), owner };
      if (b.type === "week") week.push(item);
      else if (b.type === "month") month.push(item);
      else for (let i = 0; i < Math.max(0, Math.min(7, s.n ?? 7)); i++) day.push(item);
    }
    for (const [key, r] of Object.entries(ch?.rs || {})) {
      const b = BY_KEY[key];
      if (!b || b.type !== "week" || r.d == null || !b.diffs[r.d]) continue;
      const [diff, price] = b.diffs[r.d];
      week.push({ key, name: b.name, diff, value: share(price, r.p, maxParty(b.name, diff)), owner, reset: true });
    }
    week.sort((a, b) => b.value - a.value || (a.reset ? 1 : 0) - (b.reset ? 1 : 0));
    return { week: week.slice(0, CHAR_CAP), weekOver: week.slice(CHAR_CAP), day, month };
  }

  /** 從所有結晶裡挑價值最高的 90 顆 */
  function sell(list) {
    const sorted = [...list].sort((a, b) => b.value - a.value);
    const sold = sorted.slice(0, WORLD_CAP), left = sorted.slice(WORLD_CAP);
    return { sold, left, value: sold.reduce((a, x) => a + x.value, 0), count: sold.length };
  }

  /**
   * 整個帳號：每週收入（一般週）、打黑魔法師那週的收入、每月收入（4 週，其中一週含月王）
   */
  function summary(chars) {
    const per = chars.map((ch, i) => ({ ch, ...crystals(ch, i) }));
    const weekPool = per.flatMap((c) => [...c.week, ...c.day]);
    const monthPool = per.flatMap((c) => c.month);
    const normal = sell(weekPool);
    const withMonth = sell([...weekPool, ...monthPool]);
    const monthly = monthPool.length ? normal.value * 3 + withMonth.value : normal.value * 4;
    const part = (list) => list.reduce((a, x) => a + x.value, 0);
    return {
      per: per.map((c) => ({
        week: part(c.week), day: part(c.day), month: part(c.month),
        weekCount: c.week.length + c.weekOver.length, over: c.weekOver.length, crystals: c.week.length + c.day.length,
        resets: [...c.week, ...c.weekOver].filter((x) => x.reset).length,
      })),
      normal, withMonth, monthly,
      byType: {
        week: part(per.flatMap((c) => c.week)), day: part(per.flatMap((c) => c.day)), month: part(monthPool),
        reset: part(per.flatMap((c) => c.week.filter((x) => x.reset))),
      },
    };
  }

  /** BOSS 圖示在 assets/boss.webp 裡的位置 */
  const ICON = {
    黑魔法師: 0, 瑪麗西亞: 1, 咖凌: 2, 尤比太: 3, 最初的敵對者: 4, 監視者卡洛斯: 5, 巴德利斯: 6, 燦爛的凶星: 7, 林波: 8, 受選的賽蓮: 9,
    "真·希拉": 10, 威爾: 11, 守護天使綠水靈: 12, 頓凱爾: 13, 戴斯克: 14, 露希妲: 15, 史烏: 16, 戴米安: 17, 拉圖斯: 18, 濃姬: 19,
    貝倫: 20, 梅格耐斯: 21, 西格諾斯: 22, 斑斑: 23, 血腥皇后: 24, 粉豆: 25, 比艾樂: 26, 炎魔: 27, 希拉: 28, 森蘭丸: 29,
    阿卡伊農: 30, 凡雷恩: 31, 闇黑龍王: 32, 卡翁: 33,
  };

  return { CHAR_CAP, WORLD_CAP, ICON, LIST, BY_KEY, maxParty, share, crystals, sell, summary };
})();

if (typeof module !== "undefined") module.exports = BossCore;

/* ==========================================================
 * 介面
 * ========================================================== */
if (typeof document !== "undefined") (() => {
  const C = BossCore;
  const KEY = "boss-v1";
  const newChar = (n) => ({ id: Date.now().toString(36) + Math.random().toString(36).slice(2, 6), name: n ? `角色 ${n}` : "角色 1", sel: {}, rs: {} });
  const defaults = () => { const c = newChar(0); return { page: "week", chars: [c], cur: c.id }; };
  let st = (() => {
    try {
      const s = JSON.parse(localStorage.getItem(KEY));
      if (s && Array.isArray(s.chars) && s.chars.length) return { ...defaults(), ...s };
    } catch (_) {}
    return defaults();
  })();
  const save = () => { try { localStorage.setItem(KEY, JSON.stringify(st)); } catch (_) {} };
  const cur = () => st.chars.find((c) => c.id === st.cur) || st.chars[0];
  // 舊資料：組隊人數超過上限的拉回來；舊版重製卷（每隻王幾張）改成整個帳號每隻王一次
  {
    const used = new Set();
    for (const ch of st.chars) {
      ch.rs = ch.rs || {};
      for (const [k, s] of Object.entries(ch.sel || {})) {
        const b = C.BY_KEY[k];
        if (b && b.diffs[s.d]) s.p = Math.min(s.p || 1, C.maxParty(b.name, b.diffs[s.d][0]));
        if (s.r) { if (!used.has(k) && !ch.rs[k]) ch.rs[k] = { d: s.d, p: s.p }; delete s.r; }
      }
      for (const k of Object.keys(ch.rs)) { if (used.has(k) || C.BY_KEY[k]?.type !== "week") delete ch.rs[k]; else used.add(k); }
    }
  }
  /** 這隻王本週是哪隻角色用重製卷（整個帳號每隻王每週只能重製一次） */
  const resetOwner = (key) => st.chars.find((c) => c.rs?.[key]);

  const intFmt = new Intl.NumberFormat("zh-TW", { maximumFractionDigits: 0 });
  /** 楓幣讀法：990841916 → 9億9084萬1916 */
  function zh(n) {
    let x = Math.round(Math.abs(n));
    const units = [[1e12, "兆"], [1e8, "億"], [1e4, "萬"]];
    let out = "";
    for (const [u, name] of units) { const q = Math.floor(x / u); if (q > 0) { out += `${q}${name}`; x -= q * u; } }
    if (x > 0 || !out) out += x;
    return (n < 0 ? "-" : "") + out;
  }
  /** 大數字：123.45e（億） */
  const eText = (n) => (n >= 1e8 ? `${+(n / 1e8).toFixed(n >= 1e11 ? 0 : 2)}e` : `${intFmt.format(Math.round(n / 1e4))}w`);
  /** 送禮買賣的楓幣比率（幾萬楓幣 = 1 台幣） */
  const ratio = () => { try { return Number(JSON.parse(localStorage.getItem("gift-v1"))?.ratio) || 2800; } catch (_) { return 2800; } };

  // 難度顏色和遊戲裡一樣：EASY 灰、NORMAL 藍、HARD 粉紅、CHAOS 金、EXTREME 紅
  const DIFF_COLOR = { 簡易: "c-silver", 簡單: "c-silver", 一般: "c-blue", 普通: "c-blue", 困難: "c-pink", 混沌: "c-amber", 終極: "c-red" };
  const PAGE_NAME = { week: "每週王", day: "每日王", month: "每月王", reset: "BOSS重製卷" };

  /* ---------- 分頁 ---------- */
  function renderPage() {
    $$("#bsPages [data-page]").forEach((b) => b.classList.toggle("on", b.dataset.page === st.page));
  }

  /* ---------- 角色 ---------- */
  function renderChars(sum) {
    const box = $("#bsChars");
    box.innerHTML = "";
    st.chars.forEach((ch, i) => {
      const p = sum.per[i];
      const row = document.createElement("div");
      row.className = "bs-char" + (ch.id === st.cur ? " on" : "");
      const name = document.createElement("input");
      name.type = "text";
      name.value = ch.name;
      name.title = "點一下選擇，可以直接改名";
      name.spellcheck = false;
      name.addEventListener("focus", () => { if (st.cur !== ch.id) { st.cur = ch.id; save(); render(); setTimeout(() => $$("#bsChars input")[i]?.focus(), 0); } });
      name.addEventListener("input", (e) => { ch.name = e.target.value; save(); });
      name.addEventListener("keydown", (e) => { if (e.key === "Enter") e.target.blur(); });
      const info = document.createElement("small");
      info.className = "muted";
      info.textContent = `每週王 ${Math.min(p.weekCount, C.CHAR_CAP)}/${C.CHAR_CAP}・${eText(p.week + p.day)}`;
      const del = document.createElement("button");
      del.className = "b-x";
      del.title = "刪除角色";
      del.textContent = "×";
      del.hidden = st.chars.length < 2;
      del.addEventListener("click", async (e) => {
        e.stopPropagation();
        if (!(await confirmBox(`確定要刪除「${ch.name || "角色"}」嗎？`))) return;
        st.chars = st.chars.filter((c) => c.id !== ch.id);
        if (st.cur === ch.id) st.cur = st.chars[0].id;
        save(); render();
      });
      row.addEventListener("click", (e) => { if (e.target.tagName !== "INPUT" && !e.target.closest(".b-x")) { st.cur = ch.id; save(); render(); } });
      const text = document.createElement("div");
      text.className = "bs-char-text";
      text.append(name, info);
      row.append(text, del);
      box.appendChild(row);
    });
    // 角色清單填滿左邊（總計貼齊底部），放不下時在清單裡捲動
    box.querySelector(".on")?.scrollIntoView({ block: "nearest" });
  }

  /* ---------- BOSS 列表 ---------- */
  function stepper(value, min, max, unit, onChange, title) {
    const wrap = document.createElement("label");
    wrap.className = "bs-step";
    wrap.title = title;
    const inp = document.createElement("input");
    inp.type = "text";
    inp.inputMode = "numeric";
    inp.value = value;
    const set = (v) => { const n = Math.max(min, Math.min(max, Math.floor(Number(v)) || min)); inp.value = n; onChange(n); };
    inp.addEventListener("change", () => set(inp.value));
    inp.addEventListener("wheel", (e) => { if (!e.deltaY) return; e.preventDefault(); set(Number(inp.value) + (e.deltaY < 0 ? 1 : -1)); }, { passive: false });
    const u = document.createElement("span");
    u.textContent = unit;
    wrap.append(inp, u);
    return wrap;
  }
  const cell = (cls, text = "") => { const el = document.createElement("div"); el.className = cls; el.textContent = text; return el; };
  function icon(b) {
    const ic = document.createElement("span");
    ic.className = "bs-ic";
    if (C.ICON[b.name] != null) ic.style.setProperty("--i", C.ICON[b.name]);
    return ic;
  }
  function nameCell(b, sub) {
    const name = document.createElement("div");
    name.className = "bs-name";
    name.innerHTML = `<span></span><small class="muted"></small>`;
    name.children[0].textContent = b.name;
    name.children[1].textContent = sub || "";
    return name;
  }

  /** 最上面一列：說明每一欄是什麼 */
  function headRow(cols) {
    const row = document.createElement("div");
    row.className = "bs-row bs-thead";
    cols.forEach((t, i) => {
      const c = cell(i === cols.length - 1 ? "r" : "", "");
      if (i === cols.length - 1) {
        c.append(ItemIcons.el("強烈的力量結晶"), document.createTextNode(t));
        c.title = "一顆強烈的力量結晶賣給收藏家的楓幣（組隊時是平分後的價值）";
      } else c.textContent = t;
      row.appendChild(c);
    });
    return row;
  }

  function renderList(sum) {
    const ch = cur();
    const box = $("#bsList");
    box.innerHTML = "";
    const idx = st.chars.indexOf(ch);
    const mine = C.crystals(ch, idx);
    const over = new Set(mine.weekOver.filter((x) => !x.reset).map((x) => x.key));
    const p = sum.per[idx];
    // 標題列：這隻角色的每週王數量
    const head = $("#bsListHead");
    const who = `<b>${(ch.name || "角色").replace(/[<>&]/g, "")}</b>`;
    const cnt = `每週王 <b class="${p.weekCount > C.CHAR_CAP ? "bad" : ""}">${p.weekCount}</b> / ${C.CHAR_CAP}`;
    const resets = st.chars.reduce((a, c) => a + Object.keys(c.rs || {}).length, 0);
    if (st.page === "reset") head.innerHTML = `${who}<span>這隻重製 <b>${Object.keys(ch.rs || {}).length}</b> 隻・整個帳號 <b>${resets}</b> 隻</span>`;
    else if (st.page === "week") head.innerHTML = `${who}<span>${cnt}${p.over ? `<small>超過的 ${p.over} 顆價值最低，不會賣</small>` : ""}</span>`;
    else if (st.page === "day") head.innerHTML = `${who}<span>每天一顆，不算在每週 ${C.CHAR_CAP} 顆裡</span>`;
    else head.innerHTML = `${who}<span>不算在每週 ${C.CHAR_CAP} 顆裡，算在帳號 ${C.WORLD_CAP} 顆裡</span>`;

    if (st.page === "reset") return renderResets(ch);

    const isDay = st.page === "day";
    box.className = `bs-list ${isDay ? "cols-6" : "cols-5"}`;
    box.appendChild(headRow(isDay ? ["", "名稱", "難度", "人數", "天數", "結晶錢"] : ["", "名稱", "難度", "人數", "結晶錢"]));
    for (const b of C.LIST.filter((x) => x.type === st.page)) {
      const s = ch.sel[b.key];
      const row = document.createElement("div");
      row.className = "bs-row" + (s ? " on" : "") + (over.has(b.key) ? " over" : "");
      const diffs = document.createElement("div");
      diffs.className = "bs-diffs";
      // 難度由簡單到難
      b.diffs.forEach(([d, price], i) => {
        const btn = document.createElement("button");
        btn.className = `cbtn small ${DIFF_COLOR[d] || "c-silver"}` + (s && s.d === i ? " on" : "");
        btn.textContent = d;
        const mp = C.maxParty(b.name, d);
        btn.title = `${b.name}（${d}）${zh(price)} 楓幣・最多 ${mp} 人`;
        btn.addEventListener("click", () => {
          // 點已選的難度 = 取消
          if (s && s.d === i) delete ch.sel[b.key];
          else ch.sel[b.key] = { d: i, p: Math.min(s?.p || 1, mp), ...(isDay ? { n: s?.n ?? 7 } : {}) };
          save(); render();
        });
        diffs.appendChild(btn);
      });
      const party = cell("bs-c");
      const days = cell("bs-c");
      const val = cell("bs-val");
      let sub = "";
      if (s) {
        const [d, price] = b.diffs[s.d];
        const mp = C.maxParty(b.name, d);
        party.appendChild(stepper(Math.min(s.p || 1, mp), 1, mp, "人", (v) => { s.p = v; save(); render(); }, `組隊人數（結晶價值平分），最多 ${mp} 人，可以用滾輪調整`));
        if (isDay) days.appendChild(stepper(s.n ?? 7, 0, 7, "天", (v) => { s.n = v; save(); render(); }, "每週打幾天，可以用滾輪調整"));
        const one = C.share(price, s.p, mp);
        val.textContent = zh(one * (isDay ? s.n ?? 7 : 1));
        sub = over.has(b.key) ? `超過 ${C.CHAR_CAP} 顆，不會賣` : isDay ? `每顆 ${zh(one)}` : (s.p || 1) > 1 ? `${s.p} 人平分` : "";
      } else {
        val.textContent = zh(Math.max(...b.diffs.map((d) => d[1])));
        val.classList.add("muted");
        const mps = [...new Set(b.diffs.map(([d]) => C.maxParty(b.name, d)))];
        if (mps.some((m) => m < 6)) sub = b.diffs.map(([d]) => C.maxParty(b.name, d)).every((m) => m === mps[0]) ? `最多 ${mps[0]} 人` : b.diffs.filter(([d]) => C.maxParty(b.name, d) < 6).map(([d]) => `${d}最多 ${C.maxParty(b.name, d)} 人`).join("・");
      }
      row.append(icon(b), nameCell(b, sub), diffs, party, ...(isDay ? [days] : []), val);
      box.appendChild(row);
      // 用 BOSS重製卷再打一次：下面多一列，難度、人數另外選
      if (!isDay && ch.rs?.[b.key]) box.appendChild(resetRow(ch, b, mine));
    }
  }

  /** 每週王底下的「重製」列：選這次重製要打的難度與人數 */
  function resetRow(ch, b, mine) {
    const r = ch.rs[b.key];
    const row = document.createElement("div");
    row.className = "bs-row bs-reset" + (r.d != null ? " on" : "");
    const ic = icon(b);
    ic.classList.add("bs-ic-reset");
    ic.title = "BOSS重製卷";
    const diffs = document.createElement("div");
    diffs.className = "bs-diffs";
    b.diffs.forEach(([d, price], i) => {
      const btn = document.createElement("button");
      btn.className = `cbtn small ${DIFF_COLOR[d] || "c-silver"}` + (r.d === i ? " on" : "");
      btn.textContent = d;
      const mp = C.maxParty(b.name, d);
      btn.title = `重製後打${d}：${zh(price)} 楓幣・最多 ${mp} 人`;
      btn.addEventListener("click", () => {
        if (r.d === i) { delete r.d; delete r.p; }
        else { r.d = i; r.p = Math.min(r.p || 1, mp); }
        save(); render();
      });
      diffs.appendChild(btn);
    });
    const party = cell("bs-c");
    const val = cell("bs-val");
    let sub = "BOSS重製卷・選這次要打的難度";
    if (r.d != null) {
      const [d, price] = b.diffs[r.d];
      const mp = C.maxParty(b.name, d);
      party.appendChild(stepper(Math.min(r.p || 1, mp), 1, mp, "人", (v) => { r.p = v; save(); render(); }, `重製這次的組隊人數，最多 ${mp} 人，可以用滾輪調整`));
      val.textContent = zh(C.share(price, r.p, mp));
      const sold = mine.week.some((x) => x.key === b.key && x.reset);
      sub = sold ? `BOSS重製卷${(r.p || 1) > 1 ? `・${r.p} 人平分` : ""}` : `BOSS重製卷・超過 ${C.CHAR_CAP} 顆，不會賣`;
    } else {
      val.textContent = "—";
      val.classList.add("muted");
    }
    const name = nameCell({ name: `${b.name}（重製）` }, sub);
    row.append(ic, name, diffs, party, val);
    return row;
  }

  /**
   * BOSS重製卷：整個帳號每隻每週王每週只能重製一次（一隻角色重製了，其他角色就不能再重製同一隻）。
   * 這裡選要重製哪幾隻王，難度、人數回到「每週王」那邊選
   */
  function renderResets(ch) {
    const box = $("#bsList");
    box.className = "bs-list cols-5";
    const note = document.createElement("p");
    note.className = "bs-reset-note muted";
    note.textContent = `每隻每週王每週只能重製一次，整個帳號共用（一隻角色重製了，其他角色就不能重製同一隻）。選好後到「每週王」選這次重製要打的難度與人數；多打的結晶一樣算在角色 ${C.CHAR_CAP} 顆、帳號 ${C.WORLD_CAP} 顆裡。`;
    box.appendChild(note);
    box.appendChild(headRow(["", "名稱", "重製難度", "重製", "結晶錢"]));
    for (const b of C.LIST.filter((x) => x.type === "week")) {
      const owner = resetOwner(b.key);
      const mineR = owner === ch ? ch.rs[b.key] : null;
      const row = document.createElement("div");
      row.className = "bs-row" + (mineR ? " on" : "") + (owner && owner !== ch ? " taken" : "");
      const diffs = document.createElement("div");
      diffs.className = "bs-diffs";
      if (mineR && mineR.d != null) {
        const chip = document.createElement("span");
        const d = b.diffs[mineR.d][0];
        chip.className = `cbtn small on ${DIFF_COLOR[d] || "c-silver"} bs-chip`;
        chip.textContent = d;
        diffs.appendChild(chip);
      } else if (mineR) diffs.appendChild(cell("bs-hint muted", "到每週王選"));
      const act = cell("bs-c");
      const btn = document.createElement("button");
      if (owner && owner !== ch) {
        btn.className = "cbtn small c-silver";
        btn.disabled = true;
        btn.textContent = "已被重製";
        btn.title = `「${owner.name || "角色"}」這週已經用重製卷重製這隻王`;
      } else {
        btn.className = "cbtn small c-teal" + (mineR ? " on" : "");
        btn.textContent = mineR ? "重製中" : "重製";
        btn.title = mineR ? "再點一次取消" : "這隻角色用重製卷再打一次這隻王";
        btn.addEventListener("click", () => {
          if (mineR) delete ch.rs[b.key];
          else {
            const s = ch.sel[b.key];
            ch.rs[b.key] = s ? { d: s.d, p: s.p } : {};
          }
          save(); render();
        });
      }
      act.appendChild(btn);
      const val = cell("bs-val");
      let sub = "";
      if (owner && owner !== ch) sub = `「${owner.name || "角色"}」已重製`;
      if (mineR && mineR.d != null) {
        const [d, price] = b.diffs[mineR.d];
        const mp = C.maxParty(b.name, d);
        val.textContent = zh(C.share(price, mineR.p, mp));
        if ((mineR.p || 1) > 1) sub = `${mineR.p} 人平分`;
      } else {
        val.textContent = zh(Math.max(...b.diffs.map((d) => d[1])));
        val.classList.add("muted");
      }
      row.append(icon(b), nameCell(b, sub), diffs, act, val);
      box.appendChild(row);
    }
  }

  /* ---------- 總計 ---------- */
  function renderSum(sum) {
    const r = ratio();
    const nt = (v) => `NT$ ${intFmt.format(Math.round(v / (r * 1e4)))}`;
    const kv = (k, v, s = "") => `<div class="bs-kv"><span>${k}</span><b>${v}</b>${s ? `<small>${s}</small>` : ""}</div>`;
    const n = sum.normal, m = sum.withMonth;
    const hasMonth = sum.byType.month > 0;
    $("#bsSum").innerHTML =
      `<div class="bs-sum-top"><div><div class="f-big-label">每週收入</div><div class="bs-big">${eText(n.value)}</div></div>` +
      `<div class="bs-sum-nt"><b>約 ${nt(n.value)}</b><small>${zh(n.value)}</small></div></div>` +
      `<div class="bs-cap"><div class="bs-bar"><i style="width:${Math.min(100, (n.count / C.WORLD_CAP) * 100)}%"></i></div><span>帳號每週結晶 <b>${n.count + n.left.length}</b> / ${C.WORLD_CAP}</span></div>` +
      (n.left.length ? `<p class="bs-warn">超過 ${C.WORLD_CAP} 顆：價值最低的 ${n.left.length} 顆賣不掉（少 ${zh(n.left.reduce((a, x) => a + x.value, 0))}）</p>` : "") +
      '<div class="bs-kvs">' +
      kv("每週王", eText(sum.byType.week), sum.byType.reset ? `含重製卷 ${eText(sum.byType.reset)}` : "") +
      kv("每日王", eText(sum.byType.day)) +
      (hasMonth ? kv("黑魔法師那週", eText(m.value), m.left.length ? `${m.left.length} 顆賣不掉` : "") : "") +
      "</div>" +
      `<div class="f-line total pos bs-total" title="台幣依「送禮買賣」的楓幣比率（${intFmt.format(r)} 萬楓幣 = 1 台幣）"><span>每月收入<small>4 週${hasMonth ? "，其中一週打黑魔法師" : ""}・約 ${nt(sum.monthly)}</small></span><b>${eText(sum.monthly)}</b></div>`;
  }

  function render() {
    renderPage();
    const sum = C.summary(st.chars);
    renderChars(sum);
    renderList(sum);
    renderSum(sum);
  }

  /* ---------- 事件 ---------- */
  $$("#bsPages [data-page]").forEach((b) => b.addEventListener("click", () => { st.page = b.dataset.page; save(); render(); $("#bsList").scrollTop = 0; }));
  $("#bsAdd").addEventListener("click", () => {
    const c = newChar(st.chars.length + 1);
    st.chars.push(c);
    st.cur = c.id;
    save(); render();
    setTimeout(() => { const last = $$("#bsChars input").pop(); last?.focus(); last?.select(); }, 0);
  });
  $("#bsClear").addEventListener("click", async () => {
    const ch = cur();
    if (st.page === "reset") {
      if (!Object.keys(ch.rs || {}).length) return;
      if (!(await confirmBox(`確定要清除「${ch.name || "角色"}」用的重製卷嗎？`))) return;
      ch.rs = {};
      save(); render();
      return;
    }
    const keys = Object.keys(ch.sel).filter((k) => C.BY_KEY[k]?.type === st.page);
    if (!keys.length) return;
    if (!(await confirmBox(`確定要清除「${ch.name || "角色"}」的${PAGE_NAME[st.page]}嗎？`))) return;
    keys.forEach((k) => delete ch.sel[k]);
    save(); render();
  });

  render();
  window.Boss = { onShow: render };
})();
