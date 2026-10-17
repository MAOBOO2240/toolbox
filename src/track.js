"use strict";

/* ==========================================================
 * 經驗追蹤
 * - 一隻角色一排：目前等級、經驗 %、今天練了多少（每天 00:00 重新計算）
 * - 近 30 天的平均成長速度，推算哪一天升級
 * - 資料來自 Nexon API（Rust 端快取，最新資料 10 分鐘內不重查，歷史資料查過就記住）
 * ========================================================== */

const TrackCore = (() => {
  const T = typeof ExpCore !== "undefined" ? ExpCore : require("./exp.js");
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

if (typeof module !== "undefined") module.exports = TrackCore;

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
      const t = C.today(now, r?.yesterday);
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
