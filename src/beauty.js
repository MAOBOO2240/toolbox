"use strict";

/* ==========================================================
 * 美容美髮模擬器
 * - BeautyCore：獎池解析、主打判定、抽樣（可獨立測試）
 * - 介面：角色查詢（Rust 呼叫 Nexon API）、紙娃娃、抽獎與統計
 * ========================================================== */

const BeautyCore = (() => {
  const POOLS = ["hairM", "hairF", "faceM", "faceF"];
  const POOL_NAME = { hairM: "皇家美髮（男）", hairF: "皇家美髮（女）", faceM: "皇家整形（男）", faceF: "皇家整形（女）" };
  const HAIR_COLORS = ["黑色", "紅色", "橘色", "黃色", "綠色", "藍色", "紫色", "褐色"];

  /** 解析官網機率文字。沒有標題的開頭部分視為皇家美髮（男）。 */
  function parse(text) {
    const out = { hairM: [], hairF: [], faceM: [], faceF: [] };
    let kind = "hair";
    let gender = "M";
    for (const raw of String(text).split(/\r?\n/)) {
      const line = raw.trim();
      if (!line) continue;
      const hasPct = /\d\s*%/.test(line);
      if (!hasPct) {
        if (/美髮/.test(line)) kind = "hair";
        if (/整形|整容/.test(line)) kind = "face";
        if (/[(（]\s*男\s*[)）]/.test(line)) gender = "M";
        if (/[(（]\s*女\s*[)）]/.test(line)) gender = "F";
        continue;
      }
      const m = /^(.*\S)\s+(\d+(?:\.\d+)?)\s*%$/.exec(line);
      if (!m) continue;
      const name = m[1].trim();
      const prob = parseFloat(m[2]);
      if (!name || !(prob > 0)) continue;
      out[kind + gender].push({ name, prob });
    }
    return out;
  }

  const sum = (items) => items.reduce((a, it) => a + it.prob, 0);

  /** 主打：獎池中機率最低的造型 */
  function featured(items) {
    if (!items.length) return new Set();
    const min = Math.min(...items.map((it) => it.prob));
    // 全部機率一樣就沒有主打
    if (items.every((it) => Math.abs(it.prob - min) < 1e-9)) return new Set();
    return new Set(items.filter((it) => Math.abs(it.prob - min) < 1e-9).map((it) => it.name));
  }

  function sampler(items) {
    const total = sum(items);
    const cum = [];
    let acc = 0;
    for (const it of items) { acc += it.prob; cum.push(acc); }
    return () => {
      const r = Math.random() * total;
      let lo = 0, hi = cum.length - 1;
      while (lo < hi) { const mid = (lo + hi) >> 1; if (cum[mid] > r) hi = mid; else lo = mid + 1; }
      return items[lo];
    };
  }

  /** 去掉髮型名稱前面的顏色：「黑色餘韻髮型」→「餘韻髮型」 */
  function stripColor(name) {
    for (const c of HAIR_COLORS) if (name.startsWith(c)) return name.slice(c.length);
    return name;
  }

  return { POOLS, POOL_NAME, HAIR_COLORS, parse, sum, featured, sampler, stripColor };
})();

if (typeof module !== "undefined") module.exports = BeautyCore;

