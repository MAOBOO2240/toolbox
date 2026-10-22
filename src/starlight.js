"use strict";

/* ==========================================================
 * 星光錦囊（套娃抽獎）
 * - 星光錦囊：花錢開，可能開到「玲瓏星光」
 * - 4 個玲瓏星光合成 1 個「星光結晶體」
 * - 星光結晶體 → 50% 星光原石 → 50% 星光水晶 → 50% 璀璨星光，其餘是隨機道具
 * - 每一層都可以自己決定要不要打開
 * - StarCore：解析、抽樣、期望值、自動價格、評語（可獨立測試）
 * ========================================================== */

const StarCore = (() => {
  const LEVELS = ["bag", "crystal", "ore", "water", "bright"];
  const LEVEL_NAME = { bag: "星光錦囊", crystal: "星光結晶體", ore: "星光原石", water: "星光水晶", bright: "璀璨星光" };
  const SEED = "玲瓏星光"; // 4 個合成 1 個星光結晶體
  const SYNTH = 4;
  // 可以打開的道具 → 打開後抽哪一層
  const OPENS = { 星光結晶體: "crystal", 星光原石: "ore", 星光水晶: "water", 璀璨星光: "bright" };
  const BOXES = [SEED, ...Object.keys(OPENS)];
  const FILLER = "請檢查道具機率";
  const isBox = (name) => BOXES.includes(name);

  /**
   * 解析官網的機率表：每行「名稱 機率%」。
   * 只有名稱、沒有機率的行是標題（例如「星光原石　機率」），之後的道具放進那一層。
   * 「玲瓏星光」標題底下的表是星光結晶體的內容。
   */
  function parse(text) {
    const out = { bag: [], crystal: [], ore: [], water: [], bright: [] };
    let pool = "bag";
    for (const raw of String(text).split(/\r?\n/)) {
      const line = raw.replace(/　/g, " ").trim();
      if (!line) continue;
      const m = /^(.+?)\s+(\d+(?:\.\d+)?)\s*%$/.exec(line);
      if (m) {
        const name = m[1].trim();
        const prob = parseFloat(m[2]);
        if (name && prob > 0) out[pool].push({ name, prob });
        continue;
      }
      const h = line.replace(/\s*機率\s*$/, "");
      if (h.includes("星光結晶體") || h.includes(SEED)) pool = "crystal";
      else if (h.includes("星光原石")) pool = "ore";
      else if (h.includes("星光水晶")) pool = "water";
      else if (h.includes("璀璨星光")) pool = "bright";
      else if (h.includes("星光錦囊")) pool = "bag";
    }
    return boxFirst(out);
  }

  /** 每一層裡，可以打開的道具（玲瓏星光、星光原石…）排在最前面 */
  function boxFirst(pools) {
    const out = {};
    for (const lv of Object.keys(pools)) {
      const list = pools[lv] || [];
      out[lv] = [...list.filter((it) => isBox(it.name)), ...list.filter((it) => !isBox(it.name))];
    }
    return out;
  }

  const sum = (items) => items.reduce((a, it) => a + it.prob, 0);
  function withFiller(items) {
    const total = sum(items);
    if (items.length && total < 100 - 1e-9) return [...items, { name: FILLER, prob: +(100 - total).toFixed(6), filler: true }];
    return items;
  }
  const totalOf = (list) => Math.max(100, sum(list));

  /** 依機率權重抽樣；總和超過 100% 時依比例換算 */
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

  /** 這一層開到某個道具的機率 */
  function probIn(pools, lv, name) {
    const list = withFiller(pools[lv] || []);
    const t = totalOf(list);
    return list.filter((it) => it.name === name).reduce((a, it) => a + it.prob / t, 0);
  }

  /**
   * 由最後一層往前算：
   * - ev[層]：打開一次的期望值
   * - eff[道具]：實際價值；可以打開的道具取「留著的價值」與「打開的期望值」較高者，
   *   玲瓏星光是星光結晶體價值的 1/4
   */
  function evaluate(state) {
    const val = (n) => valueOf(state.values, n);
    const ev = {};
    const eff = {};
    const effOf = (name) => (name in eff ? eff[name] : val(name));
    for (const lv of [...LEVELS].reverse()) {
      const list = withFiller(state.pools[lv] || []);
      const t = totalOf(list);
      ev[lv] = list.reduce((a, it) => a + (it.filler ? 0 : (it.prob / t) * effOf(it.name)), 0);
      const box = Object.keys(OPENS).find((k) => OPENS[k] === lv);
      if (box) eff[box] = Math.max(val(box), ev[lv]);
      if (lv === "crystal") eff[SEED] = Math.max(val(SEED), eff[LEVEL_NAME.crystal] / SYNTH);
    }
    return { ev, eff, effOf };
  }

  function analyze(state) {
    const { ev, eff, effOf } = evaluate(state);
    const list = withFiller(state.pools.bag);
    const t = totalOf(list);
    const cost = state.cost;
    const win = cost > 0 ? list.filter((it) => !it.filler && effOf(it.name) > cost).reduce((a, it) => a + it.prob / t, 0) : 0;
    // 一個星光結晶體一路開到璀璨星光的機率
    const chain = probIn(state.pools, "crystal", "星光原石") * probIn(state.pools, "ore", "星光水晶") * probIn(state.pools, "water", "璀璨星光");
    const hasValues = Object.values(state.values).some((v) => typeof v === "number" && v > 0);
    return {
      cost, ev: ev.bag, cp: cost > 0 ? ev.bag / cost : null, win, chain, hasValues,
      seed: eff[SEED] || 0, crystal: eff[LEVEL_NAME.crystal] || 0, seedRate: probIn(state.pools, "bag", SEED), evs: ev, eff,
    };
  }

  /** 取兩位有效數字，讓數字好讀 */
  function nice(n) {
    if (!(n > 0)) return 0;
    const mag = Math.pow(10, Math.floor(Math.log10(n)) - 1);
    return Math.max(1, Math.round(n / mag) * mag);
  }

  /** 每個錦囊平均會開到各層幾次 */
  function perBag(pools) {
    const per = { bag: 1 };
    per.crystal = probIn(pools, "bag", SEED) / SYNTH;
    per.ore = per.crystal * probIn(pools, "crystal", "星光原石");
    per.water = per.ore * probIn(pools, "ore", "星光水晶");
    per.bright = per.water * probIn(pools, "water", "璀璨星光");
    return per;
  }

  /**
   * 自動帶入價格：
   * - 權重 = (1 / 每個錦囊拿到這個道具的機率)^0.8（越稀有越貴；同名道具各層機率加總）
   * - 可以打開的道具留空（價值就是打開的期望值）
   * - 整體縮放成「每個錦囊期望值 ≈ 每個錦囊價格」
   */
  function autoPrice(state) {
    const per = perBag(state.pools);
    const pe = {};
    for (const lv of LEVELS) {
      const list = withFiller(state.pools[lv] || []);
      const t = totalOf(list);
      for (const it of list) {
        if (it.filler || isBox(it.name)) continue;
        pe[it.name] = (pe[it.name] || 0) + (per[lv] * it.prob) / t;
      }
    }
    const weight = {};
    for (const n in pe) if (pe[n] > 0) weight[n] = Math.pow(1 / pe[n], 0.8);
    const valuesAt = (k) => { const v = {}; for (const n in weight) v[n] = weight[n] * k; return v; };
    const evAt = (k) => analyze({ ...state, values: valuesAt(k) }).ev;
    let lo = 0, hi = 1;
    while (evAt(hi) < state.cost && hi < 1e15) hi *= 4;
    for (let i = 0; i < 60; i++) { const mid = (lo + hi) / 2; if (evAt(mid) < state.cost) lo = mid; else hi = mid; }
    const k = (lo + hi) / 2;
    const values = {};
    for (const n in weight) values[n] = nice(weight[n] * k);
    for (const b of BOXES) values[b] = null;
    return values;
  }

  const pct = (x, d = 1) => `${(x * 100).toFixed(d)}%`;
  function comment(a, fmt) {
    if (!(a.cost > 0)) return ["請先輸入星光錦囊的「每個價格」，才能計算回收率與賺錢機率。"];
    const lines = [];
    if (a.cp >= 1.05) lines.push(`期望回收 ${a.cp.toFixed(2)} 倍，長期開是划算的；`);
    else if (a.cp >= 0.95) lines.push(`期望回收 ${a.cp.toFixed(2)} 倍，大致打平，結果主要看運氣；`);
    else lines.push(`期望回收 ${a.cp.toFixed(2)} 倍，每個平均虧 ${fmt(Math.max(0, a.cost - a.ev))}；`);
    lines.push(a.win > 0 ? `單抽超過成本的機率 ${pct(a.win)}。` : "幾乎不會有單抽超過成本的結果。");
    if (a.seed > 0) lines.push(`一個玲瓏星光平均價值 ${fmt(a.seed)}（${SYNTH} 個合成一個星光結晶體，平均 ${fmt(a.crystal)}）；`);
    if (a.chain > 0) lines.push(`星光結晶體一路開到璀璨星光的機率 ${pct(a.chain)}，大約每 ${Math.max(1, Math.round(1 / a.chain))} 個一次。`);
    return lines;
  }

  return { LEVELS, LEVEL_NAME, SEED, SYNTH, OPENS, BOXES, FILLER, isBox, parse, boxFirst, sum, withFiller, sampler, valueOf, probIn, evaluate, analyze, perBag, autoPrice, comment };
})();

