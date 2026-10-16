"use strict";

/* ==========================================================
 * 黃金蘋果轉蛋模擬器
 * - 核心邏輯（解析、期望值、自動定價、評語）放在 GachaCore，可獨立測試
 * - 介面與狀態保存（localStorage）在下方 IIFE
 * ========================================================== */

const GachaCore = (() => {
  const POOLS = ["apple", "silver", "gold"];
  const POOL_NAME = { apple: "黃金蘋果", silver: "幸運的銀色箱子", gold: "幸運的金色箱子" };
  const BOX_COST = { silver: 30, gold: 100 }; // 開箱所需碎片
  const FILLER = "請檢查道具機率";

  /** 解析貼上的文字。依「幸運的銀色箱子」「幸運的金色箱子」標題分割三個獎池。 */
  function parse(text) {
    const out = { apple: [], silver: [], gold: [] };
    let cur = "apple";
    for (const raw of String(text).split(/\r?\n/)) {
      const line = raw.trim();
      if (!line) continue;
      const hasPct = /\d\s*%/.test(line);
      // 區塊標題（不含機率的行）
      if (!hasPct && line.startsWith("幸運的銀色箱子")) { cur = "silver"; continue; }
      if (!hasPct && line.startsWith("幸運的金色箱子")) { cur = "gold"; continue; }
      // 名稱 + 空白/Tab + 最後一個百分比（名稱本身可以含 %，例如「回真卷軸 100%」）
      const m = /^(.*\S)\s+(\d+(?:\.\d+)?)\s*%$/.exec(line);
      if (!m) continue;
      const name = m[1].trim();
      const prob = parseFloat(m[2]);
      if (!name || !(prob > 0)) continue;
      out[cur].push({ name, prob });
    }
    return out;
  }

  const sum = (items) => items.reduce((a, it) => a + it.prob, 0);

  /** 加上補足列（總和不足 100% 時） */
  function withFiller(items) {
    const total = sum(items);
    if (total < 100 - 1e-9) return [...items, { name: FILLER, prob: +(100 - total).toFixed(6), filler: true }];
    return items;
  }

  /** 抽樣器：依機率權重隨機回傳一個道具 */
  function sampler(items) {
    const list = withFiller(items);
    const total = sum(list);
    const cum = [];
    let acc = 0;
    for (const it of list) { acc += it.prob; cum.push(acc); }
    return () => {
      const r = Math.random() * total;
      let lo = 0, hi = cum.length - 1;
      while (lo < hi) { const mid = (lo + hi) >> 1; if (cum[mid] > r) hi = mid; else lo = mid + 1; }
      return list[lo];
    };
  }

  const valueOf = (values, name) => {
    const v = values[name];
    return typeof v === "number" && isFinite(v) ? v : 0;
  };

  /** 獎池期望值（每次抽／開箱） */
  function ev(items, values) {
    const list = withFiller(items);
    const total = Math.max(100, sum(list));
    return list.reduce((a, it) => a + (it.prob / total) * valueOf(values, it.name), 0);
  }

  /** 1 個碎片的期望價值 = 換成銀箱或金箱中較划算者 */
  function fragmentValue(pools, values) {
    const s = ev(pools.silver, values) / BOX_COST.silver;
    const g = ev(pools.gold, values) / BOX_COST.gold;
    return { value: Math.max(s, g, 0), best: g > s ? "gold" : s > 0 ? "silver" : null, silver: s, gold: g };
  }

  /** 稀有道具（蘋果 < 1%、箱子 ≤ 5%）佔期望值的比例：越高代表越「極端」 */
  const RARE_LIMIT = { apple: 1, silver: 5, gold: 5 };
  function rareShare(items, values, limit = 1) {
    const total = ev(items, values);
    if (total <= 0) return 0;
    const t = Math.max(100, sum(items));
    const rare = items.filter((it) => it.prob <= limit && (limit > 1 || it.prob < 1)).reduce((a, it) => a + (it.prob / t) * valueOf(values, it.name), 0);
    return rare / total;
  }

  function analyze(state, pool) {
    const { pools, values, cost } = state;
    const frag = fragmentValue(pools, values);
    const items = pools[pool];
    const hasValues = items.some((it) => valueOf(values, it.name) > 0);
    if (pool === "apple") {
      const itemEv = ev(items, values);
      const combined = itemEv + frag.value;
      const list = withFiller(items);
      const total = Math.max(100, sum(list));
      const win = cost > 0
        ? list.filter((it) => valueOf(values, it.name) + frag.value > cost).reduce((a, it) => a + it.prob / total, 0)
        : 0;
      return {
        pool, hasValues, cost, itemEv, frag, combined,
        cp: cost > 0 ? combined / cost : null,
        win,
        fragShare: combined > 0 ? frag.value / combined : 0,
        rare: rareShare(items, values, RARE_LIMIT.apple),
      };
    }
    const boxEv = ev(items, values);
    const list = withFiller(items);
    const total = Math.max(100, sum(list));
    const aboveAvg = list.filter((it) => valueOf(values, it.name) >= boxEv && boxEv > 0).reduce((a, it) => a + it.prob / total, 0);
    return {
      pool, hasValues, cost, boxEv,
      perFrag: boxEv / BOX_COST[pool],
      frag,
      aboveAvg,
      rare: rareShare(items, values, RARE_LIMIT[pool]),
    };
  }

  /** 每抽獲得該道具的機率（蘋果 + 碎片換箱子折算） */
  function effectiveProb(pools, name) {
    let p = 0;
    for (const it of pools.apple) if (it.name === name) p += it.prob / 100;
    for (const it of pools.silver) if (it.name === name) p += it.prob / 100 / BOX_COST.silver;
    for (const it of pools.gold) if (it.name === name) p += it.prob / 100 / BOX_COST.gold;
    return p;
  }

  const isJunk = (name, junk) => junk.some((k) => k && name.includes(k));

  /** 取兩位有效數字，讓數字好讀 */
  function nice(n) {
    if (!(n > 0)) return 0;
    const mag = Math.pow(10, Math.floor(Math.log10(n)) - 1);
    return Math.max(1, Math.round(n / mag) * mag);
  }

  /**
   * 自動帶入價格：
   * - 垃圾道具、任何獎池中機率 > 30% 的常見道具 → 留空
   * - 其餘：基準權重 = (1 / 每抽獲得機率)^0.8（非線性，越稀有越高）
   * - 整體縮放成「單抽綜合期望值 ≈ 每抽花費」，當作中性的基準起點
   */
  function autoPrice(pools, cost, junk) {
    const names = new Set();
    for (const p of POOLS) for (const it of pools[p]) names.add(it.name);
    const weight = {};
    for (const name of names) {
      const common = POOLS.some((p) => pools[p].some((it) => it.name === name && it.prob > 30));
      if (isJunk(name, junk) || common) { weight[name] = null; continue; }
      const pe = effectiveProb(pools, name);
      weight[name] = pe > 0 ? Math.pow(1 / pe, 0.8) : null;
    }
    const w = {};
    for (const k in weight) if (weight[k] !== null) w[k] = weight[k];
    const base = ev(pools.apple, w) + fragmentValue(pools, w).value;
    const k = base > 0 ? cost / base : 0;
    const values = {};
    for (const name of names) values[name] = weight[name] === null ? null : nice(weight[name] * k);
    return values;
  }

  /* ---------- AI 評語 ---------- */
  const pct = (x, d = 1) => `${(x * 100).toFixed(d)}%`;
  function pick(arr, seed) { return arr[Math.abs(seed) % arr.length]; }
  function seedOf(...nums) {
    let h = 7;
    for (const n of nums) h = (h * 31 + Math.round((n || 0) * 1000)) | 0;
    return h;
  }

  const LIB = {
    cp: [
      [1.3, [
        "數學上這是少見的正期望獎池，每投入 1 元平均可回收 {cp} 元，長期抽是有利的。",
        "期望回報 {cp} 倍，明顯高於成本；只要資金撐得住波動，長期下注是賺的。",
        "綜合期望值高出成本 {edge}，屬於「值得抽」的等級。",
      ]],
      [1.05, [
        "期望值略高於成本（{cp} 倍），長期小賺，但優勢不大，容易被運氣蓋過。",
        "整體略微划算，每抽平均多回 {edge}；適合有計畫地抽，不適合衝動加碼。",
      ]],
      [0.95, [
        "期望值與成本幾乎打平（{cp} 倍），抽獎本質上是在用時間換波動。",
        "大致損益兩平，結果幾乎完全取決於有沒有抽中稀有道具。",
      ]],
      [0.7, [
        "期望值低於成本，每抽平均損失 {loss}；把它當娛樂預算會比較健康。",
        "長期是小虧的獎池（回收 {cp} 倍），想要特定道具時直接購買可能更省。",
        "每投入 1 元平均只回收 {cp} 元，抽越多越接近這個虧損比例。",
      ]],
      [0.4, [
        "明顯負期望（回收 {cp} 倍），每抽平均虧 {loss}，不建議以獲利為目的抽。",
        "回收率偏低，除非某個道具對你有遠高於市價的價值，否則直接買比較划算。",
      ]],
      [-Infinity, [
        "回收率只有 {cp} 倍，屬於高度負期望，大部分花費都不會回來。",
        "極度不划算：每抽平均虧 {loss}，建議只在有明確目標且設好停損時才抽。",
      ]],
    ],
    win: [
      [0.5, ["單抽賺錢機率高達 {win}，多數時候都能回本，波動相對溫和。"]],
      [0.2, ["單抽賺錢機率 {win}，大約每 {every} 抽會有一次超過成本的結果。"]],
      [0.05, ["單抽賺錢機率只有 {win}，大部分抽數都低於成本，獲利集中在少數幾抽。", "只有 {win} 的機率單抽回本，連續數十抽虧損是常態。"]],
      [-Infinity, ["單抽賺錢機率僅 {win}，幾乎每抽都是虧的，收益完全仰賴極低機率的大獎。"]],
    ],
    rare: [
      [0.7, ["獎池非常極端：{rare} 的期望值集中在少數稀有道具上，短期結果會和期望值差很多。", "價值高度集中在稀有道具（占 {rare}），抽少量時很容易遠低於期望值。"]],
      [0.4, ["稀有道具貢獻約 {rare} 的期望值，結果有一定波動，樣本數要夠大才會接近期望。"]],
      [-Infinity, ["價值分布相對平均（稀有道具僅占 {rare}），實際結果會比較貼近期望值。"]],
    ],
    frag: [
      [0.4, ["碎片回饋非常關鍵，占綜合期望值的 {frag}；記得把碎片換成{box}，不要放著不用。"]],
      [0.15, ["碎片約貢獻 {frag} 的期望值，換{box}比較划算。"]],
      [-Infinity, ["碎片回饋占比低（{frag}），對整體損益影響不大。"]],
    ],
  };

  function fill(tpl, vars) {
    return tpl.replace(/\{(\w+)\}/g, (_, k) => (vars[k] ?? ""));
  }
  function tier(table, x) {
    for (const [min, list] of table) if (x >= min) return list;
    return table[table.length - 1][1];
  }

  function comment(a, fmt) {
    if (!a.hasValues) return ["目前這個獎池還沒有任何道具價值，無法評估。請先輸入每抽花費並按「自動帶入價格」，或手動填入你認為的道具價值。"];
    if (a.pool === "apple") {
      if (!(a.cost > 0)) return ["請先輸入「每抽花費」，才能計算回收率與賺錢機率。"];
      const seed = seedOf(a.cp, a.win, a.rare, a.fragShare);
      const vars = {
        cp: a.cp.toFixed(2),
        edge: fmt(Math.abs(a.combined - a.cost)),
        loss: fmt(Math.max(0, a.cost - a.combined)),
        win: pct(a.win),
        every: a.win > 0 ? Math.max(1, Math.round(1 / a.win)) : "∞",
        rare: pct(a.rare, 0),
        frag: pct(a.fragShare, 0),
        box: a.frag.best === "gold" ? "金箱" : "銀箱",
      };
      const lines = [
        fill(pick(tier(LIB.cp, a.cp), seed), vars),
        fill(pick(tier(LIB.win, a.win), seed >> 2), vars),
        fill(pick(tier(LIB.rare, a.rare), seed >> 4), vars),
      ];
      if (a.frag.value > 0) lines.push(fill(pick(tier(LIB.frag, a.fragShare), seed >> 6), vars));
      // 建議
      if (a.cp < 0.95) lines.push("建議：先決定總預算與停損點，抽到預算就停；目標道具若可交易，比較直接購買的價格。");
      else if (a.cp < 1.05) lines.push("建議：損益兩平的獎池適合「順便抽」，不要為了回本追加預算。");
      else lines.push("建議：正期望不代表短期一定賺，準備足夠的資金承受連續虧損，才有機會讓結果貼近期望值。");
      return lines;
    }
    // 箱子
    const seed = seedOf(a.boxEv, a.aboveAvg, a.rare);
    const name = a.pool === "gold" ? "金箱" : "銀箱";
    const other = a.pool === "gold" ? a.frag.silver : a.frag.gold;
    const lines = [];
    lines.push(`開一個${name}平均價值 ${fmt(a.boxEv)}，等於每個碎片值 ${fmt(a.perFrag)}。`);
    if (a.frag.best === a.pool) lines.push(pick([`以目前價值計算，碎片換${name}是最划算的選擇。`, `${name}的碎片效率較高，有碎片就優先開${name}。`], seed));
    else if (other > 0) lines.push(`另一種箱子每碎片值 ${fmt(other)}，比${name}划算，碎片建議優先換另一種。`);
    lines.push(`開出價值高於平均的機率為 ${pct(a.aboveAvg)}。`);
    lines.push(fill(pick(tier(LIB.rare, a.rare), seed >> 3), { rare: pct(a.rare, 0) }));
    return lines;
  }

  return { POOLS, POOL_NAME, BOX_COST, FILLER, parse, withFiller, sampler, ev, fragmentValue, analyze, autoPrice, comment, sum, isJunk };
})();

