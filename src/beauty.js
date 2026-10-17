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
    // 變更點裝模式：左邊換成選點裝，右邊換成目前裝備與收藏
    $("#bPool").hidden = !!st.dress;
    $("#bPick").hidden = !st.dress;
    $("#bSide").hidden = !!st.dress;
    $("#bDressSide").hidden = !st.dress;
    $("#bDress").textContent = st.dress ? "完成，回到抽獎" : "變更點裝";
    $("#bDress").classList.toggle("solid", !!st.dress);
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
      const name = document.createElement("span");
      name.className = "b-name";
      name.textContent = it.name;
      if (feat.has(it.name)) {
        const tag = document.createElement("em");
        tag.className = "b-tag";
        tag.textContent = "主打";
        name.appendChild(tag);
      }
      row.title = "點一下直接試穿";
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

  /** 目前外觀套組的點裝（沒有套組就用基本點裝），不含戒指等看不到的 */
  function cashList() {
    if (!ch?.cash) return [];
    const preset = ch.cash[`cash_item_equipment_preset_${ch.cash.preset_no}`];
    const list = preset?.length ? preset : ch.cash.cash_item_equipment_base || [];
    return list.filter((x) => !/戒指|EXP|口袋|胸章|徽章/.test(x.cash_item_equipment_slot + x.cash_item_name));
  }

  /** API 的染色稜鏡 → 紙娃娃染色參數 */
  function prismToDye(p) {
    if (!p) return null;
    const r = String(p.color_range || "");
    const range = /紅/.test(r) ? 1 : /黃/.test(r) ? 2 : /綠/.test(r) ? 3 : /青/.test(r) ? 4 : /藍/.test(r) ? 5 : /紫/.test(r) ? 6 : 0;
    const d = { range, hue: Number(p.hue) || 0, sat: Number(p.saturation) || 0, val: Number(p.value) || 0 };
    return d.hue || d.sat || d.val ? d : null;
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
      if (tried) return { name: tried.name, id: tried.id || null, color, mix, rate, gender: poolGender(tried.pool) };
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
      skin_id: st.skinId || null,
      items: mode !== "full" ? [] : st.outfit
        ? st.outfit.map((x) => ({ id: x.id, dye: x.dye || null }))
        : cashList().map((x) => ({ name: x.cash_item_name, dye: prismToDye(x.cash_item_coloring_prism) })),
    };
  }

  const showDoll = renderQueue((out, err) => {
    const look = $("#bLook"), empty = $("#bEmpty"), note = $("#bDollNote");
    note.textContent = "";
    if (out) {
      lastEquips = out.equips || [];
      if (!st.tryHair) lastHairId = out.hair_id || null;
      if (!st.tryFace) lastFaceId = out.face_id || null;
      if (st.dress) renderEquipped();
      look.dataset.w = out.w;
      look.dataset.h = out.h;
      look.dataset.cx = out.cx ?? out.w / 2;
      look.dataset.cy = out.cy ?? out.h / 2;
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
      delete look.dataset.h;
      look.src = img;
      look.hidden = false;
      empty.hidden = true;
    } else {
      look.hidden = true;
      empty.hidden = false;
    }
    if (err) note.textContent = err;
  });

  let lastEquips = [];
  let lastHairId = null, lastFaceId = null, lastSkinId = null; // 紙娃娃最後一次實際穿上的點裝（角色原本的套組解析後的結果）
  const showLast = renderQueue((out) => {
    const pic = $("#bLastPic");
    if (out) { pic.src = out.png; pic.hidden = false; } else { pic.hidden = true; }
  });

  const pan = { x: 0, y: 0 };
  const DOLL_SCALE = 3, DOLL_MIN = 1, DOLL_MAX = 12;
  const dollScale = () => Math.min(DOLL_MAX, Math.max(DOLL_MIN, Number(st.dollScale) || DOLL_SCALE));
  /** 讓角色本身（不是整張圖）位在框的正中間：大型武器不會把角色擠到旁邊 */
  function basePan(scale) {
    const look = $("#bLook");
    const w = Number(look.dataset.w), h = Number(look.dataset.h);
    if (!w || !h) return { x: 0, y: 0 };
    return { x: (w / 2 - Number(look.dataset.cx)) * scale, y: (h / 2 - Number(look.dataset.cy)) * scale };
  }
  const lookScale = () => (Number($("#bLook").dataset.w) ? dollScale() : dollScale() * 0.55);
  function placeLook() {
    const b = basePan(lookScale());
    $("#bLook").style.transform = `translate(${b.x + pan.x}px, ${b.y + pan.y}px)`;
  }
  /** 紙娃娃放大倍率：滑鼠在角色框裡用滾輪調整 */
  function sizeLook() {
    const look = $("#bLook");
    const base = Number(look.dataset.w) || 0;
    const w = base || look.naturalWidth || 0;
    if (!w) return;
    // 官方角色圖本身比較大，倍率打折
    look.style.width = `${Math.round(w * lookScale())}px`;
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
    make($("#bHairColors"), HAIR_SW, st.hairColor, (v) => (st.hairColor = v));
    make($("#bFaceColors"), FACE_SW, st.faceColor, (v) => (st.faceColor = v));
  }

  function renderDoll() {
    const tryH = st.tryHair, tryF = st.tryFace;
    if (!TAURI || (!ch && !tryH && !tryF && !st.hairColor && !st.faceColor && !st.skinId && !st.outfit?.length)) showDoll(null);
    else showDoll(lookReq(tryH, tryF, "full"));

    const wear = $("#bWear");
    wear.innerHTML = "";
    const h = currentHair(), f = currentFace();
    const skin = ch?.beauty?.character_skin?.skin_name;
    const hc = st.hairColor || h?.color || "", fc = st.faceColor || f?.color || "";
    const hairText = tryH ? (hc ? `${hc}${C.stripColor(tryH.name)}` : tryH.name) : h ? `${hc}${h.name}` : "預設髮型";
    const faceText = tryF ? (fc ? `${tryF.name}（${fc}）` : tryF.name) : f ? `${f.name}（${fc}）` : "預設臉型";
    const mixText = (x, pick) => (!pick && x && x.mix && x.rate ? `混染 ${x.mix} ${x.rate}%` : "");
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
    wear.append(linkLine("髮型", hairText, tryH ? "試穿中" : mixText(h, st.hairColor), "hair"));
    wear.append(linkLine("臉型", faceText, tryF ? "試穿中" : mixText(f, st.faceColor), "face"));
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
    st.page = b.dataset.page;
    if (st.page === "sim") st.dress = false;
    save(); renderPage();
    if (st.page === "import") checkWz();
  }));
  $$("#bKind button").forEach((b) => b.prepend(ItemIcons.el(b.dataset.kind === "hair" ? "皇家美髮券" : "皇家整形券")));
  $$("#bKind button").forEach((b) => b.addEventListener("click", () => { st.kind = b.dataset.kind; save(); renderPool(); renderRight(); }));
  $$("#bGender button").forEach((b) => b.addEventListener("click", () => {
    st.gender = b.dataset.g;
    save(); renderPool();
  }));
  $("#bLookup").addEventListener("click", () => lookup(false));
  $("#bName").addEventListener("keydown", (e) => { if (e.key === "Enter") lookup(false); });
  $("#bRestore").addEventListener("click", () => {
    st.tryHair = st.tryFace = st.hairColor = st.faceColor = null;
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
      if (e.button !== 0 || e.target.closest("button")) return;
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
    stage.addEventListener("dblclick", () => { pan.x = pan.y = 0; st.dollScale = DOLL_SCALE; save(); sizeLook(); });
  })();
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

  /* ---------- 變更點裝 ---------- */
  const SLOTS = [
    ["All", "全部", "c-silver"],
    ["Cap", "帽子", "c-pink"], ["Face", "臉飾", "c-purple"], ["Eye", "眼飾", "c-blue"], ["Ear", "耳環", "c-amber"],
    ["Coat", "上衣", "c-orange"], ["Longcoat", "套服", "c-red"], ["Pants", "褲裙", "c-teal"], ["Shoes", "鞋子", "c-green"],
    ["Glove", "手套", "c-silver"], ["Cape", "披風", "c-purple"], ["Shield", "輔助武器", "c-blue"], ["Weapon", "武器", "c-orange"],
  ];
  const SLOT_NAME = Object.fromEntries(SLOTS.map(([k, n]) => [k, n]));
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
    if (worn) opts.push(["染色", () => openDye(item.id)]);
    showMenu(x, y, item.name, opts);
  }
  function showMenu(x, y, name, opts) {
    const m = $("#bCtx");
    m.innerHTML = "";
    const title = document.createElement("div");
    title.className = "b-ctx-title";
    title.textContent = name;
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
    const list = itemLists[st.pickSlot] || [];
    const q = ($("#bPickSearch").value || "").trim();
    return list.filter((it) =>
      (!st.pickCash || it.cash) &&
      (!q || it.name.includes(q) || String(it.id).includes(q)) &&
      (!st.pickG || (() => { const g = Math.floor(it.id / 1000) % 10; return g > 1 || (st.pickG === "F" ? g === 1 : g === 0); })())
    );
  }

  function pickIcon(it) {
    const el = iconEl(it);
    el.addEventListener("mouseenter", () => { $("#bPickHover").textContent = `${it.name}・${SLOT_NAME[it.slot] || ""}（${it.id}）`; });
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
    if (!itemLists[st.pickSlot]) {
      box.innerHTML = `<div class="g-empty">讀取中…${st.pickSlot === "All" ? "（第一次讀取全部分類需要一點時間）" : ""}</div>`;
      $("#bPickCount").textContent = "";
      const slot = st.pickSlot;
      try { itemLists[slot] = await invoke("beauty_items", { gameDir: st.gamePath || null, slot }); }
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

  function renderSaved() {
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
  function closeFloat() { $("#bDyeWin").hidden = true; if (styleKind) closeStyle(); dyeId = null; }

  const RANGES = [["全色系", "c-silver"], ["紅色系", "c-red"], ["黃色系", "c-amber"], ["綠色系", "c-green"], ["青色系", "c-teal"], ["藍色系", "c-blue"], ["紫色系", "c-purple"]];
  let dyeId = null;
  function dyeItem() { return curOutfit().find((x) => x.id === dyeId); }
  function openDye(id) {
    if (!st.outfit) st.outfit = curOutfit(); // 先把角色原本的套組變成可以修改的清單
    dyeId = id;
    const it = dyeItem();
    if (!it) return;
    if (styleKind) closeStyle();
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
    closeCtx();
    $("#bDyeWin").hidden = true;
    styleKind = kind;
    styleShown = STYLE_PAGE;
    $("#bStyleTitle").textContent = STYLE_TITLE[kind];
    $("#bStyleSearch").value = "";
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
  }

  function styleItems() {
    const list = styleKind === "skin" ? skins || [] : styleLists[styleKind] || [];
    const q = $("#bStyleSearch").value.trim();
    let out = q ? list.filter((x) => x.name.includes(q) || String(x.id).includes(q)) : list;
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
  $("#bStyleSearch").addEventListener("input", () => { clearTimeout(styleSearchTimer); styleSearchTimer = setTimeout(() => { styleShown = STYLE_PAGE; renderStyles(); }, 200); });
  $("#bStyleClose").addEventListener("click", closeStyle);
  $("#bStyleWin").addEventListener("pointerdown", (e) => { if (e.target === $("#bStyleWin")) closeStyle(); });
  $$("#bStyleSize button").forEach((b) => b.addEventListener("click", () => { st.styleSize = b.dataset.size; save(); applyStyleSize(); }));
  $("#bStyleFavOnly").addEventListener("click", () => { st.styleFavOnly = !st.styleFavOnly; styleShown = STYLE_PAGE; save(); renderStyles(); });

  $("#bDress").addEventListener("click", () => {
    st.dress = !st.dress;
    if (!st.dress) closeFloat();
    save(); renderPage(); renderPick(); renderEquipped();
  });
  $$("#bPickG button").forEach((b) => b.addEventListener("click", () => { st.pickG = b.dataset.g; shown = PAGE; save(); renderPick(); }));
  $("#bPickCash").addEventListener("click", () => { st.pickCash = !st.pickCash; shown = PAGE; save(); renderPick(); });
  let searchTimer;
  $("#bPickSearch").addEventListener("input", () => { clearTimeout(searchTimer); searchTimer = setTimeout(() => { shown = PAGE; renderPick(); }, 200); });
  $("#bOutfitClear").addEventListener("click", () => { closeFloat(); setOutfit([]); });
  $("#bSaveLook").addEventListener("click", () => {
    const add = curOutfit().filter((x) => !isFav(x.id));
    if (!add.length) return toast(curOutfit().length ? "目前裝備都已經收藏了" : "目前沒有穿任何點裝");
    st.favs = [...add.map(plain), ...st.favs];
    save(); renderSaved();
    toast(`已收藏 ${add.length} 件`);
  });

  // 在「解析」頁或變更點裝時，左上角變成「上一頁」回到抽獎模擬
  if (window.BackHooks) window.BackHooks["view-beauty"] = {
    label: () => (st.page === "import" || st.dress ? "上一頁" : null),
    back: () => {
      if (st.page !== "import" && !st.dress) return false;
      st.page = "sim"; st.dress = false; save(); renderAll(); return true;
    },
  };

  renderAll();
  // 重新開啟時，若之前有查過角色，從本機快取取回（不會用掉 API 次數，除非快取已過期）
  if (st.charName && TAURI) lookup(false);
  window.Beauty = { onShow: renderAll };
})();