if (typeof module !== "undefined") module.exports = StarCore;

/* ==========================================================
 * 介面
 * ========================================================== */
if (typeof document !== "undefined") (() => {
  const C = StarCore;
  const KEY = "starlight-v1";
  const DRAW_CAP = 2_000_000;

  const DEFAULT_TEXT = `靈魂艾爾達碎片交換券(30個)	9.00%
靈魂艾爾達	7.18%
暗黑輪迴星火	21.00%
特別附加潛在能力賦予卷軸	7.80%
傳說潛在能力卷軸50%	0.85%
傳說潛在能力卷軸100%	0.55%
星力14星強化券	18.00%
星力15星強化券	11.00%
星力16星強化券	7.50%
星力17星強化券	3.50%
星力18星強化券	1.51%
星力19星強化券	0.67%
星力20星強化券	0.48%
突破1星強化券100%(21星)	0.53%
突破1星強化券100%(22星)	0.22%
追加1星強化券30%(23星)	0.21%
玲瓏星光	10.00%

玲瓏星光
星光結晶體	機率
星力18星強化券	18.00%
星力19星強化券	12.00%
星力20星強化券	6.00%
突破1星強化券30%(23星)	10.00%
突破1星強化券50%(23星)	4.00%
星光原石	50.00%

星光原石	機率
星力19星強化券	10.00%
星力20星強化券	8.00%
星力21星強化券	2.00%
突破1星強化券30%(23星)	8.00%
突破1星強化券50%(23星)	6.00%
突破1星強化券100%(23星)	5.00%
突破1星強化券30%(24星)	7.00%
突破1星強化券50%(24星)	4.00%
星光水晶	50.00%

星光水晶	機率
突破1星強化券50%(23星)	20.00%
突破1星強化券100%(23星)	15.00%
突破1星強化券30%(24星)	8.00%
突破1星強化券50%(24星)	4.00%
突破1星強化券100%(24星)	2.00%
突破1星強化券30%(25星)	0.70%
突破1星強化券50%(25星)	0.30%
璀璨星光	50.00%

璀璨星光	機率
突破1星強化券30%(24星)	29.00%
突破1星強化券50%(24星)	19.00%
突破1星強化券100%(24星)	14.00%
突破1星強化券30%(25星)	20.00%
突破1星強化券50%(25星)	9.00%
突破1星強化券100%(25星)	4.00%
突破1星強化券30%(26星)	3.00%
突破1星強化券50%(26星)	2.00%`;

  const zeroLv = () => ({ bag: 0, crystal: 0, ore: 0, water: 0, bright: 0 });
  const blankStats = () => ({ spent: 0, bags: 0, synth: 0, opened: zeroLv() });
  function defaults() {
    return {
      pools: C.parse(DEFAULT_TEXT), values: {}, cost: 0, tab: "bag", page: "sim", autoOpen: false,
      stats: blankStats(),
      batch: {}, batchDraws: zeroLv(), // 本次獲得
      hist: {}, histDraws: zeroLv(), // 累計（算實際機率）
      owned: {}, // 保管箱裡現有的（合成、打開後會減少）
    };
  }
  let st = (() => {
    const d = defaults();
    try {
      const s = JSON.parse(localStorage.getItem(KEY));
      if (s && s.pools) {
        return {
          ...d, ...s,
          pools: { ...zeroLvPools(), ...s.pools },
          stats: { ...blankStats(), ...s.stats, opened: { ...zeroLv(), ...(s.stats || {}).opened } },
          batchDraws: { ...zeroLv(), ...s.batchDraws }, histDraws: { ...zeroLv(), ...s.histDraws },
        };
      }
    } catch (_) {}
    return d;
  })();
  function zeroLvPools() { return { bag: [], crystal: [], ore: [], water: [], bright: [] }; }
  // 舊資料：可以打開的道具移到最前面
  st.pools = C.boxFirst(st.pools);
  let saveTimer;
  const save = () => {
    clearTimeout(saveTimer);
    saveTimer = setTimeout(() => { try { localStorage.setItem(KEY, JSON.stringify(st)); } catch (_) {} }, 200);
  };

  /* ---------- 數字 ---------- */
  const intFmt = new Intl.NumberFormat("zh-TW", { maximumFractionDigits: 0 });
  const decFmt = new Intl.NumberFormat("zh-TW", { maximumFractionDigits: 2 });
  function big(n) {
    const a = Math.abs(n), sign = n < 0 ? "-" : "";
    if (a >= 1e8) return `${sign}${decFmt.format(a / 1e8)}億`;
    if (a >= 1e4) return `${sign}${decFmt.format(a / 1e4)}萬`;
    return `${sign}${a >= 100 ? intFmt.format(a) : decFmt.format(a)}`;
  }
  const parseNum = (s) => { const n = parseFloat(String(s).replace(/[,\s%]/g, "")); return isFinite(n) ? n : null; };

  const key = (lv, name) => `${lv}|${name}`;
  const splitKey = (k) => { const i = k.indexOf("|"); return [k.slice(0, i), k.slice(i + 1)]; };
  const inPool = (lv, name) => (st.pools[lv] || []).some((it) => it.name === name);
  const TAB_CLASS = { bag: "c-purple", crystal: "c-amber", ore: "c-orange", water: "c-blue", bright: "c-pink" };

  /* ---------- 分頁 ---------- */
  function renderPage() {
    $$("#sPages button").forEach((b) => b.classList.toggle("on", b.dataset.page === st.page));
    $("#sPageSim").hidden = st.page !== "sim";
    $("#sPageImport").hidden = st.page !== "import";
    window.refreshBack?.();
  }

  /* ---------- 獎池 ---------- */
  /** 分頁下面顯示保管箱裡有幾抽可以開；有得開的分頁會亮起來 */
  function renderTabs() {
    $$("#sTabs button").forEach((b) => {
      const lv = b.dataset.lv;
      b.classList.toggle("on", lv === st.tab);
      const small = b.querySelector("small");
      if (lv === "bag") { small.textContent = "花錢抽"; b.classList.remove("ready", "empty"); return; }
      const n = availOf(lv);
      small.textContent = `可以抽 ${intFmt.format(n)} 個`;
      b.classList.toggle("ready", n > 0);
      b.classList.toggle("empty", n === 0);
      b.title = lv === "crystal" ? `每 ${C.SYNTH} 個玲瓏星光可以抽一次星光結晶體` : `保管箱裡有 ${n} 個${C.LEVEL_NAME[lv]}`;
    });
  }

  function boxTag(name) {
    if (!C.isBox(name)) return null;
    const t = document.createElement("small");
    t.className = "s-box-tag";
    t.textContent = name === C.SEED ? `${C.SYNTH} 個合成` : "可開啟";
    return t;
  }

  function renderPool() {
    renderTabs();
    const items = st.pools[st.tab];
    const total = C.sum(items);
    const warn = $("#sWarn");
    warn.hidden = !items.length || Math.abs(total - 100) < 0.05;
    warn.textContent = total > 100
      ? `機率總和 ${decFmt.format(total)}%，模擬時依比例換算。`
      : `機率總和 ${decFmt.format(total)}%，不足的部分以「${C.FILLER}」補足。`;
    const { eff } = C.evaluate(st);
    const box = $("#sTable");
    box.innerHTML = "";
    if (!items.length) { box.innerHTML = '<div class="g-empty">這一層還沒有道具，可以到「解析」貼上機率文字，或在上方新增。</div>'; return; }
    const rareCut = Math.min(...items.map((it) => it.prob));
    items.forEach((it, idx) => {
      const row = document.createElement("div");
      row.className = "g-row s-row" + (C.isBox(it.name) ? " box" : "") + (it.prob === rareCut && !C.isBox(it.name) ? " jackpot" : "");
      const drawBtn = document.createElement("button");
      drawBtn.className = "g-draw";
      drawBtn.textContent = "抽";
      drawBtn.title = st.tab === "bag" && !C.isBox(it.name)
        ? "一直開星光錦囊到這個道具為止"
        : "一直開星光錦囊，並把開到的星光一路合成、開啟，直到拿到這個道具";
      drawBtn.addEventListener("click", () => drawUntil(it.name));
      const cell = document.createElement("div");
      cell.className = "g-namecell";
      const nm = document.createElement("span");
      nm.className = "s-name";
      nm.textContent = it.name;
      makeCopyable(nm, it.name);
      cell.append(ItemIcons.el(it.name), nm);
      const tag = boxTag(it.name);
      if (tag) cell.append(tag);
      const prob = document.createElement("span");
      prob.className = "r g-prob";
      prob.textContent = `${decFmt.format(it.prob)}%`;
      const val = document.createElement("input");
      val.type = "text";
      val.inputMode = "decimal";
      val.className = "g-val";
      // 可以打開的道具：沒填價值時顯示打開的期望值
      val.placeholder = C.isBox(it.name) && eff[it.name] > 0 ? `≈${big(eff[it.name])}` : "—";
      const v = st.values[it.name];
      val.value = typeof v === "number" ? intFmt.format(v) : "";
      val.addEventListener("focus", () => { const x = st.values[it.name]; val.value = typeof x === "number" ? String(x) : ""; val.select(); });
      val.addEventListener("input", () => { st.values[it.name] = val.value.trim() === "" ? null : parseNum(val.value); save(); renderRight(); renderPriceBtn(); });
      val.addEventListener("blur", () => { const x = st.values[it.name]; val.value = typeof x === "number" ? intFmt.format(x) : ""; renderPool(); });
      const del = document.createElement("button");
      del.className = "g-del";
      del.textContent = "✕";
      del.title = "移除這個道具";
      del.addEventListener("click", () => { st.pools[st.tab].splice(idx, 1); pickers = null; save(); renderPool(); renderRight(); });
      row.append(drawBtn, cell, prob, val, del);
      box.appendChild(row);
    });
  }

  /* ---------- 統計、分析 ---------- */
  const ownedOf = (name) => Object.entries(st.owned).reduce((a, [k, n]) => a + (splitKey(k)[1] === name ? n : 0), 0);
  /** 這一層保管箱裡還能開幾次 */
  const availOf = (lv) => (lv === "crystal" ? ownedOf(C.LEVEL_NAME.crystal) + Math.floor(ownedOf(C.SEED) / C.SYNTH) : ownedOf(C.LEVEL_NAME[lv]));
  function totalValue() {
    const { effOf } = C.evaluate(st);
    let v = 0;
    for (const [k, n] of Object.entries(st.owned)) v += effOf(splitKey(k)[1]) * n;
    return v;
  }

  function renderStats() {
    const s = st.stats;
    const value = totalValue();
    const profit = value - s.spent;
    $("#slSpent").textContent = big(s.spent);
    $("#slValue").textContent = big(value);
    const pEl = $("#slProfit");
    pEl.textContent = (profit > 0 ? "+" : "") + big(profit);
    pEl.className = profit > 0 ? "pos" : profit < 0 ? "neg" : "";
    $("#sBags").textContent = intFmt.format(s.bags);
    $("#sSeeds").textContent = intFmt.format(ownedOf(C.SEED));
    $("#sAutoOpen").classList.toggle("on", !!st.autoOpen);
    // 抽獎按鈕抽的是左邊選到的那一層
    const lv = st.tab;
    const avail = lv === "bag" ? Infinity : availOf(lv);
    $$("[data-sdraw]").forEach((b) => (b.disabled = !avail));
    $("#sDrawInfo").textContent = lv === "bag"
      ? "目前抽：星光錦囊"
      : avail ? `目前抽：${C.LEVEL_NAME[lv]}（剩 ${intFmt.format(avail)} 抽）` : `沒有可以抽的${C.LEVEL_NAME[lv]}`;
    renderTabs();
  }

  const metric = (label, value, cls = "") => `<div class="g-metric"><span>${label}</span><b class="${cls}">${value}</b></div>`;
  function renderAnalysis() {
    const a = C.analyze(st);
    const ok = st.cost > 0;
    let html = "";
    html += metric("每個期望值", ok || a.hasValues ? big(a.ev) : "—");
    html += metric("CP 值", a.cp === null ? "—" : `${a.cp.toFixed(2)} 倍`, a.cp === null ? "" : a.cp >= 1 ? "pos" : "neg");
    html += metric("單抽賺錢機率", ok ? `${(a.win * 100).toFixed(1)}%` : "—");
    html += metric("玲瓏星光價值", a.seed > 0 ? big(a.seed) : "—");
    $("#sMetrics").innerHTML = html;
    const c = $("#sComment");
    c.innerHTML = "<b>AI 評語</b>";
    c.append(document.createTextNode(C.comment(a, big).join("")));
  }

  /* ---------- 本次獲得、保管箱 ---------- */
  function rankOf(k) {
    const [lv, name] = splitKey(k);
    const li = C.LEVELS.indexOf(lv);
    const i = (st.pools[lv] || []).findIndex((it) => it.name === name);
    return li * 10000 + (i < 0 ? 9999 : i);
  }
  const sortedKeys = (obj) => Object.keys(obj).filter((k) => obj[k] > 0).sort((a, b) => rankOf(a) - rankOf(b));
  const isRare = (lv, name) => {
    const items = st.pools[lv] || [];
    if (!items.length || C.isBox(name)) return false;
    const it = items.find((x) => x.name === name);
    return !!it && it.prob <= Math.min(...items.filter((x) => !C.isBox(x.name)).map((x) => x.prob)) + 1e-9;
  };

  function renderBatch() {
    const box = $("#sBatch");
    box.innerHTML = "";
    const d = st.batchDraws;
    const parts = C.LEVELS.filter((lv) => d[lv]).map((lv) => `${C.LEVEL_NAME[lv]} ${intFmt.format(d[lv])}`);
    $("#sBatchBadge").textContent = parts.join("・");
    const keys = sortedKeys(st.batch);
    if (!keys.length) { box.innerHTML = '<span class="g-chip-empty">按下抽獎後，這裡會顯示這次的結果</span>'; return; }
    for (const k of keys) {
      const [lv, name] = splitKey(k);
      const chip = document.createElement("span");
      chip.className = "g-chip s-chip" + (C.isBox(name) ? " box" : "") + (isRare(lv, name) ? " rare" : "");
      chip.append(ItemIcons.el(name), document.createTextNode(name));
      const n = document.createElement("b");
      n.textContent = `×${st.batch[k]}`;
      chip.append(n);
      box.appendChild(chip);
    }
  }

  function renderHistory() {
    const box = $("#sHistory");
    box.innerHTML = "";
    const { effOf } = C.evaluate(st);
    const drawn = { ...st.hist };
    for (const [k, n] of Object.entries(st.batch)) drawn[k] = (drawn[k] || 0) + n;
    const shown = { ...drawn };
    for (const [k, n] of Object.entries(st.owned)) if (n > 0) shown[k] = Math.max(shown[k] || 0, 1);
    const keys = sortedKeys(shown);
    if (!keys.length) { box.innerHTML = '<div class="g-empty">保管箱還是空的</div>'; return; }
    let lastLv = null;
    for (const k of keys) {
      const [lv, name] = splitKey(k);
      const draws = (st.histDraws[lv] || 0) + (st.batchDraws[lv] || 0);
      if (lv !== lastLv) {
        lastLv = lv;
        const g = document.createElement("div");
        g.className = "g-group";
        g.textContent = `${C.LEVEL_NAME[lv]}・開了 ${intFmt.format(draws)} 個`;
        box.appendChild(g);
      }
      const owned = st.owned[k] || 0;
      const fromPool = inPool(lv, name);
      const actual = fromPool && draws ? ((drawn[k] || 0) / draws) * 100 : null;
      const it = (st.pools[lv] || []).find((x) => x.name === name);
      const expect = it ? (it.prob / Math.max(100, C.sum(st.pools[lv]))) * 100 : 0;
      const cls = actual === null || !expect || Math.abs(actual - expect) < 1e-9 ? "same" : actual > expect ? "up" : "down";
      const row = document.createElement("div");
      row.className = "g-hrow s-hrow" + (isRare(lv, name) ? " feat" : "");
      const nm = document.createElement("span");
      nm.className = "g-hname";
      const t = document.createElement("span");
      t.className = "t";
      t.textContent = name;
      nm.append(ItemIcons.el(name), t);
      nm.title = name;
      const cnt = document.createElement("span");
      cnt.className = "c-count" + (owned ? "" : " zero");
      cnt.textContent = `×${intFmt.format(owned)}`;
      row.append(nm, cnt);
      row.insertAdjacentHTML("beforeend", `<span class="muted">${effOf(name) && owned ? big(effOf(name) * owned) : "—"}</span><span class="${cls}">${actual === null ? "—" : `${actual.toFixed(2)}%${cls === "up" ? " ▲" : cls === "down" ? " ▼" : ""}`}</span>`);
      box.appendChild(row);
    }
  }

  function renderRight() {
    renderStats();
    renderAnalysis();
    renderBatch();
    renderHistory();
  }

  function renderAll() {
    renderPage();
    renderPool();
    renderRight();
    renderPriceBtn();
    if (st.page === "import") renderRuleLine();
    if (document.activeElement !== $("#sCost")) $("#sCost").value = st.cost ? intFmt.format(st.cost) : "";
  }

  /* ---------- 模擬 ---------- */
  let pickers = null;
  const pick = (lv) => (pickers ||= {})[lv] ||= C.sampler(st.pools[lv]);
  let got = null; // 「抽到為止」用：這次拿到的道具名稱

  function beginBatch() {
    for (const [k, n] of Object.entries(st.batch)) st.hist[k] = (st.hist[k] || 0) + n;
    for (const lv of C.LEVELS) st.histDraws[lv] += st.batchDraws[lv];
    st.batch = {};
    st.batchDraws = zeroLv();
    pickers = null;
  }
  function gain(lv, it) {
    const k = key(lv, it.name);
    st.batch[k] = (st.batch[k] || 0) + 1;
    if (it.filler) return;
    st.owned[k] = (st.owned[k] || 0) + 1;
    got?.add(it.name);
  }
  /** 從保管箱拿掉 n 個某道具（不分是哪一層開到的） */
  function take(name, n) {
    for (const k of Object.keys(st.owned)) {
      if (n <= 0) break;
      if (splitKey(k)[1] !== name) continue;
      const d = Math.min(n, st.owned[k]);
      st.owned[k] -= d;
      n -= d;
      if (st.owned[k] <= 0) delete st.owned[k];
    }
  }
  function openLevel(lv) {
    st.batchDraws[lv] += 1;
    if (lv !== "bag") st.stats.opened[lv] += 1;
    gain(lv, pick(lv)());
  }
  function buyBag() {
    st.stats.spent += st.cost || 0;
    st.stats.bags += 1;
    openLevel("bag");
  }
  /** 開一次某一層：星光結晶體用掉 4 個玲瓏星光（舊版合成好的星光結晶體優先用），其他層用掉 1 個該道具 */
  function openBox(lv) {
    if (lv === "crystal") {
      if (ownedOf(C.LEVEL_NAME.crystal) > 0) take(C.LEVEL_NAME.crystal, 1);
      else take(C.SEED, C.SYNTH);
      st.stats.synth += 1;
    } else take(C.LEVEL_NAME[lv], 1);
    openLevel(lv);
  }
  /** 保管箱裡的星光全部一路開到底（「抽到自動開到底」、「抽」到指定道具時用） */
  function openEverything() {
    let n = 0;
    for (let guard = 0; guard < 1e6; guard++) {
      const lv = C.LEVELS.slice(1).find((x) => availOf(x) > 0 && st.pools[x].length);
      if (!lv) break;
      openBox(lv);
      n++;
    }
    return n;
  }
  function commit(msg) {
    save();
    renderRight();
    if (msg) toast(msg);
  }

  /** 抽左邊選到的那一層：星光錦囊花錢；其他層用保管箱裡的星光 */
  function drawN(n, fresh) {
    const lv = st.tab;
    if (!st.pools[lv].length) { toast(`${C.LEVEL_NAME[lv]}的獎池是空的`); return false; }
    if (lv !== "bag") {
      const avail = availOf(lv);
      if (!avail) { toast(lv === "crystal" ? `玲瓏星光不足（每 ${C.SYNTH} 個抽一次）` : `保管箱沒有${C.LEVEL_NAME[lv]}`); return false; }
      n = Math.min(n, avail);
    }
    if (fresh) beginBatch();
    for (let i = 0; i < n; i++) {
      if (lv === "bag") buyBag(); else openBox(lv);
      if (st.autoOpen) openEverything();
    }
    commit();
    if (lv !== "bag" && !availOf(lv)) return false; // 抽完了，按住也停下來
  }
  function drawUntil(name) {
    if (!st.pools.bag.length) return toast("星光錦囊的獎池是空的");
    // 錦囊本身就有的道具只開錦囊；更深的道具要一路合成、打開
    const deep = !(inPool("bag", name)) || C.isBox(name) && name !== C.SEED;
    const reachable = C.LEVELS.some((lv) => inPool(lv, name)) || name === C.LEVEL_NAME.crystal;
    if (!reachable) return toast("獎池裡沒有這個道具");
    beginBatch();
    got = new Set();
    try {
      for (let i = 1; i <= DRAW_CAP; i++) {
        buyBag();
        if (deep || st.autoOpen) openEverything();
        if (got.has(name)) return commit(`拿到「${name}」，共開了 ${intFmt.format(i)} 個星光錦囊`);
        got.clear();
      }
      commit(`已達 ${intFmt.format(DRAW_CAP)} 個上限仍未拿到`);
    } finally {
      got = null;
    }
  }

  function holdable(btn, fn) {
    let t1 = null, t2 = null;
    const stop = () => { clearTimeout(t1); clearInterval(t2); t1 = t2 = null; };
    btn.addEventListener("pointerdown", (e) => {
      if (e.button !== 0 || btn.disabled) return;
      if (fn(true) === false) return;
      t1 = setTimeout(() => { t2 = setInterval(() => { if (btn.disabled || fn(true) === false) stop(); }, 110); }, 380);
    });
    ["pointerup", "pointerleave", "pointercancel"].forEach((ev) => btn.addEventListener(ev, stop));
  }

  /* ---------- 自動帶入價格 ---------- */
  const hasAnyValue = () => Object.values(st.values).some((v) => typeof v === "number" && isFinite(v));
  function renderPriceBtn() {
    const btn = $("#sAutoPrice");
    const any = hasAnyValue();
    btn.textContent = any ? "移除全部價格" : "自動帶入價格";
    btn.classList.toggle("c-teal", !any);
    btn.classList.toggle("c-red", any);
    btn.title = any ? "清空所有道具的價值" : "依機率估算基準價值：越稀有越高；星光類道具留空（價值就是打開的期望值）";
  }

  /** 解析頁：合成規則一行小字 */
  function renderRuleLine() {
    const box = $("#sRuleLine");
    box.innerHTML = "";
    const add = (...nodes) => { const span = document.createElement("span"); span.className = "c-rule"; span.append(...nodes); box.append(span); };
    const txt = (t) => document.createTextNode(t);
    const p = (lv, n) => `${decFmt.format(C.probIn(st.pools, lv, n) * 100)}%`;
    add(ItemIcons.el(C.SEED), txt(`玲瓏星光 ×${C.SYNTH} → `), ItemIcons.el("星光結晶體"), txt("星光結晶體"));
    add(txt(`${p("crystal", "星光原石")} → `), ItemIcons.el("星光原石"), txt("星光原石"));
    add(txt(`${p("ore", "星光水晶")} → `), ItemIcons.el("星光水晶"), txt("星光水晶"));
    add(txt(`${p("water", "璀璨星光")} → `), ItemIcons.el("璀璨星光"), txt("璀璨星光"));
  }

  /* ---------- 事件 ---------- */
  $$("#sPages button").forEach((b) => b.addEventListener("click", () => { st.page = b.dataset.page; save(); renderAll(); }));
  if (window.BackHooks) window.BackHooks["view-star"] = {
    label: () => (st.page !== "sim" ? "上一頁" : null),
    back: () => { if (st.page === "sim") return false; st.page = "sim"; save(); renderAll(); return true; },
  };
  $$("#sTabs button").forEach((b) => {
    b.prepend(ItemIcons.el(C.LEVEL_NAME[b.dataset.lv]));
    b.addEventListener("click", () => { st.tab = b.dataset.lv; save(); renderPool(); renderStats(); });
  });
  $("#sCost").addEventListener("focus", (e) => { e.target.value = st.cost ? String(st.cost) : ""; e.target.select(); });
  $("#sCost").addEventListener("input", (e) => { st.cost = Math.max(0, parseNum(e.target.value) || 0); save(); renderRight(); });
  $("#sCost").addEventListener("blur", (e) => { e.target.value = st.cost ? intFmt.format(st.cost) : ""; });
  $("#sAutoPrice").addEventListener("click", async () => {
    if (hasAnyValue()) {
      if (!(await confirmBox("確定要移除所有道具的價格嗎？"))) return;
      st.values = {};
      save(); renderAll();
      return;
    }
    if (!(st.cost > 0)) return toast("請先輸入星光錦囊每個價格，自動價格以它為基準");
    st.values = C.autoPrice(st);
    save(); renderAll();
    toast("已帶入基準價格（星光類道具留空），可再依實際行情調整");
  });

  $("#sAdd").addEventListener("click", () => {
    const name = $("#sNewName").value.trim();
    const prob = parseNum($("#sNewProb").value);
    if (!name || !(prob > 0)) return toast("請輸入道具名稱與機率");
    st.pools[st.tab].push({ name, prob });
    const v = parseNum($("#sNewValue").value);
    if (v !== null) st.values[name] = v;
    $("#sNewName").value = $("#sNewProb").value = $("#sNewValue").value = "";
    pickers = null;
    save(); renderPool(); renderRight();
    $("#sNewName").focus();
  });
  ["#sNewName", "#sNewProb", "#sNewValue"].forEach((id) => $(id).addEventListener("keydown", (e) => { if (e.key === "Enter") $("#sAdd").click(); }));

  $("#sClearItems").addEventListener("click", async () => {
    if (!C.LEVELS.some((lv) => st.pools[lv].length)) return;
    if (!(await confirmBox("確定要清除五層獎池的所有道具嗎？"))) return;
    for (const lv of C.LEVELS) st.pools[lv] = [];
    pickers = null;
    save(); renderAll();
  });

  $$("[data-sdraw]").forEach((b) => holdable(b, (fresh) => drawN(Number(b.dataset.sdraw), fresh)));
  $("#sAutoOpen").addEventListener("click", () => {
    st.autoOpen = !st.autoOpen;
    save(); renderStats();
    toast(st.autoOpen ? "抽到的星光會自動一路打開到底" : "抽到的星光會留在保管箱；點左邊亮起來的分頁再按抽獎就能打開");
  });
  $("#sReset").addEventListener("click", async () => {
    if (!(await confirmBox("確定要重置花費、數量與保管箱嗎？"))) return;
    st.stats = blankStats();
    st.batch = {}; st.hist = {}; st.owned = {};
    st.batchDraws = zeroLv(); st.histDraws = zeroLv();
    save(); renderAll();
  });

  // 解析頁
  /** 套用解析結果（貼上解析與官網更新共用） */
  function applyPools(parsed) {
    const got2 = C.LEVELS.filter((lv) => parsed[lv] && parsed[lv].length);
    if (!got2.length) return null;
    for (const lv of got2) st.pools[lv] = parsed[lv];
    if (!got2.includes(st.tab)) st.tab = got2[0];
    pickers = null;
    save(); renderAll();
    return got2.map((lv) => `${C.LEVEL_NAME[lv]} ${parsed[lv].length} 項`).join("、");
  }
  $("#sParse").addEventListener("click", () => {
    const msg = applyPools(C.parse($("#sPaste").value));
    if (!msg) { $("#sParseResult").textContent = "沒有解析到任何「名稱 機率%」的道具"; return; }
    $("#sParseResult").textContent = `已套用：${msg}`;
    toast(`已套用：${msg}`);
  });
  OfficialPools.register("star", { card: $("#sPaste").closest(".card"), parse: C.parse, apply: applyPools });

  renderAll();
  window.Starlight = { onShow: renderAll };
})();
