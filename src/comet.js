"use strict";

/* ==========================================================
 * 閃亮彗星
 * - 閃亮彗星：花錢抽，或用 20 個彗星碎片換一顆
 * - 璀璨彗星：只能用 80 個彗星碎片換
 * - 道具依等級分解成彗星碎片：S 10、A 5、B 3、C 1
 * - CometCore：解析、抽樣、期望值、評語（可獨立測試）
 * ========================================================== */

const CometCore = (() => {
  const POOLS = ["shiny", "bright"];
  const POOL_NAME = { shiny: "閃亮彗星", bright: "璀璨彗星" };
  const GRADES = ["S", "A", "B", "C"];
  const FRAG = { S: 10, A: 5, B: 3, C: 1 }; // 分解獲得的彗星碎片
  const SHINY_FRAG = 20; // 碎片換一顆閃亮彗星
  const BRIGHT_FRAG = 80; // 碎片換一顆璀璨彗星
  const FILLER = "請檢查道具機率";

  /**
   * 解析官網的機率表。每行「等級 名稱 機率%」，以 Tab 或空白分隔。
   * 遇到含「璀璨彗星」的標題行後，之後的道具放進璀璨彗星獎池。
   * 兩件共用一個機率（機率寫在兩件中間那一行）時，兩件平分這個機率（各一半）。
   */
  function parse(text) {
    const out = { shiny: [], bright: [] };
    let pool = "shiny";
    let pending = []; // 還沒有機率的道具
    let partnerProb = null; // 剛剛那一行單獨的機率，下一件沒有機率的道具也用它
    const push = (grade, name, prob) => out[pool].push({ grade, name, prob });
    for (const raw of String(text).split(/\r?\n/)) {
      const line = raw.replace(/　/g, " ").trim();
      if (!line) continue;
      const onlyPct = /^(\d+(?:\.\d+)?)\s*%$/.exec(line);
      if (onlyPct) {
        const p = parseFloat(onlyPct[1]);
        if (pending.length >= 2) {
          // 機率前面已經列出整組：平分
          for (const it of pending) push(it.grade, it.name, +(p / pending.length).toFixed(6));
          partnerProb = null;
        } else {
          // 機率夾在兩件中間：前一件拿一半，下一件拿另一半
          const half = +(p / 2).toFixed(6);
          for (const it of pending) push(it.grade, it.name, half);
          partnerProb = half;
        }
        pending = [];
        continue;
      }
      const m = /^([SABC])\s+(.+?)(?:\s+(\d+(?:\.\d+)?)\s*%)?$/.exec(line);
      if (!m) {
        // 標題列：切換獎池
        if (!/%/.test(line)) {
          if (line.includes("璀璨彗星")) pool = "bright";
          else if (line.includes("閃亮彗星")) pool = "shiny";
        }
        continue;
      }
      const [, grade, name0, p] = m;
      const name = name0.trim();
      if (!name || name === "道具名稱") continue;
      if (p !== undefined) {
        partnerProb = null;
        pending = [];
        const prob = parseFloat(p);
        if (prob > 0) push(grade, name, prob);
      } else if (partnerProb !== null) {
        push(grade, name, partnerProb);
        partnerProb = null;
      } else {
        pending.push({ grade, name });
      }
    }
    return out;
  }

  const sum = (items) => items.reduce((a, it) => a + it.prob, 0);

  function withFiller(items) {
    const total = sum(items);
    if (items.length && total < 100 - 1e-9) return [...items, { grade: "C", name: FILLER, prob: +(100 - total).toFixed(6), filler: true }];
    return items;
  }

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

  /** 一件道具的實際價值：留著的價值與分解成碎片的價值取較高者 */
  const effective = (values, it, fragValue) => (it.filler ? 0 : Math.max(valueOf(values, it.name), FRAG[it.grade] * fragValue));

  function poolEv(items, values, fragValue) {
    const list = withFiller(items);
    const total = Math.max(100, sum(list));
    return list.reduce((a, it) => a + (it.prob / total) * effective(values, it, fragValue), 0);
  }

  /**
   * 碎片價值：20 碎片可換一顆閃亮彗星（= 每顆價格），80 碎片可換一顆璀璨彗星；取較划算者
   */
  function fragmentValue(state) {
    const byShiny = state.cost > 0 ? state.cost / SHINY_FRAG : 0;
    const brightEv = poolEv(state.pools.bright, state.values, byShiny);
    const byBright = brightEv / BRIGHT_FRAG;
    return { value: Math.max(byShiny, byBright), byShiny, byBright, best: byBright > byShiny ? "bright" : "shiny", brightEv };
  }

  function analyze(state, pool) {
    const frag = fragmentValue(state);
    const items = state.pools[pool];
    const list = withFiller(items);
    const total = Math.max(100, sum(list));
    const hasValues = items.some((it) => valueOf(state.values, it.name) > 0) || state.cost > 0;
    const ev = poolEv(items, state.values, frag.value);
    const top = pool === "shiny" ? "A" : "S"; // 獎池最高等級視為大獎
    const topEv = list.filter((it) => it.grade === top && !it.filler).reduce((a, it) => a + (it.prob / total) * effective(state.values, it, frag.value), 0);
    if (pool === "shiny") {
      const cost = state.cost;
      const win = cost > 0 ? list.filter((it) => effective(state.values, it, frag.value) > cost).reduce((a, it) => a + it.prob / total, 0) : 0;
      return { pool, hasValues, cost, ev, cp: cost > 0 ? ev / cost : null, win, frag, rare: ev > 0 ? topEv / ev : 0 };
    }
    const above = list.filter((it) => ev > 0 && effective(state.values, it, frag.value) >= ev).reduce((a, it) => a + it.prob / total, 0);
    return { pool, hasValues, ev, perFrag: ev / BRIGHT_FRAG, frag, above, rare: ev > 0 ? topEv / ev : 0, shinyEq: state.cost > 0 ? ev / state.cost : null };
  }

  /** 取兩位有效數字，讓數字好讀 */
  function nice(n) {
    if (!(n > 0)) return 0;
    const mag = Math.pow(10, Math.floor(Math.log10(n)) - 1);
    return Math.max(1, Math.round(n / mag) * mag);
  }

  /**
   * 自動帶入價格：
   * - 會自動分解的道具 = 垃圾，留空（價值就是分解出的碎片）
   * - 其他道具：權重 = (1 / 每抽獲得機率)^0.8 × 等級係數（越稀有、等級越高越貴）；
   *   璀璨彗星的道具依「每抽閃亮彗星平均能換到幾顆璀璨彗星」折算機率
   * - 整體縮放成「閃亮彗星每抽期望值 ≈ 每顆價格」
   */
  function autoPrice(state, isJunk) {
    const { pools, cost } = state;
    const tot = (p) => Math.max(100, sum(withFiller(pools[p])));
    // 每抽平均拿到的碎片：有設定自動分解就只算那些；完全沒設定時，假設全部都會分解
    const anyJunk = pools.shiny.some((it) => isJunk("shiny", it));
    const fragPerDraw = pools.shiny.reduce((a, it) => a + (!anyJunk || isJunk("shiny", it) ? (it.prob / tot("shiny")) * FRAG[it.grade] : 0), 0);
    const brightPerDraw = Math.max(fragPerDraw / BRIGHT_FRAG, 1e-4);
    const weight = {};
    for (const pool of POOLS) {
      for (const it of pools[pool]) {
        if (isJunk(pool, it)) continue;
        const pe = (it.prob / tot(pool)) * (pool === "bright" ? brightPerDraw : 1);
        // 等級越高越值錢：同樣稀有時 S > A > B > C
        const tier = { S: 2, A: 1.4, B: 1, C: 0.7 }[it.grade] || 1;
        if (pe > 0) weight[it.name] = Math.max(weight[it.name] || 0, Math.pow(1 / pe, 0.8) * tier);
      }
    }
    const valuesAt = (k) => { const v = {}; for (const n in weight) v[n] = weight[n] * k; return v; };
    const evAt = (k) => analyze({ ...state, values: valuesAt(k) }, "shiny").ev;
    let k = 0;
    const base = evAt(0);
    if (cost > base) {
      let lo = 0, hi = 1;
      while (evAt(hi) < cost && hi < 1e15) hi *= 4;
      for (let i = 0; i < 60; i++) { const mid = (lo + hi) / 2; if (evAt(mid) < cost) lo = mid; else hi = mid; }
      k = (lo + hi) / 2;
    } else {
      // 光是碎片就超過成本時，道具仍給一個合理的基準價值
      let hi = 1;
      while (evAt(hi) < base * 1.5 && hi < 1e15) hi *= 4;
      k = hi;
    }
    // 留著的道具，價格至少要等於分解成碎片的價值（比這低的話直接分解就好）
    const fragFloor = cost > 0 ? cost / SHINY_FRAG : 0;
    const values = {};
    for (const pool of POOLS) {
      for (const it of pools[pool]) {
        if (!(it.name in weight)) { values[it.name] = values[it.name] ?? null; continue; }
        values[it.name] = Math.max(values[it.name] || 0, nice(Math.max(weight[it.name] * k, FRAG[it.grade] * fragFloor)));
      }
    }
    return values;
  }

  const pct = (x, d = 1) => `${(x * 100).toFixed(d)}%`;

  function comment(a, fmt) {
    if (a.pool === "shiny") {
      if (!(a.cost > 0)) return ["請先輸入閃亮彗星的「每顆價格」，才能計算碎片價值、回收率與賺錢機率。"];
      const lines = [];
      if (a.cp >= 1.05) lines.push(`期望回收 ${a.cp.toFixed(2)} 倍，長期抽是划算的；`);
      else if (a.cp >= 0.95) lines.push(`期望回收 ${a.cp.toFixed(2)} 倍，大致打平，結果主要看運氣；`);
      else lines.push(`期望回收 ${a.cp.toFixed(2)} 倍，每抽平均虧 ${fmt(Math.max(0, a.cost - a.ev))}；`);
      lines.push(a.win > 0 ? `單抽超過成本的機率 ${pct(a.win)}，大約每 ${Math.max(1, Math.round(1 / a.win))} 抽一次。` : "幾乎不會有單抽超過成本的結果。");
      if (a.frag.best === "bright" && a.frag.byBright > 0) lines.push(`碎片拿去換璀璨彗星比較划算（每碎片 ${fmt(a.frag.byBright)}）。`);
      else lines.push(`碎片換回閃亮彗星較划算（每碎片 ${fmt(a.frag.byShiny)}）。`);
      return lines;
    }
    if (!(a.ev > 0)) return ["請先輸入閃亮彗星的每顆價格或道具價值，才能評估璀璨彗星。"];
    const lines = [`開一顆璀璨彗星平均價值 ${fmt(a.ev)}，等於每碎片 ${fmt(a.perFrag)}`];
    if (a.shinyEq !== null) lines.push(`（約 ${a.shinyEq.toFixed(2)} 顆閃亮彗星的價格）。`);
    else lines.push("。");
    lines.push(a.frag.best === "bright" ? "以目前價值計算，碎片優先換璀璨彗星。" : "以目前價值計算，碎片換閃亮彗星比較划算。");
    return lines;
  }

  return { POOLS, POOL_NAME, GRADES, FRAG, SHINY_FRAG, BRIGHT_FRAG, FILLER, parse, sum, withFiller, sampler, valueOf, effective, poolEv, fragmentValue, analyze, comment, autoPrice };
})();

