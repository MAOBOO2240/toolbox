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

  /** 大獎貢獻的期望值占比 */
  function jackpotShare(items, values, jackpot) {
    const total = ev(items, values);
    if (total <= 0) return 0;
    const t = Math.max(100, sum(items));
    return items.filter((it) => jackpot.has(it.name)).reduce((a, it) => a + (it.prob / t) * valueOf(values, it.name), 0) / total;
  }

  function analyze(state, pool, jackpot = null) {
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
        rare: jackpot ? jackpotShare(items, values, jackpot) : rareShare(items, values, RARE_LIMIT.apple),
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
      rare: jackpot ? jackpotShare(items, values, jackpot) : rareShare(items, values, RARE_LIMIT[pool]),
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

  // 「秘／祕」視為同一字，避免關鍵字與道具名稱用字不同而比對不到
  const norm = (s) => s.replace(/祕/g, "秘");
  const isJunk = (name, junk) => junk.some((k) => k && norm(name).includes(norm(k)));

  /** 取兩位有效數字，讓數字好讀 */
  function nice(n) {
    if (!(n > 0)) return 0;
    const mag = Math.pow(10, Math.floor(Math.log10(n)) - 1);
    return Math.max(1, Math.round(n / mag) * mag);
  }

  /** 大獎：黃金蘋果獎池中，分界道具「上面」的道具（不含分界道具本身）；找不到分界道具時回傳 null */
  const DEFAULT_JACKPOT_MARKER = "漆黑的BOSS飾品碎片";
  function jackpotSet(pools, marker) {
    const m = String(marker || "").trim();
    if (!m) return null;
    const idx = pools.apple.findIndex((it) => it.name === m);
    if (idx < 0) return null;
    return new Set(pools.apple.slice(0, idx).map((it) => it.name));
  }
  const CHEAP_VALUE = 10; // 非大獎、非垃圾的道具：幾乎沒有價值

  /**
   * 自動帶入價格：
   * - 垃圾道具、任何獎池中機率 > 30% 的常見道具 → 留空
   * - 其餘：基準權重 = (1 / 每抽獲得機率)^0.8（非線性，越稀有越高）
   * - 整體縮放成「單抽綜合期望值 ≈ 每抽花費」，當作中性的基準起點
   */
  function autoPrice(pools, cost, junk, jackpot = null) {
    const names = new Set();
    for (const p of POOLS) for (const it of pools[p]) names.add(it.name);
    const weight = {}; // 會依成本縮放的權重（大獎）
    const fixed = {};  // 固定價值（非大獎 = 10；垃圾、常見 = 留空）
    for (const name of names) {
      const common = POOLS.some((p) => pools[p].some((it) => it.name === name && it.prob > 30));
      if (isJunk(name, junk) || common) { fixed[name] = null; continue; }
      if (jackpot && !jackpot.has(name)) { fixed[name] = CHEAP_VALUE; continue; }
      const pe = effectiveProb(pools, name);
      if (pe > 0) weight[name] = Math.pow(1 / pe, 0.8);
      else fixed[name] = null;
    }
    // 縮放大獎價值，讓「單抽綜合期望值 ≈ 每抽花費」
    const total = (k) => {
      const v = {};
      for (const n in fixed) if (fixed[n] !== null) v[n] = fixed[n];
      for (const n in weight) v[n] = weight[n] * k;
      return ev(pools.apple, v) + fragmentValue(pools, v).value;
    };
    const a = total(0);
    const b = total(1) - a;
    let k = b > 0 ? (cost - a) / b : 0;
    if (b > 0 && k <= 0) k = (cost * 0.5) / b; // 成本比便宜道具還低時，大獎仍保留合理價值
    const values = {};
    for (const name of names) values[name] = name in weight ? nice(weight[name] * k) : fixed[name];
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

  return { POOLS, POOL_NAME, BOX_COST, FILLER, DEFAULT_JACKPOT_MARKER, CHEAP_VALUE, jackpotSet, parse, withFiller, sampler, ev, fragmentValue, analyze, autoPrice, comment, sum, isJunk };
})();

if (typeof module !== "undefined") module.exports = GachaCore;

/* ==========================================================
 * 介面
 * ========================================================== */
