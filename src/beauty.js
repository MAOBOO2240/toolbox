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
  let saveTimer;
  const save = () => {
    clearTimeout(saveTimer);
    saveTimer = setTimeout(() => { try { localStorage.setItem(KEY, JSON.stringify(st)); } catch (_) {} }, 200);
  };

  const HAIR_SW = [["黑色", "#2a2a2e"], ["紅色", "#d8433f"], ["橘色", "#ec8a34"], ["黃色", "#eac53a"], ["綠色", "#58a85c"], ["藍色", "#4677d6"], ["紫色", "#8c55cc"], ["褐色", "#8b5b3b"]];
  const FACE_SW = [["黑色", "#2a2a2e"], ["藍色", "#3a6fe0"], ["紅色", "#d23344"], ["綠色", "#5bb032"], ["褐色", "#c8902a"], ["祖母綠", "#26b6c6"], ["紫色", "#8a3ad0"], ["紫水晶", "#d24aa8"], ["白色", "#e9e9ee"]];
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
      const name = document.createElement("span");
      name.className = "b-name";
      name.textContent = it.name;
      if (feat.has(it.name)) {
        const tag = document.createElement("em");
        tag.className = "b-tag";
        tag.textContent = "主打";
        name.appendChild(tag);
      }
      name.title = "點一下直接試穿";
      name.addEventListener("click", () => tryOn(pool, it.name));
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
          if (renderCache.size > 80) renderCache.delete(renderCache.keys().next().value);
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

  /** 組出要畫的造型：角色原本的造型，加上試穿（換造型時保留角色原本的顏色與混染） */
  function lookReq(tryH, tryF, mode) {
    const h = currentHair(), f = currentFace();
    const g = charGender() || gender();
    // 選了下方的顏色就用那個顏色（不混染），否則保留角色原本的顏色與混染
    const part = (cur, tried, pick) => {
      const color = pick || cur?.color || null;
      const mix = pick ? null : cur?.mix || null;
      const rate = pick ? null : cur ? cur.rate : null;
      if (tried) return { name: tried.name, color, mix, rate, gender: poolGender(tried.pool) };
      if (cur) return { name: cur.name, color, mix, rate, gender: g };
      if (pick) return { name: "", color, gender: g };
      return null;
    };
    return {
      game_dir: st.gamePath || null,
      skin: ch?.beauty?.character_skin?.skin_name || null,
      gender: g,
      hair: part(h, tryH, st.hairColor),
      face: part(f, tryF, st.faceColor),
      mode,
    };
  }

  const showDoll = renderQueue((out, err) => {
    const look = $("#bLook"), empty = $("#bEmpty"), note = $("#bDollNote");
    note.textContent = "";
    if (out) {
      look.dataset.w = out.w;
      look.src = out.png;
      look.hidden = false;
      sizeLook();
      empty.hidden = true;
      if (out.warnings?.length) note.textContent = out.warnings.join("；");
      return;
    }
    // 讀不到遊戲資料：有角色就先顯示官方角色圖
    const img = ch?.basic?.character_image;
    if (img && !(st.tryHair || st.tryFace)) {
      delete look.dataset.w;
      look.src = img;
      look.hidden = false;
      empty.hidden = true;
    } else {
      look.hidden = true;
      empty.hidden = false;
    }
    if (err) note.textContent = err;
  });

  const showLast = renderQueue((out) => {
    const pic = $("#bLastPic");
    if (out) { pic.src = out.png; pic.hidden = false; } else { pic.hidden = true; }
  });

  /** 紙娃娃用整數倍放大，保持像素清楚；放大鏡切換大小 */
  function sizeLook() {
    const look = $("#bLook");
    const base = Number(look.dataset.w) || 0;
    const w = base || look.naturalWidth || 0;
    if (!w) return;
    const scale = base ? (st.zoom ? 5 : 3) : (st.zoom ? 3 : 1.6);
    look.style.width = `${Math.round(w * scale)}px`;
    $("#bZoom").classList.toggle("on", !!st.zoom);
    $("#bZoomPlus").setAttribute("d", st.zoom ? "M7.5 10.5h6" : "M10.5 7.5v6M7.5 10.5h6");
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
    make($("#bHairColors"), HAIR_SW, st.hairColor, (v) => (st.hairColor = v));
    make($("#bFaceColors"), FACE_SW, st.faceColor, (v) => (st.faceColor = v));
  }

  function renderDoll() {
    const tryH = st.tryHair, tryF = st.tryFace;
    if (!TAURI || (!ch && !tryH && !tryF && !st.hairColor && !st.faceColor)) showDoll(null);
    else showDoll(lookReq(tryH, tryF, "full"));

    const wear = $("#bWear");
    wear.innerHTML = "";
    const h = currentHair(), f = currentFace();
    const skin = ch?.beauty?.character_skin?.skin_name;
    const hc = st.hairColor || h?.color || "", fc = st.faceColor || f?.color || "";
    const hairText = tryH ? (hc ? `${hc}${C.stripColor(tryH.name)}` : tryH.name) : h ? `${hc}${h.name}` : "—";
    const faceText = tryF ? (fc ? `${tryF.name}（${fc}）` : tryF.name) : f ? `${f.name}（${fc}）` : "—";
    const mixText = (x, pick) => (!pick && x && x.mix && x.rate ? `混染 ${x.mix} ${x.rate}%` : "");
    wear.append(wearLine("髮型", hairText, tryH ? "試穿中" : mixText(h, st.hairColor)));
    wear.append(wearLine("臉型", faceText, tryF ? "試穿中" : mixText(f, st.faceColor)));
    if (skin) wear.append(wearLine("膚色", skin));
    if (ch?.cash) {
      const list = ch.cash[`cash_item_equipment_preset_${ch.cash.preset_no}`]?.length
        ? ch.cash[`cash_item_equipment_preset_${ch.cash.preset_no}`]
        : ch.cash.cash_item_equipment_base || [];
      const shown = list.filter((x) => !/戒指|EXP/.test(x.cash_item_equipment_slot + x.cash_item_name));
      if (shown.length) {
        const det = document.createElement("details");
        det.className = "b-cash";
        const sum = document.createElement("summary");
        sum.textContent = `點裝 ${shown.length} 件`;
        det.append(sum);
        for (const x of shown) det.append(wearLine(x.cash_item_equipment_slot, x.cash_item_name));
        wear.append(det);
      }
    }

    const tryBox = $("#bTry");
    tryBox.hidden = !(tryH || tryF || st.hairColor || st.faceColor);
    $("#bTryText").textContent = [tryH && `髮型：${C.stripColor(tryH.name)}`, tryF && `臉型：${tryF.name}`, st.hairColor && `髮色：${st.hairColor}`, st.faceColor && `瞳色：${st.faceColor}`].filter(Boolean).join("　");
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
    const btns = [$("#bLookup"), $("#bRefresh")];
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
      if (fn() === false) return;
      t1 = setTimeout(() => { t2 = setInterval(() => { if (fn() === false) stop(); }, 110); }, 380);
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
    $("#bLast").classList.toggle("feat", !!st.last?.feat);
    $("#bLast").classList.toggle("empty", !st.last);
    if (st.last && TAURI) {
      const L = { pool: st.last.pool, name: st.last.name };
      showLast(lookReq(isHair(L.pool) ? L : null, isHair(L.pool) ? null : L, "head"));
    } else showLast(null);

    const box = $("#bBatch");
    box.innerHTML = "";
    const keys = sortedKeys(st.batch);
    const n = Object.values(st.batchDraws).reduce((a, b) => a + b, 0);
    $("#bBatchBadge").textContent = n ? `${intFmt.format(n)} 抽` : "";
    if (!keys.length) box.innerHTML = '<span class="g-chip-empty">按下抽獎後，這裡會顯示這次的結果</span>';
    for (const k of keys) {
      const [pool, name] = splitKey(k);
      const chip = document.createElement("span");
      chip.className = "g-chip" + (isFeat(pool, name) ? " rare" : "");
      chip.append(document.createTextNode(name));
      const b = document.createElement("b");
      b.textContent = `×${st.batch[k]}`;
      chip.append(b);
      box.appendChild(chip);
    }

    const hist = $("#bHistory");
    hist.innerHTML = "";
    // 保管箱只顯示左邊目前選的種類（皇家美髮／皇家整形）
    const hk = sortedKeys(st.hist).filter((k) => splitKey(k)[0].startsWith(st.kind || "hair"));
    if (!hk.length) hist.innerHTML = `<div class="g-empty">${st.kind === "face" ? "皇家整形" : "皇家美髮"}的保管箱還是空的</div>`;
    let lastPool = null;
    for (const k of hk) {
      const [pool, name] = splitKey(k);
      if (pool !== lastPool) {
        lastPool = pool;
        const g = document.createElement("div");
        g.className = "g-group";
        g.textContent = `${C.POOL_NAME[pool]}・${intFmt.format(st.histDraws[pool])} 抽`;
        hist.appendChild(g);
      }
      const count = st.hist[k];
      const draws = st.histDraws[pool] || 0;
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
    st.page = b.dataset.page; save(); renderPage();
    if (st.page === "import") checkWz();
  }));
  $$("#bKind button").forEach((b) => b.addEventListener("click", () => { st.kind = b.dataset.kind; save(); renderPool(); renderRight(); }));
  $$("#bGender button").forEach((b) => b.addEventListener("click", () => {
    st.gender = b.dataset.g;
    save(); renderPool();
  }));
  $("#bLookup").addEventListener("click", () => lookup(false));
  $("#bRefresh").addEventListener("click", () => lookup(true));
  $("#bName").addEventListener("keydown", (e) => { if (e.key === "Enter") lookup(false); });
  $("#bTryReset").addEventListener("click", () => { st.tryHair = st.tryFace = st.hairColor = st.faceColor = null; save(); renderSwatches(); renderDoll(); renderRight(); });
  $("#bZoom").addEventListener("click", () => { st.zoom = !st.zoom; save(); sizeLook(); });
  $("#bLook").addEventListener("load", sizeLook);
  $("#bCost").addEventListener("focus", (e) => { e.target.value = st.cost ? String(st.cost) : ""; e.target.select(); });
  $("#bCost").addEventListener("input", (e) => { st.cost = Math.max(0, parseNum(e.target.value) || 0); save(); });
  $("#bCost").addEventListener("blur", (e) => { e.target.value = st.cost ? intFmt.format(st.cost) : ""; });

  holdable($("#bDraw"), () => drawN(1));
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
  $("#bParse").addEventListener("click", () => {
    const parsed = C.parse($("#bPaste").value);
    const got = C.POOLS.filter((p) => parsed[p].length);
    if (!got.length) { $("#bParseResult").textContent = "沒有解析到任何「名稱 + 機率%」的造型"; return; }
    for (const p of got) st.pools[p] = parsed[p];
    save(); renderAll();
    const msg = got.map((p) => `${C.POOL_NAME[p]} ${parsed[p].length} 項`).join("、");
    $("#bParseResult").textContent = `已套用：${msg}`;
    toast(`已套用：${msg}`);
  });
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

  renderAll();
  // 重新開啟時，若之前有查過角色，從本機快取取回（不會用掉 API 次數，除非快取已過期）
  if (st.charName && TAURI) lookup(false);
  window.Beauty = { onShow: renderAll };
})();