if (typeof module !== "undefined") module.exports = CometCore;

/* ==========================================================
 * 介面
 * ========================================================== */
if (typeof document !== "undefined") (() => {
  const C = CometCore;
  const KEY = "comet-v1";
  const DRAW_CAP = 2_000_000;

  const DEFAULT_TEXT = `A	白熊老闆帽	1.10%
A	白熊老闆	1.10%
A	白熊老闆皮鞋	0.80%
A	白熊，請看這裡	0.80%
A	白熊修補迴紋針	0.70%
A	蝴蝶結名牌戒指	1.40%
A	蝴蝶結聊天戒指	1.40%
B	橡樹果實松鼠帽子	1.60%
B	貪吃松鼠帽子	1.60%
B	橡樹果實松鼠套裝	1.40%
B	貪吃松鼠套裝	1.40%
B	橡樹果實松鼠尾巴	1.50%
B	貪吃松鼠尾巴	1.50%
B	露營馬克杯	1.50%
C	星座套裝組帽子(處女座)	2.57%
C	星座套裝組帽子(天秤座)	2.57%
C	星座套裝組帽子(金牛座)	2.57%
C	萬聖節西瓜帽	2.57%
C	藍拳擊手套	2.57%
C	褐色繃帶手	2.57%
C	黑色繃帶手	2.57%
C	白色繃帶手	2.57%
C	黑色人工智能07號套服	2.57%
C	白色人工智能11號套服	2.57%
C	寬版丹寧風(男)	2.57%
C	粉紅畫工服	2.57%
C	朱紅連帽坎肩	2.57%
C	粉紅星長衫	2.57%
C	粉紅冒險服	2.57%
C	粉紅連帽坎肩	2.57%
C	黃格子褲	2.57%
C	郊遊牛仔短褲	2.57%
C	迷彩褲	2.57%
C	繽紛彩虹短褲	2.57%
C	紅色足球鞋	2.57%
C	藍色棒球鞋	2.57%
C	紅色棒球鞋	2.57%
C	黑色棒球鞋	2.57%
C	粉紅蝴蝶結	2.57%
C	黑天使翅膀	2.57%
C	暗黑披風	2.57%
C	老虎尾巴	2.57%
C	韓式沐浴手套	2.57%
C	黑金剛手機	2.57%
C	燈籠	2.57%
C	日本弓	2.57%
璀璨彗星
S	深海月亮蝴蝶結	1.80%
S	深海王子的襯衫	1.74%
S	深海公主的禮服	1.74%
S	深海的彩虹珍珠	1.90%
S	深海的花	1.85%
S	深海的寶石樹木	1.53%
A	喵嗚天使	1.27%
A	霓虹天使	1.27%
A	夢境衝擊
1.22%
A	露娜閃光
A	夢境疾馳
1.31%
A	露娜疾馳
A	影子貓背包	1.31%
A	露米貓背包	1.31%
A	惡夢蹦蹦蹦	1.05%
B	夜之血皇冠	1.36%
B	蚊子騎士長袍	1.36%
B	蚊子騎士禮服	1.39%
B	絲絨之翼斗篷	1.43%
B	致命節奏	1.36%
B	危險步伐	1.36%
B	劇毒尖刺	1.35%
A	閃亮復古	1.27%
A	粉色心願	1.27%
A	重疊的回憶
1.24%
A	綻放泡泡糖
A	純白潮流鞋
1.29%
A	珊瑚紅暖腳靴
A	閃閃星星	1.29%
A	甜蜜星動	1.14%
A	甜蜜星星名牌戒指	1.30%
A	甜蜜星星聊天戒指	1.30%
B	祕術舞會面具	1.41%
B	祕術禮帽	1.41%
B	黃昏的祕術西裝	1.34%
B	黎明的祕術洋裝	1.34%
B	黃昏的祕術皮鞋	1.37%
B	黎明的祕術鞋子	1.37%
B	黃昏與黎明的塔羅牌	1.35%
B	黃昏與黎明之劍	1.30%
A	黃金太陽皇冠	1.28%
A	太陽蝴蝶結皇冠	1.28%
A	向日葵貴族套服	1.24%
A	向日葵女神禮服	1.24%
A	黃金向日葵斗篷	1.29%
A	黃金太陽騎士靴	1.30%
A	太陽女神高跟鞋	1.30%
A	太陽領主長杖	1.24%
B	侍女喵喵髮圈	1.41%
B	主人狗狗髮圈	1.41%
B	喵喵侍女服	1.30%
B	汪汪主人服	1.30%
B	黃金耳釘	1.35%
B	黃金單眼鏡	1.35%
B	喵喵侍女鞋子	1.41%
B	汪汪主人鞋子	1.41%
B	侍女喵喵尾巴	1.35%
B	主人狗狗尾巴	1.35%
B	侍女掃把	1.29%
B	主人茶壺	1.29%
A	華曜月海	1.22%
A	赤夜服裝	1.24%
A	赤月服裝	1.24%
A	抑制的束縛	1.28%
A	赤月塔影	1.28%
A	望月食光	1.24%
A	半妖耳環	1.29%
A	華曜圖紋	1.28%
A	赤夜妖刀	1.23%
B	愛心守護者耳機	1.41%
B	魔法蝴蝶結女巫帽	1.41%
B	玫瑰天使法師禮服	1.39%
B	純淨天使法師長袍	1.39%
B	愛之結界	1.39%
B	愛心蕾絲手套	1.36%
B	魔幻蝴蝶結鞋子	1.39%
B	純淨天使靴子	1.39%
B	愛心守護者短杖	1.30%`;

  const zeroPools = () => ({ shiny: 0, bright: 0 });
  const blankStats = () => ({ spent: 0, draws: 0, bright: 0, frags: 0, split: 0 });
  function defaults() {
    return {
      pools: C.parse(DEFAULT_TEXT), values: {}, cost: 0, tab: "shiny", page: "sim",
      stats: blankStats(),
      batch: {}, batchSplit: {}, batchDraws: zeroPools(), // 本次：抽到的、其中被自動分解的
      hist: {}, histDraws: zeroPools(), // 累計抽到的（算實際機率）
      owned: {}, // 保管箱裡現有的（分解後會減少）
      auto: { grades: {}, items: {}, keep: {} },
    };
  }
  let st = (() => {
    const d = defaults();
    try {
      const s = JSON.parse(localStorage.getItem(KEY));
      if (s && s.pools) return { ...d, ...s, stats: { ...blankStats(), ...s.stats }, batchDraws: { ...zeroPools(), ...s.batchDraws }, histDraws: { ...zeroPools(), ...s.histDraws }, auto: { grades: {}, items: {}, keep: {}, ...s.auto } };
    } catch (_) {}
    return d;
  })();
  // 成對道具改成平分機率：獎池還是預設內容時，重新解析一次
  if (!st.pairHalf) {
    const fresh = C.parse(DEFAULT_TEXT);
    const names = (p) => p.map((it) => it.name).join("|");
    if (C.POOLS.every((k) => names(st.pools[k] || []) === names(fresh[k]))) st.pools = fresh;
    st.pairHalf = true;
  }
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
  const valueOf = (name) => C.valueOf(st.values, name);

  const key = (pool, name) => `${pool}|${name}`;
  const splitKey = (k) => { const i = k.indexOf("|"); return [k.slice(0, i), k.slice(i + 1)]; };
  const itemOf = (pool, name) => st.pools[pool].find((it) => it.name === name);
  const gradeOf = (pool, name) => itemOf(pool, name)?.grade || "C";
  const topGrade = (pool) => (pool === "shiny" ? "A" : "S");
  /** 是否自動分解：個別指定的一定分解；整個等級設定時，個別取消（保留）的不分解 */
  const isAuto = (pool, it) => {
    const k = key(pool, it.name);
    if (st.auto.items[k]) return true;
    return !!(st.auto.grades[it.grade] && !st.auto.keep[k]);
  };

  /* ---------- 分頁 ---------- */
  function renderPage() {
    $$("#cPages button").forEach((b) => b.classList.toggle("on", b.dataset.page === st.page));
    $("#cPageSim").hidden = st.page !== "sim";
    $("#cPageImport").hidden = st.page !== "import";
    $("#cPageAuto").hidden = st.page !== "auto";
    window.refreshBack?.();
  }

  /* ---------- 獎池 ---------- */
  function gradeBadge(g) {
    const b = document.createElement("span");
    b.className = `c-grade g${g}`;
    b.textContent = g;
    return b;
  }

  /** 璀璨彗星分頁：碎片夠開的時候亮起來，下面顯示可以抽幾抽 */
  const brightAvail = () => Math.floor(st.stats.frags / C.BRIGHT_FRAG);
  function renderTabs() {
    $$("#cTabs button").forEach((b) => {
      const p = b.dataset.pool;
      b.classList.toggle("on", p === st.tab);
      const small = b.querySelector("small");
      if (p === "shiny") { small.textContent = "花錢抽"; b.classList.remove("ready", "empty"); return; }
      const n = brightAvail();
      small.textContent = n ? `可抽 ${intFmt.format(n)} 抽` : `需 ${C.BRIGHT_FRAG} 碎片`;
      b.classList.toggle("ready", n > 0);
      b.classList.toggle("empty", n === 0);
      b.title = `每 ${C.BRIGHT_FRAG} 個彗星碎片可以開一顆璀璨彗星`;
    });
  }

  function renderPool() {
    renderTabs();
    const items = st.pools[st.tab];
    const total = C.sum(items);
    const warn = $("#cWarn");
    warn.hidden = !items.length || Math.abs(total - 100) < 0.05;
    warn.textContent = total > 100
      ? `機率總和 ${decFmt.format(total)}%（成對道具各自計算），模擬時依比例換算。`
      : `機率總和 ${decFmt.format(total)}%，不足的部分以「${C.FILLER}」補足。`;
    const box = $("#cTable");
    box.innerHTML = "";
    if (!items.length) { box.innerHTML = '<div class="g-empty">這個獎池還沒有道具，可以到「解析」貼上機率文字，或在上方新增。</div>'; return; }
    items.forEach((it, idx) => {
      const row = document.createElement("div");
      row.className = "g-row c-row" + (it.grade === topGrade(st.tab) ? " jackpot" : "");
      const drawBtn = document.createElement("button");
      drawBtn.className = "g-draw";
      drawBtn.textContent = "抽";
      drawBtn.title = st.tab === "shiny" ? "一直抽到這個道具為止" : "用現有碎片一直開璀璨彗星到這個道具為止";
      drawBtn.addEventListener("click", () => drawUntil(st.tab, it.name));
      const cell = document.createElement("div");
      cell.className = "g-namecell";
      const nm = document.createElement("span");
      nm.className = "c-name";
      nm.textContent = it.name;
      nm.title = `${it.name}（${it.grade} 級・分解得 ${C.FRAG[it.grade]} 碎片）`;
      cell.append(gradeBadge(it.grade), ItemIcons.el(it.name), nm);
      if (isAuto(st.tab, it)) {
        const a = document.createElement("small");
        a.className = "c-auto-tag";
        a.textContent = "自動分解";
        cell.append(a);
      }
      const prob = document.createElement("span");
      prob.className = "r g-prob";
      prob.textContent = `${decFmt.format(it.prob)}%`;
      const val = document.createElement("input");
      val.type = "text";
      val.inputMode = "decimal";
      val.className = "g-val";
      val.placeholder = "—";
      const v = st.values[it.name];
      val.value = typeof v === "number" ? intFmt.format(v) : "";
      val.addEventListener("focus", () => { const x = st.values[it.name]; val.value = typeof x === "number" ? String(x) : ""; val.select(); });
      val.addEventListener("input", () => { st.values[it.name] = val.value.trim() === "" ? null : parseNum(val.value); save(); renderRight(); renderPriceBtn(); });
      val.addEventListener("blur", () => { const x = st.values[it.name]; val.value = typeof x === "number" ? intFmt.format(x) : ""; });
      const del = document.createElement("button");
      del.className = "g-del";
      del.textContent = "✕";
      del.title = "移除這個道具";
      del.addEventListener("click", () => { st.pools[st.tab].splice(idx, 1); save(); renderPool(); renderRight(); });
      row.append(drawBtn, cell, prob, val, del);
      box.appendChild(row);
    });
  }

  /* ---------- 統計、分析、結果 ---------- */
  const fragWorth = () => C.fragmentValue(st).value;
  /** 總價值 = 保管箱裡的道具價值 + 手上碎片的價值 */
  function totalValue() {
    let v = 0;
    for (const [k, n] of Object.entries(st.owned)) v += valueOf(splitKey(k)[1]) * n;
    return v + st.stats.frags * fragWorth();
  }

  function renderStats() {
    const s = st.stats;
    const value = totalValue();
    const profit = value - s.spent;
    $("#cSpent").textContent = big(s.spent);
    $("#cValue").textContent = big(value);
    const pEl = $("#cProfit");
    pEl.textContent = (profit > 0 ? "+" : "") + big(profit);
    pEl.className = profit > 0 ? "pos" : profit < 0 ? "neg" : "";
    $("#cDraws").textContent = s.bright ? `${intFmt.format(s.draws)}＋${intFmt.format(s.bright)}` : intFmt.format(s.draws);
    $("#cFrags").textContent = intFmt.format(s.frags);
    // 抽獎按鈕抽的是左邊選到的獎池：閃亮彗星花錢抽，璀璨彗星用碎片開
    const avail = st.tab === "bright" ? brightAvail() : Infinity;
    $$("[data-cdraw]").forEach((b) => (b.disabled = !avail));
    $("#cDrawInfo").textContent = st.tab === "shiny"
      ? `目前抽：閃亮彗星・${st.keepFrags ? "碎片保留" : `碎片滿 ${C.SHINY_FRAG} 個自動換閃亮彗星`}`
      : avail ? `目前開：璀璨彗星（剩 ${intFmt.format(avail)} 顆，每顆 ${C.BRIGHT_FRAG} 碎片）`
      : st.keepFrags ? `碎片不足，開璀璨彗星需要 ${C.BRIGHT_FRAG} 個碎片` : "要開璀璨彗星請打開「保留碎片」";
    $("#cKeepFrags").checked = !!st.keepFrags;
    const ag = C.GRADES.filter((g) => st.auto.grades[g]);
    const ai = Object.keys(st.auto.items).filter((k) => st.auto.items[k]).length;
    const ak = Object.keys(st.auto.keep).filter((k) => st.auto.keep[k] && st.auto.grades[gradeOf(...splitKey(k))]).length;
    $("#cAutoBtn").title = ag.length || ai
      ? `自動分解：${[ag.length ? ag.join("、") + " 級" : "", ai ? `指定 ${ai} 件` : ""].filter(Boolean).join("＋")}${ak ? `（保留 ${ak} 件）` : ""}`
      : "自動分解：未設定（點一下設定）";
    renderTabs();
  }

  const metric = (label, value, cls = "") => `<div class="g-metric"><span>${label}</span><b class="${cls}">${value}</b></div>`;
  function renderAnalysis() {
    const a = C.analyze(st, st.tab);
    let html = "";
    if (st.tab === "shiny") {
      const ok = st.cost > 0;
      html += metric("每抽期望值", ok || a.hasValues ? big(a.ev) : "—");
      html += metric("CP 值", a.cp === null ? "—" : `${a.cp.toFixed(2)} 倍`, a.cp === null ? "" : a.cp >= 1 ? "pos" : "neg");
      html += metric("單抽賺錢機率", ok ? `${(a.win * 100).toFixed(1)}%` : "—");
      html += metric("每碎片價值", a.frag.value > 0 ? big(a.frag.value) : "—");
    } else {
      html += metric("每顆期望值", a.ev > 0 ? big(a.ev) : "—");
      html += metric("約等於閃亮", a.shinyEq === null ? "—" : `${a.shinyEq.toFixed(2)} 顆`);
      html += metric("每碎片價值", a.ev > 0 ? big(a.perFrag) : "—");
      html += metric("高於平均機率", a.ev > 0 ? `${(a.above * 100).toFixed(1)}%` : "—");
    }
    $("#cMetrics").innerHTML = html;
    const c = $("#cComment");
    c.innerHTML = "<b>AI 評語</b>";
    c.append(document.createTextNode(C.comment(a, big).join("")));
  }

  function sortedKeys(obj) {
    const order = {};
    C.POOLS.forEach((p, pi) => st.pools[p].forEach((it, i) => { const k = key(p, it.name); if (!(k in order)) order[k] = pi * 10000 + i; }));
    const rank = (k) => order[k] ?? C.POOLS.indexOf(splitKey(k)[0]) * 10000 + 9999;
    return Object.keys(obj).filter((k) => obj[k] > 0).sort((a, b) => rank(a) - rank(b));
  }

  function renderBatch() {
    const box = $("#cBatch");
    box.innerHTML = "";
    const d = st.batchDraws;
    const parts = [];
    if (d.shiny) parts.push(`閃亮 ${intFmt.format(d.shiny)} 抽`);
    if (d.bright) parts.push(`璀璨 ${intFmt.format(d.bright)} 顆`);
    const split = Object.values(st.batchSplit).reduce((a, b) => a + b, 0);
    if (split) parts.push(`自動分解 ${intFmt.format(split)} 件`);
    $("#cBatchBadge").textContent = parts.join("・");
    // 分解掉的不顯示，只列留下來的
    const kept = (k) => st.batch[k] - (st.batchSplit[k] || 0);
    const keys = sortedKeys(st.batch).filter((k) => kept(k) > 0);
    if (!keys.length) { box.innerHTML = `<span class="g-chip-empty">${Object.keys(st.batch).length ? "這次抽到的都分解了" : "按下抽獎後，這裡會顯示這次的結果"}</span>`; return; }
    for (const k of keys) {
      const [pool, name] = splitKey(k);
      const g = gradeOf(pool, name);
      const chip = document.createElement("span");
      chip.className = "g-chip c-chip" + (g === topGrade(pool) || g === "S" ? " rare" : "");
      chip.append(gradeBadge(g), ItemIcons.el(name), document.createTextNode(name));
      const n = document.createElement("b");
      n.textContent = `×${kept(k)}`;
      chip.append(n);
      box.appendChild(chip);
    }
  }

  let openSplit = null; // 保管箱中正在輸入分解數量的道具
  function renderHistory() {
    const box = $("#cHistory");
    box.innerHTML = "";
    // 抽到過的道具都列出；數量是目前還留著的
    const drawn = { ...st.hist };
    for (const [k, n] of Object.entries(st.batch)) drawn[k] = (drawn[k] || 0) + n;
    // 分解光的道具不顯示
    const keys = sortedKeys(drawn).filter((k) => (st.owned[k] || 0) > 0);
    if (!keys.length) { box.innerHTML = `<div class="g-empty">${Object.keys(drawn).length ? "保管箱裡的道具都分解了" : "保管箱還是空的"}</div>`; return; }
    let lastPool = null;
    for (const k of keys) {
      const [pool, name] = splitKey(k);
      if (pool !== lastPool) {
        lastPool = pool;
        const g = document.createElement("div");
        g.className = "g-group";
        const draws = (st.histDraws[pool] || 0) + (st.batchDraws[pool] || 0);
        g.textContent = `${C.POOL_NAME[pool]}・${intFmt.format(draws)} ${pool === "shiny" ? "抽" : "顆"}`;
        box.appendChild(g);
      }
      const owned = st.owned[k] || 0;
      const draws = (st.histDraws[pool] || 0) + (st.batchDraws[pool] || 0);
      const actual = draws ? (drawn[k] / draws) * 100 : 0;
      const it = itemOf(pool, name);
      const expect = it ? (it.prob / Math.max(100, C.sum(st.pools[pool]))) * 100 : 0;
      const cls = !expect || Math.abs(actual - expect) < 1e-9 ? "same" : actual > expect ? "up" : "down";
      const row = document.createElement("div");
      row.className = "g-hrow c-hrow" + (gradeOf(pool, name) === topGrade(pool) ? " feat" : "");
      const nm = document.createElement("span");
      nm.className = "g-hname";
      const t = document.createElement("span");
      t.className = "t";
      t.textContent = name;
      nm.append(gradeBadge(gradeOf(pool, name)), ItemIcons.el(name), t);
      nm.title = name;
      // 名稱右邊的分解按鈕
      if (owned) {
        const sp = document.createElement("button");
        sp.className = "c-split-btn" + (openSplit === k ? " on" : "");
        sp.textContent = "分解";
        sp.title = `分解「${name}」（每件 ${C.FRAG[gradeOf(pool, name)] || 0} 碎片）`;
        sp.addEventListener("click", () => { openSplit = openSplit === k ? null : k; renderHistory(); });
        nm.append(sp);
      }
      const cnt = document.createElement("span");
      cnt.className = "c-count" + (owned ? "" : " zero");
      cnt.textContent = `×${intFmt.format(owned)}`;
      if (!owned) cnt.title = "已全部分解";
      row.append(nm, cnt);
      row.insertAdjacentHTML("beforeend", `<span class="muted">${valueOf(name) && owned ? big(valueOf(name) * owned) : "—"}</span><span class="${cls}">${actual.toFixed(2)}%${cls === "up" ? " ▲" : cls === "down" ? " ▼" : ""}</span>`);
      box.appendChild(row);
      if (openSplit === k && owned) box.appendChild(splitRow(pool, name, owned));
    }
  }

  /** 保管箱：輸入數量分解 */
  function splitRow(pool, name, owned) {
    const g = gradeOf(pool, name);
    const wrap = document.createElement("div");
    wrap.className = "c-split-row";
    const input = document.createElement("input");
    input.type = "number";
    input.min = "1";
    input.max = String(owned);
    input.value = "1";
    const info = document.createElement("span");
    info.className = "muted";
    const upd = () => { const n = Math.min(owned, Math.max(0, Math.floor(Number(input.value) || 0))); info.textContent = `件，可得 ${intFmt.format(n * C.FRAG[g])} 碎片`; };
    upd();
    const go = (n) => {
      n = Math.min(owned, Math.max(0, Math.floor(n)));
      if (!n) return;
      decompose(key(pool, name), g, n);
      openSplit = null;
      save(); renderRight();
      toast(`分解「${name}」${n} 件，獲得 ${n * C.FRAG[g]} 碎片`);
    };
    input.addEventListener("input", upd);
    input.addEventListener("keydown", (e) => { if (e.key === "Enter") go(Number(input.value)); if (e.key === "Escape") { openSplit = null; renderHistory(); } });
    const btn = document.createElement("button");
    btn.className = "cbtn c-amber small solid";
    btn.textContent = "分解";
    btn.addEventListener("click", () => go(Number(input.value)));
    const all = document.createElement("button");
    all.className = "cbtn c-red small";
    all.textContent = "全部分解";
    all.addEventListener("click", () => go(owned));
    const label = document.createElement("span");
    label.textContent = "分解";
    wrap.append(label, input, info, btn, all);
    setTimeout(() => { input.focus(); input.select(); }, 0);
    return wrap;
  }

  // 分解數量輸入列：點其他地方就關閉
  document.addEventListener("pointerdown", (e) => {
    if (!openSplit || e.target.closest(".c-split-row, .c-split-btn")) return;
    openSplit = null;
    renderHistory();
  });

  function decompose(k, grade, n) {
    st.owned[k] = (st.owned[k] || 0) - n;
    if (st.owned[k] <= 0) delete st.owned[k];
    st.stats.frags += n * C.FRAG[grade];
    st.stats.split += n;
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
    if (st.page === "auto") renderAutoWin();
    if (document.activeElement !== $("#cCost")) $("#cCost").value = st.cost ? intFmt.format(st.cost) : "";
  }

  /* ---------- 模擬 ---------- */
  function beginBatch() {
    for (const [k, n] of Object.entries(st.batch)) st.hist[k] = (st.hist[k] || 0) + n;
    for (const p of C.POOLS) st.histDraws[p] += st.batchDraws[p];
    st.batch = {};
    st.batchSplit = {};
    st.batchDraws = zeroPools();
    openSplit = null;
  }
  /** 抽到一件：自動分解的直接換成碎片，其餘放進保管箱 */
  function gain(pool, it) {
    const k = key(pool, it.name);
    st.batch[k] = (st.batch[k] || 0) + 1;
    st.batchDraws[pool] += 1;
    if (it.filler) return;
    if (isAuto(pool, it)) {
      st.batchSplit[k] = (st.batchSplit[k] || 0) + 1;
      st.stats.frags += C.FRAG[it.grade];
      st.stats.split += 1;
    } else {
      st.owned[k] = (st.owned[k] || 0) + 1;
    }
  }
  function shinyDrawer(useFrags) {
    const pick = C.sampler(st.pools.shiny);
    return () => {
      if (useFrags) st.stats.frags -= C.SHINY_FRAG;
      else st.stats.spent += st.cost || 0;
      st.stats.draws += 1;
      const it = pick();
      gain("shiny", it);
      return it;
    };
  }
  function brightOpener() {
    const pick = C.sampler(st.pools.bright);
    return () => {
      st.stats.frags -= C.BRIGHT_FRAG;
      st.stats.bright += 1;
      const it = pick();
      gain("bright", it);
      return it;
    };
  }
  function commit(msg) {
    save();
    renderRight();
    if (msg) toast(msg);
  }

  /** 沒有保留碎片時：碎片滿 20 個就自動換成閃亮彗星抽掉；回傳換到的道具 */
  function autoExchange() {
    const got = [];
    if (st.keepFrags || !st.pools.shiny.length) return got;
    const d = shinyDrawer(true);
    for (let guard = 0; st.stats.frags >= C.SHINY_FRAG && guard < 1e5; guard++) got.push(d());
    return got;
  }
  /** 抽左邊選到的獎池：閃亮彗星花錢抽；璀璨彗星用碎片開，碎片不夠就停 */
  function drawN(n, fresh) {
    const pool = st.tab;
    if (!st.pools[pool].length) { toast(`${C.POOL_NAME[pool]}獎池是空的`); return false; }
    if (pool === "bright") {
      const avail = brightAvail();
      if (!avail) { toast(st.keepFrags ? `碎片不足，開璀璨彗星需要 ${C.BRIGHT_FRAG} 個碎片` : "要開璀璨彗星請先打開「保留碎片」"); return false; }
      n = Math.min(n, avail);
    }
    if (fresh) beginBatch();
    const d = pool === "bright" ? brightOpener() : shinyDrawer(false);
    for (let i = 0; i < n; i++) { d(); autoExchange(); }
    commit();
    if (pool === "bright" && !brightAvail()) return false; // 碎片用完，按住也停下來
  }
  function drawUntil(pool, name) {
    if (pool === "shiny") {
      beginBatch();
      const d = shinyDrawer(false);
      for (let i = 0; i < DRAW_CAP; i++) {
        const hit = d().name === name;
        if (autoExchange().some((it) => it.name === name) || hit) return commit(`抽到「${name}」，共 ${intFmt.format(i + 1)} 抽`);
      }
      return commit(`已達 ${intFmt.format(DRAW_CAP)} 抽上限仍未抽到`);
    }
    if (st.stats.frags < C.BRIGHT_FRAG) return toast(`碎片不足，開璀璨彗星需要 ${C.BRIGHT_FRAG} 個碎片`);
    beginBatch();
    const open = brightOpener();
    let n = 0;
    while (st.stats.frags >= C.BRIGHT_FRAG) {
      n++;
      if (open().name === name) return commit(`開到「${name}」，共 ${n} 顆璀璨彗星`);
    }
    commit(`碎片用完了，開了 ${n} 顆仍未開到「${name}」`);
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

  /* ---------- 自動分解視窗 ---------- */
  function renderAutoWin() {
    $$("#cAutoGrades button").forEach((b) => b.classList.toggle("on", !!st.auto.grades[b.dataset.g]));
    const box = $("#cAutoList");
    box.innerHTML = "";
    for (const pool of C.POOLS) {
      if (!st.pools[pool].length) continue;
      const h = document.createElement("div");
      h.className = "c-auto-group";
      h.textContent = C.POOL_NAME[pool];
      box.append(h);
      const grid = document.createElement("div");
      grid.className = "c-auto-grid";
      for (const it of st.pools[pool]) {
        const k = key(pool, it.name);
        const byGrade = !!st.auto.grades[it.grade];
        const kept = byGrade && !!st.auto.keep[k] && !st.auto.items[k];
        const on = isAuto(pool, it);
        const card = document.createElement("button");
        card.className = "c-auto-item" + (on ? " on" : "") + (kept ? " kept" : "");
        card.title = on ? "點一下取消這件的自動分解" : kept ? `點一下恢復分解（${it.grade} 級全部分解）` : "點一下設定自動分解";
        const nm = document.createElement("span");
        nm.className = "t";
        nm.textContent = it.name;
        const fr = document.createElement("small");
        fr.textContent = `${C.FRAG[it.grade]} 碎片`;
        card.append(gradeBadge(it.grade), ItemIcons.el(it.name), nm, fr);
        card.addEventListener("click", () => {
          if (byGrade) {
            // 整個等級分解中：點一下改成保留這件，再點一次恢復分解
            delete st.auto.items[k];
            if (st.auto.keep[k]) delete st.auto.keep[k]; else st.auto.keep[k] = true;
          } else if (st.auto.items[k]) delete st.auto.items[k];
          else st.auto.items[k] = true;
          save(); renderAutoWin(); renderPool(); renderStats();
        });
        grid.append(card);
      }
      box.append(grid);
    }
  }

  /* ---------- 自動帶入價格 ---------- */
  const hasAnyValue = () => Object.values(st.values).some((v) => typeof v === "number" && isFinite(v));
  function renderPriceBtn() {
    const btn = $("#cAutoPrice");
    const any = hasAnyValue();
    btn.textContent = any ? "移除全部價格" : "自動帶入價格";
    btn.classList.toggle("c-teal", !any);
    btn.classList.toggle("c-red", any);
    btn.title = any ? "清空所有道具的價值" : "依機率估算基準價值：越稀有越高；會自動分解的道具（垃圾）留空";
  }

  /** 規則：一行小字（等級分解碎片、碎片兌換） */
  function renderRuleLine() {
    const box = $("#cRuleLine");
    if (box.childElementCount) return;
    const add = (...nodes) => { const span = document.createElement("span"); span.className = "c-rule"; span.append(...nodes); box.append(span); };
    const txt = (t) => document.createTextNode(t);
    for (const g of C.GRADES) add(gradeBadge(g), txt(`${C.FRAG[g]}`));
    add(ItemIcons.el("彗星碎片"), txt("分解得到的彗星碎片"));
    add(txt(`${C.SHINY_FRAG} 個 → `), ItemIcons.el("閃亮彗星"), txt("閃亮彗星"));
    add(txt(`${C.BRIGHT_FRAG} 個 → `), ItemIcons.el("璀璨彗星"), txt("璀璨彗星"));
  }

  /* ---------- 事件 ---------- */
  $$("#cPages button").forEach((b) => b.addEventListener("click", () => { st.page = b.dataset.page; save(); renderAll(); }));
  if (window.BackHooks) window.BackHooks["view-comet"] = {
    label: () => (st.page !== "sim" ? "上一頁" : null),
    back: () => { if (st.page === "sim") return false; st.page = "sim"; save(); renderAll(); return true; },
  };
  $$("#cTabs button").forEach((b) => b.prepend(ItemIcons.el(b.dataset.pool === "shiny" ? "閃亮彗星" : "璀璨彗星")));
  $$("#cTabs button").forEach((b) => b.addEventListener("click", () => { st.tab = b.dataset.pool; save(); renderPool(); renderAnalysis(); renderStats(); }));
  $("#cCost").addEventListener("focus", (e) => { e.target.value = st.cost ? String(st.cost) : ""; e.target.select(); });
  $("#cCost").addEventListener("input", (e) => { st.cost = Math.max(0, parseNum(e.target.value) || 0); save(); renderRight(); });
  $("#cAutoPrice").addEventListener("click", async () => {
    if (hasAnyValue()) {
      if (!(await confirmBox("確定要移除所有道具的價格嗎？"))) return;
      st.values = {};
      save(); renderAll();
      return;
    }
    if (!(st.cost > 0)) return toast("請先輸入閃亮彗星每顆價格，自動價格以它為基準");
    st.values = C.autoPrice(st, isAuto);
    save(); renderAll();
    toast("已帶入基準價格（自動分解的道具留空），可再依實際行情調整");
  });
  $("#cCost").addEventListener("blur", (e) => { e.target.value = st.cost ? intFmt.format(st.cost) : ""; });

  $("#cAdd").addEventListener("click", () => {
    const name = $("#cNewName").value.trim();
    const prob = parseNum($("#cNewProb").value);
    const grade = $("#cNewGrade").value;
    if (!name || !(prob > 0)) return toast("請輸入道具名稱與機率");
    st.pools[st.tab].push({ grade, name, prob });
    const v = parseNum($("#cNewValue").value);
    if (v !== null) st.values[name] = v;
    $("#cNewName").value = $("#cNewProb").value = $("#cNewValue").value = "";
    save(); renderPool(); renderRight();
    $("#cNewName").focus();
  });
  ["#cNewName", "#cNewProb", "#cNewValue"].forEach((id) => $(id).addEventListener("keydown", (e) => { if (e.key === "Enter") $("#cAdd").click(); }));

  $("#cClearItems").addEventListener("click", async () => {
    if (!C.POOLS.some((p) => st.pools[p].length)) return;
    if (!(await confirmBox("確定要清除閃亮彗星、璀璨彗星的所有道具嗎？"))) return;
    for (const p of C.POOLS) st.pools[p] = [];
    save(); renderAll();
  });

  $$("[data-cdraw]").forEach((b) => holdable(b, (fresh) => drawN(Number(b.dataset.cdraw), fresh)));
  $("#cKeepFrags").addEventListener("change", (e) => {
    st.keepFrags = e.target.checked;
    save(); renderRight();
    toast(st.keepFrags ? "已保留碎片：可以選「璀璨彗星」開璀璨彗星" : `碎片滿 ${C.SHINY_FRAG} 個會自動換成閃亮彗星`);
  });

  $("#cReset").addEventListener("click", async () => {
    if (!(await confirmBox("確定要重置花費、抽數、碎片與保管箱嗎？"))) return;
    st.stats = blankStats();
    st.batch = {}; st.batchSplit = {}; st.hist = {}; st.owned = {};
    st.batchDraws = zeroPools(); st.histDraws = zeroPools();
    save(); renderAll();
  });

  // 自動分解視窗
  $("#cAutoBtn").addEventListener("click", () => { st.page = "auto"; save(); renderAll(); });
  $$("#cAutoGrades button").forEach((b) => b.addEventListener("click", () => {
    const g = b.dataset.g;
    st.auto.grades[g] = !st.auto.grades[g];
    save(); renderAutoWin(); renderPool(); renderStats();
  }));
  $("#cAutoNone").addEventListener("click", () => {
    st.auto = { grades: {}, items: {}, keep: {} };
    save(); renderAutoWin(); renderPool(); renderStats();
  });
  // 立刻把保管箱裡符合自動分解的道具分解掉
  $("#cAutoNow").addEventListener("click", () => {
    let n = 0, f = 0;
    for (const k of Object.keys(st.owned)) {
      const [pool, name] = splitKey(k);
      const it = itemOf(pool, name);
      if (!it || !isAuto(pool, it)) continue;
      const cnt = st.owned[k];
      decompose(k, it.grade, cnt);
      n += cnt;
      f += cnt * C.FRAG[it.grade];
    }
    save(); renderRight();
    toast(n ? `分解了保管箱中 ${n} 件，獲得 ${f} 碎片` : "保管箱裡沒有符合自動分解的道具");
  });

  // 解析頁
  $("#cParse").addEventListener("click", () => {
    const parsed = C.parse($("#cPaste").value);
    const got = C.POOLS.filter((p) => parsed[p].length);
    if (!got.length) { $("#cParseResult").textContent = "沒有解析到任何「等級 名稱 機率%」的道具"; return; }
    for (const p of got) st.pools[p] = parsed[p];
    if (!got.includes(st.tab)) st.tab = got[0];
    save(); renderAll();
    const msg = got.map((p) => `${C.POOL_NAME[p]} ${parsed[p].length} 項`).join("、");
    $("#cParseResult").textContent = `已套用：${msg}`;
    toast(`已套用：${msg}`);
  });

  renderAll();
  window.Comet = { onShow: renderAll };
})();