if (typeof module !== "undefined") module.exports = GachaCore;

/* ==========================================================
 * 介面
 * ========================================================== */
if (typeof document !== "undefined") (() => {
  const C = GachaCore;
  const KEY = "gacha-v1";
  const DRAW_CAP = 2_000_000; // 「抽到為止」的安全上限

  const DEFAULT_TEXT = `靈魂赫卡忒：使魔造型交換券	0.60%
靈魂赫卡忒：盧米埃造型交換券	0.60%
漆黑的BOSS飾品碎片	3.00%
經驗值4倍券(30分)	2.50%
經驗值3倍券(30分)	4.00%
神祕冥界武器變換箱	0.40%
神祕冥界防具變換箱	0.40%
星力16星強化券	0.40%
星力15星強化券	0.40%
星力14星強化券	0.40%
鈦之心	0.45%
妖精之心	0.45%
帽子內襯交換券	0.60%
臉飾內襯交換券	0.60%
武器內襯交換券	0.60%
特別附加潛在能力賦予卷軸	0.80%
睿智葫蘆	0.40%
破曉墜飾	0.90%
星耀耳環	0.90%
暮光印記	0.90%
黎明守護者天使戒指	0.90%
稀有航海師武器選擇箱	3.00%
稀有航海師防具選擇箱	5.00%
核心寶石20個交換券	5.00%
暗黑輪迴星火	9.00%
奧術之河水滴石	1.35%
太初的水滴石	1.35%
水中信紙眼飾	1.50%
凝聚力量的結晶石	1.50%
銀花戒指	1.50%
戴雅希杜斯耳環	1.50%
混沌闇黑龍王的項鍊	1.50%
金花草腰帶	1.50%
梅克奈特墜飾	1.50%
皇家暗黑合金護肩	1.50%
支配者墜飾	1.50%
星力13星強化券	1.50%
星力12星強化券	1.50%
水晶愛心	1.50%
鋰愛心	1.50%
製作物品 20格欄位背包	1.50%
製作法20格背包	1.50%
椅子20格欄位背包	1.50%
稱號20格名片夾	1.50%
卷軸20格背包	1.50%
選擇祕法符文交換券(10個)	1.50%
選擇真實符文交換券(10個)	1.50%
回真卷軸 100%	1.50%
稀有潛在能力賦予卷軸100%	1.50%
幸運日卷軸	1.50%
機器人商店使用卷 (30天)	1.50%
初級能量硬幣(A級)	1.50%
微弱烙印的靈魂石	1.50%
星力11星強化券	1.50%
星力10星強化券	1.50%
選擇欄位8格擴充券	1.50%
黃金愛心	1.50%
經驗累積的祕藥	0.65%
獲得財物的祕藥	0.65%
忍耐的祕藥	0.80%
覺醒的祕藥	0.80%
無敵的祕藥	0.80%
最上級力量強化祕藥	0.80%
最上級智慧強化祕藥	0.80%
最上級敏捷強化祕藥	0.80%
最上級幸運強化祕藥	0.80%
傳說中的英雄祕藥	0.80%
傳說中的祝福祕藥	0.80%
最上級英雄的祕藥	0.80%
最上級祝福的祕藥	0.80%
幸運的銀色箱子
特別附加潛在能力賦予卷軸	30.00%
睿智葫蘆	6.00%
破曉墜飾	2.00%
星耀耳環	2.00%
暮光印記	2.00%
黎明守護者天使戒指	2.00%
稀有航海師武器選擇箱	3.00%
稀有航海師防具選擇箱	3.00%
核心寶石20個交換券	15.00%
暗黑輪迴星火	35.00%
幸運的金色箱子
靈魂赫卡忒：使魔造型交換券	10.00%
靈魂赫卡忒：盧米埃造型交換券	10.00%
帽子內襯交換券	7.00%
臉飾內襯交換券	7.00%
武器內襯交換券	7.00%
神祕冥界武器變換箱	8.00%
神祕冥界防具變換箱	9.00%
星力18星強化券	5.00%
星力17星強化券	5.00%
星力16星強化券	7.00%
星力15星強化券	7.00%
鈦之心	9.00%
妖精之心	9.00%`;

  const blankStats = () => ({ spent: 0, value: 0, draws: 0, frags: 0 });
  function defaults() {
    return { pools: C.parse(DEFAULT_TEXT), values: {}, cost: 0, junk: "", tab: "apple", stats: blankStats(), history: {}, batch: {} };
  }

  let st = (() => {
    try {
      const s = JSON.parse(localStorage.getItem(KEY));
      if (s && s.pools) return { ...defaults(), ...s, stats: { ...blankStats(), ...s.stats } };
    } catch (_) {}
    return defaults();
  })();

  let saveTimer;
  function save() {
    clearTimeout(saveTimer);
    saveTimer = setTimeout(() => { try { localStorage.setItem(KEY, JSON.stringify(st)); } catch (_) {} }, 200);
  }

  const junkList = () => st.junk.split(/\r?\n/).map((s) => s.trim()).filter(Boolean);

  /* ---------- 數字格式 ---------- */
  const intFmt = new Intl.NumberFormat("zh-TW", { maximumFractionDigits: 0 });
  const decFmt = new Intl.NumberFormat("zh-TW", { maximumFractionDigits: 2 });
  function big(n) {
    const a = Math.abs(n);
    const sign = n < 0 ? "-" : "";
    if (a >= 1e8) return `${sign}${decFmt.format(a / 1e8)} 億`;
    if (a >= 1e4) return `${sign}${decFmt.format(a / 1e4)} 萬`;
    return `${sign}${a >= 100 ? intFmt.format(a) : decFmt.format(a)}`;
  }
  const parseNum = (s) => {
    const n = parseFloat(String(s).replace(/[,\s]/g, ""));
    return isFinite(n) ? n : null;
  };

  /* ---------- 道具排序（與左側獎池一致） ---------- */
  function orderMap() {
    const order = {};
    let i = 0;
    for (const p of C.POOLS) for (const it of st.pools[p]) if (!(it.name in order)) order[it.name] = i++;
    order[C.FILLER] = i;
    return order;
  }
  const rareSet = () => {
    const s = new Set();
    for (const p of C.POOLS) for (const it of st.pools[p]) if ((p === "apple" && it.prob < 1) || (p !== "apple" && it.prob <= 5)) s.add(it.name);
    return s;
  };

  /* ---------- 左側：獎池表格 ---------- */
  function renderTabs() {
    $$("#gTabs button").forEach((b) => {
      const p = b.dataset.pool;
      b.classList.toggle("on", p === st.tab);
      b.querySelector("small").textContent = `${st.pools[p].length} 項`;
    });
  }

  function renderTable() {
    const items = st.pools[st.tab];
    const total = C.sum(items);
    const warn = $("#gWarn");
    warn.hidden = total <= 100 + 1e-6;
    warn.textContent = `機率總和為 ${decFmt.format(total)}%，超過 100%，請檢查道具機率（模擬時會依比例換算）。`;

    const box = $("#gTable");
    box.innerHTML = "";
    const rows = C.withFiller(items);
    rows.forEach((it, idx) => {
      const row = document.createElement("div");
      row.className = "g-row" + (it.filler ? " filler" : "");
      const v = st.values[it.name];
      row.innerHTML = `
        <button class="g-draw" ${it.filler ? "disabled" : ""} title="抽到這個道具為止">抽</button>
        <span class="g-name"></span>
        <span class="g-prob r">${decFmt.format(it.prob)}%</span>
        <input class="g-val r" type="text" inputmode="decimal" ${it.filler ? "disabled" : ""} placeholder="—" />
        ${it.filler ? "<span></span>" : '<button class="g-del" title="移除這個道具">✕</button>'}`;
      row.querySelector(".g-name").textContent = it.name;
      const inp = row.querySelector(".g-val");
      inp.value = it.filler ? "0" : typeof v === "number" ? intFmt.format(v) : "";
      if (!it.filler) {
        inp.addEventListener("focus", () => { const x = st.values[it.name]; inp.value = typeof x === "number" ? String(x) : ""; inp.select(); });
        inp.addEventListener("input", () => {
          const n = parseNum(inp.value);
          st.values[it.name] = inp.value.trim() === "" ? null : n;
          save();
          renderAnalysis();
          renderResults();
        });
        inp.addEventListener("blur", () => { const x = st.values[it.name]; inp.value = typeof x === "number" ? intFmt.format(x) : ""; });
        row.querySelector(".g-draw").addEventListener("click", () => drawUntil(st.tab, it.name));
        row.querySelector(".g-del").addEventListener("click", () => {
          st.pools[st.tab].splice(idx, 1);
          save();
          renderAll();
        });
      }
      box.appendChild(row);
    });
  }

  /* ---------- 右側：統計、分析、獲得道具 ---------- */
  function renderStats() {
    const s = st.stats;
    const profit = s.value - s.spent;
    $("#sSpent").textContent = big(s.spent);
    $("#sValue").textContent = big(s.value);
    const pEl = $("#sProfit");
    pEl.textContent = (profit > 0 ? "+" : "") + big(profit);
    pEl.className = profit > 0 ? "pos" : profit < 0 ? "neg" : "";
    $("#sDraws").textContent = intFmt.format(s.draws);
    $("#sFrags").textContent = intFmt.format(s.frags);
    $("#gOpenSilver").disabled = s.frags < C.BOX_COST.silver;
    $("#gOpenGold").disabled = s.frags < C.BOX_COST.gold;
  }

  function metric(label, value, tone = "") {
    return `<div class="g-metric"><span>${label}</span><b class="${tone}">${value}</b></div>`;
  }

  function renderAnalysis() {
    const a = C.analyze(st, st.tab);
    $("#gAnTitle").textContent = `期望值分析 · ${C.POOL_NAME[st.tab]}`;
    let html = "";
    if (st.tab === "apple") {
      const cpTone = a.cp === null ? "" : a.cp >= 1 ? "pos" : "neg";
      html += metric("道具期望值 / 抽", big(a.itemEv));
      html += metric("碎片期望值 / 抽", big(a.frag.value));
      html += metric("綜合期望值 (EV)", big(a.combined));
      html += metric("單抽 CP 值", a.cp === null ? "—" : `${a.cp.toFixed(2)} 倍`, cpTone);
      html += metric("單抽賺錢機率", st.cost > 0 ? `${(a.win * 100).toFixed(1)}%` : "—");
      const s = st.stats;
      if (s.draws > 0 && s.spent > 0) html += metric("實際回收率", `${((s.value / s.spent) * 100).toFixed(1)}%`, s.value >= s.spent ? "pos" : "neg");
    } else {
      html += metric("每箱期望值", big(a.boxEv));
      html += metric("每碎片價值", big(a.perFrag));
      html += metric("高於平均機率", `${(a.aboveAvg * 100).toFixed(1)}%`);
      html += metric("稀有道具占比", `${(a.rare * 100).toFixed(0)}%`);
    }
    $("#gMetrics").innerHTML = html;
    const ai = $("#gAi");
    ai.innerHTML = "";
    for (const line of C.comment(a, big)) {
      const p = document.createElement("p");
      p.textContent = line;
      ai.appendChild(p);
    }
  }

  function resultRow(name, count, rare, order) {
    const v = typeof st.values[name] === "number" ? st.values[name] : 0;
    const el = document.createElement("div");
    el.className = "g-res-row" + (rare.has(name) ? " rare" : "") + (name === C.FILLER ? " filler" : "");
    el.innerHTML = `<span class="g-res-name"></span><span class="g-res-count">×${intFmt.format(count)}</span><span class="g-res-val">${v ? big(v * count) : "—"}</span>`;
    el.querySelector(".g-res-name").textContent = name;
    return el;
  }

  function renderResults() {
    const order = orderMap();
    const rare = rareSet();
    const sortKeys = (obj) => Object.keys(obj).sort((a, b) => (order[a] ?? 1e9) - (order[b] ?? 1e9));
    const batchKeys = sortKeys(st.batch);
    const histKeys = sortKeys(st.history);
    $("#gBatchBox").hidden = batchKeys.length === 0;
    const bBox = $("#gBatch");
    bBox.innerHTML = "";
    for (const k of batchKeys) bBox.appendChild(resultRow(k, st.batch[k], rare, order));
    const hBox = $("#gHistory");
    hBox.innerHTML = "";
    for (const k of histKeys) hBox.appendChild(resultRow(k, st.history[k], rare, order));
    $("#gResEmpty").hidden = batchKeys.length + histKeys.length > 0;
    const n = [...Object.values(st.batch), ...Object.values(st.history)].reduce((a, b) => a + b, 0);
    $("#gResCount").textContent = n ? `共 ${intFmt.format(n)} 個` : "";
  }

  function renderAll() {
    renderTabs();
    renderTable();
    renderStats();
    renderAnalysis();
    renderResults();
    if (document.activeElement !== $("#gCost")) $("#gCost").value = st.cost ? intFmt.format(st.cost) : "";
    if (document.activeElement !== $("#gJunk")) $("#gJunk").value = st.junk;
    $("#gJunkCount").textContent = junkList().length ? `(${junkList().length})` : "";
  }

  /* ---------- 模擬 ---------- */
  // 開始新一輪：把上一輪「本次中獎」併入下方清單
  function beginBatch() {
    for (const [k, n] of Object.entries(st.batch)) st.history[k] = (st.history[k] || 0) + n;
    st.batch = {};
  }
  function gain(item) {
    st.batch[item.name] = (st.batch[item.name] || 0) + 1;
    const v = st.values[item.name];
    if (typeof v === "number" && isFinite(v)) st.stats.value += v;
  }
  function makeApple() {
    const pick = C.sampler(st.pools.apple);
    return () => {
      const it = pick();
      st.stats.draws += 1;
      st.stats.spent += st.cost || 0;
      st.stats.frags += 1; // 每抽必得 1 碎片
      gain(it);
      return it;
    };
  }
  function makeBox(pool) {
    const pick = C.sampler(st.pools[pool]);
    return () => {
      st.stats.frags -= C.BOX_COST[pool];
      const it = pick();
      gain(it);
      return it;
    };
  }
  function finish(msg) {
    save();
    renderStats();
    renderAnalysis();
    renderResults();
    if (msg) toast(msg);
  }

  function drawN(n) {
    if (!st.pools.apple.length) return toast("黃金蘋果獎池是空的，請先貼上獎池文字");
    beginBatch();
    const draw = makeApple();
    for (let i = 0; i < n; i++) draw();
    finish();
  }

  function openBox(pool) {
    if (st.stats.frags < C.BOX_COST[pool]) return;
    if (!st.pools[pool].length) return toast(`${C.POOL_NAME[pool]}獎池是空的`);
    beginBatch();
    makeBox(pool)();
    finish();
  }

  /** 抽到該道具為止（箱子道具：抽蘋果累積碎片 → 開箱，直到開出） */
  function drawUntil(pool, name) {
    if (!st.pools.apple.length) return toast("黃金蘋果獎池是空的，請先貼上獎池文字");
    beginBatch();
    const draw = makeApple();
    const start = st.stats.draws;
    let got = false;
    if (pool === "apple") {
      while (st.stats.draws - start < DRAW_CAP) if (draw().name === name) { got = true; break; }
    } else {
      const open = makeBox(pool);
      while (st.stats.draws - start < DRAW_CAP) {
        if (st.stats.frags < C.BOX_COST[pool]) { draw(); continue; }
        if (open().name === name) { got = true; break; }
      }
    }
    const used = st.stats.draws - start;
    finish(got ? `抽到「${name}」！共用了 ${intFmt.format(used)} 抽` : `已達 ${intFmt.format(DRAW_CAP)} 抽上限仍未抽到`);
  }

  /* ---------- 事件 ---------- */
  function doParse() {
    const parsed = C.parse($("#gPaste").value);
    const got = C.POOLS.filter((p) => parsed[p].length);
    if (!got.length) return toast("沒有解析到任何「名稱 + 機率%」的道具");
    for (const p of got) st.pools[p] = parsed[p];
    if (!got.includes(st.tab)) st.tab = got[0];
    save();
    renderAll();
    toast(got.map((p) => `${C.POOL_NAME[p]} ${parsed[p].length} 項`).join("、"));
  }
  $("#gParse").addEventListener("click", doParse);
  $("#gPaste").addEventListener("paste", () => setTimeout(doParse, 0));

  $$("#gTabs button").forEach((b) => b.addEventListener("click", () => { st.tab = b.dataset.pool; save(); renderTabs(); renderTable(); renderAnalysis(); }));

  $("#gCost").addEventListener("focus", (e) => { e.target.value = st.cost ? String(st.cost) : ""; });
  $("#gCost").addEventListener("input", (e) => { st.cost = Math.max(0, parseNum(e.target.value) || 0); save(); renderAnalysis(); });
  $("#gCost").addEventListener("blur", (e) => { e.target.value = st.cost ? intFmt.format(st.cost) : ""; });

  $("#gJunkBtn").addEventListener("click", () => { const b = $("#gJunkBox"); b.hidden = !b.hidden; if (!b.hidden) $("#gJunk").focus(); });
  $("#gJunk").addEventListener("input", (e) => {
    st.junk = e.target.value;
    $("#gJunkCount").textContent = junkList().length ? `(${junkList().length})` : "";
    save();
  });

  $("#gAutoPrice").addEventListener("click", () => {
    if (!(st.cost > 0)) return toast("請先輸入每抽花費，自動價格以它為基準");
    st.values = C.autoPrice(st.pools, st.cost, junkList());
    save();
    renderAll();
    toast("已自動帶入基準價格，可再依實際行情調整");
  });

  $("#gClearValues").addEventListener("click", async () => {
    if (!(await confirmBox("確定要清空所有道具的價值嗎？", { title: "清空價值", ok: "清空", danger: true }))) return;
    st.values = {};
    save();
    renderAll();
  });

  $$("[data-draw]").forEach((b) => b.addEventListener("click", () => drawN(Number(b.dataset.draw))));
  $("#gOpenSilver").addEventListener("click", () => openBox("silver"));
  $("#gOpenGold").addEventListener("click", () => openBox("gold"));
  $("#gReset").addEventListener("click", async () => {
    if (!(await confirmBox("確定要把花費、價值、抽數、碎片與獲得道具全部歸零嗎？", { title: "重置統計", ok: "重置", danger: true }))) return;
    st.stats = blankStats();
    st.history = {};
    st.batch = {};
    save();
    renderAll();
  });

  renderAll();
  window.Gacha = { onShow: renderAll };
})();
