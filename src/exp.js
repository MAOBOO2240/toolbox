"use strict";

/* ==========================================================
 * 經驗值計算
 * - ExpCore：經驗值表、等差倍率、練到目標等級的時間（可獨立測試）
 * - 介面：角色等級、經驗 %、練功地圖的怪物等級（怪物經驗值從遊戲資料讀取）
 * ========================================================== */

const ExpCore = (() => {
  /** 升級所需經驗值：TABLE[等級] = 從這一級升到下一級要多少（KMS／TMS 相同） */
  const TABLE = [
    0, 15, 34, 57, 92, 135, 372, 560, 840, 1242,
    1242, 1242, 1242, 1242, 1242, 1490, 1788, 2145, 2574, 3088,
    3705, 4446, 5335, 6402, 7682, 9218, 11061, 13273, 15927, 19112,
    19112, 19112, 19112, 19112, 19112, 22934, 27520, 33024, 39628, 47553,
    51357, 55465, 59902, 64694, 69869, 75458, 81494, 88013, 95054, 102658,
    110870, 119739, 129318, 139663, 150836, 162902, 175934, 190008, 205208, 221624,
    221624, 221624, 221624, 221624, 221624, 238245, 256113, 275321, 295970, 318167,
    342029, 367681, 395257, 424901, 456768, 488741, 522952, 559558, 598727, 640637,
    685481, 733464, 784806, 839742, 898523, 961419, 1028718, 1100728, 1177778, 1260222,
    1342136, 1429374, 1522283, 1621231, 1726611, 1838840, 1958364, 2085657, 2221224, 2365603,
    2365603, 2365603, 2365603, 2365603, 2365603, 2519367, 2683125, 2857528, 3043267, 3241079,
    3451749, 3676112, 3915059, 4169537, 4440556, 4729192, 5036589, 5363967, 5712624, 6083944,
    6479400, 6900561, 7349097, 7826788, 8335529, 8877338, 9454364, 10068897, 10723375, 11420394,
    12162719, 12953295, 13795259, 14691950, 15646926, 16663976, 17747134, 18900697, 20129242, 21437642,
    22777494, 24201087, 25713654, 27320757, 29028304, 30842573, 32770233, 34818372, 36994520, 39306677,
    41763344, 44373553, 47146900, 50093581, 53224429, 56550955, 60085389, 63840725, 67830770, 72070193,
    76574580, 81360491, 86445521, 91848366, 97588888, 103688193, 110168705, 117054249, 124370139, 132143272,
    138750435, 145687956, 152972353, 160620970, 168652018, 177084618, 185938848, 195235790, 204997579, 215247457,
    226009829, 237310320, 249175836, 261634627, 274716358, 288452175, 302874783, 318018522, 333919448, 350615420,
    368146191, 386553500, 405881175, 426175233, 447483994, 469858193, 493351102, 518018657, 543919589, 571115568,
    2207026470, 2471869646, 2768494003, 3100713283, 3472798876, 3889534741, 4356278909, 4879032378, 5464516263, 6120258214,
    7344309856, 8152183940, 9048924173, 10044305832, 11149179473, 13379015367, 14583126750, 15895608157, 17326212891, 18885572051,
    22662686461, 24249074513, 25946509728, 27762765408, 29706158986, 35647390783, 38142708137, 40812697706, 43669586545, 46726457603,
    56071749123, 57753901596, 59486518643, 61271114202, 63109247628, 75731097153, 78003030067, 80343120969, 82753414598, 85236017035,
    102283220442, 105351717055, 108512268566, 111767636622, 115120665720, 138144798864, 142289142829, 146557817113, 150954551626, 155483188174,
    186579825808, 192177220582, 197942537199, 203880813314, 209997237713, 216297154844, 222786069489, 229469651573, 236353741120, 243444353353,
    1731919984062, 1749239183902, 1766731575741, 1784398891498, 1802242880412, 2342915744535, 2366344901980, 2390008350999, 2413908434508, 2438047518853,
    5412465491853, 5466590146771, 5521256048238, 5576468608720, 5632233294807, 11377111255510, 12514822381061, 13766304619167, 15142935081083, 16657228589191,
    33647601750165, 37012361925181, 40713598117699, 44784957929468, 49263453722414, 99512176519276, 109463394171214, 120409733588335, 132450706947169, 145695777641870,
    294305470836577, 323736017920234, 356109619712257, 391720581683482, 430892639851830, 870403132500696, 957443445750765, 1053187790325841, 1158506569358425, 1737759854037637,
  ];
  const MAX = 300;
  const need = (lv) => (lv >= 1 && lv < MAX ? TABLE[lv] : 0);

  /**
   * 等差倍率：d = 角色等級 - 怪物等級
   * 差 1 級內 120%、2～4 級 110%、5～9 級 105%、10 級 100%；
   * 角色比怪物高 11～20 級 99%～95%、21～39 級 89%～71%、40 級以上 70%；
   * 怪物比角色高 10～20 級 100%～90%、21～35 級 70%～14%、36 級以上 10%
   */
  function levelMult(d) {
    if (d >= 40) return 0.7;
    if (d >= 21) return Math.round((0.89 - (d - 21) * 0.01) * 100) / 100;
    if (d >= 19) return 0.95;
    if (d >= 17) return 0.96;
    if (d >= 15) return 0.97;
    if (d >= 13) return 0.98;
    if (d >= 11) return 0.99;
    if (d === 10) return 1;
    if (d >= 5) return 1.05;
    if (d >= 2) return 1.1;
    if (d >= -1) return 1.2;
    if (d >= -4) return 1.1;
    if (d >= -9) return 1.05;
    if (d >= -20) return Math.round((1 - (-10 - d) * 0.01) * 100) / 100;
    if (d >= -35) return Math.round((0.7 - (-21 - d) * 0.04) * 100) / 100;
    return 0.1;
  }

  /** 從某一級的某個 % 練到目標等級總共要多少經驗 */
  function remain(level, pct, target) {
    let sum = 0;
    for (let lv = level; lv < Math.min(target, MAX); lv++) sum += need(lv) * (lv === level ? 1 - pct / 100 : 1);
    return sum;
  }

  /**
   * 每升一級重新計算等差（練功地圖不變）。
   * o = { level, pct, target, mobLevel, mobExp, kills（每小時打幾隻）, bonus（經驗加成 %）, hours（每天練幾小時） }
   */
  function plan(o) {
    const rows = [];
    let total = 0, hours = 0;
    const level = Math.max(1, Math.min(MAX - 1, Math.floor(o.level)));
    const target = Math.max(level + 1, Math.min(MAX, Math.floor(o.target)));
    for (let lv = level; lv < target; lv++) {
      const n = need(lv);
      const left = n * (lv === level ? 1 - Math.min(99.999, Math.max(0, o.pct)) / 100 : 1);
      const d = lv - o.mobLevel;
      const mult = levelMult(d);
      // 怪物比角色高 40 級以上，每隻最多 100 經驗
      let perKill = o.mobExp * mult * (1 + (o.bonus || 0) / 100);
      if (d <= -40) perKill = Math.min(perKill, 100);
      const perHour = perKill * (o.kills || 0);
      const h = perHour > 0 ? left / perHour : Infinity;
      total += left;
      hours += h;
      rows.push({ level: lv, need: n, left, d, mult, perKill, perHour, hours: h, pctPerHour: n ? (perHour / n) * 100 : 0, cum: hours });
    }
    const days = o.hours > 0 ? hours / o.hours : Infinity;
    return { rows, total, hours, days, level, target };
  }

  return { TABLE, MAX, need, levelMult, remain, plan };
})();

