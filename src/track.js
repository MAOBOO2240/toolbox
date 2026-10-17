"use strict";

/* ==========================================================
 * 經驗追蹤
 * - 一隻角色一排：目前等級、經驗 %、今天練了多少（每天 00:00 重新計算）
 * - 近 30 天的平均成長速度，推算哪一天升級
 * - 資料來自 Nexon API（Rust 端快取，最新資料 10 分鐘內不重查，歷史資料查過就記住）
 * ========================================================== */

/** 升級所需經驗值（KMS／TMS 相同） */
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
  return { TABLE, MAX, need };
})();

const TrackCore = (() => {
  const T = ExpCore;
  // 累積經驗：PREFIX[等級] = 從 1 級練到這一級總共的經驗
  const PREFIX = [0, 0];
  for (let lv = 1; lv < T.MAX; lv++) PREFIX[lv + 1] = PREFIX[lv] + T.need(lv);
  const total = (s) => (s ? PREFIX[s.level] + Math.max(0, Number(s.exp) || 0) : 0);

  /** 今天練了多少：同一級顯示 %，升級了顯示升了幾級 */
  function today(now, y) {
    if (!now || !y) return null;
    if (now.level > y.level) return { levels: now.level - y.level, pct: null };
    return { levels: 0, pct: Math.max(0, now.rate - y.rate) };
  }

  /**
   * 平均每天的經驗：past 是 days 天前那天結束時的資料，
   * 經過的時間 = (days - 1) 天 + 今天已經過的時間
   */
  function perDay(now, past, days, hoursToday) {
    if (!now || !past || !days) return null;
    const elapsed = Math.max(0.25, days - 1 + hoursToday / 24);
    return Math.max(0, total(now) - total(past)) / elapsed;
  }

  /** 預計升級：還差多少 ÷ 每天平均 → 幾天後 */
  function eta(now, daily) {
    if (!now || now.level >= T.MAX) return null;
    const left = Math.max(0, T.need(now.level) - (Number(now.exp) || 0));
    if (!(daily > 0)) return null;
    return { days: left / daily, level: now.level + 1 };
  }

  return { PREFIX, total, today, perDay, eta };
})();

if (typeof module !== "undefined") module.exports = { ExpCore, TrackCore };

/* ==========================================================
 * 介面
 * ========================================================== */