if (typeof document !== "undefined") (() => {
  const C = BeautyCore;
  const KEY = "beauty-v1";
  const DRAW_CAP = 1_000_000;
  const DEFAULT_COST = 44;

  const DEFAULT_TEXT = `黑色餘韻髮型	1.50%
黑色層次髮型	1.50%
黑色後梳黑造型	6.00%
黑色飛揚定型造型	6.00%
黑色粉彩哥德	6.00%
黑色閃亮髮型	6.00%
黑色閃光造型	6.00%
黑色青澀短髮造型	13.40%
黑色酷帥逆風造型	13.40%
黑色橡果造型	13.40%
黑色泰斯造型	13.40%
黑色浪子小捲造型	13.40%
皇家美髮
(女)
黑色微撥髮型	1.50%
黑色鷹嘴豆髮型	1.50%
黑色可愛風格頭	6.00%
黑色短劉海髮	6.00%
黑色春漾髮型	6.00%
黑色奧德賽髮型	6.00%
黑色陽光頭	6.00%
黑色小啾紮髮造型	13.40%
黑色華爾滋造型	13.40%
黑色漸層短髮造型	13.40%
黑色圈圈髮尾造型	13.40%
黑色高馬尾造型	13.40%
皇家整形
(男)
神經藥臉部	2.50%
小可愛臉型	2.50%
愛睏的臉型	9.50%
靈動眼臉型	9.50%
仁王眼臉型	9.50%
克勞烏臉型	9.50%
大哥臉	9.50%
好無聊臉型	9.50%
帥氣的臉	9.50%
光頭上司臉型	9.50%
正直的臉型	9.50%
娃娃臉臉型	9.50%
皇家整形
(女)
神經藥臉部	2.50%
小可愛臉型	2.50%
愛睏的臉型	9.50%
靈動眼臉型	9.50%
悠莉臉型	9.50%
希亞臉型	9.50%
丹鳳眼臉型	9.50%
很搶眼臉型	9.50%
粉紅眼影臉型	9.50%
希拉臉型	9.50%
真實的臉	9.50%
鷹勾眼臉形	9.50%`;

  const zero = () => ({ hairM: 0, hairF: 0, faceM: 0, faceF: 0 });
  const blankStats = () => ({ draws: 0, spent: 0, hits: 0 });
  function defaults() {
    return {
      pools: C.parse(DEFAULT_TEXT), kind: "hair", gender: "F", page: "sim", cost: DEFAULT_COST, costV2: true, gamePath: "",
      stats: blankStats(), batch: {}, batchDraws: zero(), hist: {}, histDraws: zero(),
      charName: "", tryHair: null, tryFace: null,
      dress: false, outfit: null, favs: [], styleFavs: {}, styleSize: "m", styleFavOnly: false, pickSlot: "All", pickG: "", pickCash: true, skinId: null,
    };
  }
  let st = (() => {
    const d = defaults();
    try {
      const s = JSON.parse(localStorage.getItem(KEY));
      if (s && s.pools) return { ...d, ...s, stats: { ...blankStats(), ...s.stats }, batchDraws: { ...zero(), ...s.batchDraws }, histDraws: { ...zero(), ...s.histDraws } };
    } catch (_) {}
    return d;
  })();
  // 舊版預設價格是 0，改成每張 44
  if (!st.costV2) { if (!st.cost) st.cost = DEFAULT_COST; st.costV2 = true; }
  // 舊版試穿只存名稱
  const fixTry = (t, kind) => (typeof t === "string" ? { pool: kind + (st.gender || "F"), name: t } : t || null);
  st.tryHair = fixTry(st.tryHair, "hair");
  st.tryFace = fixTry(st.tryFace, "face");
  // 戒指欄位分成名牌戒指、聊天戒指
  if (st.pickSlot === "Ring") st.pickSlot = "RingTag";
  // 舊版收藏的是整套，改成一件一件收藏
  if (Array.isArray(st.looks)) {
    const seen = new Set((st.favs || []).map((x) => x.id));
    st.favs = [...(st.favs || [])];
    for (const lk of st.looks) for (const it of lk.outfit || []) if (!seen.has(it.id)) { seen.add(it.id); st.favs.push(it); }
    delete st.looks;
  }
  if (!Array.isArray(st.favs)) st.favs = [];
  if (!st.styleFavs || typeof st.styleFavs !== "object") st.styleFavs = {};
  let saveTimer;
  const save = () => {
    clearTimeout(saveTimer);
    saveTimer = setTimeout(() => { try { localStorage.setItem(KEY, JSON.stringify(st)); } catch (_) {} }, 200);
  };

  // 顏色與順序和遊戲裡的選擇畫面相同（第一格黑色在遊戲裡是打勾的預設色）
  const HAIR_SW = [["黑色", "#2a2a2e"], ["紅色", "#e95273"], ["橘色", "#f18d49"], ["黃色", "#f7d743"], ["綠色", "#8ecf1f"], ["藍色", "#419eeb"], ["紫色", "#aa5ce7"], ["褐色", "#a87056"]];
  const FACE_SW = [["黑色", "#2a2a2e"], ["藍色", "#409be9"], ["紅色", "#e85072"], ["綠色", "#8acc1e"], ["褐色", "#a46b53"], ["祖母綠", "#35dff7"], ["紫色", "#a75ae4"], ["紫水晶", "#f483e3"]];
  // 舊版有第 9 種「白色」瞳色，遊戲裡沒有，拿掉
  if (st.faceColor === "白色") st.faceColor = null;
  if (st.faceMix && (st.faceMix.base === "白色" || st.faceMix.mix === "白色")) st.faceMix = null;
  let ch = null; // 目前載入的角色（API 結果，不保存，重開時從 Rust 快取取回）

  const intFmt = new Intl.NumberFormat("zh-TW", { maximumFractionDigits: 0 });
  const decFmt = new Intl.NumberFormat("zh-TW", { maximumFractionDigits: 2 });
  const parseNum = (s) => { const n = parseFloat(String(s).replace(/[,\s%]/g, "")); return isFinite(n) ? n : null; };
  const isHair = (pool) => pool.startsWith("hair");
  /** 角色性別只用來決定預設要抽男生還是女生的造型，使用者仍可自由切換 */
  const charGender = () => (ch?.basic?.character_gender === "男" ? "M" : ch?.basic?.character_gender === "女" ? "F" : null);
  const gender = () => st.gender || "F";
  const curPool = () => `${st.kind || "hair"}${gender()}`;

  /* ---------- 分頁 ---------- */
  function renderPage() {
    $$("#bPages button").forEach((b) => b.classList.toggle("on", b.dataset.page === st.page));
    $("#bPageSim").hidden = st.page !== "sim";
    $("#bPageImport").hidden = st.page !== "import";
    // 變更點裝模式：左邊換成選點裝，右邊換成目前裝備與收藏
    $("#bPool").hidden = !!st.dress;
    $("#bPick").hidden = !st.dress;
    $("#bSide").hidden = !!st.dress;
    $("#bDressSide").hidden = !st.dress;
    // 變更點裝是獨立的工具：標題換掉，右上角的抽獎模擬／解析道具不顯示
    $("#bPages").hidden = !!st.dress;
    const title = document.querySelector("#view-beauty .nav-title");
    if (title) title.textContent = st.dress ? "變更點裝" : "美容美髮模擬器";
    window.refreshBack?.();
  }

  /* ---------- 獎池 ---------- */
  function renderPool() {
    $$("#bKind button").forEach((b) => b.classList.toggle("on", b.dataset.kind === st.kind));
    $$("#bGender button").forEach((b) => b.classList.toggle("on", b.dataset.g === gender()));
    const pool = curPool();
    const items = st.pools[pool];
    const feat = C.featured(items);
    const box = $("#bTable");
    box.innerHTML = "";
    if (!items.length) {
      box.innerHTML = '<div class="g-empty">這個獎池還沒有造型，請到「解析」貼上機率文字。</div>';
    }
    for (const it of items) {
      const row = document.createElement("div");
      row.className = "g-row b-row" + (feat.has(it.name) ? " jackpot" : "");
      const btn = document.createElement("button");
      btn.className = "g-draw";
      btn.textContent = "抽";
      btn.title = "一直抽到這個造型為止";
      btn.addEventListener("click", () => drawUntil(it.name));
      // 名稱欄：只有游標在文字上才是複製，欄位其他地方跟整列一樣是試穿
      const name = document.createElement("span");
      name.className = "b-name";
      const text = document.createElement("span");
      text.className = "b-ntext";
      text.textContent = it.name;
      makeCopyable(text, it.name);
      name.appendChild(text);
      if (feat.has(it.name)) {
        const tag = document.createElement("em");
        tag.className = "b-tag";
        tag.textContent = "主打";
        name.appendChild(tag);
      }
      row.title = "點一下直接試穿（點名稱是複製名稱）";
      row.addEventListener("click", (e) => { if (!e.target.closest(".g-draw")) tryOn(pool, it.name); });
      const prob = document.createElement("span");
      prob.className = "r b-prob";
      prob.textContent = `${decFmt.format(it.prob)}%`;
      row.append(btn, name, prob);
      box.appendChild(row);
    }
    const total = C.sum(items);
    const featProb = items.filter((it) => feat.has(it.name)).reduce((a, it) => a + it.prob, 0);
    $("#bOdds").textContent = items.length
      ? (featProb ? `主打機率 ${decFmt.format(featProb)}%・平均 ${intFmt.format(Math.ceil(100 / featProb))} 抽中一次主打` : "")
        + (Math.abs(total - 100) > 0.01 ? `（總機率 ${decFmt.format(total)}%）` : "")
      : "";
  }

  /* ---------- 角色與紙娃娃 ---------- */
  function wearLine(label, main, extra) {
    const row = document.createElement("div");
    row.className = "b-wear-row";
    const l = document.createElement("span");
    l.className = "muted";
    l.textContent = label;
    const v = document.createElement("span");
    v.textContent = main;
    row.append(l, v);
    if (extra) {
      const e = document.createElement("small");
      e.className = "muted";
      e.textContent = extra;
      row.append(e);
    }
    return row;
  }

  function currentHair() {
    const h = ch?.beauty?.character_hair;
    return h ? { name: h.hair_name, color: h.base_color, mix: h.mix_color, rate: Number(h.mix_rate) || 0 } : null;
  }
  function currentFace() {
    const f = ch?.beauty?.character_face;
    return f ? { name: f.face_name, color: f.base_color, mix: f.mix_color, rate: Number(f.mix_rate) || 0 } : null;
  }

  /* ---------- 從遊戲資料畫紙娃娃 ---------- */
  const poolGender = (pool) => (pool.endsWith("F") ? "F" : "M");
  const renderCache = new Map();
  /** 同一時間只送一個請求；請求進行中又有新的，只畫最新的那個 */
  function renderQueue(apply) {
    let gen = 0, busy = false, pending = null;
    async function pump() {
      if (busy || !pending) return;
      const { req, my } = pending;
      pending = null;
      busy = true;
      const k = JSON.stringify(req);
      let out = renderCache.get(k) || null, err = null;
      if (!out) {
        try {
          out = await invoke("beauty_render", { req });
          renderCache.set(k, out);
          if (renderCache.size > 60) renderCache.delete(renderCache.keys().next().value);
        } catch (e) { err = String(e); }
      }
      busy = false;
      if (my === gen) apply(out, err);
      pump();
    }
    return (req) => {
      const my = ++gen;
      if (!req) { pending = null; apply(null, null); return; }
      pending = { req, my };
      pump();
    };
  }

  /** 目前外觀套組的點裝（沒有套組就用基本點裝），不含戒指等看不到的 */
  function cashList() { return cashListOf(ch); }
  function cashListOf(c) {
    if (!c?.cash) return [];
    const preset = c.cash[`cash_item_equipment_preset_${c.cash.preset_no}`];
    const list = preset?.length ? preset : c.cash.cash_item_equipment_base || [];
    // 戒指也送去，名牌戒指、聊天戒指會改變名牌與對話框（其他戒指 Rust 會略過）
    return list.filter((x) => !/EXP|口袋|胸章|徽章/.test(x.cash_item_equipment_slot + x.cash_item_name));
  }
  /** 一般裝備（非點裝）的披風：沒有點裝披風時才有，用來讓紙娃娃也穿上披風 */
  function gearCape(c) {
    const list = c?.gear?.item_equipment;
    if (!Array.isArray(list)) return null;
    const isCape = (x) => /披風|망토|cape/i.test(`${x.item_equipment_slot || ""}${x.item_equipment_part || ""}`);
    const cape = list.find((x) => isCape(x) && x.item_name);
    return cape ? { name: cape.item_name, dye: null } : null;
  }

  /** 戒指（名牌戒指、聊天戒指會改變名牌與對話框的樣子）；外觀套組沒有戒指時看基本點裝 */
  function ringsOf(c) {
    if (!c?.cash) return [];
    const isRing = (x) => /戒指|반지|ring/i.test(`${x.cash_item_equipment_slot || ""}${x.cash_item_equipment_part || ""}`);
    const preset = (c.cash[`cash_item_equipment_preset_${c.cash.preset_no}`] || []).filter(isRing);
    const list = preset.length ? preset : (c.cash.cash_item_equipment_base || []).filter(isRing);
    return [...new Set(list.map((x) => x.cash_item_name).filter(Boolean))];
  }
  /** 名牌、對話框的外觀（依戒指，從遊戲資料讀取） */
  const tagCache = new Map();
  /** rings = 戒指名稱（API）、ids = 戒指編號（紙娃娃實際戴上的） */
  function fetchTags({ rings = [], ids = null } = {}) {
    if (!TAURI) return Promise.resolve(null);
    const k = JSON.stringify([st.gamePath || "", rings, ids]);
    if (!tagCache.has(k)) tagCache.set(k, invoke("beauty_tags", { gameDir: st.gamePath || null, rings, ids }).catch(() => { tagCache.delete(k); return null; }));
    return tagCache.get(k);
  }
  const ringIdsOf = (list) => (list || []).filter((x) => /^Ring/.test(x.slot)).map((x) => x.id);

  /** 某個角色目前的樣子（不含試穿），給左邊側欄的紙娃娃用 */
  function charReq(c) {
    const g = c?.basic?.character_gender === "男" ? "M" : "F";
    const part = (x, key) => (x ? { name: x[key], color: x.base_color || null, mix: x.mix_color || null, rate: Number(x.mix_rate) || 0, gender: g } : null);
    return {
      game_dir: st.gamePath || null,
      skin: c?.beauty?.character_skin?.skin_name || null,
      gender: g,
      hair: part(c?.beauty?.character_hair, "hair_name"),
      face: part(c?.beauty?.character_face, "face_name"),
      mode: "full",
      skin_id: null,
      items: withCape(c, cashListOf(c).map((x) => ({ name: x.cash_item_name, dye: prismToDye(x.cash_item_coloring_prism) }))),
    };
  }
  /** 點裝裡沒有披風時，補上一般裝備的披風 */
  function withCape(c, items) {
    const cape = gearCape(c);
    return cape ? [...items, cape] : items;
  }
  async function charLook(name) {
    const c = await invoke("maple_lookup", { name, force: false });
    try {
      const out = await invoke("beauty_render", { req: charReq(c) });
      const tags = await fetchTags({ ids: ringIdsOf(out.equips) });
      return { name: c.name, png: out.png, w: out.w, h: out.h, cx: out.cx, cy: out.cy, fx: out.fx, fy: out.fy, anim: out.anim || null, tags };
    } catch (e) {
      // 讀不到遊戲資料時改用官方角色圖
      if (c?.basic?.character_image) return { name: c.name, png: c.basic.character_image, official: true, tags: await fetchTags({ rings: ringsOf(c) }) };
      throw e;
    }
  }

  /** API 的染色稜鏡 → 紙娃娃染色參數 */
  function prismToDye(p) {
    if (!p) return null;
    const r = String(p.color_range || "");
    // 青綠色系要先判斷，否則會被當成綠色系
    const range = /紅|빨간|red/i.test(r) ? 1 : /黃|노란|yellow/i.test(r) ? 2 : /青|청록|cyan/i.test(r) ? 4
      : /綠|초록|green/i.test(r) ? 3 : /藍|파란|blue/i.test(r) ? 5 : /紫|洋紅|자주|purple|magenta/i.test(r) ? 6 : 0;
    const d = { range, hue: Number(p.hue) || 0, sat: Number(p.saturation) || 0, val: Number(p.value) || 0 };
    return d.hue || d.sat || d.val ? d : null;
  }

  /** 組出要畫的造型：角色原本的造型，加上試穿（換造型時保留角色原本的顏色與混染） */
  function lookReq(tryH, tryF, mode) {
    const h = currentHair(), f = currentFace();
    const g = charGender() || gender();
    // 選了下方的顏色就用那個顏色（不混染），否則保留角色原本的顏色與混染
    // 自己設定的混染（mx）優先：基本顏色＋混合顏色＋混合比例
    const part = (cur, tried, pick, mx) => {
      const color = mx ? mx.base : pick || cur?.color || null;
      const mix = mx ? mx.mix : pick ? null : cur?.mix || null;
      const rate = mx ? mx.rate : pick ? null : cur ? cur.rate : null;
      if (tried) return { name: tried.name, id: tried.id || null, color, mix, rate, gender: poolGender(tried.pool) };
      if (cur) return { name: cur.name, color, mix, rate, gender: g };
      if (pick || mx) return { name: "", color, mix, rate, gender: g };
      return null;
    };
    return {
      game_dir: st.gamePath || null,
      skin: ch?.beauty?.character_skin?.skin_name || null,
      gender: g,
      hair: part(h, tryH, st.hairColor, st.hairMix),
      face: part(f, tryF, st.faceColor, st.faceMix),
      mode,
      skin_id: st.skinId || null,
      // 自己換過點裝時照自己的；沒換過就用角色目前的點裝（再補上一般裝備的披風）
      items: mode !== "full" ? [] : st.outfit
        ? st.outfit.map((x) => ({ id: x.id, dye: x.dye || null }))
        : withCape(ch, cashList().map((x) => ({ name: x.cash_item_name, dye: prismToDye(x.cash_item_coloring_prism) }))),
    };
  }

  /** 美容美髮目前的紙娃娃（給側欄「同步美容美髮」用，一律是站姿） */
  let lastDoll = null;
  function announceDoll(d) {
    lastDoll = d;
    document.dispatchEvent(new CustomEvent("beauty-doll", { detail: d }));
  }
  const dollOf = (out) => ({ name: st.charName, png: out.png, w: out.w, h: out.h, cx: out.cx, cy: out.cy, fx: out.fx, fy: out.fy, anim: out.anim || null });
  const officialDoll = () => (ch?.basic?.character_image && !(st.tryHair || st.tryFace) ? { name: st.charName, png: ch.basic.character_image, official: true } : null);
  // 中間的角色在做其他動作時，側欄另外畫一張站姿
  const announceQueue = renderQueue((out) => announceDoll(out ? dollOf(out) : officialDoll()));

  /* ---------- 中間的角色：每一格動作、道具特效 ---------- */
  let act = "stand";
  const Actor = (() => {
    const box = $("#bActor"), cv = $("#bCanvas"), g = cv.getContext("2d");
    let cur = null, gen = 0, raf = 0, t0 = 0, drawn = "";
    const loadImg = (src) => new Promise((res) => { const im = new Image(); im.onload = () => res(im); im.onerror = () => res(null); im.src = src; });
    const dur = (f) => Math.max(30, Number(f.delay) || 120);
    const total = (fs) => fs.reduce((a, f) => a + dur(f), 0);
    const pick = (fs, sum, t) => {
      if (fs.length < 2 || !sum) return 0;
      let x = t % sum;
      for (let i = 0; i < fs.length; i++) { x -= dur(fs[i]); if (x < 0) return i; }
      return fs.length - 1;
    };
    const withImgs = (fs) => Promise.all(fs.map(async (f) => ({ ...f, img: await loadImg(f.png) })));
    async function set(anim) {
      const my = ++gen;
      const frames = await withImgs(anim.frames || []);
      const effects = await Promise.all((anim.effects || []).map(async (e) => ({ ...e, frames: await withImgs(e.frames || []) })));
      if (my !== gen || !frames.length) return false;
      // 畫布範圍：所有畫格，加上特效跟著每一格錨點時的範圍（原點 = 角色腳下）
      let x0 = Infinity, y0 = Infinity, x1 = -Infinity, y1 = -Infinity;
      const add = (x, y, w, h) => { x0 = Math.min(x0, x); y0 = Math.min(y0, y); x1 = Math.max(x1, x + w); y1 = Math.max(y1, y + h); };
      const anchorOf = (b, name) => (name && b.anchors?.[name]) || [0, 0];
      for (const f of frames) add(f.x, f.y, f.w, f.h);
      for (const e of effects) for (const f of e.frames) for (const b of frames) { const a = anchorOf(b, e.anchor); add(a[0] + f.x, a[1] + f.y, f.w, f.h); }
      for (const e of effects) e.sum = total(e.frames);
      cur = { frames, effects, x0, y0, sum: total(frames), anchorOf, center: anim.center || [0, -40], brow: frames[0].anchors?.brow || [0, -55] };
      cv.width = Math.max(1, x1 - x0);
      cv.height = Math.max(1, y1 - y0);
      cv.style.left = `${x0}px`;
      cv.style.top = `${y0}px`;
      cv.style.width = `${cv.width}px`;
      cv.style.height = `${cv.height}px`;
      // 對話框的箭頭在頭頂上方
      $("#bBalloon").style.top = `${cur.brow[1] - 42}px`;
      box.hidden = false;
      drawn = "";
      t0 = performance.now();
      cancelAnimationFrame(raf);
      const moving = frames.length > 1 || effects.some((e) => e.frames.length > 1);
      const step = (now) => {
        // 看不到時（切到其他工具）不重畫
        if (box.offsetParent) draw(now - t0);
        if (moving && my === gen) raf = requestAnimationFrame(step);
      };
      draw(0);
      if (moving) raf = requestAnimationFrame(step);
      return true;
    }
    function draw(t) {
      if (!cur) return;
      const bi = pick(cur.frames, cur.sum, t);
      const eis = cur.effects.map((e) => pick(e.frames, e.sum, t));
      const key = `${bi}|${eis.join(",")}`;
      if (key === drawn) return;
      drawn = key;
      g.clearRect(0, 0, cv.width, cv.height);
      const b = cur.frames[bi];
      const effects = (front) => cur.effects.forEach((e, i) => {
        if ((e.z >= 0) !== front) return;
        const f = e.frames[eis[i]];
        if (!f?.img) return;
        const a = cur.anchorOf(b, e.anchor);
        g.drawImage(f.img, a[0] + f.x - cur.x0, a[1] + f.y - cur.y0);
      });
      effects(false);
      if (b.img) g.drawImage(b.img, b.x - cur.x0, b.y - cur.y0);
      effects(true);
    }
    function clear() { gen++; cur = null; cancelAnimationFrame(raf); box.hidden = true; }
    return { set, clear, center: () => cur?.center || [0, -40], on: () => !!cur && !box.hidden };
  })();
  /** 沒有動畫資料時（舊的結果），把單張圖當成一格 */
  const animOf = (out) => out.anim || {
    frames: [{ png: out.png, x: -(out.fx ?? out.cx), y: -(out.fy ?? out.cy), w: out.w, h: out.h, delay: 0, anchors: {} }],
    effects: [], center: [out.cx - (out.fx ?? out.cx), out.cy - (out.fy ?? out.cy)],
  };

  const showDoll = renderQueue(async (out, err) => {
    const look = $("#bLook"), empty = $("#bEmpty"), note = $("#bDollNote");
    note.textContent = "";
    if (act === "stand") announceDoll(out ? dollOf(out) : officialDoll());
    if (out) {
      lastEquips = out.equips || [];
      if (!st.tryHair) lastHairId = out.hair_id || null;
      if (!st.tryFace) lastFaceId = out.face_id || null;
      if (st.dress) renderEquipped();
      refreshTags();
      const notes = [...(out.warnings || []), ...(tags?.notes || [])];
      if (notes.length) note.textContent = notes.join("；");
      if (await Actor.set(animOf(out))) {
        look.hidden = true;
        empty.hidden = true;
        sizeLook();
      }
      return;
    }
    Actor.clear();
    refreshTags();
    // 讀不到遊戲資料：有角色就先顯示官方角色圖
    const img = ch?.basic?.character_image;
    if (img && !(st.tryHair || st.tryFace)) {
      look.src = img;
      look.hidden = false;
      empty.hidden = true;
    } else {
      look.hidden = true;
      empty.hidden = false;
    }
    if (err) note.textContent = err;
  });

  /* ---------- 名牌、對話框（依名牌戒指、聊天戒指，圖片依原點拼起來） ---------- */
  let tags = null, tagsFor = "";
  const putCanvas = (cv, art) => {
    cv.width = art.canvas.width;
    cv.height = art.canvas.height;
    const g = cv.getContext("2d");
    g.clearRect(0, 0, cv.width, cv.height);
    g.drawImage(art.canvas, 0, 0);
  };
  let tagGen = 0;
  const tagMeasure = document.createElement("canvas").getContext("2d");
  async function applyNameTag() {
    const my = ++tagGen;
    const el = $("#bNameTag"), txt = $("#bNameText"), cv = el.querySelector(".nt-art");
    txt.textContent = st.charName || "";
    el.hidden = !st.charName;
    const t = tags?.name_tag;
    const ok = !!t && (await UiArt.ready(t));
    if (my !== tagGen) return;
    txt.removeAttribute("style");
    el.classList.toggle("plain", !ok);
    if (!ok || el.hidden) { UiArt.stop("beauty-name"); return; }
    // 名牌的上緣貼在腳下（去掉圖片上方透明的部分）
    // 用畫布量字寬（角色框還沒顯示時也量得到）
    tagMeasure.font = getComputedStyle(txt).font || '12px "Microsoft JhengHei", sans-serif';
    const art = UiArt.nameTag(t, Math.ceil(tagMeasure.measureText(txt.textContent).width) + 4);
    if (!art) { el.classList.add("plain"); UiArt.stop("beauty-name"); return; }
    // 會動的名牌依每一格的時間換圖
    UiArt.play("beauty-name", art.frames, (f) => putCanvas(cv, f));
    cv.style.left = `${art.x}px`;
    cv.style.top = "0px";
    Object.assign(txt.style, { left: `${art.left}px`, width: `${art.right - art.left}px`, top: `${art.cy - art.y - 7}px`, color: t.clr || "#fff" });
  }
  async function layoutBalloon() {
    const el = $("#bBalloon"), txt = $("#bBalloonText"), cv = el.querySelector(".bb-art");
    const b = tags?.balloon;
    const ok = !!b && (await UiArt.ready(b));
    txt.removeAttribute("style");
    el.classList.toggle("plain", !ok);
    if (!ok || el.hidden) return;
    const art = UiArt.balloon(b, Math.ceil(txt.offsetWidth), Math.ceil(txt.offsetHeight));
    if (!art) { el.classList.add("plain"); return; }
    putCanvas(cv, art);
    // 華麗的對話框下緣裝飾會往下垂：整個往上移，最低的地方只比錨點低一點點，不會蓋到頭
    const lift = Math.max(0, art.y + art.canvas.height - 6);
    cv.style.left = `${art.x}px`;
    cv.style.top = `${art.y - lift}px`;
    Object.assign(txt.style, { left: `${art.left}px`, top: `${art.top - lift}px`, color: b.clr || "#000" });
  }
  function applyTags(t) {
    tags = t;
    applyNameTag();
    layoutBalloon();
    document.dispatchEvent(new CustomEvent("beauty-tags", { detail: t }));
  }
  /** 名牌、對話框依紙娃娃實際戴上的戒指（讀不到遊戲資料時用 API 的戒指名稱） */
  function refreshTags() {
    const q = TAURI && (st.outfit || lastEquips.length) ? { ids: ringIdsOf(curOutfit()) } : { rings: ringsOf(ch) };
    const k = JSON.stringify([st.gamePath || "", q]);
    applyNameTag();
    if (k === tagsFor) return;
    tagsFor = k;
    fetchTags(q).then((t) => { if (tagsFor === k) applyTags(t); });
  }
  /** 讓角色說話：輸入框裡有字就一直顯示，字清空後對話框才消失 */
  function say(text) {
    const el = $("#bBalloon");
    if (!text) { el.hidden = true; return; }
    $("#bBalloonText").textContent = text;
    el.hidden = false;
    layoutBalloon();
  }

  let lastEquips = [];
  let lastHairId = null, lastFaceId = null, lastSkinId = null; // 紙娃娃最後一次實際穿上的點裝（角色原本的套組解析後的結果）
  const showLast = renderQueue((out) => {
    const pic = $("#bLastPic");
    if (out) { pic.src = out.png; pic.hidden = false; } else { pic.hidden = true; }
  });

  const pan = { x: 0, y: 0 };
  const DOLL_SCALE = 3, DOLL_MIN = 1, DOLL_MAX = 12;
  const dollScale = () => Math.min(DOLL_MAX, Math.max(DOLL_MIN, Number(st.dollScale) || DOLL_SCALE));
  // 官方角色圖本身比較大，倍率打折
  const lookScale = () => (Actor.on() ? dollScale() : dollScale() * 0.55);
  /** 讓角色本身（不是整張圖）位在框的正中間：大型武器不會把角色擠到旁邊 */
  function placeLook() {
    const s = lookScale();
    if (Actor.on()) {
      const [cx, cy] = Actor.center();
      const actor = $("#bActor");
      actor.style.transform = `translate(${pan.x - cx * s}px, ${pan.y - cy * s}px) scale(${s})`;
      actor.style.setProperty("--ui", String(Math.min(s, 2) / s));
      return;
    }
    $("#bLook").style.transform = `translate(${pan.x}px, ${pan.y}px)`;
  }
  /** 紙娃娃放大倍率：滑鼠在角色框裡用滾輪調整 */
  function sizeLook() {
    const look = $("#bLook");
    if (!Actor.on() && look.naturalWidth) look.style.width = `${Math.round(look.naturalWidth * lookScale())}px`;
    placeLook();
  }

  function renderSwatches() {
    const make = (box, list, cur, set) => {
      box.innerHTML = "";
      for (const [name, hex] of list) {
        const b = document.createElement("button");
        b.className = "b-sw" + (cur === name ? " on" : "");
        b.style.setProperty("--c", hex);
        b.title = cur === name ? `${name}（再按一次恢復角色原本的顏色）` : name;
        b.addEventListener("click", () => { set(cur === name ? null : name); save(); renderSwatches(); renderDoll(); renderRight(); });
        box.appendChild(b);
      }
    };
    // 選單一顏色就不混染
    make($("#bHairColors"), HAIR_SW, st.hairMix ? null : st.hairColor, (v) => { st.hairColor = v; st.hairMix = null; if (mixKind === "hair") closeMix(); });
    make($("#bFaceColors"), FACE_SW, st.faceMix ? null : st.faceColor, (v) => { st.faceColor = v; st.faceMix = null; if (mixKind === "face") closeMix(); });
    for (const kind of ["hair", "face"]) {
      const mx = st[MIX_KEY[kind]];
      const link = $(kind === "hair" ? "#bHairMix" : "#bFaceMix");
      link.classList.toggle("on", !!mx);
      if (mx) {
        link.style.setProperty("--a", mixHex(kind, mx.base));
        link.style.setProperty("--b", mixHex(kind, mx.mix));
        link.title = `${MIX_TITLE[kind]}中：${mx.base} ${100 - mx.rate}%＋${mx.mix} ${mx.rate}%（點一下調整）`;
      } else link.title = `${MIX_TITLE[kind]}：基本顏色加上另一個顏色混合`;
    }
  }

  /* ---------- 髮色／瞳色混染 ----------
   * 和遊戲裡的混染一樣：先選一個基本顏色，再從其他顏色選一個混合顏色，最後調整混合比例 */
  const MIX_KEY = { hair: "hairMix", face: "faceMix" };
  const MIX_TITLE = { hair: "髮色混染", face: "瞳色混染" };
  const MIX_SW = { hair: HAIR_SW, face: FACE_SW };
  const mixHex = (kind, name) => (MIX_SW[kind].find(([n]) => n === name) || [, "#888"])[1];
  let mixKind = null, mixTimer = null;
  /** API 或使用者選的顏色名稱對到混染的顏色 */
  function mixName(kind, s) {
    if (!s) return null;
    const names = MIX_SW[kind].map(([n]) => n);
    return names.find((n) => s === n) || names.find((n) => s.includes(n.replace(/色$/, ""))) || (/棕/.test(s) ? "褐色" : null);
  }
  function openMix(kind) {
    $("#bDyeWin").hidden = true;
    if (!$("#bActMenu").hidden) actMenu(false); if (!$("#bStanceMenu").hidden) stanceMenu(false);
    if (styleKind) closeStyle();
    mixKind = kind;
    const key = MIX_KEY[kind];
    if (!st[key]) {
      // 從目前的顏色開始：選過的顏色 → 角色原本的顏色與混染 → 黑色
      const cur = kind === "hair" ? currentHair() : currentFace();
      const pick = kind === "hair" ? st.hairColor : st.faceColor;
      const names = MIX_SW[kind].map(([n]) => n);
      const base = mixName(kind, pick) || mixName(kind, cur?.color) || names[0];
      let mix = !pick && cur?.mix && cur.rate ? mixName(kind, cur.mix) : null;
      if (!mix || mix === base) mix = names.find((n) => n !== base);
      const rate = !pick && cur?.mix && cur.rate ? Math.min(99, Math.max(1, Math.round(cur.rate))) : 50;
      st[key] = { base, mix, rate };
      if (kind === "hair") st.hairColor = null; else st.faceColor = null;
      applyMix(true);
    }
    $("#bMixTitle").textContent = MIX_TITLE[kind];
    $("#bMixWin").hidden = false;
    renderMix();
  }
  function closeMix() { $("#bMixWin").hidden = true; mixKind = null; }
  function renderMix() {
    if (!mixKind) return;
    const mx = st[MIX_KEY[mixKind]];
    if (!mx) return closeMix();
    const sw = (box, pickName, disabledName, onPick) => {
      box.innerHTML = "";
      for (const [name, hex] of MIX_SW[mixKind]) {
        const b = document.createElement("button");
        b.className = "b-sw" + (pickName === name ? " on" : "");
        b.style.setProperty("--c", hex);
        b.title = name === disabledName ? `${name}（已經是基本顏色）` : name;
        b.disabled = name === disabledName;
        b.addEventListener("click", () => onPick(name));
        box.appendChild(b);
      }
    };
    sw($("#bMixBase"), mx.base, null, (n) => {
      mx.base = n;
      if (mx.mix === n) mx.mix = MIX_SW[mixKind].map(([x]) => x).find((x) => x !== n);
      renderMix(); applyMix(true);
    });
    sw($("#bMixMix"), mx.mix, mx.base, (n) => { mx.mix = n; renderMix(); applyMix(true); });
    const r = $("#bMixRate");
    if (document.activeElement !== r) r.value = mx.rate;
    r.style.setProperty("--a", mixHex(mixKind, mx.base));
    r.style.setProperty("--b", mixHex(mixKind, mx.mix));
    $("#bMixBaseV").textContent = `${mx.base} ${100 - mx.rate}%`;
    $("#bMixMixV").textContent = `${mx.mix} ${mx.rate}%`;
  }
  /** 套用到紙娃娃：拉桿拖動時也一直更新（畫圖的佇列只留最新的一張，不會塞車） */
  let mixFrame = 0;
  function applyMix(now) {
    save();
    renderSwatches();
    clearTimeout(mixTimer);
    if (now) { cancelAnimationFrame(mixFrame); mixFrame = 0; renderDoll(); renderRight(); return; }
    if (!mixFrame) mixFrame = requestAnimationFrame(() => { mixFrame = 0; renderDoll(); });
    mixTimer = setTimeout(renderRight, 200);
  }
  $("#bMixRate").addEventListener("input", (e) => {
    const mx = mixKind && st[MIX_KEY[mixKind]];
    if (!mx) return;
    mx.rate = Math.min(99, Math.max(1, Number(e.target.value) || 50));
    renderMix(); applyMix(false);
  });
  $("#bMixHalf").addEventListener("click", () => {
    const mx = mixKind && st[MIX_KEY[mixKind]];
    if (!mx) return;
    mx.rate = 50;
    $("#bMixRate").value = 50;
    renderMix(); applyMix(true);
  });
  $("#bMixOff").addEventListener("click", () => {
    if (!mixKind) return;
    st[MIX_KEY[mixKind]] = null;
    closeMix();
    applyMix(true);
  });
  $("#bMixDone").addEventListener("click", closeMix);
  // 按了其他會叫出介面的按鈕（變更點裝、動作、解析道具、髮型臉型…）就收起混染視窗；
  // 混染視窗本身、髮色瞳色那兩行、紙娃娃不算
  document.addEventListener("pointerdown", (e) => {
    if (!mixKind) return;
    const t = e.target;
    if (t.closest("#bMixWin, .b-colors")) return;
    if (t.closest("button, .b-link, select, [data-page], .b-ftab")) closeMix();
  }, true);
  // 離開美容美髮（回到主頁面或開其他工具）：收起所有浮動視窗
  document.addEventListener("viewchange", (e) => {
    if (e.detail === "beauty") return;
    closeMix();
    $("#bDyeWin").hidden = true;
    dyeId = null;
    if (styleKind) closeStyle();
    actMenu(false);
    stanceMenu(false);
    closeCtx();
  });
  $("#bMixClose").addEventListener("click", closeMix);
  $$(".b-mix-link").forEach((b) => b.addEventListener("click", () => (mixKind === b.dataset.kind ? closeMix() : openMix(b.dataset.kind))));

  function renderDoll() {
    const tryH = st.tryHair, tryF = st.tryFace;
    applyNameTag();
    if (!TAURI || (!ch && !tryH && !tryF && !st.hairColor && !st.faceColor && !st.hairMix && !st.faceMix && !st.skinId && !st.outfit?.length)) { showDoll(null); announceQueue(null); }
    else {
      const req = lookReq(tryH, tryF, "full");
      showDoll({ ...req, action: act, stance: st.stance || "basic" });
      // 側欄要的是站姿；中間在做其他動作時另外畫
      if (act !== "stand") announceQueue(req);
    }

    const wear = $("#bWear");
    wear.innerHTML = "";
    const h = currentHair(), f = currentFace();
    const skin = ch?.beauty?.character_skin?.skin_name;
    const hc = st.hairMix?.base || st.hairColor || h?.color || "", fc = st.faceMix?.base || st.faceColor || f?.color || "";
    const hairText = tryH ? (hc ? `${hc}${C.stripColor(tryH.name)}` : tryH.name) : h ? `${hc}${h.name}` : "預設髮型";
    const faceText = tryF ? (fc ? `${tryF.name}（${fc}）` : tryF.name) : f ? `${f.name}（${fc}）` : "預設臉型";
    const mixText = (x, pick, mx) => (mx ? `混染 ${mx.mix} ${mx.rate}%` : !pick && x && x.mix && x.rate ? `混染 ${x.mix} ${x.rate}%` : "");
    // 髮型、臉型、膚色都是藍字，點了開大視窗挑選
    const linkLine = (label, text, extra, kind) => {
      const row = wearLine(label, "", extra);
      const link = document.createElement("button");
      link.className = "b-link";
      link.textContent = text;
      link.title = `點一下${{ skin: "變更膚色", hair: "變更髮型", face: "變更臉型" }[kind]}`;
      link.addEventListener("click", () => openStyle(kind));
      row.children[1].replaceWith(link);
      return row;
    };
    wear.append(linkLine("髮型", hairText, tryH ? "試穿中" : mixText(h, st.hairColor, st.hairMix), "hair"));
    wear.append(linkLine("臉型", faceText, tryF ? "試穿中" : mixText(f, st.faceColor, st.faceMix), "face"));
    wear.append(linkLine("膚色", curSkinName(), st.skinId ? "試穿中" : "", "skin"));
  }

  function curSkinName() {
    if (st.skinId) return (skins || []).find((x) => x.id === st.skinId)?.name || `膚色 ${st.skinId}`;
    return ch?.beauty?.character_skin?.skin_name || "奶油皮膚";
  }

  function tryOn(pool, name) {
    if (isHair(pool)) st.tryHair = { pool, name }; else st.tryFace = { pool, name };
    save();
    renderDoll();
  }

  async function lookup(force) {
    const name = $("#bName").value.trim();
    if (!name) {
      ch = null;
      st.charName = "";
      save();
      renderPool();
      $("#bStatus").textContent = "輸入角色名稱可以載入目前的造型；不輸入則使用空白紙娃娃";
      renderDoll();
      return;
    }
    if (!TAURI) return toast("預覽模式無法呼叫 API");
    const btns = [$("#bLookup")];
    btns.forEach((b) => (b.disabled = true));
    $("#bStatus").textContent = "查詢中…";
    try {
      ch = await invoke("maple_lookup", { name, force: !!force });
      st.charName = ch.name;
      // 換了角色才依角色性別切換預設（之後使用者自己切換的會保留）
      if (st.genderFor !== ch.name && charGender()) { st.gender = charGender(); st.genderFor = ch.name; }
      save();
      const t = new Date(ch.fetched_at * 1000);
      const when = `${t.getMonth() + 1}/${t.getDate()} ${String(t.getHours()).padStart(2, "0")}:${String(t.getMinutes()).padStart(2, "0")}`;
      $("#bStatus").textContent =
        `${ch.basic?.world_name || ""}・${ch.basic?.character_class || ""}・Lv.${ch.basic?.character_level || "?"}　資料時間 ${when}` +
        (ch.from_cache ? "（快取）" : "");
      renderPool();
      renderDoll();
    } catch (e) {
      $("#bStatus").textContent = String(e);
    } finally {
      btns.forEach((b) => (b.disabled = false));
    }
  }

  /* ---------- 抽獎 ---------- */
  const key = (pool, name) => `${pool}|${name}`;
  const splitKey = (k) => { const i = k.indexOf("|"); return [k.slice(0, i), k.slice(i + 1)]; };

  function beginBatch() {
    for (const [k, n] of Object.entries(st.batch)) st.hist[k] = (st.hist[k] || 0) + n;
    for (const p of C.POOLS) st.histDraws[p] += st.batchDraws[p];
    st.batch = {};
    st.batchDraws = zero();
  }

  function drawer() {
    const pool = curPool();
    const pick = C.sampler(st.pools[pool]);
    const feat = C.featured(st.pools[pool]);
    return () => {
      const it = pick();
      const k = key(pool, it.name);
      st.batch[k] = (st.batch[k] || 0) + 1;
      st.batchDraws[pool] += 1;
      st.stats.draws += 1;
      st.stats.spent += st.cost || 0;
      if (feat.has(it.name)) st.stats.hits += 1;
      st.last = { pool, name: it.name, feat: feat.has(it.name) };
      return it;
    };
  }

  function afterDraw(msg) {
    if (st.last) tryOn(st.last.pool, st.last.name);
    save();
    renderRight();
    if (msg) toast(msg);
  }

  function drawN(n) {
    if (!st.pools[curPool()].length) { toast("這個獎池是空的"); return false; }
    beginBatch();
    const d = drawer();
    for (let i = 0; i < n; i++) d();
    afterDraw();
  }

  function drawUntil(name) {
    if (!st.pools[curPool()].length) return;
    beginBatch();
    const d = drawer();
    for (let i = 0; i < DRAW_CAP; i++) {
      if (d().name === name) return afterDraw(`抽到「${name}」，共 ${intFmt.format(i + 1)} 抽`);
    }
    afterDraw("已達上限仍未抽到");
  }

  function holdable(btn, fn) {
    let t1 = null, t2 = null;
    const stop = () => { clearTimeout(t1); clearInterval(t2); t1 = t2 = null; };
    btn.addEventListener("pointerdown", (e) => {
      if (e.button !== 0 || btn.disabled) return;
      if (fn(false) === false) return;
      t1 = setTimeout(() => { t2 = setInterval(() => { if (fn(true) === false) stop(); }, 110); }, 380);
    });
    ["pointerup", "pointerleave", "pointercancel"].forEach((ev) => btn.addEventListener(ev, stop));
  }

  function sortedKeys(obj) {
    const order = {};
    C.POOLS.forEach((p, pi) => st.pools[p].forEach((it, i) => { order[key(p, it.name)] = pi * 1000 + i; }));
    return Object.keys(obj).sort((a, b) => (order[a] ?? 1e9) - (order[b] ?? 1e9));
  }
  const isFeat = (pool, name) => C.featured(st.pools[pool]).has(name);

  function renderRight() {
    $("#bSpent").textContent = intFmt.format(st.stats.spent);
    $("#bDraws").textContent = intFmt.format(st.stats.draws);
    $("#bHits").textContent = intFmt.format(st.stats.hits);
    const last = $("#bLastName");
    last.textContent = st.last ? st.last.name : "按下單抽開始";
    $("#bLast .b-last-tag").hidden = !st.last;
    $("#bLast").classList.toggle("feat", !!st.last?.feat);
    $("#bLast").classList.toggle("empty", !st.last);
    if (st.last && TAURI) {
      const L = { pool: st.last.pool, name: st.last.name };
      showLast(lookReq(isHair(L.pool) ? L : null, isHair(L.pool) ? null : L, "head"));
    } else showLast(null);

    const hist = $("#bHistory");
    hist.innerHTML = "";
    // 保管箱只顯示左邊目前選的種類（皇家美髮／皇家整形）
    // 保管箱包含這一次抽到的，數量才會和總抽數一致
    const all = { ...st.hist };
    for (const [k, n] of Object.entries(st.batch)) all[k] = (all[k] || 0) + n;
    const drawsOf = (p) => (st.histDraws[p] || 0) + (st.batchDraws[p] || 0);
    const hk = sortedKeys(all).filter((k) => splitKey(k)[0].startsWith(st.kind || "hair"));
    if (!hk.length) hist.innerHTML = `<div class="g-empty">${st.kind === "face" ? "皇家整形" : "皇家美髮"}的保管箱還是空的</div>`;
    let lastPool = null;
    for (const k of hk) {
      const [pool, name] = splitKey(k);
      if (pool !== lastPool) {
        lastPool = pool;
        const g = document.createElement("div");
        g.className = "g-group";
        g.textContent = `${C.POOL_NAME[pool]}・${intFmt.format(drawsOf(pool))} 抽`;
        hist.appendChild(g);
      }
      const count = all[k];
      const draws = drawsOf(pool);
      const actual = draws ? (count / draws) * 100 : 0;
      const expect = st.pools[pool].filter((it) => it.name === name).reduce((a, it) => a + it.prob, 0);
      const cls = !expect || Math.abs(actual - expect) < 1e-9 ? "same" : actual > expect ? "up" : "down";
      const row = document.createElement("div");
      row.className = "g-hrow b-hrow" + (isFeat(pool, name) ? " feat" : "");
      const nm = document.createElement("span");
      nm.textContent = name;
      row.append(nm);
      row.insertAdjacentHTML("beforeend", `<span>×${intFmt.format(count)}</span><span class="${cls}">${actual.toFixed(2)}%${cls === "up" ? " ▲" : cls === "down" ? " ▼" : ""}</span>`);
      hist.appendChild(row);
    }
  }

  function renderAll() {
    renderPage();
    if (st.dress) { renderPick(); renderEquipped(); }
    renderSwatches();
    renderPool();
    renderDoll();
    renderRight();
    if (document.activeElement !== $("#bCost")) $("#bCost").value = st.cost ? intFmt.format(st.cost) : "";
    if (document.activeElement !== $("#bName")) $("#bName").value = st.charName || "";
    if (document.activeElement !== $("#bGamePath")) $("#bGamePath").value = st.gamePath || "";
  }

  /* ---------- 事件 ---------- */
  $$("#bPages button").forEach((b) => b.addEventListener("click", () => {
    if (!$("#bActMenu").hidden) actMenu(false); if (!$("#bStanceMenu").hidden) stanceMenu(false); // 換頁時收起動作選單
    st.page = b.dataset.page;
    save(); renderPage();
    if (st.page === "import") checkWz();
  }));
  // 用一般的皇家美髮券／整形券圖示（同名的選擇型排除；說明有「隨機」的優先）
  $$("#bKind button").forEach((b) => b.prepend(ItemIcons.el(b.dataset.kind === "hair" ? "皇家美髮券#隨機|!選擇|!自選|!挑選" : "皇家整形券#隨機|!選擇|!自選|!挑選")));
  $$("#bKind button").forEach((b) => b.addEventListener("click", () => { st.kind = b.dataset.kind; save(); renderPool(); renderRight(); }));
  /* 視窗小時皇家美髮／整形會蓋到女／男：先讓女／男換到下一行，還是不夠就拿掉券的圖示 */
  function fitTabs() {
    const tabs = $("#bPool .b-tabs"), kind = $("#bKind");
    if (!tabs || !tabs.clientWidth) return;
    tabs.classList.remove("stack", "no-ic");
    const gap = parseFloat(getComputedStyle(tabs).columnGap) || 0;
    if (kind.scrollWidth <= kind.clientWidth + Math.max(0, gap - 2)) return;
    tabs.classList.add("stack");
    if (kind.scrollWidth > kind.clientWidth + 1) tabs.classList.add("no-ic");
  }
  if (typeof ResizeObserver !== "undefined") {
    const ro = new ResizeObserver(fitTabs);
    ro.observe($("#bPool .b-tabs"));
    $$("#bKind button").forEach((b) => ro.observe(b));
  }
  $$("#bGender button").forEach((b) => b.addEventListener("click", () => {
    st.gender = b.dataset.g;
    save(); renderPool();
  }));
  $("#bLookup").addEventListener("click", () => lookup(false));
  $("#bName").addEventListener("keydown", (e) => { if (e.key === "Enter") lookup(false); });
  $("#bRestore").addEventListener("click", () => {
    st.tryHair = st.tryFace = st.hairColor = st.faceColor = st.hairMix = st.faceMix = null;
    closeMix();
    st.outfit = null;
    st.skinId = null;
    closeFloat();
    save(); renderSwatches(); renderDoll(); renderRight(); renderPick();
  });
  // 角色框：滾輪放大縮小（以游標為中心）、按住左鍵拖曳移動、按兩下回到預設
  (() => {
    const stage = $("#bStage");
    let drag = null;
    stage.addEventListener("wheel", (e) => {
      if (e.target.closest(".b-chatbar")) return;
      e.preventDefault();
      e.stopPropagation();
      if (e.deltaY === 0) return;
      const old = dollScale();
      const next = Math.min(DOLL_MAX, Math.max(DOLL_MIN, old * (e.deltaY < 0 ? 1.15 : 1 / 1.15)));
      if (next === old) return;
      // 以游標為中心縮放：pan 是使用者的位移，角色置中的部分另外依倍率計算
      const r = stage.getBoundingClientRect();
      const cx = e.clientX - (r.left + r.width / 2);
      const cy = e.clientY - (r.top + r.height / 2);
      const k = next / old;
      pan.x = cx * (1 - k) + pan.x * k;
      pan.y = cy * (1 - k) + pan.y * k;
      st.dollScale = Math.round(next * 1000) / 1000;
      save();
      sizeLook();
    }, { passive: false });
    stage.addEventListener("pointerdown", (e) => {
      if (e.button !== 0 || e.target.closest("button, .b-chatbar")) return;
      drag = { x: e.clientX - pan.x, y: e.clientY - pan.y };
      stage.setPointerCapture(e.pointerId);
      stage.classList.add("dragging");
      e.preventDefault();
    });
    stage.addEventListener("pointermove", (e) => {
      if (!drag) return;
      pan.x = e.clientX - drag.x;
      pan.y = e.clientY - drag.y;
      placeLook();
    });
    const end = () => { drag = null; stage.classList.remove("dragging"); };
    stage.addEventListener("pointerup", end);
    stage.addEventListener("pointercancel", end);
    stage.addEventListener("dblclick", (e) => {
      if (e.target.closest(".b-chatbar")) return;
      pan.x = pan.y = 0; st.dollScale = DOLL_SCALE; save(); sizeLook();
    });
  })();
  $("#bLook").addEventListener("load", sizeLook);
  // 聊天：輸入什麼角色就說什麼，清空輸入框對話框才消失
  $("#bChat").addEventListener("input", (e) => { if (!e.isComposing) say(e.target.value.trim()); });
  $("#bChat").addEventListener("compositionend", (e) => say(e.target.value.trim()));
  $("#bChat").addEventListener("keydown", (e) => {
    if (e.key !== "Enter" || e.isComposing) return;
    e.preventDefault();
    say(e.target.value.trim());
    e.target.blur();
  });
  // 動作：選單選了不會收起，再按一次動作按鈕才收起；右邊的按鈕回到站立
  const setAct = (v) => {
    act = v || "stand";
    const b = $(`#bActMenu [data-v="${act}"]`);
    $("#bActText").textContent = b ? b.textContent : "站立";
    $$("#bActMenu .cbtn").forEach((x) => x.classList.toggle("on", x === b));
    renderDoll();
  };
  // 拿武器的動作（跟遊戲美容美髮的選項一樣）：決定站姿與普通攻擊的動作，不管角色實際拿什麼武器
  const setStance = (v) => {
    st.stance = v || "basic";
    const b = $(`#bStanceMenu [data-v="${st.stance}"]`);
    $("#bStanceText").textContent = b ? b.textContent : "基本動作";
    $$("#bStanceMenu .cbtn").forEach((x) => x.classList.toggle("on", x === b));
    save();
    renderDoll();
  };
  // 選單搬到 body 底下，才不會被紙娃娃框切掉；往下打開，下面放不下時才改往上。
  // 選了項目不會收起，再按一次按鈕才收起
  function dropdown(btnId, menuId) {
    const m = $(menuId), btn = $(btnId);
    document.body.appendChild(m);
    const place = () => {
      if (m.hidden) return;
      const r = btn.getBoundingClientRect();
      const gap = 6, w = m.offsetWidth, h = m.offsetHeight;
      const left = Math.max(8, Math.min(window.innerWidth - w - 8, r.right - w));
      let top = r.bottom + gap;
      if (top + h > window.innerHeight - 8) top = Math.max(8, r.top - gap - h);
      m.style.left = `${left}px`;
      m.style.top = `${top}px`;
    };
    const toggle = (open) => {
      m.hidden = !open;
      btn.classList.toggle("open", open);
      place();
    };
    window.addEventListener("resize", place);
    document.addEventListener("scroll", place, true);
    btn.addEventListener("click", () => toggle(m.hidden));
    document.addEventListener("keydown", (e) => { if (e.key === "Escape" && !m.hidden) toggle(false); });
    // 點選單和按鈕以外的地方就收起
    document.addEventListener("pointerdown", (e) => { if (!m.hidden && !m.contains(e.target) && !btn.contains(e.target)) toggle(false); }, true);
    return toggle;
  }
  const actMenu0 = dropdown("#bAct", "#bActMenu");
  const stanceMenu = dropdown("#bStance", "#bStanceMenu");
  // 一個選單打開時收起另一個
  const actMenu = (open) => { actMenu0(open); if (open) stanceMenu(false); };
  $("#bStance").addEventListener("click", () => { if (!$("#bStanceMenu").hidden) actMenu0(false); });
  $("#bAct").addEventListener("click", () => { if (!$("#bActMenu").hidden) stanceMenu(false); });
  $$("#bActMenu .cbtn").forEach((b) => b.addEventListener("click", () => setAct(b.dataset.v)));
  $$("#bActMenu .cbtn").forEach((b) => b.classList.toggle("on", b.dataset.v === act));
  $$("#bStanceMenu .cbtn").forEach((b) => b.addEventListener("click", () => setStance(b.dataset.v)));
  setStance(st.stance);
  $("#bCost").addEventListener("focus", (e) => { e.target.value = st.cost ? String(st.cost) : ""; e.target.select(); });
  $("#bCost").addEventListener("input", (e) => { st.cost = Math.max(0, parseNum(e.target.value) || 0); save(); });
  $("#bCost").addEventListener("blur", (e) => { e.target.value = st.cost ? intFmt.format(st.cost) : ""; });

  // 按住單抽會連續抽；抽到主打就停下來
  holdable($("#bDraw"), (repeat) => {
    if (drawN(1) === false) return false;
    if (st.last?.feat) {
      if (repeat) toast(`抽到主打「${st.last.name}」，已停止連抽`);
      return false;
    }
  });
  $("#bReset").addEventListener("click", async () => {
    if (!(await confirmBox("確定要重置花費、抽數與保管箱嗎？"))) return;
    st.stats = blankStats(); st.batch = {}; st.hist = {}; st.batchDraws = zero(); st.histDraws = zero(); st.last = null;
    save(); renderRight();
  });
  $("#bClearItems").addEventListener("click", async () => {
    if (!C.POOLS.some((p) => st.pools[p].length)) return;
    if (!(await confirmBox("確定要清除四個獎池的所有造型嗎？"))) return;
    for (const p of C.POOLS) st.pools[p] = [];
    save(); renderAll();
  });
  /** 套用解析結果（貼上解析與官網更新共用） */
  function applyPools(parsed) {
    const got = C.POOLS.filter((p) => parsed[p] && parsed[p].length);
    if (!got.length) return null;
    for (const p of got) st.pools[p] = parsed[p];
    save(); renderAll();
    return got.map((p) => `${C.POOL_NAME[p]} ${parsed[p].length} 項`).join("、");
  }
  $("#bParse").addEventListener("click", () => {
    const msg = applyPools(C.parse($("#bPaste").value));
    if (!msg) { $("#bParseResult").textContent = "沒有解析到任何「名稱 + 機率%」的造型"; return; }
    $("#bParseResult").textContent = `已套用：${msg}`;
    toast(`已套用：${msg}`);
  });
  OfficialPools.register("beauty", { card: $("#bPaste").closest(".card"), parse: C.parse, apply: applyPools });
  async function checkWz() {
    if (!TAURI) return;
    const box = $("#bWzStatus");
    box.textContent = "讀取遊戲資料中…";
    const r = await invoke("beauty_wz_status", { gameDir: st.gamePath || null });
    box.textContent = r.ok ? `${r.message}（${r.path}）` : r.message;
    box.classList.toggle("bad", !r.ok);
    if (r.ok) { renderCache.clear(); renderDoll(); renderRight(); }
  }
  let wzTimer;
  $("#bGamePath").addEventListener("input", (e) => {
    st.gamePath = e.target.value.trim();
    save();
    clearTimeout(wzTimer);
    wzTimer = setTimeout(checkWz, 600);
  });
  $("#bPickGame").addEventListener("click", async () => {
    if (!TAURI) return;
    const dir = await invoke("pick_game_dir");
    if (!dir) return;
    st.gamePath = dir;
    $("#bGamePath").value = dir;
    save();
    checkWz();
  });

  /* ---------- 變更點裝 ---------- */
  const SLOTS = [
    ["All", "全部", "c-silver"],
    ["Cap", "帽子", "c-pink"], ["Face", "臉飾", "c-purple"], ["Eye", "眼飾", "c-blue"], ["Ear", "耳環", "c-amber"],
    ["Coat", "上衣", "c-orange"], ["Longcoat", "套服", "c-red"], ["Pants", "褲裙", "c-teal"], ["Shoes", "鞋子", "c-green"],
    ["Glove", "手套", "c-silver"], ["Cape", "披風", "c-purple"], ["Shield", "輔助武器", "c-blue"], ["Weapon", "武器", "c-orange"],
    ["RingTag", "名牌戒指", "c-indigo"], ["RingChat", "聊天戒指", "c-pink"],
  ];
  // 名牌戒指、聊天戒指各算一格（可以同時戴）；兩種都從同一份戒指清單分出來
  const SLOT_NAME = Object.fromEntries(SLOTS.map(([k, n]) => [k, n]));
  const listKey = (slot) => (/^Ring/.test(slot) ? "Ring" : slot);
  const isRing = (it) => /^Ring/.test(it?.slot || "");
  const itemLists = {}; // 欄位 → 道具清單（Rust 讀完後快取）
  const iconCache = new Map(); // 編號 → 圖示 data URL（null = 沒有圖示）
  const PAGE = 240;
  let shown = PAGE;
  const plain = ({ id, name, slot, dye }) => (dye ? { id, name, slot, dye: { ...dye } } : { id, name, slot });

  /** 目前穿著的點裝：有自訂就用自訂，否則是角色原本的套組 */
  const curOutfit = () => st.outfit || lastEquips.map(plain);

  function setOutfit(list) {
    st.outfit = list;
    save();
    renderDoll();
    renderEquipped();
    markIcons();
  }

  /** 穿上：同一個欄位只能有一件；套服和上衣、褲裙互斥。toggle = 已穿著時脫下 */
  function wear(item, toggle = true) {
    const cur = curOutfit();
    if (cur.some((x) => x.id === item.id)) {
      if (toggle) setOutfit(cur.filter((x) => x.id !== item.id));
      return;
    }
    const clash = { Longcoat: ["Coat", "Pants"], Coat: ["Longcoat"], Pants: ["Longcoat"] }[item.slot] || [];
    const list = cur.filter((x) => x.slot !== item.slot && !clash.includes(x.slot));
    list.push(plain(item));
    setOutfit(list);
  }

  const isFav = (id) => st.favs.some((x) => x.id === id);
  function addFav(item) {
    if (isFav(item.id)) return toast("已經在收藏裡了");
    st.favs.unshift(plain(item));
    save(); renderSaved();
    toast(`已收藏「${item.name}」`);
  }
  function removeFav(item) {
    st.favs = st.favs.filter((x) => x.id !== item.id);
    save(); renderSaved();
  }

  /* 圖示：進入畫面的才向 Rust 要，一次要一批 */
  let iconWant = new Set(), iconTimer = null, iconBusy = false;
  const io = typeof IntersectionObserver !== "undefined" ? new IntersectionObserver((ents) => {
    for (const e of ents) if (e.isIntersecting) { iconWant.add(Number(e.target.dataset.id)); io.unobserve(e.target); }
    clearTimeout(iconTimer);
    iconTimer = setTimeout(loadIcons, 40);
  }, { root: null, rootMargin: "120px" }) : null;

  async function loadIcons() {
    if (iconBusy || !TAURI) return;
    const ids = [...iconWant].filter((id) => !iconCache.has(id)).slice(0, 60);
    ids.forEach((id) => iconWant.delete(id));
    if (!ids.length) return;
    iconBusy = true;
    try {
      const pngs = await invoke("beauty_icons", { gameDir: st.gamePath || null, ids });
      ids.forEach((id, i) => iconCache.set(id, pngs[i] || null));
    } catch (_) { ids.forEach((id) => iconCache.set(id, null)); }
    iconBusy = false;
    $$("#view-beauty [data-id]:not([data-done])").forEach(fillIcon);
    if (iconWant.size) setTimeout(loadIcons, 0);
  }

  function fillIcon(el) {
    const id = Number(el.dataset.id);
    if (!iconCache.has(id) || el.dataset.done) return;
    el.dataset.done = "1";
    const png = iconCache.get(id);
    if (png) { const img = new Image(); img.src = png; img.alt = ""; img.draggable = false; el.prepend(img); }
    else el.classList.add("noicon");
  }

  /* ---------- 拖曳：拖到收藏 = 收藏；拖到角色或目前裝備 = 穿上 ---------- */
  let suppressClick = false;
  function draggable(el, item, from) {
    el.addEventListener("pointerdown", (e) => {
      if (e.button !== 0) return;
      const sx = e.clientX, sy = e.clientY;
      let ghost = null, over = null;
      const zoneOf = (x, y) => {
        const t = document.elementFromPoint(x, y);
        if (!t) return null;
        if (t.closest(".b-saved")) return from === "fav" ? null : "fav";
        if (from === "fav" && t.closest("#bPick")) return "unfav";
        if (t.closest(".b-equipped") || t.closest("#bStage")) return "wear";
        return null;
      };
      const mark = (z) => {
        if (z === over) return;
        over = z;
        $(".b-saved")?.classList.toggle("drop", z === "fav");
        if (z === "fav") setFolder("outfit");
        $("#bPick")?.classList.toggle("drop-del", z === "unfav");
        $(".b-equipped")?.classList.toggle("drop", z === "wear");
        $("#bStage").classList.toggle("drop", z === "wear");
      };
      const move = (ev) => {
        if (!ghost) {
          if (Math.hypot(ev.clientX - sx, ev.clientY - sy) < 6) return;
          ghost = document.createElement("div");
          ghost.className = "b-ghost";
          const img = el.querySelector("img");
          if (img) ghost.append(img.cloneNode());
          else ghost.textContent = item.name.slice(0, 2);
          document.body.append(ghost);
          document.body.classList.add("b-dragging");
        }
        ghost.style.transform = `translate(${ev.clientX - 22}px, ${ev.clientY - 22}px)`;
        mark(zoneOf(ev.clientX, ev.clientY));
      };
      const up = (ev) => {
        window.removeEventListener("pointermove", move);
        window.removeEventListener("pointerup", up);
        if (!ghost) return;
        ghost.remove();
        document.body.classList.remove("b-dragging");
        const z = zoneOf(ev.clientX, ev.clientY);
        mark(null);
        suppressClick = true;
        setTimeout(() => (suppressClick = false), 0);
        if (z === "fav") addFav(item);
        else if (z === "unfav") { removeFav(item); toast(`已移除收藏「${item.name}」`); }
        else if (z === "wear") wear(item, false);
      };
      window.addEventListener("pointermove", move);
      window.addEventListener("pointerup", up);
    });
    el.addEventListener("click", (e) => { if (suppressClick) { e.stopImmediatePropagation(); e.preventDefault(); } }, true);
    el.addEventListener("contextmenu", (e) => { e.preventDefault(); openCtx(e.clientX, e.clientY, item); });
  }

  /* ---------- 右鍵選單 ---------- */
  function openCtx(x, y, item) {
    const worn = curOutfit().some((o) => o.id === item.id);
    const opts = [
      [isFav(item.id) ? "取消收藏" : "收藏", () => (isFav(item.id) ? removeFav(item) : addFav(item))],
      [worn ? "脫下" : "穿上", () => wear(item, true)],
    ];
    if (worn && !isRing(item)) opts.push(["染色", () => openDye(item.id)]);
    showMenu(x, y, item.name, opts);
  }
  function showMenu(x, y, name, opts) {
    if (!$("#bActMenu").hidden) actMenu(false); if (!$("#bStanceMenu").hidden) stanceMenu(false); // 叫出其他介面時收起動作選單
    const m = $("#bCtx");
    m.innerHTML = "";
    const title = document.createElement("div");
    title.className = "b-ctx-title";
    title.textContent = name;
    // 名稱點一下就複製（選單不會關掉，方便再選別的）
    makeCopyable(title, name);
    title.title = `${name}（點一下複製名稱）`;
    m.append(title);
    for (const [label, fn] of opts) {
      const b = document.createElement("button");
      b.textContent = label;
      b.addEventListener("click", () => { closeCtx(); fn(); });
      m.append(b);
    }
    m.hidden = false;
    const r = m.getBoundingClientRect();
    m.style.left = `${Math.min(x, innerWidth - r.width - 8)}px`;
    m.style.top = `${Math.min(y, innerHeight - r.height - 8)}px`;
  }
  const closeCtx = () => { $("#bCtx").hidden = true; };
  document.addEventListener("pointerdown", (e) => { if (!e.target.closest("#bCtx")) closeCtx(); });
  document.addEventListener("keydown", (e) => { if (e.key === "Escape") { closeCtx(); closeFloat(); } });
  window.addEventListener("blur", closeCtx);

  function iconEl(item, tag = "button", from = "") {
    const el = document.createElement(tag);
    el.className = "b-icon";
    el.dataset.id = item.id;
    el.title = item.name;
    if (iconCache.has(item.id)) fillIcon(el); else io?.observe(el);
    draggable(el, item, from);
    return el;
  }

  function markIcons() {
    const on = new Set(curOutfit().map((x) => x.id));
    $$("#bIcons .b-icon, #bSaved .b-icon").forEach((el) => el.classList.toggle("on", on.has(Number(el.dataset.id))));
  }

  function filtered() {
    const ring = /^Ring/.test(st.pickSlot);
    const list = (itemLists[listKey(st.pickSlot)] || []).filter((it) => !ring || it.slot === st.pickSlot);
    const q = ($("#bPickSearch").value || "").trim();
    return list.filter((it) =>
      (!st.pickCash || it.cash) &&
      (!q || it.name.includes(q) || String(it.id).includes(q)) &&
      (!st.pickG || (() => { const g = Math.floor(it.id / 1000) % 10; return g > 1 || (st.pickG === "F" ? g === 1 : g === 0); })())
    );
  }

  function pickIcon(it) {
    const el = iconEl(it);
    el.addEventListener("mouseenter", () => showHover(it));
    el.addEventListener("click", () => wear(it));
    return el;
  }

  async function renderPick() {
    if (!st.dress) return;
    const slots = $("#bSlots");
    if (slots.childElementCount !== SLOTS.length) {
      slots.innerHTML = "";
      for (const [k, n, c] of SLOTS) {
        const b = document.createElement("button");
        b.className = `cbtn ${c}`;
        b.dataset.slot = k;
        b.textContent = n;
        b.addEventListener("click", () => { st.pickSlot = k; shown = PAGE; save(); renderPick(); });
        slots.appendChild(b);
      }
    }
    $$("#bSlots button").forEach((b) => b.classList.toggle("on", b.dataset.slot === st.pickSlot));
    $$("#bPickG button").forEach((b) => b.classList.toggle("on", b.dataset.g === (st.pickG || "")));
    $("#bPickCash").classList.toggle("on", !!st.pickCash);
    const box = $("#bIcons");
    if (!TAURI) { box.innerHTML = '<div class="g-empty">預覽模式無法讀取遊戲資料</div>'; return; }
    if (!itemLists[listKey(st.pickSlot)]) {
      box.innerHTML = `<div class="g-empty">讀取中…${st.pickSlot === "All" ? "（第一次讀取全部分類需要一點時間）" : ""}</div>`;
      $("#bPickCount").textContent = "";
      const slot = st.pickSlot, key = listKey(slot);
      try { itemLists[key] = await invoke("beauty_items", { gameDir: st.gamePath || null, slot: key }); }
      catch (e) { box.innerHTML = `<div class="g-empty">${String(e)}</div>`; return; }
      if (slot !== st.pickSlot) return;
    }
    const list = filtered();
    box.innerHTML = "";
    $("#bPickCount").textContent = `${list.length} 件`;
    if (!list.length) { box.innerHTML = '<div class="g-empty">沒有符合的道具</div>'; return; }
    const frag = document.createDocumentFragment();
    for (const it of list.slice(0, shown)) frag.appendChild(pickIcon(it));
    box.appendChild(frag);
    pickMore(box, list);
    markIcons();
  }

  /** 捲到最下面自動載入更多（接在後面，不重畫已經顯示的） */
  let pickSentinelIO = null;
  function pickMore(box, list) {
    pickSentinelIO?.disconnect();
    if (list.length <= shown) return;
    const more = document.createElement("div");
    more.className = "b-more muted";
    more.textContent = `往下捲動載入更多（還有 ${list.length - shown} 件）`;
    box.appendChild(more);
    pickSentinelIO = new IntersectionObserver((ents) => {
      if (!ents.some((e) => e.isIntersecting)) return;
      pickSentinelIO.disconnect();
      more.remove();
      const frag = document.createDocumentFragment();
      for (const it of list.slice(shown, shown + PAGE)) frag.appendChild(pickIcon(it));
      shown += PAGE;
      box.appendChild(frag);
      markIcons();
      pickMore(box, list);
    }, { root: box, rootMargin: "300px" });
    pickSentinelIO.observe(more);
  }

  const PALETTE_SVG = '<svg viewBox="0 0 24 24"><path d="M12 3a9 9 0 0 0 0 18c1.1 0 1.6-.8 1.6-1.6 0-.4-.2-.8-.4-1.1-.3-.3-.4-.7-.4-1.1 0-.9.7-1.6 1.6-1.6H16a5 5 0 0 0 5-5C21 6.6 17 3 12 3Z"/><circle cx="7.5" cy="11" r="1.3"/><circle cx="10" cy="7.3" r="1.3"/><circle cx="14.5" cy="7.3" r="1.3"/><circle cx="17" cy="11" r="1.3"/></svg>';

  function renderEquipped() {
    const box = $("#bEqList");
    box.innerHTML = "";
    const list = curOutfit();
    if (!list.length) box.innerHTML = '<div class="g-empty">目前沒有穿任何點裝</div>';
    const order = Object.fromEntries(SLOTS.map(([k], i) => [k, i]));
    for (const it of [...list].sort((a, b) => (order[a.slot] ?? 99) - (order[b.slot] ?? 99))) {
      const row = document.createElement("div");
      row.className = "b-eq-row";
      const ic = iconEl(it, "span");
      const txt = document.createElement("div");
      txt.className = "b-eq-text";
      txt.innerHTML = `<span></span><small class="muted"></small>`;
      txt.children[0].textContent = it.name;
      txt.children[1].textContent = (SLOT_NAME[it.slot] || "") + (it.dye ? "・已染色" : "");
      const dye = document.createElement("button");
      dye.className = "b-dye-btn" + (it.dye ? " on" : "");
      dye.title = "染色";
      dye.innerHTML = PALETTE_SVG;
      dye.addEventListener("click", () => openDye(it.id));
      if (isRing(it)) dye.style.visibility = "hidden"; // 戒指不能染色
      const del = document.createElement("button");
      del.className = "b-x";
      del.title = "脫下";
      del.textContent = "×";
      del.addEventListener("click", () => setOutfit(curOutfit().filter((x) => x.id !== it.id)));
      row.append(ic, txt, dye, del);
      box.appendChild(row);
    }
    renderSaved();
  }

  /* 收藏資料夾：時裝／外觀 */
  function setFolder(f) {
    if (st.savedTab === f) return;
    st.savedTab = f;
    save();
    syncFolder();
    if (f === "look") renderLookFavs();
  }
  function syncFolder() {
    const f = st.savedTab === "look" ? "look" : "outfit";
    $$("#bFolder .b-ftab").forEach((b) => b.classList.toggle("on", b.dataset.f === f));
    $("#bFolder").classList.toggle("look", f === "look");
    $("#bFOutfit").hidden = f !== "outfit";
    $("#bFLook").hidden = f !== "look";
  }
  $$("#bFolder .b-ftab").forEach((b) => b.addEventListener("click", () => setFolder(b.dataset.f)));

  function renderSaved() {
    syncFolder();
    if (st.savedTab === "look") renderLookFavs();
    $("#bSavedCount").textContent = st.favs.length ? `${st.favs.length} 件` : "";
    // 有收藏之後就不用再提示怎麼收藏
    $("#bFavHint").hidden = st.favs.length > 0;
    const box = $("#bSaved");
    box.innerHTML = "";
    if (!st.favs.length) { box.innerHTML = '<div class="g-empty">還沒有收藏的時裝</div>'; return; }
    for (const it of st.favs) {
      const el = iconEl(it, "button", "fav");
      el.addEventListener("click", () => wear(it));
      box.appendChild(el);
    }
    markIcons();
  }

  /* ---------- 浮動視窗：染色、膚色 ---------- */
  function closeFloat() { $("#bDyeWin").hidden = true; closeMix(); if (styleKind) closeStyle(); dyeId = null; }

  const RANGES = [["全色系", "c-silver"], ["紅色系", "c-red"], ["黃色系", "c-amber"], ["綠色系", "c-green"], ["青色系", "c-teal"], ["藍色系", "c-blue"], ["紫色系", "c-purple"]];
  let dyeId = null;
  function dyeItem() { return curOutfit().find((x) => x.id === dyeId); }
  function openDye(id) {
    if (!$("#bActMenu").hidden) actMenu(false); if (!$("#bStanceMenu").hidden) stanceMenu(false); // 叫出其他介面時收起動作選單
    if (!st.outfit) st.outfit = curOutfit(); // 先把角色原本的套組變成可以修改的清單
    dyeId = id;
    const it = dyeItem();
    if (!it) return;
    if (styleKind) closeStyle();
    closeMix();
    $("#bDyeTitle").textContent = `染色：${it.name}`;
    const box = $("#bDyeRanges");
    if (!box.childElementCount) {
      RANGES.forEach(([n, c], i) => {
        const b = document.createElement("button");
        b.className = `cbtn small ${c}`;
        b.textContent = n;
        b.dataset.r = i;
        b.addEventListener("click", () => setDye({ range: i }));
        box.appendChild(b);
      });
    }
    $("#bDyeWin").hidden = false;
    syncDye();
  }
  function syncDye() {
    const d = dyeItem()?.dye || { range: 0, hue: 0, sat: 0, val: 0 };
    $$("#bDyeRanges button").forEach((b) => b.classList.toggle("on", Number(b.dataset.r) === (d.range || 0)));
    for (const [k, el] of [["hue", "Hue"], ["sat", "Sat"], ["val", "Val"]]) {
      const v = d[k] || 0;
      if (document.activeElement !== $(`#bDye${el}`)) $(`#bDye${el}`).value = v;
      $(`#bDye${el}V`).textContent = k === "hue" ? `${v}°` : v > 0 ? `+${v}` : `${v}`;
    }
  }
  function setDye(patch) {
    const it = dyeItem();
    if (!it) return;
    const d = { range: 0, hue: 0, sat: 0, val: 0, ...(it.dye || {}), ...patch };
    const empty = !d.hue && !d.sat && !d.val;
    st.outfit = curOutfit().map((x) => (x.id === it.id ? (empty && !d.range ? plain({ ...x, dye: null }) : { ...x, dye: d }) : x));
    save();
    syncDye();
    renderDoll();
    clearTimeout(setDye.t);
    setDye.t = setTimeout(renderEquipped, 250);
  }
  $("#bDyeHue").addEventListener("input", (e) => setDye({ hue: Number(e.target.value) }));
  $("#bDyeSat").addEventListener("input", (e) => setDye({ sat: Number(e.target.value) }));
  $("#bDyeVal").addEventListener("input", (e) => setDye({ val: Number(e.target.value) }));
  $("#bDyeReset").addEventListener("click", () => setDye({ range: 0, hue: 0, sat: 0, val: 0 }));
  $("#bDyeDone").addEventListener("click", closeFloat);
  $("#bDyeClose").addEventListener("click", closeFloat);

  /* ---------- 膚色／髮型／臉型 大視窗 ---------- */
  let skins = null;
  const styleLists = {}; // hair / face → 清單
  const thumbCache = new Map();
  let styleKind = null, styleGen = 0, styleShown = 0;
  const STYLE_PAGE = 120;
  const STYLE_TITLE = { skin: "變更膚色", hair: "變更髮型", face: "變更臉型" };
  const famOf = (kind, id) => (kind === "hair" ? Math.floor(id / 10) * 10 : kind === "face" ? id - (Math.floor(id / 100) % 10) * 100 : id);

  async function openStyle(kind) {
    if (!$("#bActMenu").hidden) actMenu(false); if (!$("#bStanceMenu").hidden) stanceMenu(false); // 叫出其他介面時收起動作選單
    closeCtx();
    $("#bDyeWin").hidden = true;
    closeMix();
    styleKind = kind;
    styleShown = STYLE_PAGE;
    $("#bStyleTitle").textContent = STYLE_TITLE[kind];
    $("#bStyleG").hidden = kind === "skin";
    // 搜尋文字依視窗種類記住，關掉再開也還在（按搜尋框右邊的叉叉才清除）
    $("#bStyleSearch").value = (st.styleSearch ||= {})[kind] || "";
    syncSearchX();
    $("#bStyleWin").hidden = false;
    const grid = $("#bStyleGrid");
    if (!TAURI) { grid.innerHTML = '<div class="g-empty">預覽模式無法讀取遊戲資料</div>'; return; }
    const my = ++styleGen;
    try {
      if (kind === "skin" && !skins) { grid.innerHTML = '<div class="g-empty">讀取中…</div>'; skins = await invoke("beauty_skins", { gameDir: st.gamePath || null }); }
      if (kind !== "skin" && !styleLists[kind]) { grid.innerHTML = '<div class="g-empty">讀取中…</div>'; styleLists[kind] = await invoke("beauty_styles", { gameDir: st.gamePath || null, kind }); }
    } catch (e) { grid.innerHTML = `<div class="g-empty">${String(e)}</div>`; return; }
    if (my === styleGen) renderStyles();
  }
  function closeStyle() { $("#bStyleWin").hidden = true; styleKind = null; styleGen++; thumbWant.length = 0; }

  /* 髮型／臉型／膚色的收藏（依造型系列記，不分顏色） */
  const styleFavs = (kind) => (st.styleFavs[kind] ||= []);
  const isStyleFav = (kind, id) => styleFavs(kind).includes(famOf(kind, id));
  function toggleStyleFav(kind, it) {
    const fam = famOf(kind, it.id);
    const list = styleFavs(kind);
    const i = list.indexOf(fam);
    if (i >= 0) list.splice(i, 1); else list.unshift(fam);
    save();
    toast(i >= 0 ? `已取消收藏「${it.name}」` : `已收藏「${it.name}」`);
    $$("#bStyleGrid .b-style-card").forEach((c) => c.classList.toggle("fav", isStyleFav(kind, Number(c.dataset.id))));
    if (st.styleFavOnly) renderStyles();
    if (st.savedTab === "look") renderLookFavs();
  }

  /* ---------- 收藏的外觀：髮型、臉型、膚色 ---------- */
  const LOOK_KINDS = [["hair", "髮型"], ["face", "臉型"], ["skin", "膚色"]];
  async function styleList(kind) {
    if (!TAURI) return [];
    if (kind === "skin") return (skins ||= await invoke("beauty_skins", { gameDir: st.gamePath || null }));
    return (styleLists[kind] ||= await invoke("beauty_styles", { gameDir: st.gamePath || null, kind }));
  }
  function curIdOf(kind) {
    if (kind === "skin") return st.skinId || lastSkinId;
    if (kind === "hair") return st.tryHair?.id || lastHairId;
    return st.tryFace?.id || lastFaceId;
  }
  function markLookFavs() {
    $$("#bLookFavs .b-lf-card").forEach((c) => {
      const cur = curIdOf(c.dataset.kind);
      c.classList.toggle("on", !!cur && famOf(c.dataset.kind, cur) === Number(c.dataset.fam));
    });
  }
  // 外觀卡片的頭像（只畫頭，不穿點裝），依序一張一張產生
  const lfWant = [];
  let lfBusy = false;
  async function pumpLookThumbs() {
    if (lfBusy) return;
    lfBusy = true;
    while (lfWant.length) {
      const card = lfWant.shift();
      if (!card.isConnected) continue;
      const k = card.dataset.req;
      let png = thumbCache.get(k);
      if (png === undefined) {
        try { png = (await invoke("beauty_render", { req: JSON.parse(k) })).png; } catch (_) { png = null; }
        thumbCache.set(k, png);
      }
      const box = card.querySelector(".b-lf-pic");
      box.innerHTML = "";
      if (png) { const img = new Image(); img.src = png; img.alt = ""; box.append(img); }
      else box.innerHTML = '<span class="muted">—</span>';
    }
    lfBusy = false;
  }
  let lfGen = 0;
  async function renderLookFavs() {
    const box = $("#bLookFavs");
    const my = ++lfGen;
    const total = LOOK_KINDS.reduce((n, [k]) => n + styleFavs(k).length, 0);
    if (!total) {
      box.innerHTML = '<div class="g-empty">還沒有收藏的外觀。在髮型、臉型、膚色視窗按卡片右上角的愛心就會收藏到這裡</div>';
      return;
    }
    const lists = {};
    for (const [k] of LOOK_KINDS) {
      if (!styleFavs(k).length) continue;
      try { lists[k] = await styleList(k); } catch (_) { lists[k] = []; }
    }
    if (my !== lfGen) return;
    box.innerHTML = "";
    lfWant.length = 0;
    for (const [kind, label] of LOOK_KINDS) {
      const favsK = styleFavs(kind);
      if (!favsK.length) continue;
      const group = document.createElement("div");
      group.className = "b-lf-group";
      const head = document.createElement("div");
      head.className = "b-lf-head";
      head.innerHTML = `<span>${label}</span><span class="muted">${favsK.length}</span>`;
      const more = document.createElement("button");
      more.className = "link-btn";
      more.textContent = "全部" + label;
      more.addEventListener("click", () => openStyle(kind));
      head.append(more);
      const grid = document.createElement("div");
      grid.className = "b-lf-grid";
      for (const fam of favsK) {
        const it = (lists[kind] || []).find((x) => famOf(kind, x.id) === fam) || { id: fam, name: String(fam) };
        const card = document.createElement("button");
        card.className = "b-lf-card";
        card.dataset.kind = kind;
        card.dataset.fam = fam;
        card.title = `${it.name}（${it.id}）`;
        const req = thumbReq(kind, it);
        req.mode = "head";
        card.dataset.req = JSON.stringify(req);
        const pic = document.createElement("div");
        pic.className = "b-lf-pic";
        const nm = document.createElement("span");
        nm.className = "b-lf-name";
        nm.textContent = it.name;
        card.append(pic, nm);
        card.addEventListener("click", () => { pickStyle(kind, it); markLookFavs(); });
        card.addEventListener("contextmenu", (e) => {
          e.preventDefault();
          showMenu(e.clientX, e.clientY, it.name, [["套用", () => { pickStyle(kind, it); markLookFavs(); }], ["取消收藏", () => toggleStyleFav(kind, it)]]);
        });
        grid.append(card);
        if (TAURI) lfWant.push(card); else pic.innerHTML = '<span class="muted">—</span>';
      }
      group.append(head, grid);
      box.append(group);
    }
    markLookFavs();
    pumpLookThumbs();
  }

  function styleItems() {
    const list = styleKind === "skin" ? skins || [] : styleLists[styleKind] || [];
    const q = $("#bStyleSearch").value.trim();
    let out = q ? list.filter((x) => x.name.includes(q) || String(x.id).includes(q)) : list;
    // 全部／女／男（聯名等共用的兩邊都列出）
    if (styleKind !== "skin" && st.styleG) out = out.filter((x) => !x.g || x.g === st.styleG);
    if (st.styleFavOnly) out = out.filter((x) => isStyleFav(styleKind, x.id));
    else if (styleFavs(styleKind).length) {
      // 收藏的排在最前面
      out = [...out.filter((x) => isStyleFav(styleKind, x.id)), ...out.filter((x) => !isStyleFav(styleKind, x.id))];
    }
    return out;
  }
  const THUMB_SCALE = { l: 2, m: 1.5, s: 1 };
  function applyStyleSize() {
    const size = st.styleSize || "m";
    $("#bStyleGrid").className = `b-style-grid size-${size}`;
    $$("#bStyleSize button").forEach((b) => b.classList.toggle("on", b.dataset.size === size));
    $("#bStyleFavOnly").classList.toggle("on", !!st.styleFavOnly);
    $("#bStyleG").hidden = styleKind === "skin";
    $$("#bStyleG button").forEach((b) => b.classList.toggle("on", b.dataset.g === (st.styleG || "")));
    $$("#bStyleGrid .b-style-pic img").forEach((img) => { if (img.naturalWidth) img.style.width = `${img.naturalWidth * THUMB_SCALE[size]}px`; });
  }
  function currentStyleId() {
    if (styleKind === "skin") return st.skinId || lastSkinId;
    if (styleKind === "hair") return st.tryHair?.id || lastHairId;
    return st.tryFace?.id || lastFaceId;
  }

  /** 預覽圖：不穿點裝的全身，只換這一項 */
  function thumbReq(kind, item) {
    const req = lookReq(st.tryHair, st.tryFace, "full");
    req.items = [];
    if (kind === "skin") req.skin_id = item.id;
    else if (req[kind]) req[kind] = { ...req[kind], id: item.id, name: item.name, gender: req.gender };
    else req[kind] = { id: item.id, name: item.name, gender: req.gender };
    return req;
  }

  const thumbWant = [];
  let thumbBusy = false;
  const thumbIO = typeof IntersectionObserver !== "undefined" ? new IntersectionObserver((ents) => {
    for (const e of ents) if (e.isIntersecting) { thumbWant.push(e.target); thumbIO.unobserve(e.target); }
    pumpThumbs();
  }, { root: null, rootMargin: "200px" }) : null;

  async function pumpThumbs() {
    if (thumbBusy) return;
    thumbBusy = true;
    while (thumbWant.length) {
      const card = thumbWant.shift();
      if (!card.isConnected || styleKind == null) continue;
      const req = JSON.parse(card.dataset.req);
      const k = card.dataset.req;
      let png = thumbCache.get(k);
      if (png === undefined) {
        try { png = (await invoke("beauty_render", { req })).png; } catch (_) { png = null; }
        thumbCache.set(k, png);
        if (thumbCache.size > 600) thumbCache.delete(thumbCache.keys().next().value);
      }
      putThumb(card, png);
    }
    thumbBusy = false;
  }
  function putThumb(card, png) {
    const box = card.querySelector(".b-style-pic");
    box.innerHTML = "";
    if (png) { const img = new Image(); img.src = png; img.alt = ""; img.onload = () => { img.style.width = `${img.naturalWidth * THUMB_SCALE[st.styleSize || "m"]}px`; }; box.append(img); }
    else box.innerHTML = '<span class="muted">無法顯示</span>';
  }

  function renderStyles() {
    const grid = $("#bStyleGrid");
    grid.innerHTML = "";
    thumbWant.length = 0;
    applyStyleSize();
    const list = styleItems();
    $("#bStyleCount").textContent = `${list.length} 種`;
    if (!list.length) { grid.innerHTML = `<div class="g-empty">${st.styleFavOnly ? "還沒有收藏，按卡片右上角的愛心或右鍵選「收藏」" : "沒有符合的項目"}</div>`; return; }
    const cur = currentStyleId();
    const kind = styleKind;
    const frag = document.createDocumentFragment();
    for (const it of list.slice(0, styleShown)) {
      const card = document.createElement("button");
      card.className = "b-style-card" + (cur && famOf(kind, cur) === famOf(kind, it.id) ? " on" : "") + (isStyleFav(kind, it.id) ? " fav" : "");
      card.title = `${it.name}（${it.id}）`;
      card.dataset.id = it.id;
      const req = thumbReq(kind, it);
      card.dataset.req = JSON.stringify(req);
      const pic = document.createElement("div");
      pic.className = "b-style-pic";
      if (kind === "skin" && it.color) pic.style.setProperty("--skin", it.color);
      const nm = document.createElement("span");
      nm.className = "b-style-name";
      nm.textContent = it.name;
      const heart = document.createElement("span");
      heart.className = "b-heart";
      heart.title = "收藏／取消收藏";
      heart.innerHTML = '<svg viewBox="0 0 24 24"><path d="M12 20.5s-7.5-4.6-7.5-10.2A4.3 4.3 0 0 1 12 7.6a4.3 4.3 0 0 1 7.5 2.7c0 5.6-7.5 10.2-7.5 10.2Z"/></svg>';
      heart.addEventListener("click", (e) => { e.stopPropagation(); toggleStyleFav(kind, it); });
      card.append(pic, nm, heart);
      card.addEventListener("contextmenu", (e) => {
        e.preventDefault();
        showMenu(e.clientX, e.clientY, it.name, [
          [isStyleFav(kind, it.id) ? "取消收藏" : "收藏", () => toggleStyleFav(kind, it)],
          ["套用", () => pickStyle(kind, it)],
        ]);
      });
      const cached = thumbCache.get(card.dataset.req);
      if (cached !== undefined) putThumb(card, cached); else thumbIO?.observe(card);
      card.addEventListener("click", () => pickStyle(kind, it));
      frag.append(card);
    }
    if (list.length > styleShown) {
      const more = document.createElement("div");
      more.className = "b-style-more muted";
      more.textContent = "往下捲動載入更多…";
      frag.append(more);
      const sentinel = new IntersectionObserver((ents) => {
        if (ents.some((e) => e.isIntersecting)) { sentinel.disconnect(); styleShown += STYLE_PAGE; renderStyles(); }
      });
      setTimeout(() => sentinel.observe(more), 50);
    }
    grid.append(frag);
  }

  function pickStyle(kind, it) {
    if (kind === "skin") st.skinId = it.id;
    else if (kind === "hair") st.tryHair = { pool: "hair" + gender(), name: it.name, id: it.id };
    else st.tryFace = { pool: "face" + gender(), name: it.name, id: it.id };
    save();
    renderDoll();
    renderRight();
    const cur = famOf(kind, it.id);
    $$("#bStyleGrid .b-style-card").forEach((c) => c.classList.toggle("on", famOf(kind, Number(c.dataset.id)) === cur));
  }

  let styleSearchTimer;
  function syncSearchX() { $("#bStyleSearchX").hidden = !$("#bStyleSearch").value; }
  function searchChanged() {
    syncSearchX();
    if (styleKind) { (st.styleSearch ||= {})[styleKind] = $("#bStyleSearch").value; save(); }
    clearTimeout(styleSearchTimer);
    styleSearchTimer = setTimeout(() => { styleShown = STYLE_PAGE; renderStyles(); }, 200);
  }
  $("#bStyleSearch").addEventListener("input", searchChanged);
  $("#bStyleSearchX").addEventListener("click", () => { $("#bStyleSearch").value = ""; searchChanged(); $("#bStyleSearch").focus(); });
  $("#bStyleClose").addEventListener("click", closeStyle);
  $("#bStyleWin").addEventListener("pointerdown", (e) => { if (e.target === $("#bStyleWin")) closeStyle(); });
  $$("#bStyleSize button").forEach((b) => b.addEventListener("click", () => { st.styleSize = b.dataset.size; save(); applyStyleSize(); }));
  $("#bStyleFavOnly").addEventListener("click", () => { st.styleFavOnly = !st.styleFavOnly; styleShown = STYLE_PAGE; save(); renderStyles(); });
  $$("#bStyleG button").forEach((b) => b.addEventListener("click", () => { st.styleG = b.dataset.g; styleShown = STYLE_PAGE; save(); renderStyles(); }));

  $$("#bPickG button").forEach((b) => b.addEventListener("click", () => { st.pickG = b.dataset.g; shown = PAGE; save(); renderPick(); }));
  $("#bPickCash").addEventListener("click", () => { st.pickCash = !st.pickCash; shown = PAGE; save(); renderPick(); });
  let searchTimer;
  $("#bPickSearch").addEventListener("input", () => { clearTimeout(searchTimer); searchTimer = setTimeout(() => { shown = PAGE; renderPick(); }, 200); });
  $("#bOutfitClear").addEventListener("click", () => { closeFloat(); setOutfit([]); });

  /* ---------- 收放：選擇點裝、目前裝備 ---------- */
  function renderFold() {
    for (const [key, card, btn] of [["foldPick", "#bPick", "#bPickFold"], ["foldEq", ".b-equipped", "#bEqFold"]]) {
      const el = document.querySelector(card), b = $(btn);
      if (!el || !b) continue;
      const on = !!st[key];
      el.classList.toggle("folded", on);
      b.classList.toggle("on", on);
      b.title = on ? "展開" : "收起";
    }
  }
  for (const [key, btn] of [["foldPick", "#bPickFold"], ["foldEq", "#bEqFold"]]) {
    $(btn)?.addEventListener("click", () => { st[key] = !st[key]; save(); renderFold(); });
  }
  renderFold();

  /* ---------- 下面那一行的道具名稱：點一下複製 ---------- */
  function showHover(it) {
    const box = $("#bPickHover");
    box.innerHTML = "";
    if (!it) { box.textContent = "滑鼠移到圖示上可以看名稱；點一下穿上，再點一次脫下"; return; }
    const name = document.createElement("span");
    name.className = "b-hover-name";
    name.textContent = it.name;
    makeCopyable(name, it.name);
    const rest = document.createElement("span");
    rest.textContent = `・${SLOT_NAME[it.slot] || ""}（${it.id}）`;
    box.append(name, rest);
  }
  $("#bSaveLook").addEventListener("click", () => {
    const add = curOutfit().filter((x) => !isFav(x.id));
    if (!add.length) return toast(curOutfit().length ? "目前裝備都已經收藏了" : "目前沒有穿任何點裝");
    st.favs = [...add.map(plain), ...st.favs];
    save(); renderSaved();
    toast(`已收藏 ${add.length} 件`);
  });

  // 在「解析」頁或變更點裝時，左上角變成「上一頁」回到抽獎模擬
  if (window.BackHooks) window.BackHooks["view-beauty"] = {
    label: () => (!st.dress && st.page === "import" ? "上一頁" : null),
    back: () => {
      // 變更點裝是獨立工具，直接回分類；美容美髮的解析頁才回抽獎模擬
      if (st.dress || st.page !== "import") return false;
      if (!$("#bActMenu").hidden) actMenu(false); if (!$("#bStanceMenu").hidden) stanceMenu(false);
      st.page = "sim"; save(); renderAll(); return true;
    },
  };

  renderAll();
  // 重新開啟時，若之前有查過角色，從本機快取取回（不會用掉 API 次數，除非快取已過期）
  if (st.charName && TAURI) lookup(false);
  window.Beauty = {
    onShow: renderAll,
    current: () => ({ name: st.charName || "", doll: lastDoll, tags }),
    /** 側欄輸入角色名稱且開著同步時，直接查詢美容美髮的角色 */
    setName: async (name) => { $("#bName").value = name; await lookup(false); },
    /** 從工具箱的「變更點裝」磁磚進來時直接切到變更點裝 */
    setDress: (on) => {
      if (!!st.dress === !!on) return;
      if (!$("#bActMenu").hidden) actMenu(false); if (!$("#bStanceMenu").hidden) stanceMenu(false);
      st.dress = !!on;
      if (!st.dress) closeFloat();
      if (st.dress) st.page = "sim";
      save(); renderPage(); renderPick(); renderEquipped();
    },
    charLook,
  };
  document.dispatchEvent(new CustomEvent("beauty-ready"));
})();