if (typeof module !== "undefined") module.exports = ExpCore;

/* ==========================================================
 * 介面
 * ========================================================== */
if (typeof document !== "undefined") (() => {
  const C = ExpCore;
  const KEY = "exp-v1";
  const defaults = () => ({ name: "", level: 260, pct: 0, target: 261, mobLevel: 262, mobExp: "", kills: 20000, bonus: 0, hours: 2 });
  let st = (() => { try { return { ...defaults(), ...(JSON.parse(localStorage.getItem(KEY)) || {}) }; } catch (_) { return defaults(); } })();
  const save = () => { try { localStorage.setItem(KEY, JSON.stringify(st)); } catch (_) {} };
  const gameDir = () => { try { return JSON.parse(localStorage.getItem("beauty-v1"))?.gamePath || null; } catch (_) { return null; } };

  const intFmt = new Intl.NumberFormat("zh-TW", { maximumFractionDigits: 0 });
  const num = (v) => { const n = parseFloat(String(v).replace(/[,\s%]/g, "")); return isFinite(n) ? n : 0; };
  /** 經驗值讀法：1234567890 → 12億3456萬 */
  function zh(n) {
    if (!isFinite(n)) return "—";
    let x = Math.round(Math.abs(n));
    const units = [[1e12, "兆"], [1e8, "億"], [1e4, "萬"]];
    let out = "";
    for (const [u, name] of units) { const q = Math.floor(x / u); if (q > 0) { out += `${q}${name}`; x -= q * u; } }
    if (x > 0 || !out) out += x;
    return out;
  }
  /** 表格用的短讀法：2兆3429億、171.3億、1234萬 */
  function short(n) {
    if (n >= 1e12) return `${+(n / 1e12).toFixed(2)}兆`;
    if (n >= 1e8) return `${+(n / 1e8).toFixed(n >= 1e10 ? 0 : 1)}億`;
    if (n >= 1e4) return `${intFmt.format(Math.round(n / 1e4))}萬`;
    return intFmt.format(n);
  }
  const pctText = (p) => `${p >= 100 ? intFmt.format(p) : +p.toFixed(p >= 10 ? 1 : 2)}%`;
  function timeText(h) {
    if (!isFinite(h)) return "—";
    if (h < 1) return `${Math.max(1, Math.round(h * 60))} 分鐘`;
    if (h < 48) return `${+h.toFixed(1)} 小時`;
    return `${intFmt.format(Math.round(h))} 小時`;
  }

  /* ---------- 怪物經驗值（遊戲資料） ---------- */
  let mobs = null, mobErr = "";
  async function loadMobs() {
    if (mobs || !TAURI) return;
    $("#xMobHint").textContent = "讀取遊戲資料中…（第一次需要一點時間）";
    try { mobs = await invoke("exp_mobs", { gameDir: gameDir() }); mobErr = ""; }
    catch (e) { mobErr = String(e); }
    render();
  }
  /** 這個等級的怪物（找不到同等級就用最接近的較低等級） */
  function mobAt(lv) {
    if (!mobs?.length) return null;
    let best = null;
    for (const m of mobs) if (m.level <= lv && (!best || m.level > best.level)) best = m;
    return best && best.level === lv ? best : best ? { ...best, near: true } : null;
  }

  /* ---------- 畫面 ---------- */
  const FIELDS = [
    ["#xLevel", "level"], ["#xPct", "pct"], ["#xTarget", "target"], ["#xMobLevel", "mobLevel"],
    ["#xMobExp", "mobExp"], ["#xKills", "kills"], ["#xBonus", "bonus"], ["#xHours", "hours"],
  ];
  function syncInputs() {
    for (const [id, k] of FIELDS) { const el = $(id); if (document.activeElement !== el) el.value = st[k] === "" ? "" : st[k]; }
    if (document.activeElement !== $("#xName")) $("#xName").value = st.name || "";
  }

  function render() {
    syncInputs();
    const level = Math.max(1, Math.min(C.MAX - 1, Math.floor(num(st.level)) || 1));
    const target = Math.max(level + 1, Math.min(C.MAX, Math.floor(num(st.target)) || level + 1));
    const mobLevel = Math.floor(num(st.mobLevel)) || level;
    const m = mobAt(mobLevel);
    const custom = String(st.mobExp).trim() !== "";
    const mobExp = custom ? num(st.mobExp) : m ? m.exp : 0;

    // 怪物經驗值的說明
    const hint = $("#xMobHint");
    if (custom) hint.textContent = "使用自己輸入的經驗值（清空就改回遊戲資料）";
    else if (m) hint.textContent = `遊戲資料：Lv.${m.level}${m.near ? "（最接近的等級）" : ""} 經驗值最高的「${m.name || m.id}」${intFmt.format(m.exp)}${m.count > 1 ? `，同等級 ${m.count} 種怪物最低 ${intFmt.format(m.min)}` : ""}`;
    else if (!TAURI) hint.textContent = "預覽模式讀不到遊戲資料，請自己輸入每隻怪的經驗值";
    else hint.textContent = mobErr ? `${mobErr}；請自己輸入每隻怪的經驗值` : "讀取遊戲資料中…";
    $("#xMobExp").placeholder = m ? intFmt.format(m.exp) : "每隻怪的經驗值";

    const d = level - mobLevel;
    const mult = C.levelMult(d);
    $("#xDiff").textContent = `${d === 0 ? "和怪物同等級" : d > 0 ? `角色比怪物高 ${d} 級` : `怪物比角色高 ${-d} 級`}：經驗 ${Math.round(mult * 100)}%`;
    $("#xDiff").className = `x-diff ${mult > 1 ? "up" : mult < 1 ? "down" : ""}`;

    const out = $("#xOut");
    if (!mobExp) { out.innerHTML = '<p class="f-empty">輸入怪物等級（或每隻怪的經驗值）就會算出要練多久</p>'; return; }
    const p = C.plan({ level, pct: num(st.pct), target, mobLevel, mobExp, kills: num(st.kills), bonus: num(st.bonus), hours: num(st.hours) });
    const first = p.rows[0];
    const days = p.days;
    const daysText = !isFinite(days) ? "—" : days < 1 ? "不到 1 天" : `${days >= 10 ? intFmt.format(Math.ceil(days)) : +days.toFixed(1)} 天`;
    const line = (k, v, s) => `<div class="f-line"><span>${k}${s ? `<small>${s}</small>` : ""}</span><b>${v}</b></div>`;
    out.innerHTML =
      `<div class="f-big-label">練到 Lv.${target} 需要</div>` +
      `<div class="f-big">${daysText}</div>` +
      `<div class="f-sub">總共 ${timeText(p.hours)}（每天 ${+num(st.hours).toFixed(1)} 小時）</div>` +
      '<div class="f-lines">' +
      line(`到 Lv.${level + 1} 還差`, intFmt.format(Math.ceil(first.left)), `${zh(first.left)}・剩 ${pctText(100 - Math.min(99.999, num(st.pct)))}`) +
      (target > level + 1 ? line(`到 Lv.${target} 總共還差`, intFmt.format(Math.ceil(p.total)), zh(p.total)) : "") +
      line("每隻怪", intFmt.format(Math.round(first.perKill)), `${intFmt.format(mobExp)} × 等差 ${Math.round(first.mult * 100)}%${num(st.bonus) ? ` × 加成 ${100 + num(st.bonus)}%` : ""}`) +
      line("每小時", intFmt.format(Math.round(first.perHour)), `目前等級每小時 ${pctText(first.pctPerHour)}`) +
      "</div>";

    // 每一級的明細
    const tb = $("#xRows");
    tb.innerHTML = "";
    $("#xRowsCard").hidden = p.rows.length < 2;
    for (const r of p.rows) {
      const tr = document.createElement("div");
      tr.className = "x-row";
      tr.innerHTML = `<span>Lv.${r.level}</span><span title="${intFmt.format(r.need)}">${short(r.need)}</span><span class="${r.mult > 1 ? "up" : r.mult < 1 ? "down" : ""}">${Math.round(r.mult * 100)}%</span><span>${pctText(r.pctPerHour)}</span><span>${timeText(r.hours)}</span><span>${num(st.hours) > 0 ? +(r.cum / num(st.hours)).toFixed(1) : "—"}</span>`;
      tb.appendChild(tr);
    }
  }

  /* ---------- 輸入 ---------- */
  for (const [id, k] of FIELDS) {
    $(id).addEventListener("input", (e) => {
      st[k] = k === "mobExp" ? e.target.value.replace(/[^\d,]/g, "") : e.target.value;
      if (k === "level" && num(st.target) <= num(st.level)) st.target = Math.min(C.MAX, Math.floor(num(st.level)) + 1);
      save(); render();
    });
    $(id).addEventListener("blur", render);
  }
  // 滾輪調整
  const STEP = { level: 1, pct: 1, target: 1, mobLevel: 1, kills: 500, bonus: 10, hours: 0.5 };
  for (const [id, k] of FIELDS) {
    if (!STEP[k]) continue;
    $(id).addEventListener("wheel", (e) => {
      if (!e.deltaY) return;
      e.preventDefault();
      const max = { level: C.MAX - 1, target: C.MAX, mobLevel: 300, pct: 99.999, hours: 24 }[k] ?? Infinity;
      const min = { level: 1, target: 2, mobLevel: 1 }[k] ?? 0;
      let v = num(st[k]) + (e.deltaY < 0 ? 1 : -1) * STEP[k];
      v = Math.max(min, Math.min(max, +v.toFixed(3)));
      st[k] = v;
      if (k === "level" && num(st.target) <= v) st.target = Math.min(C.MAX, v + 1);
      e.target.value = v;
      save(); render();
    }, { passive: false });
  }
  $("#xName").addEventListener("input", (e) => { st.name = e.target.value; save(); });

  /** 用角色名稱帶入目前等級與經驗 %（使用 API） */
  async function fill() {
    const name = $("#xName").value.trim();
    if (!name) return toast("請輸入角色名稱");
    if (!TAURI) return toast("預覽模式無法呼叫 API");
    $("#xFill").disabled = true;
    $("#xFillStatus").textContent = "查詢中…";
    try {
      const c = await invoke("maple_lookup", { name, force: false });
      const lv = Number(c?.basic?.character_level);
      const pct = parseFloat(String(c?.basic?.character_exp_rate ?? "").replace(/[%\s]/g, ""));
      if (!lv) throw new Error("查不到等級");
      st.name = c.name || name;
      st.level = lv;
      st.pct = isFinite(pct) ? pct : 0;
      if (num(st.target) <= lv) st.target = Math.min(C.MAX, lv + 1);
      save();
      const t = new Date((c.fetched_at || 0) * 1000);
      $("#xFillStatus").textContent = `${c.basic?.character_class || ""} Lv.${lv}・${st.pct}%　資料時間 ${t.getMonth() + 1}/${t.getDate()} ${String(t.getHours()).padStart(2, "0")}:${String(t.getMinutes()).padStart(2, "0")}`;
      render();
    } catch (e) {
      $("#xFillStatus").textContent = String(e?.message || e);
    } finally {
      $("#xFill").disabled = false;
    }
  }
  $("#xFill").addEventListener("click", fill);
  $("#xName").addEventListener("keydown", (e) => { if (e.key === "Enter") fill(); });

  render();
  window.ExpCalc = { onShow: () => { render(); loadMobs(); } };
})();