if (typeof document !== "undefined") (() => {
  const C = TrackCore, E = ExpCore;
  const KEY = "track-v1";
  let st = (() => { try { return { names: [], data: {}, ...(JSON.parse(localStorage.getItem(KEY)) || {}) }; } catch (_) { return { names: [], data: {} }; } })();
  const save = () => { try { localStorage.setItem(KEY, JSON.stringify(st)); } catch (_) {} };
  let busy = false, timer = null;

  const fmtPct = (p) => `${p.toFixed(3)}%`;
  function dateText(days) {
    const d = new Date(Date.now() + days * 86400000);
    const y = d.getFullYear() !== new Date().getFullYear() ? `${d.getFullYear()}年` : "";
    return `${y}${d.getMonth() + 1}月${d.getDate()}日`;
  }
  const hoursToday = () => { const n = new Date(); return n.getHours() + n.getMinutes() / 60; };

  function render() {
    const box = $("#tkList");
    box.innerHTML = "";
    if (!st.names.length) {
      box.innerHTML = '<div class="card g-empty tk-empty">輸入角色名稱新增要追蹤的角色</div>';
      return;
    }
    const head = document.createElement("div");
    head.className = "tk-row tk-head";
    head.innerHTML = "<span></span><span>角色</span><span>等級</span><span>經驗</span><span>今天</span><span>平均每天</span><span>預計升級</span><span></span>";
    box.appendChild(head);
    st.names.forEach((name) => {
      const r = st.data[name];
      const now = r?.now;
      const row = document.createElement("div");
      row.className = "card tk-row";
      const pic = document.createElement("span");
      pic.className = "tk-pic";
      if (now?.image) { const img = new Image(); img.src = now.image; img.alt = ""; img.draggable = false; pic.append(img); }
      const who = document.createElement("span");
      who.className = "tk-name";
      who.innerHTML = "<b></b><small class=\"muted\"></small>";
      who.children[0].textContent = name;
      who.children[1].textContent = r?.error ? r.error : [now?.world, now?.class].filter(Boolean).join("・");
      if (r?.error) who.children[1].classList.add("bad");
      const cell = (text, cls = "") => { const s = document.createElement("span"); s.className = `tk-cell ${cls}`; s.textContent = text; return s; };
      // 今天的起點：昨天的資料；凌晨 Nexon 還沒準備好時，用今天第一次查到的資料
      const t = C.today(now, r?.yesterday || r?.first_today);
      const daily = C.perDay(now, r?.past, r?.past_days, hoursToday());
      const eta = C.eta(now, daily);
      const dailyPct = daily != null && now ? (daily / Math.max(1, E.need(now.level))) * 100 : null;
      const del = document.createElement("button");
      del.className = "b-x";
      del.title = "移除";
      del.textContent = "×";
      del.addEventListener("click", async () => {
        if (!(await confirmBox(`確定不再追蹤「${name}」嗎？`))) return;
        st.names = st.names.filter((n) => n !== name);
        delete st.data[name];
        save(); render();
      });
      const avg = cell(dailyPct == null ? "—" : `+${fmtPct(dailyPct)}`, dailyPct > 0 ? "up" : "");
      if (r?.past_days && r.past_days < 30) avg.title = `角色資料只查得到近 ${r.past_days} 天`;
      row.append(
        pic, who,
        cell(now ? `Lv.${now.level}` : "—", "tk-lv"),
        cell(now ? fmtPct(now.rate) : "—"),
        cell(!t ? "—" : t.levels ? `+${t.levels} 級` : `+${fmtPct(t.pct)}`, t && (t.levels || t.pct > 0) ? "up" : ""),
        avg,
        cell(now?.level >= E.MAX ? "已滿級" : eta ? `${dateText(eta.days)} Lv.${eta.level}` : "—", eta ? "tk-eta" : ""),
        del,
      );
      box.appendChild(row);
    });
  }

  async function refresh(force) {
    if (busy || !st.names.length) { render(); return; }
    if (!TAURI) { $("#tkStatus").textContent = "預覽模式無法呼叫 API"; render(); return; }
    busy = true;
    $("#tkRefresh").disabled = true;
    $("#tkStatus").textContent = "更新中…";
    try {
      const list = await invoke("exp_track", { names: st.names, force: !!force });
      for (const r of list) st.data[r.name] = r;
      save();
      const n = new Date();
      $("#tkStatus").textContent = `更新時間 ${String(n.getHours()).padStart(2, "0")}:${String(n.getMinutes()).padStart(2, "0")}`;
    } catch (e) {
      $("#tkStatus").textContent = String(e);
    } finally {
      busy = false;
      $("#tkRefresh").disabled = false;
      render();
    }
  }

  function add() {
    const name = $("#tkName").value.trim();
    if (!name) return;
    if (st.names.includes(name)) { toast("已經在追蹤這隻角色了"); return; }
    st.names.push(name);
    $("#tkName").value = "";
    save();
    refresh(false);
  }
  $("#tkAddBtn").addEventListener("click", add);
  $("#tkName").addEventListener("keydown", (e) => { if (e.key === "Enter") add(); });
  $("#tkRefresh").addEventListener("click", () => refresh(true));

  render();
  window.Track = {
    onShow: () => {
      refresh(false);
      // 開著的時候每 10 分鐘更新一次（Rust 端 10 分鐘內不會重查）
      clearInterval(timer);
      timer = setInterval(() => { if ($("#view-track").classList.contains("active")) refresh(false); else clearInterval(timer); }, 10 * 60 * 1000);
    },
  };
})();
