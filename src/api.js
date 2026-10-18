"use strict";

/* ==========================================================
 * API 剩餘
 * - 用到的每個 API 一行：剩多少／共多少，下面一條剩餘量
 * - Nexon Open API：本機今天用了幾次（內建金鑰大家共用，只算這台電腦）；可以換成自己的金鑰
 * - GitHub API（自動更新）：向 GitHub 查剩餘次數（查詢本身不算次數）
 * - Discord Webhook（斷線通知）：沒有每日上限，只顯示今天送出幾則
 * ========================================================== */
if (typeof document !== "undefined") (() => {
  const intFmt = new Intl.NumberFormat("zh-TW", { maximumFractionDigits: 0 });
  let page = "list", busy = false, timer = null;
  let last = { nexon: null, github: null, githubErr: "", discord: null };

  const hhmm = (sec) => { const d = new Date(sec * 1000); return `${String(d.getHours()).padStart(2, "0")}:${String(d.getMinutes()).padStart(2, "0")}`; };

  /** 一行：名稱、用在哪裡、剩多少／共多少、剩餘量條 */
  function row({ name, sub, left, total, note, unlimited, err, color }) {
    const el = document.createElement("div");
    el.className = "card api-row";
    const head = document.createElement("div");
    head.className = "api-head";
    head.innerHTML = `<span class="api-ic ${color}"></span><span class="api-name"><b></b><small class="muted"></small></span><span class="api-num"></span>`;
    head.querySelector("b").textContent = name;
    head.querySelector("small").textContent = sub;
    const num = head.querySelector(".api-num");
    const bar = document.createElement("div");
    bar.className = "api-bar";
    const fill = document.createElement("i");
    bar.append(fill);
    const foot = document.createElement("div");
    foot.className = "api-note muted";
    if (err) {
      num.innerHTML = `<span class="bad">讀取失敗</span>`;
      fill.style.width = "0%";
      foot.textContent = err;
    } else if (unlimited) {
      num.innerHTML = `<b>不限</b>`;
      bar.classList.add("free");
      fill.style.width = "100%";
      foot.textContent = note || "";
    } else if (total == null) {
      num.innerHTML = `<span class="muted">讀取中…</span>`;
      fill.style.width = "0%";
    } else {
      const pct = total > 0 ? Math.max(0, Math.min(100, (left / total) * 100)) : 0;
      num.innerHTML = `<span>剩</span><b></b><span class="muted">/ ${intFmt.format(total)}</span>`;
      num.querySelector("b").textContent = intFmt.format(Math.max(0, left));
      fill.style.width = `${pct}%`;
      bar.classList.add(pct > 50 ? "ok" : pct > 20 ? "warn" : "low");
      foot.textContent = note || "";
    }
    el.append(head, bar, foot);
    return el;
  }

  function render() {
    $("#apiPageList").hidden = page !== "list";
    $("#apiPageSet").hidden = page !== "set";
    $("#apiSetBtn").classList.toggle("on", page === "set");
    window.refreshBack?.();
    if (page === "set") return renderSet();
    const box = $("#apiList");
    box.innerHTML = "";
    const n = last.nexon;
    box.append(row({
      name: "Nexon Open API",
      sub: `美容美髮、經驗追蹤、側欄角色・每天 00:00 重置・${n?.own_key ? `自己的金鑰 ${n.key_mask || ""}` : "內建金鑰"}`,
      left: n ? n.limit - n.used : null,
      total: n ? n.limit : null,
      color: "c-orange",
      note: n ? `今天用了 ${intFmt.format(n.used)} 次${n.own_key ? "" : "（內建金鑰大家共用，這裡只算這台電腦用掉的）"}${headerNote(n.headers)}` : "",
    }));
    const g = last.github;
    box.append(row({
      name: "GitHub API",
      sub: `自動更新・每小時重置${g?.reset ? `・${hhmm(g.reset)} 重置` : ""}`,
      left: g ? g.remaining : null,
      total: g ? g.limit : null,
      err: last.githubErr,
      color: "c-indigo",
      note: g ? `檢查更新、下載新版本時使用${g.token ? "" : "（沒有更新權限時是匿名額度）"}` : "",
    }));
    const d = last.discord;
    box.append(row({
      name: "Discord Webhook",
      sub: "斷線通知",
      unlimited: true,
      color: "c-purple",
      note: `沒有每日上限，今天送出 ${intFmt.format(d || 0)} 則`,
    }));
    $("#apiFoot").textContent = TAURI ? "" : "預覽模式無法讀取實際次數";
  }
  /** Nexon 回應如果有帶次數限制的資訊，一起顯示 */
  function headerNote(h) {
    if (!h) return "";
    const rem = Object.entries(h).find(([k]) => k.includes("remaining"));
    return rem ? `・Nexon 回報剩 ${rem[1]}` : "";
  }

  async function refresh() {
    if (busy) return;
    busy = true;
    $("#apiRefresh").disabled = true;
    try {
      if (!TAURI) {
        last = { nexon: { own_key: false, used: 37, limit: 1000, headers: {} }, github: { limit: 5000, remaining: 4987, reset: Date.now() / 1000 + 1800, token: true }, githubErr: "", discord: 2 };
      } else {
        const [n, d] = await Promise.all([invoke("api_usage").catch(() => null), invoke("discord_sent").catch(() => 0)]);
        last.nexon = n;
        last.discord = d;
        render();
        try { last.github = await invoke("github_rate"); last.githubErr = ""; } catch (e) { last.githubErr = String(e); }
      }
    } finally {
      busy = false;
      $("#apiRefresh").disabled = false;
      render();
    }
  }

  /* ---------- 設定：自己的金鑰 ---------- */
  function renderSet() {
    const n = last.nexon;
    $("#apiKeyNow").textContent = n?.own_key
      ? `目前使用自己的金鑰 ${n.key_mask || ""}，每日上限 ${intFmt.format(n.limit)} 次`
      : "目前使用內建金鑰";
    $("#apiKeyDel").disabled = !n?.own_key;
    if (document.activeElement !== $("#apiLimitIn")) $("#apiLimitIn").value = n?.own_key ? String(n.limit) : "";
  }
  async function saveKey() {
    const key = $("#apiKeyIn").value.trim();
    const lim = $("#apiLimitIn").value.trim();
    const limit = lim ? Math.max(1, Math.floor(Number(lim.replace(/[,\s]/g, "")) || 0)) || null : null;
    if (!key && !last.nexon?.own_key) { $("#apiSetMsg").textContent = "請先貼上 API Key"; return; }
    if (!TAURI) { $("#apiSetMsg").textContent = "預覽模式無法儲存"; return; }
    $("#apiKeySave").disabled = true;
    $("#apiSetMsg").textContent = key ? "確認金鑰中…" : "儲存中…";
    try {
      last.nexon = await invoke("api_key_set", { key: key || null, limit });
      $("#apiKeyIn").value = "";
      $("#apiSetMsg").textContent = "";
      toast(key ? "已改用自己的金鑰" : "已更新每日上限");
      page = "list";
      render();
    } catch (e) {
      $("#apiSetMsg").textContent = String(e);
    } finally {
      $("#apiKeySave").disabled = false;
    }
  }
  async function delKey() {
    if (!(await confirmBox("確定要改回內建金鑰嗎？"))) return;
    try {
      last.nexon = await invoke("api_key_set", { key: null, limit: null });
      toast("已改回內建金鑰");
      render();
    } catch (e) {
      $("#apiSetMsg").textContent = String(e);
    }
  }

  $("#apiRefresh").addEventListener("click", refresh);
  $("#apiSetBtn").addEventListener("click", () => { page = page === "set" ? "list" : "set"; $("#apiSetMsg").textContent = ""; render(); });
  $("#apiKeySave").addEventListener("click", saveKey);
  $("#apiKeyIn").addEventListener("keydown", (e) => { if (e.key === "Enter") saveKey(); });
  $("#apiKeyDel").addEventListener("click", delKey);
  $("#apiOpenSite").addEventListener("click", () => { if (TAURI) invoke("open_url", { url: "https://openapi.nexon.com/" }).catch((e) => toast(String(e))); });
  // 設定頁：左上角變成「上一頁」
  if (window.BackHooks) window.BackHooks["view-api"] = {
    label: () => (page === "set" ? "上一頁" : null),
    back: () => { if (page !== "set") return false; page = "list"; render(); return true; },
  };

  render();
  window.ApiUsage = {
    onShow: () => {
      page = "list";
      refresh();
      clearInterval(timer);
      timer = setInterval(() => { if ($("#view-api").classList.contains("active")) refresh(); else clearInterval(timer); }, 60 * 1000);
    },
  };
})();
