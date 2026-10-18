"use strict";

/* ==========================================================
 * 斷線通知：顯示 Rust 背景監看的狀態與設定
 * ========================================================== */
if (typeof document !== "undefined") (() => {
  let cfg = { on_close: true, on_disconnect: true, threshold: 20, sound: true, sound_name: "beep2", sound_file: "", volume: 100, mode: "windows", on_login: true, discord_url: "", discord_user: "", quiet: [] };
  let status = { running: false, conns: 0, state: "off", at_login: false, lost_secs: 0, notify_in: 0, active_loss: false, events: [] };

  const STATE = {
    off: ["未偵測到新楓之谷", "開啟遊戲後會自動開始監看", "off"],
    waiting: ["新楓之谷已開啟", "進入遊戲後開始監看", "wait"],
    online: ["連線中", "", "ok"],
    lost: ["連線中斷", "", "warn"],
    down: ["已斷線", "已經通知你了；重新進入遊戲後會自動恢復監看", "bad"],
    login: ["回到登入畫面", "已經通知你了；重新進入遊戲後會自動恢復監看", "bad"],
  };

  const pad = (n) => String(n).padStart(2, "0");
  function timeText(ms) {
    const d = new Date(ms);
    return `${d.getMonth() + 1}/${d.getDate()} ${pad(d.getHours())}:${pad(d.getMinutes())}:${pad(d.getSeconds())}`;
  }

  function renderStatus() {
    const [title, detail0, cls] = STATE[status.state] || STATE.off;
    let detail = detail0;
    if (status.state === "online") detail = `遊戲中（${status.conns} 條連線）`;
    if (status.state === "waiting" && status.at_login) detail = "在登入畫面，進入遊戲後開始監看";
    if (status.state === "lost") {
      detail = status.at_login
        ? `回到登入畫面 ${status.lost_secs} 秒，${status.notify_in} 秒後通知`
        : `已中斷 ${status.lost_secs} 秒，${status.notify_in} 秒後通知`;
    }
    $("#wStatus").className = `card w-status ${cls}`;
    $("#wState").textContent = title;
    $("#wDetail").textContent = detail;
    const log = $("#wLog");
    log.innerHTML = "";
    if (!status.events.length) { log.innerHTML = '<div class="g-empty">還沒有任何紀錄</div>'; return; }
    for (const ev of status.events) {
      const row = document.createElement("div");
      row.className = "w-ev";
      const t = document.createElement("span");
      t.className = "muted";
      t.textContent = timeText(ev.at);
      const x = document.createElement("span");
      x.textContent = ev.text;
      row.append(t, x);
      log.append(row);
    }
  }

  function renderCfg() {
    $("#wOnDisc").checked = cfg.on_disconnect;
    $("#wOnClose").checked = !cfg.on_close; // 開關是「遊戲關閉時不通知」
    $("#wOnLogin").checked = cfg.on_login;
    if (document.activeElement !== $("#wDcUrl")) $("#wDcUrl").value = cfg.discord_url;
    if (document.activeElement !== $("#wDcUser")) $("#wDcUser").value = cfg.discord_user;
    $("#wDcBox").hidden = cfg.mode === "windows";
    $$("#wQuiet button").forEach((b) => b.classList.toggle("on", (cfg.quiet || []).includes(Number(b.dataset.h))));
    $("#wSound").checked = cfg.sound;
    $("#wTh").value = cfg.threshold;
    $("#wThV").textContent = `${cfg.threshold} 秒`;
    $$("#wMode button").forEach((b) => b.classList.toggle("on", b.dataset.v === cfg.mode));
    if (![...$("#wSoundName").options].some((o) => o.value === cfg.sound_name)) cfg.sound_name = "beep1";
    $("#wSoundName").value = cfg.sound_name;
    $("#wPick").hidden = cfg.sound_name !== "file";
    $("#wFileName").textContent = cfg.sound_name === "file" ? `音效檔：${cfg.sound_file ? cfg.sound_file.split(/[\\/]/).pop() : "尚未選擇"}` : "";
    $("#wVol").value = cfg.volume;
    $("#wVolV").textContent = `${cfg.volume}%`;
    $$(".w-sound-row, #wVol").forEach((el) => el.classList.toggle("dim", !cfg.sound));
  }

  const saveCfg = () => { if (TAURI) invoke("watch_set", { cfg }).catch(() => {}); };

  /** 用到 Windows 通知時，確認 Windows 有沒有允許這個程式的通知 */
  const WARN = {
    app: "Windows 已關閉「工具箱」的通知，斷線時不會跳出 Windows 通知。",
    user: "Windows 的通知整個被關閉了，斷線時不會跳出 Windows 通知。",
    policy: "通知被系統原則停用了（公司或學校電腦常見）。",
  };
  async function checkToast(announce) {
    const uses = cfg.mode !== "discord" && (cfg.on_disconnect || cfg.on_close || cfg.on_login);
    if (!TAURI || !uses) { $("#wWarn").hidden = true; return; }
    const st = await invoke("toast_status").catch(() => "unknown");
    const msg = WARN[st];
    $("#wWarn").hidden = !msg;
    if (msg) {
      $("#wWarnText").textContent = msg;
      if (announce) toast(msg);
    }
  }

  $("#wOnDisc").addEventListener("change", (e) => { cfg.on_disconnect = e.target.checked; saveCfg(); checkToast(false); });
  $("#wOnClose").addEventListener("change", (e) => { cfg.on_close = !e.target.checked; saveCfg(); checkToast(false); });
  $("#wOnLogin").addEventListener("change", (e) => { cfg.on_login = e.target.checked; saveCfg(); checkToast(false); });
  $$("#wQuiet button").forEach((b) => b.addEventListener("click", () => {
    const h = Number(b.dataset.h);
    const q = new Set(cfg.quiet || []);
    if (q.has(h)) q.delete(h); else q.add(h);
    cfg.quiet = [...q].sort((a, b2) => a - b2);
    saveCfg();
    renderCfg();
  }));
  $("#wDcUrl").addEventListener("input", (e) => { cfg.discord_url = e.target.value.trim(); saveCfg(); dcStatus(""); });
  $("#wDcUser").addEventListener("input", (e) => {
    cfg.discord_user = e.target.value.replace(/\D/g, "");
    if (e.target.value !== cfg.discord_user) e.target.value = cfg.discord_user;
    saveCfg();
    dcStatus("");
  });
  const dcStatus = (t, cls = "") => { const el = $("#wDcStatus"); el.textContent = t; el.className = `w-dc-status muted ${cls}`; };
  $("#wDcTest").addEventListener("click", async () => {
    if (!cfg.discord_url) { dcStatus("請先貼上 Webhook 網址", "bad"); return; }
    if (!TAURI) { dcStatus("預覽模式無法傳送"); return; }
    dcStatus("傳送中…");
    try {
      await invoke("watch_discord_test", { url: cfg.discord_url, user: cfg.discord_user });
      dcStatus("已送出，請到 Discord 確認", "ok");
    } catch (e) { dcStatus(String(e), "bad"); }
  });
  $("#wSound").addEventListener("change", (e) => { cfg.sound = e.target.checked; saveCfg(); renderCfg(); });
  $$("#wMode button").forEach((b) => b.addEventListener("click", () => {
    cfg.mode = b.dataset.v;
    saveCfg();
    renderCfg();
    checkToast(false);
    if (cfg.mode !== "windows" && !cfg.discord_url) $("#wDcUrl").focus();
  }));
  $("#wSoundName").addEventListener("change", (e) => { cfg.sound_name = e.target.value; saveCfg(); renderCfg(); });
  $("#wPick").addEventListener("click", async () => {
    if (!TAURI) return;
    const f = await invoke("pick_sound_file").catch(() => null);
    if (f) { cfg.sound_file = f; saveCfg(); renderCfg(); }
  });
  $("#wPreview").addEventListener("click", () => { if (TAURI) invoke("watch_preview", { cfg }).catch(() => {}); });
  $("#wVol").addEventListener("input", (e) => { cfg.volume = Number(e.target.value); $("#wVolV").textContent = `${cfg.volume}%`; saveCfg(); });
  $("#wOpenSettings").addEventListener("click", () => { if (TAURI) invoke("open_notify_settings").catch(() => {}); });
  $("#wTh").addEventListener("input", (e) => { cfg.threshold = Number(e.target.value); $("#wThV").textContent = `${cfg.threshold} 秒`; saveCfg(); });
  $("#wTest").addEventListener("click", () => { if (TAURI) invoke("watch_test").catch(() => {}); else toast("預覽模式無法顯示通知"); });
  $("#wClear").addEventListener("click", async () => {
    if (TAURI) await invoke("watch_clear_log").catch(() => {});
    status.events = [];
    renderStatus();
  });

  async function load(announce) {
    if (!TAURI) { renderCfg(); renderStatus(); return; }
    try { [cfg, status] = await invoke("watch_get"); } catch (_) {}
    renderCfg();
    renderStatus();
    checkToast(announce);
  }
  if (TAURI) listen("watch-status", (e) => { status = e.payload; if ($("#view-watch").classList.contains("active")) renderStatus(); });
  // 程式一開啟就檢查一次：斷線通知有開、但 Windows 擋掉了通知時提醒
  load(true);
  window.Watch = { onShow: () => load(false) };
})();
