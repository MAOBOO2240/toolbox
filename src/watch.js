"use strict";

/* ==========================================================
 * 斷線通知：顯示 Rust 背景監看的狀態與設定
 * ========================================================== */
if (typeof document !== "undefined") (() => {
  let cfg = { on_close: true, on_disconnect: true, threshold: 20, sound: true };
  let status = { running: false, conns: 0, state: "off", lost_secs: 0, events: [] };

  const STATE = {
    off: ["未偵測到新楓之谷", "開啟遊戲後會自動開始監看", "off"],
    waiting: ["新楓之谷已開啟", "等待遊戲連上伺服器…", "wait"],
    online: ["連線中", "", "ok"],
    lost: ["連線中斷", "", "warn"],
    down: ["已斷線", "已經通知你了；重新連上後會自動恢復監看", "bad"],
  };

  const pad = (n) => String(n).padStart(2, "0");
  function timeText(ms) {
    const d = new Date(ms);
    return `${d.getMonth() + 1}/${d.getDate()} ${pad(d.getHours())}:${pad(d.getMinutes())}:${pad(d.getSeconds())}`;
  }

  function renderStatus() {
    const [title, detail0, cls] = STATE[status.state] || STATE.off;
    let detail = detail0;
    if (status.state === "online") detail = `遊戲有 ${status.conns} 條連線`;
    if (status.state === "lost") detail = `已中斷 ${status.lost_secs} 秒，超過 ${cfg.threshold} 秒就會通知`;
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
    $("#wOnClose").checked = cfg.on_close;
    $("#wSound").checked = cfg.sound;
    $("#wTh").value = cfg.threshold;
    $("#wThV").textContent = `${cfg.threshold} 秒`;
  }

  const saveCfg = () => { if (TAURI) invoke("watch_set", { cfg }).catch(() => {}); };

  $("#wOnDisc").addEventListener("change", (e) => { cfg.on_disconnect = e.target.checked; saveCfg(); });
  $("#wOnClose").addEventListener("change", (e) => { cfg.on_close = e.target.checked; saveCfg(); });
  $("#wSound").addEventListener("change", (e) => { cfg.sound = e.target.checked; saveCfg(); });
  $("#wTh").addEventListener("input", (e) => { cfg.threshold = Number(e.target.value); $("#wThV").textContent = `${cfg.threshold} 秒`; saveCfg(); });
  $("#wTest").addEventListener("click", () => { if (TAURI) invoke("notify_test").catch(() => {}); else toast("預覽模式無法顯示通知"); });
  $("#wClear").addEventListener("click", async () => {
    if (TAURI) await invoke("watch_clear_log").catch(() => {});
    status.events = [];
    renderStatus();
  });

  async function load() {
    if (!TAURI) { renderCfg(); renderStatus(); return; }
    try { [cfg, status] = await invoke("watch_get"); } catch (_) {}
    renderCfg();
    renderStatus();
  }
  if (TAURI) listen("watch-status", (e) => { status = e.payload; if ($("#view-watch").classList.contains("active")) renderStatus(); });
  load();
  window.Watch = { onShow: load };
})();