if (typeof document !== "undefined") (() => {
  const C = GachaCore;
  const KEY = "gacha-v2";
  const DRAW_CAP = 2_000_000; // 「抽到為止」的安全上限
  const SHORT = { apple: "蘋果", silver: "銀箱", gold: "金箱" };

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

  // 預設垃圾道具清單（空格或換行分隔）
  const DEFAULT_JUNK = "葫蘆 神秘 之心 內襯 破曉 星耀 暮光 黎明 航海師 核心寶石 星火 水滴石 水中 結晶石 銀花 杜斯耳環 混沌闇黑龍王 金花草 梅克奈特 黑合金 支配者墜飾 愛心 20格 稀有潛在 商店使用 初級能量 微弱烙印 星力10 星力11 星力12 星力13 星力14 8格 忍耐 覺醒 無敵 最上級 傳說中的";

  const zeroPools = () => ({ apple: 0, silver: 0, gold: 0 });
  const blankStats = () => ({ spent: 0, value: 0, draws: 0, frags: 0 });
  function defaults() {
    return {
      pools: C.parse(DEFAULT_TEXT), values: {}, cost: 0, junk: DEFAULT_JUNK, jackpotMarker: C.DEFAULT_JACKPOT_MARKER, tab: "apple", page: "sim",
      stats: blankStats(), batch: {}, batchDraws: zeroPools(), hist: {}, histDraws: zeroPools(),
    };
  }

  let st = (() => {
    const d = defaults();
    try {
      const s = JSON.parse(localStorage.getItem(KEY));
      if (s && s.pools) return { ...d, ...s, stats: { ...blankStats(), ...s.stats }, batchDraws: { ...zeroPools(), ...s.batchDraws }, histDraws: { ...zeroPools(), ...s.histDraws } };
      const old = JSON.parse(localStorage.getItem("gacha-v1"));
      if (old && old.pools) return { ...d, pools: old.pools, values: old.values || {}, cost: old.cost || 0 };
    } catch (_) {}
    return d;
  })();

  let saveTimer;
  function save() {
    clearTimeout(saveTimer);
    saveTimer = setTimeout(() => { try { localStorage.setItem(KEY, JSON.stringify(st)); } catch (_) {} }, 200);
  }

  const junkList = () => st.junk.split(/\s+/).map((s) => s.trim()).filter(Boolean);
  const jackpot = () => C.jackpotSet(st.pools, st.jackpotMarker);

  /* ---------- 數字 ---------- */
  const intFmt = new Intl.NumberFormat("zh-TW", { maximumFractionDigits: 0 });
  const decFmt = new Intl.NumberFormat("zh-TW", { maximumFractionDigits: 2 });
  function big(n) {
    const a = Math.abs(n), sign = n < 0 ? "-" : "";
    if (a >= 1e8) return `${sign}${decFmt.format(a / 1e8)}億`;
    if (a >= 1e4) return `${sign}${decFmt.format(a / 1e4)}萬`;
    return `${sign}${a >= 100 ? intFmt.format(a) : decFmt.format(a)}`;
  }
  const parseNum = (s) => {
    const n = parseFloat(String(s).replace(/[,\s%]/g, ""));
    return isFinite(n) ? n : null;
  };
  const valueOf = (name) => (typeof st.values[name] === "number" && isFinite(st.values[name]) ? st.values[name] : 0);

  /* ---------- 分頁 ---------- */
  function renderPage() {
    $$("#gPages button").forEach((b) => b.classList.toggle("on", b.dataset.page === st.page));
    $("#gPageSim").hidden = st.page !== "sim";
    $("#gPageImport").hidden = st.page !== "import";
  }

  /* ---------- 獎池表格 ---------- */
  /** 箱子分頁：碎片夠開的時候亮起來，下面顯示可以開幾箱 */
  const boxAvail = (pool) => Math.floor(st.stats.frags / C.BOX_COST[pool]);
  function renderTabs() {
    $$("#gTabs button").forEach((b) => {
      const p = b.dataset.pool;
      b.classList.toggle("on", p === st.tab);
      const small = b.querySelector("small");
      if (p === "apple") { small.textContent = "花錢抽"; b.classList.remove("ready", "empty"); return; }
      const n = boxAvail(p);
      small.textContent = n ? `可開 ${intFmt.format(n)} 箱` : `需 ${C.BOX_COST[p]} 碎片`;
      b.classList.toggle("ready", n > 0);
      b.classList.toggle("empty", n === 0);
      b.title = `每 ${C.BOX_COST[p]} 個蘋果碎片可以開一次${C.POOL_NAME[p]}`;
    });
  }

  function textInput(cls, value, opts = {}) {
    const el = document.createElement("input");
    el.type = "text";
    el.className = cls;
    el.value = value;
    if (opts.decimal) el.inputMode = "decimal";
    if (opts.placeholder) el.placeholder = opts.placeholder;
    if (opts.disabled) el.disabled = true;
    return el;
  }

  function renderTable() {
    const items = st.pools[st.tab];
    const total = C.sum(items);
    const warn = $("#gWarn");
    warn.hidden = total <= 100 + 1e-6;
    warn.textContent = `機率總和 ${decFmt.format(total)}% 超過 100%，請檢查道具機率（模擬時會依比例換算）。`;

    const box = $("#gTable");
    box.innerHTML = "";
    if (!items.length) {
      box.innerHTML = '<div class="g-empty">這個獎池還沒有道具，可以到「解析/垃圾」貼上文字，或在上方新增。</div>';
      return;
    }
    const jp = jackpot();
    C.withFiller(items).forEach((it, idx) => {
      const row = document.createElement("div");
      row.className = "g-row" + (it.filler ? " filler" : "") + (jp && jp.has(it.name) ? " jackpot" : "");
      const drawBtn = document.createElement("button");
      drawBtn.className = "g-draw";
      drawBtn.textContent = "抽";
      drawBtn.title = st.tab === "apple" ? "一直抽到這個道具為止" : "用現有碎片一直開箱到這個道具為止";
      const name = textInput("g-name", it.name, { disabled: it.filler });
      const prob = textInput("g-prob", it.filler ? `${decFmt.format(it.prob)}%` : `${it.prob}%`, { decimal: true, disabled: it.filler });
      const v = st.values[it.name];
      const val = textInput("g-val", it.filler ? "0" : typeof v === "number" ? intFmt.format(v) : "", { decimal: true, placeholder: "—", disabled: it.filler });
      // 名稱左邊顯示道具圖示
      const nameCell = document.createElement("div");
      nameCell.className = "g-namecell";
      if (!it.filler) nameCell.append(ItemIcons.el(it.name));
      nameCell.append(name);
      row.append(drawBtn, nameCell, prob, val);
      if (it.filler) {
        drawBtn.disabled = true;
        row.append(document.createElement("span"));
        box.appendChild(row);
        return;
      }
      const del = document.createElement("button");
      del.className = "g-del";
      del.textContent = "✕";
      del.title = "移除這個道具";
      row.append(del);

      drawBtn.addEventListener("click", () => drawUntil(st.tab, it.name));
      del.addEventListener("click", () => { st.pools[st.tab].splice(idx, 1); save(); renderPool(); renderRight(); });
      name.addEventListener("change", () => {
        const nn = name.value.trim();
        if (!nn || nn === it.name) { name.value = it.name; return; }
        if (!(nn in st.values) && it.name in st.values) st.values[nn] = st.values[it.name];
        it.name = nn;
        save(); renderRight();
      });
      prob.addEventListener("focus", () => { prob.value = String(it.prob); prob.select(); });
      prob.addEventListener("input", () => {
        const n = parseNum(prob.value);
        if (n === null || n < 0) return;
        it.prob = n;
        save(); renderRight();
      });
      prob.addEventListener("blur", () => renderPool());
      val.addEventListener("focus", () => { const x = st.values[it.name]; val.value = typeof x === "number" ? String(x) : ""; val.select(); });
      val.addEventListener("input", () => {
        st.values[it.name] = val.value.trim() === "" ? null : parseNum(val.value);
        save(); renderRight(); renderPriceBtn();
      });
      val.addEventListener("blur", () => { const x = st.values[it.name]; val.value = typeof x === "number" ? intFmt.format(x) : ""; });
      box.appendChild(row);
    });
  }

  const hasAnyValue = () => Object.values(st.values).some((v) => typeof v === "number" && isFinite(v));
  function renderPriceBtn() {
    const btn = $("#gAutoPrice");
    const any = hasAnyValue();
    btn.textContent = any ? "移除全部價格" : "自動帶入價格";
    btn.classList.toggle("c-teal", !any);
    btn.classList.toggle("c-red", any);
    btn.title = any ? "清空所有道具的價值" : "依機率估算基準價值：越稀有越高；垃圾道具與機率超過 30% 的道具會留空";
  }

  function renderPool() {
    renderTabs();
    renderTable();
    renderPriceBtn();
  }

  /* ---------- 統計、分析、結果 ---------- */
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
    // 抽獎按鈕抽的是左邊選到的獎池：黃金蘋果花錢抽，箱子用碎片開
    const pool = st.tab;
    const avail = pool === "apple" ? Infinity : boxAvail(pool);
    $$("[data-draw]").forEach((b) => (b.disabled = !avail));
    $("#gDrawInfo").textContent = pool === "apple"
      ? "目前抽：黃金蘋果・每抽獲得 1 個碎片・按住可連續抽"
      : avail ? `目前開：${C.POOL_NAME[pool]}（剩 ${intFmt.format(avail)} 箱，每箱 ${C.BOX_COST[pool]} 碎片）` : `碎片不足，開${C.POOL_NAME[pool]}需要 ${C.BOX_COST[pool]} 個碎片`;
    renderTabs();
  }

  const metric = (label, value, cls = "") => `<div class="g-metric"><span>${label}</span><b class="${cls}">${value}</b></div>`;

  function renderAnalysis() {
    const a = C.analyze(st, st.tab, jackpot());
    let html = "";
    if (st.tab === "apple") {
      html += metric("每抽期望值", a.hasValues ? big(a.combined) : "—");
      html += metric("CP 值", a.cp === null || !a.hasValues ? "—" : `${a.cp.toFixed(2)} 倍`, a.cp === null ? "" : a.cp >= 1 ? "pos" : "neg");
      html += metric("單抽賺錢機率", a.hasValues && st.cost > 0 ? `${(a.win * 100).toFixed(1)}%` : "—");
      html += metric("碎片價值", a.hasValues ? big(a.frag.value) : "—");
    } else {
      html += metric("每箱期望值", a.hasValues ? big(a.boxEv) : "—");
      html += metric("每碎片價值", a.hasValues ? big(a.perFrag) : "—");
      html += metric("高於平均機率", a.hasValues ? `${(a.aboveAvg * 100).toFixed(1)}%` : "—");
      html += metric("大獎價值占比", a.hasValues ? `${(a.rare * 100).toFixed(0)}%` : "—");
    }
    $("#gMetrics").innerHTML = html;
    // 評語只取最重要的兩句，保持簡潔
    const lines = C.comment(a, big).slice(0, 2);
    const c = $("#gComment");
    c.innerHTML = "<b>AI 評語</b>";
    c.append(document.createTextNode(lines.join("")));
  }

  const key = (pool, name) => `${pool}|${name}`;
  const splitKey = (k) => { const i = k.indexOf("|"); return [k.slice(0, i), k.slice(i + 1)]; };
  function sortedKeys(obj) {
    const order = {};
    for (const p of C.POOLS) st.pools[p].forEach((it, i) => { const k = key(p, it.name); if (!(k in order)) order[k] = C.POOLS.indexOf(p) * 10000 + i; });
    const rank = (k) => order[k] ?? C.POOLS.indexOf(splitKey(k)[0]) * 10000 + 9999;
    return Object.keys(obj).sort((a, b) => rank(a) - rank(b));
  }
  // 金色字：只有大獎（分界道具以上）才顯示；找不到分界道具時改用機率判斷
  const isRare = (pool, name) => {
    const jp = jackpot();
    if (jp) return jp.has(name);
    return st.pools[pool].some((it) => it.name === name && (pool === "apple" ? it.prob < 1 : it.prob <= 5));
  };

  function renderBatch() {
    const box = $("#gBatch");
    box.innerHTML = "";
    const d = st.batchDraws;
    const parts = [];
    if (d.apple) parts.push(`${intFmt.format(d.apple)} 抽`);
    if (d.silver) parts.push(`銀箱 ${intFmt.format(d.silver)} 次`);
    if (d.gold) parts.push(`金箱 ${intFmt.format(d.gold)} 次`);
    $("#gBatchBadge").textContent = parts.join("・");
    const keys = sortedKeys(st.batch);
    if (!keys.length) {
      box.innerHTML = '<span class="g-chip-empty">按下抽獎或開箱後，這裡會顯示這次的結果</span>';
      return;
    }
    for (const k of keys) {
      const [pool, name] = splitKey(k);
      const chip = document.createElement("span");
      chip.className = "g-chip" + (isRare(pool, name) ? " rare" : "");
      if (pool !== "apple") {
        const src = document.createElement("span");
        src.className = "src";
        src.textContent = SHORT[pool];
        chip.append(src);
      }
      chip.append(ItemIcons.el(name), document.createTextNode(name));
      const n = document.createElement("b");
      n.textContent = `×${st.batch[k]}`;
      chip.append(n);
      box.appendChild(chip);
    }
  }

  function renderHistory() {
    const box = $("#gHistory");
    box.innerHTML = "";
    const keys = sortedKeys(st.hist);
    if (!keys.length) {
      box.innerHTML = '<div class="g-empty">保管箱還是空的</div>';
      return;
    }
    let lastPool = null;
    for (const k of keys) {
      const [pool, name] = splitKey(k);
      if (pool !== lastPool) {
        lastPool = pool;
        const g = document.createElement("div");
        g.className = "g-group";
        g.textContent = `${C.POOL_NAME[pool]}・${intFmt.format(st.histDraws[pool])} ${pool === "apple" ? "抽" : "次"}`;
        box.appendChild(g);
      }
      const count = st.hist[k];
      const draws = st.histDraws[pool] || 0;
      const actual = draws ? (count / draws) * 100 : 0;
      const expect = st.pools[pool].filter((it) => it.name === name).reduce((a, it) => a + it.prob, 0);
      const cls = !expect || Math.abs(actual - expect) < 1e-9 ? "same" : actual > expect ? "up" : "down";
      const arrow = cls === "up" ? " ▲" : cls === "down" ? " ▼" : "";
      const row = document.createElement("div");
      row.className = "g-hrow";
      const nameEl = document.createElement("span");
      nameEl.className = "g-hname";
      const txt = document.createElement("span");
      txt.className = "t";
      txt.textContent = name;
      nameEl.append(ItemIcons.el(name), txt);
      nameEl.title = name;
      row.append(nameEl);
      row.insertAdjacentHTML("beforeend",
        `<span>×${intFmt.format(count)}</span><span class="muted">${valueOf(name) ? big(valueOf(name) * count) : "—"}</span><span class="${cls}">${actual.toFixed(2)}%${arrow}</span>`);
      box.appendChild(row);
    }
  }

  function renderRight() {
    renderStats();
    renderAnalysis();
    renderBatch();
    renderHistory();
  }

  /* ---------- 第二頁：解析與垃圾清單 ---------- */
  function renderImport() {
    if (document.activeElement !== $("#gJunk")) $("#gJunk").value = st.junk;
    if (document.activeElement !== $("#gMarker")) $("#gMarker").value = st.jackpotMarker;
    renderMarkerInfo();
    renderJunkMatch();
  }
  function renderMarkerInfo() {
    const jp = jackpot();
    $("#gMarkerInfo").textContent = jp ? `大獎 ${jp.size} 項：${[...jp].join("、")}` : "黃金蘋果獎池中找不到這個道具，改用機率判斷";
  }
  function renderJunkMatch() {
    const junk = junkList();
    const names = [];
    for (const p of C.POOLS) for (const it of st.pools[p]) if (!names.includes(it.name) && C.isJunk(it.name, junk)) names.push(it.name);
    $("#gJunkCount").textContent = `${junk.length} 個關鍵字・目前符合 ${names.length} 項道具`;
    const box = $("#gJunkMatch");
    box.innerHTML = "";
    for (const n of names) {
      const s = document.createElement("span");
      s.textContent = n;
      box.appendChild(s);
    }
  }

  function renderAll() {
    renderPage();
    renderPool();
    renderRight();
    renderImport();
    if (document.activeElement !== $("#gCost")) $("#gCost").value = st.cost ? intFmt.format(st.cost) : "";
  }

  /* ---------- 模擬 ---------- */
  // 新的一次操作：上一次「本次獲得」併入保管箱
  function beginBatch() {
    for (const [k, n] of Object.entries(st.batch)) st.hist[k] = (st.hist[k] || 0) + n;
    for (const p of C.POOLS) st.histDraws[p] += st.batchDraws[p];
    st.batch = {};
    st.batchDraws = zeroPools();
  }
  function gain(pool, item) {
    const k = key(pool, item.name);
    st.batch[k] = (st.batch[k] || 0) + 1;
    st.batchDraws[pool] += 1;
    st.stats.value += valueOf(item.name);
  }
  function appleDrawer() {
    const pick = C.sampler(st.pools.apple);
    return () => {
      const it = pick();
      st.stats.draws += 1;
      st.stats.spent += st.cost || 0;
      st.stats.frags += 1; // 每抽必得 1 碎片
      gain("apple", it);
      return it;
    };
  }
  // 開箱只消耗碎片，不花錢
  function boxOpener(pool) {
    const pick = C.sampler(st.pools[pool]);
    return () => {
      st.stats.frags -= C.BOX_COST[pool];
      const it = pick();
      gain(pool, it);
      return it;
    };
  }
  function commit(msg) {
    save();
    renderRight();
    if (msg) toast(msg);
  }

  /** 抽左邊選到的獎池：黃金蘋果花錢抽；銀箱、金箱用碎片開，碎片不夠就停 */
  function drawN(n, fresh) {
    const pool = st.tab;
    if (!st.pools[pool].length) { toast(`${C.POOL_NAME[pool]}獎池是空的`); return false; }
    if (pool !== "apple") {
      const avail = boxAvail(pool);
      if (!avail) { toast(`碎片不足，開${C.POOL_NAME[pool]}需要 ${C.BOX_COST[pool]} 個碎片`); return false; }
      n = Math.min(n, avail);
    }
    if (fresh) beginBatch();
    const draw = pool === "apple" ? appleDrawer() : boxOpener(pool);
    for (let i = 0; i < n; i++) draw();
    commit();
    if (pool !== "apple" && !boxAvail(pool)) return false; // 碎片用完，按住也停下來
  }

  function drawUntil(pool, name) {
    if (pool === "apple") {
      beginBatch();
      const draw = appleDrawer();
      for (let i = 0; i < DRAW_CAP; i++) {
        if (draw().name === name) return commit(`抽到「${name}」，共 ${intFmt.format(i + 1)} 抽`);
      }
      return commit(`已達 ${intFmt.format(DRAW_CAP)} 抽上限仍未抽到`);
    }
    // 箱子：只用現有碎片開箱，碎片用完就停止
    const need = C.BOX_COST[pool];
    if (st.stats.frags < need) return toast(`碎片不足，開${SHORT[pool]}需要 ${need} 個碎片`);
    beginBatch();
    const open = boxOpener(pool);
    let n = 0;
    while (st.stats.frags >= need) {
      n++;
      if (open().name === name) return commit(`開到「${name}」，共開 ${n} 箱`);
    }
    commit(`碎片用完了，開了 ${n} 箱仍未開到「${name}」`);
  }

  /** 按住按鈕可連續執行；每一次都算獨立的一次，上一次的結果會進保管箱 */
  function holdable(btn, fn) {
    let t1 = null, t2 = null;
    const stop = () => { clearTimeout(t1); clearInterval(t2); t1 = t2 = null; };
    btn.addEventListener("pointerdown", (e) => {
      if (e.button !== 0 || btn.disabled) return;
      if (fn(true) === false) return;
      t1 = setTimeout(() => {
        t2 = setInterval(() => { if (btn.disabled || fn(true) === false) stop(); }, 110);
      }, 380);
    });
    ["pointerup", "pointerleave", "pointercancel"].forEach((ev) => btn.addEventListener(ev, stop));
  }

  /* ---------- 事件 ---------- */
  $$("#gPages button").forEach((b) => b.addEventListener("click", () => { st.page = b.dataset.page; save(); renderAll(); window.refreshBack?.(); }));
  // 在「解析/垃圾」頁時，左上角變成「上一頁」回到抽獎模擬
  if (window.BackHooks) window.BackHooks["view-gacha"] = {
    label: () => (st.page === "import" ? "上一頁" : null),
    back: () => { if (st.page !== "import") return false; st.page = "sim"; save(); renderAll(); return true; },
  };
  // 獎池分頁加上遊戲道具圖示
  const TAB_ITEM = { apple: "黃金蘋果", silver: "幸運的銀色箱子", gold: "幸運的金色箱子" };
  $$("#gTabs button").forEach((b) => b.prepend(ItemIcons.el(TAB_ITEM[b.dataset.pool])));
  $$("#gTabs button").forEach((b) => b.addEventListener("click", () => { st.tab = b.dataset.pool; save(); renderPool(); renderAnalysis(); renderStats(); }));

  $("#gCost").addEventListener("focus", (e) => { e.target.value = st.cost ? String(st.cost) : ""; e.target.select(); });
  $("#gCost").addEventListener("input", (e) => { st.cost = Math.max(0, parseNum(e.target.value) || 0); save(); renderAnalysis(); });
  $("#gCost").addEventListener("blur", (e) => { e.target.value = st.cost ? intFmt.format(st.cost) : ""; });

  $("#gAutoPrice").addEventListener("click", async () => {
    if (hasAnyValue()) {
      if (!(await confirmBox("確定要移除所有道具的價格嗎？"))) return;
      st.values = {};
      save();
      renderAll();
      return;
    }
    if (!(st.cost > 0)) return toast("請先輸入每抽成本，自動價格以它為基準");
    st.values = C.autoPrice(st.pools, st.cost, junkList(), jackpot());
    save();
    renderAll();
    toast("已帶入基準價格，可再依實際行情調整");
  });

  $("#gAdd").addEventListener("click", () => {
    const name = $("#gNewName").value.trim();
    const prob = parseNum($("#gNewProb").value);
    if (!name || !(prob > 0)) return toast("請輸入道具名稱與機率");
    st.pools[st.tab].push({ name, prob });
    const v = parseNum($("#gNewValue").value);
    if (v !== null) st.values[name] = v;
    $("#gNewName").value = $("#gNewProb").value = $("#gNewValue").value = "";
    save();
    renderPool();
    renderRight();
    $("#gNewName").focus();
  });
  ["#gNewName", "#gNewProb", "#gNewValue"].forEach((id) =>
    $(id).addEventListener("keydown", (e) => { if (e.key === "Enter") $("#gAdd").click(); })
  );

  // 清除道具：三個獎池一起清空
  $("#gClearItems").addEventListener("click", async () => {
    if (!C.POOLS.some((p) => st.pools[p].length)) return;
    if (!(await confirmBox("確定要清除黃金蘋果、銀色箱子、金色箱子的所有道具嗎？"))) return;
    for (const p of C.POOLS) st.pools[p] = [];
    save();
    renderAll();
  });

  $$("[data-draw]").forEach((b) => holdable(b, (fresh) => drawN(Number(b.dataset.draw), fresh)));

  $("#gReset").addEventListener("click", async () => {
    if (!(await confirmBox("確定要重置花費、價值、抽數、碎片與保管箱嗎？"))) return;
    st.stats = blankStats();
    st.batch = {}; st.hist = {};
    st.batchDraws = zeroPools(); st.histDraws = zeroPools();
    save();
    renderAll();
  });

  // 第二頁
  /** 套用解析結果（貼上解析與官網更新共用）；回傳說明文字，沒有道具時回傳 null */
  function applyPools(parsed) {
    const got = C.POOLS.filter((p) => parsed[p] && parsed[p].length);
    if (!got.length) return null;
    for (const p of got) st.pools[p] = parsed[p];
    if (!got.includes(st.tab)) st.tab = got[0];
    save();
    renderAll();
    return got.map((p) => `${C.POOL_NAME[p]} ${parsed[p].length} 項`).join("、");
  }
  $("#gParse").addEventListener("click", () => {
    const msg = applyPools(C.parse($("#gPaste").value));
    if (!msg) { $("#gParseResult").textContent = "沒有解析到任何「名稱 + 機率%」的道具"; return; }
    $("#gParseResult").textContent = `已套用：${msg}`;
    toast(`已套用：${msg}`);
  });
  OfficialPools.register("gacha", { card: $("#gPaste").closest(".card"), parse: C.parse, apply: applyPools });
  $("#gJunk").addEventListener("input", (e) => { st.junk = e.target.value; save(); renderJunkMatch(); });
  $("#gMarker").addEventListener("input", (e) => { st.jackpotMarker = e.target.value.trim(); save(); renderMarkerInfo(); renderPool(); renderRight(); });
  $("#gJunkReset").addEventListener("click", async () => {
    if (!(await confirmBox("確定要把垃圾道具清單恢復成預設內容嗎？"))) return;
    st.junk = DEFAULT_JUNK;
    save();
    $("#gJunk").value = st.junk;
    renderJunkMatch();
  });

  renderAll();
  window.Gacha = { onShow: renderAll };
})();
