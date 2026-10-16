"use strict";

/* 計時器頁面：卡片設定、即時顯示、按鍵指派 */
(() => {
  const tapi = TAURI ? {
    get: () => invoke("get_timers"),
    update: (timer) => invoke("update_timer", { timer }),
    add: () => invoke("add_timer"),
    del: (id) => invoke("delete_timer", { id }),
    start: (id) => invoke("timer_start", { id }),
    stop: (id) => invoke("timer_stop", { id }),
    setAll: (on) => invoke("set_all", { on }),
    gameOnly: (on) => invoke("set_game_only", { on }),
    resetOverlay: (id) => invoke("reset_overlay", { id }),
    preview: (timer) => invoke("preview_sound", { timer }),
    pickFile: () => invoke("pick_sound_file"),
    capture: () => invoke("start_capture"),
    cancelCapture: () => invoke("cancel_capture"),
    onTick: (cb) => listen("timer-tick", (e) => cb(e.payload)),
    onCaptured: (cb) => listen("key-captured", (e) => cb(e.payload)),
  } : mockApi();

  const MODE_TIPS = {
    normal: "一般：時間到通知後停止。按下指派按鍵可重新開始。",
    count: "次數：時間到自動重新倒數，跑完設定的次數後停止（0 = 無限重複）。",
    key: "指派按鍵：時間到後進入超時並向上計時，按下指派按鍵才重新計時。",
  };
  const NOTIFY_TIPS = {
    short: "短：只通知一次。",
    long: "長：每 3 秒重複通知，直到重新計時；沒有指派按鍵時只通知一聲。",
  };
  const COMP_TIP = "超逾回補：超時期間按下指派按鍵，會以「原本時間 − 超時時間」重新計時；超時超過原本時間則用完整時間。";
  const LOCK_TIP = "鎖定後滑鼠會穿透浮動視窗，不會擋到遊戲；解除鎖定可拖曳移動、拉右下角調整大小。";
  const SOUNDS = [
    ["beep1", "提示音 1（叮咚）"],
    ["beep2", "提示音 2（三短嗶）"],
    ["beep3", "提示音 3（琶音）"],
    ["tts", "語音唸出名稱"],
    ["file", "自訂音效檔…"],
  ];
  const KEY_LABEL = {
    Left: "←", Right: "→", Up: "↑", Down: "↓",
    Mouse4: "滑鼠側鍵(後)", Mouse5: "滑鼠側鍵(前)", MouseMid: "滑鼠中鍵",
    Space: "空白鍵", NumAdd: "Num+",
  };
  const keyLabel = (k) => k.split("+").map((p) => KEY_LABEL[p] || p).join(" + ");

  let file = { game_only: true, timers: [] };
  let snaps = {};
  const openSettings = new Set();
  let capturing = null; // { id, index }
  let loaded = false;

  const byId = (id) => file.timers.find((t) => t.id === id);
  const card = (id) => document.querySelector(`.tcard[data-id="${CSS.escape(id)}"]`);

  /* ---------- 格式 ---------- */
  function fmt(totalSec) {
    totalSec = Math.max(0, totalSec);
    const h = Math.floor(totalSec / 3600);
    const m = Math.floor((totalSec % 3600) / 60);
    const s = totalSec % 60;
    const pad = (n) => String(n).padStart(2, "0");
    return h > 0 ? `${h}:${pad(m)}:${pad(s)}` : `${pad(m)}:${pad(s)}`;
  }

  /** 剩餘 30% 起由原色漸變為紅色，回傳 0~1 的紅色程度 */
  function redness(snap) {
    if (snap.phase !== "running" || !snap.total_ms) return 0;
    const frac = snap.remain_ms / snap.total_ms;
    return frac >= 0.3 ? 0 : Math.min(1, 1 - frac / 0.3);
  }

  /* ---------- 儲存 ---------- */
  const saveTimers = {};
  function save(t, now = false) {
    clearTimeout(saveTimers[t.id]);
    const go = () => run(() => tapi.update(t));
    if (now) go(); else saveTimers[t.id] = setTimeout(go, 250);
  }

  /* ---------- 卡片 ---------- */
  function seg(cls, items, tips) {
    return `<div class="seg ${cls}">${items.map(([v, l]) => `<button data-v="${v}" data-tip="${tips?.[v] || ""}">${l}</button>`).join("")}</div>`;
  }
  const sw = (cls) => `<label class="switch sm"><input type="checkbox" class="${cls}" /><span class="slider"></span></label>`;

  function cardHtml(t) {
    return `
      <div class="tc-head">
        <input class="tc-name f-name" maxlength="20" spellcheck="false" />
        ${t.builtin ? '<span class="badge">預設</span>' : ""}
        <span class="spacer"></span>
        <span class="tc-en-label">啟用</span>${sw("f-enabled")}
      </div>
      <div class="tc-display"><span class="tc-time">00:00</span><span class="tc-status">待命</span></div>
      <div class="tc-actions">
        <button class="pill primary small f-start">開始</button>
        <button class="pill ghost danger small f-stop">結束</button>
        <span class="spacer"></span>
        <button class="link-btn f-toggle">設定 <span class="caret">›</span></button>
      </div>
      <div class="tc-settings">
        <div class="field"><span class="lbl">時間</span>
          <div class="inline"><input type="number" class="num-in f-min" min="0" max="999" /><span>分</span><input type="number" class="num-in f-sec" min="0" max="59" /><span>秒</span></div></div>
        <div class="field"><span class="lbl">冷卻時間</span>
          <div class="inline"><input type="number" class="num-in f-cd" min="0" max="3600" /><span>秒</span><small class="muted">冷卻中按鍵無效</small></div></div>
        <div class="field"><span class="lbl">重新計時</span>${seg("f-mode", [["normal", "一般"], ["count", "次數"], ["key", "指派按鍵"]], MODE_TIPS)}</div>
        <div class="field r-repeat"><span class="lbl">次數</span>
          <div class="inline"><input type="number" class="num-in f-repeat" min="0" max="9999" /><span>次</span><small class="muted">0 = 無限重複</small></div></div>
        <div class="field"><span class="lbl">指派按鍵</span><div class="keys f-keys"></div></div>
        <div class="field r-comp"><span class="lbl" data-tip="${COMP_TIP}">超逾回補 <i class="info">i</i></span>${sw("f-comp")}</div>
        <div class="field"><span class="lbl">通知</span>${seg("f-notify", [["short", "短"], ["long", "長"]], NOTIFY_TIPS)}</div>
        <div class="field"><span class="lbl">音效</span>
          <div class="inline"><select class="sel f-sound">${SOUNDS.map(([v, l]) => `<option value="${v}">${l}</option>`).join("")}</select>
          <button class="link-btn f-preview">▶ 試聽</button></div></div>
        <div class="field r-file"><span class="lbl">音效檔</span>
          <div class="inline"><button class="pill ghost small f-pick">選擇檔案</button><small class="muted fname"></small></div></div>
        <p class="sub">浮動視窗</p>
        <div class="field"><span class="lbl">顯示</span>${sw("f-ov")}</div>
        <div class="field"><span class="lbl" data-tip="${LOCK_TIP}">鎖定 <i class="info">i</i></span>${sw("f-lock")}</div>
        <div class="field"><span class="lbl">透明度</span>
          <div class="inline"><input type="range" class="range f-opacity" min="20" max="100" /><span class="f-op-val muted"></span></div></div>
        <div class="field"><span class="lbl"></span>
          <div class="inline"><button class="pill ghost small f-reset">重設位置</button>
          ${t.builtin ? "" : '<button class="pill ghost danger small f-del">刪除計時器</button>'}</div></div>
      </div>`;
  }

  function fillCard(el, t) {
    const q = (s) => el.querySelector(s);
    if (document.activeElement !== q(".f-name")) q(".f-name").value = t.name;
    q(".f-enabled").checked = t.enabled;
    q(".f-min").value = Math.floor(t.duration_ms / 60000);
    q(".f-sec").value = Math.floor((t.duration_ms % 60000) / 1000);
    q(".f-cd").value = Math.round(t.cooldown_ms / 1000);
    q(".f-repeat").value = t.repeat;
    q(".f-comp").checked = t.overflow_comp;
    q(".f-sound").value = t.sound;
    q(".fname").textContent = t.sound_file ? t.sound_file.split(/[\\/]/).pop() : "尚未選擇";
    q(".f-ov").checked = t.overlay.show;
    q(".f-lock").checked = t.overlay.locked;
    q(".f-opacity").value = Math.round(t.overlay.opacity * 100);
    q(".f-op-val").textContent = `${Math.round(t.overlay.opacity * 100)}%`;
    el.querySelectorAll(".f-mode button").forEach((b) => b.classList.toggle("on", b.dataset.v === t.mode));
    el.querySelectorAll(".f-notify button").forEach((b) => b.classList.toggle("on", b.dataset.v === t.notify));
    q(".r-repeat").hidden = t.mode !== "count";
    q(".r-comp").hidden = t.mode !== "key";
    q(".r-file").hidden = t.sound !== "file";
    el.classList.toggle("disabled", !t.enabled);
    el.classList.toggle("open", openSettings.has(t.id));
    renderKeys(el, t);
  }

  function renderKeys(el, t) {
    const box = el.querySelector(".f-keys");
    box.innerHTML = "";
    const cap = capturing && capturing.id === t.id ? capturing : null;
    t.keys.forEach((k, i) => {
      const b = document.createElement("button");
      b.className = "keycap";
      const waiting = cap && cap.index === i;
      b.classList.toggle("waiting", !!waiting);
      b.textContent = waiting ? "按下新按鍵…（Esc 取消指派）" : keyLabel(k);
      b.title = "點擊後按新按鍵替換；按 Esc 取消這個指派";
      b.addEventListener("click", (e) => { e.stopPropagation(); beginCapture(t.id, i); });
      box.appendChild(b);
    });
    const add = document.createElement("button");
    add.className = "keycap add";
    const waitingNew = cap && cap.index === null;
    add.classList.toggle("waiting", !!waitingNew);
    add.textContent = waitingNew ? "按下按鍵…（Esc 取消）" : "＋ 指派按鍵";
    add.addEventListener("click", (e) => { e.stopPropagation(); beginCapture(t.id, null); });
    box.appendChild(add);
  }

  function beginCapture(id, index) {
    if (document.activeElement) document.activeElement.blur();
    const prev = capturing;
    capturing = { id, index };
    if (prev && prev.id !== id) fillCard(card(prev.id), byId(prev.id));
    renderKeys(card(id), byId(id));
    run(() => tapi.capture());
  }

  function endCapture() {
    if (!capturing) return;
    const { id } = capturing;
    capturing = null;
    run(() => tapi.cancelCapture());
    const t = byId(id);
    if (t) renderKeys(card(id), t);
  }

  function onCaptured({ key, cancel }) {
    if (!capturing) return;
    const { id, index } = capturing;
    capturing = null;
    const t = byId(id);
    if (!t) return;
    if (cancel) {
      if (index !== null) { t.keys.splice(index, 1); toast("已取消指派按鍵"); save(t, true); }
    } else if (index !== null) {
      t.keys[index] = key;
      t.keys = [...new Set(t.keys)];
      save(t, true);
    } else if (!t.keys.includes(key)) {
      t.keys.push(key);
      save(t, true);
    }
    renderKeys(card(id), t);
  }

  function wireCard(el, t) {
    const q = (s) => el.querySelector(s);
    const set = (fn, now) => { fn(); fillCard(el, t); save(t, now); };

    q(".f-name").addEventListener("input", (e) => { t.name = e.target.value.trim() || "計時器"; save(t); });
    q(".f-enabled").addEventListener("change", (e) => set(() => (t.enabled = e.target.checked), true));
    const setDur = () => {
      const m = Math.max(0, parseInt(q(".f-min").value) || 0);
      const s = Math.min(59, Math.max(0, parseInt(q(".f-sec").value) || 0));
      t.duration_ms = Math.max(1, m * 60 + s) * 1000;
      save(t);
    };
    q(".f-min").addEventListener("input", setDur);
    q(".f-sec").addEventListener("input", setDur);
    q(".f-cd").addEventListener("input", (e) => { t.cooldown_ms = Math.max(0, parseInt(e.target.value) || 0) * 1000; save(t); });
    q(".f-repeat").addEventListener("input", (e) => { t.repeat = Math.max(0, parseInt(e.target.value) || 0); save(t); });
    el.querySelectorAll(".f-mode button").forEach((b) => b.addEventListener("click", () => set(() => (t.mode = b.dataset.v), true)));
    el.querySelectorAll(".f-notify button").forEach((b) => b.addEventListener("click", () => set(() => (t.notify = b.dataset.v), true)));
    q(".f-comp").addEventListener("change", (e) => set(() => (t.overflow_comp = e.target.checked), true));
    q(".f-sound").addEventListener("change", (e) => set(() => (t.sound = e.target.value), true));
    q(".f-preview").addEventListener("click", () => run(() => tapi.preview(t)));
    q(".f-pick").addEventListener("click", () => run(async () => {
      const p = await tapi.pickFile();
      if (p) set(() => (t.sound_file = p), true);
    }));
    q(".f-ov").addEventListener("change", (e) => set(() => (t.overlay.show = e.target.checked), true));
    q(".f-lock").addEventListener("change", (e) => set(() => (t.overlay.locked = e.target.checked), true));
    q(".f-opacity").addEventListener("input", (e) => {
      t.overlay.opacity = Number(e.target.value) / 100;
      q(".f-op-val").textContent = `${e.target.value}%`;
      save(t);
    });
    q(".f-reset").addEventListener("click", () => run(() => tapi.resetOverlay(t.id)));
    q(".f-del")?.addEventListener("click", () => run(async () => {
      if (!confirm(`確定要刪除「${t.name}」嗎？`)) return;
      await tapi.del(t.id);
      file.timers = file.timers.filter((x) => x.id !== t.id);
      renderAll();
    }));
    q(".f-start").addEventListener("click", () => run(async () => {
      await tapi.start(t.id);
      if (!t.enabled) { t.enabled = true; fillCard(el, t); }
    }));
    q(".f-stop").addEventListener("click", () => run(() => tapi.stop(t.id)));
    q(".f-toggle").addEventListener("click", () => {
      openSettings.has(t.id) ? openSettings.delete(t.id) : openSettings.add(t.id);
      el.classList.toggle("open", openSettings.has(t.id));
    });
  }

  function renderAll() {
    const grid = $("#timerGrid");
    grid.innerHTML = "";
    for (const t of file.timers) {
      const el = document.createElement("div");
      el.className = "tcard";
      el.dataset.id = t.id;
      el.innerHTML = cardHtml(t);
      grid.appendChild(el);
      fillCard(el, t);
      wireCard(el, t);
      if (snaps[t.id]) paint(el, t, snaps[t.id]);
    }
    $("#gameOnly").checked = file.game_only;
    updateAllBtn();
  }

  function updateAllBtn() {
    const any = file.timers.some((t) => t.enabled);
    $("#allBtn").textContent = any ? "全部關閉" : "全部開啟";
  }

  /* ---------- 即時顯示 ---------- */
  function statusText(t, s) {
    if (!t.enabled) return "已停用";
    const cd = s.cooldown_left_ms > 0 ? ` · 冷卻 ${Math.ceil(s.cooldown_left_ms / 1000)} 秒` : "";
    switch (s.phase) {
      case "running": {
        const round = t.mode === "count" ? ` · 第 ${s.round}${t.repeat ? ` / ${t.repeat}` : ""} 輪` : "";
        return `倒數中${round}${cd}`;
      }
      case "done": return "已結束";
      case "over": return `超時${cd}`;
      default: return "待命";
    }
  }

  function paint(el, t, s) {
    const time = el.querySelector(".tc-time");
    let text;
    if (s.phase === "over") text = `+${fmt(Math.floor(s.over_ms / 1000))}`;
    else if (s.phase === "running") text = fmt(Math.ceil(s.remain_ms / 1000));
    else if (s.phase === "done") text = "00:00";
    else text = fmt(Math.round(t.duration_ms / 1000));
    if (time.textContent !== text) time.textContent = text;
    time.className = `tc-time ${s.phase}`;
    time.style.setProperty("--heat", redness(s).toFixed(3));
    el.querySelector(".tc-status").textContent = statusText(t, s);
    const startBtn = el.querySelector(".f-start");
    const label = s.phase === "running" || s.phase === "over" ? "重新計時" : "開始";
    if (startBtn.textContent !== label) startBtn.textContent = label;
    el.querySelector(".f-stop").disabled = s.phase === "idle";
  }

  function onTick(p) {
    for (const s of p.timers) {
      snaps[s.id] = s;
      const t = byId(s.id);
      const el = t && card(s.id);
      if (el) paint(el, t, s);
    }
    const chip = $("#fgChip");
    chip.classList.toggle("game", p.fg.is_game);
    chip.classList.toggle("paused", !p.listening);
    const who = p.fg.is_main ? "本程式" : p.fg.exe || "—";
    chip.querySelector("span").textContent = `前景：${who} · ${p.listening ? "監聽中" : "暫停監聽"}`;
  }

  /* ---------- 上方工具列 ---------- */
  $("#allBtn").addEventListener("click", () => run(async () => {
    const on = !file.timers.some((t) => t.enabled);
    file = await tapi.setAll(on);
    renderAll();
    toast(on ? "已全部開啟" : "已全部關閉");
  }));
  $("#gameOnly").addEventListener("change", (e) => run(async () => {
    file.game_only = e.target.checked;
    await tapi.gameOnly(e.target.checked);
  }));
  $("#addTimerBtn").addEventListener("click", () => run(async () => {
    const t = await tapi.add();
    file.timers.push(t);
    openSettings.add(t.id);
    renderAll();
    card(t.id)?.scrollIntoView({ behavior: "smooth", block: "nearest" });
    card(t.id)?.querySelector(".f-name").focus();
  }));
  document.addEventListener("click", (e) => {
    if (capturing && !e.target.closest(".keycap")) endCapture();
  });

  async function load() {
    file = await tapi.get();
    renderAll();
    loaded = true;
  }

  tapi.onTick(onTick);
  tapi.onCaptured(onCaptured);
  load();

  window.Timers = {
    onShow() { if (loaded) file.timers.forEach((t) => fillCard(card(t.id), t)); },
  };

  /* ---------- 瀏覽器預覽用的模擬後端 ---------- */
  function mockApi() {
    const ov = (i) => ({ show: true, locked: false, opacity: 0.9, x: 120, y: 160 + i * 130, w: 300, h: 110 });
    const base = { cooldown_ms: 0, mode: "key", repeat: 0, keys: [], overflow_comp: false, notify: "short", sound_file: "", enabled: true };
    const f = {
      game_only: true, next_id: 4,
      timers: [
        { ...base, id: "t1", name: "輪迴碑石", builtin: true, duration_ms: 590000, sound: "beep1", keys: ["S", "Left", "Right"], overlay: ov(0) },
        { ...base, id: "t2", name: "燃燒戒指", builtin: true, duration_ms: 905000, sound: "beep2", keys: ["Ctrl+F1"], overlay: ov(1) },
        { ...base, id: "t3", name: "靈魂亞努斯", builtin: true, duration_ms: 120000, cooldown_ms: 50000, sound: "beep3", keys: ["Mouse4"], overlay: ov(2) },
      ],
    };
    const rt = {};
    let tickCb = () => {}, capCb = () => {}, cap = false;
    setInterval(() => {
      const now = Date.now();
      tickCb({
        fg: { exe: "MapleStory.exe", is_game: true, is_self: false, is_main: false },
        listening: true,
        timers: f.timers.map((t) => {
          const r = rt[t.id];
          if (!r || !t.enabled) return { id: t.id, phase: "idle", remain_ms: t.duration_ms, over_ms: 0, total_ms: t.duration_ms, cooldown_left_ms: 0, round: 0 };
          const el = now - r.start;
          const cd = Math.max(0, t.cooldown_ms - el);
          if (el < r.total) return { id: t.id, phase: "running", remain_ms: r.total - el, over_ms: 0, total_ms: r.total, cooldown_left_ms: cd, round: 1 };
          return { id: t.id, phase: "over", remain_ms: 0, over_ms: el - r.total, total_ms: r.total, cooldown_left_ms: cd, round: 1 };
        }),
      });
    }, 100);
    document.addEventListener("keydown", (e) => {
      if (!cap) return;
      cap = false;
      if (e.key === "Escape") capCb({ key: "", cancel: true });
      else capCb({ key: [e.ctrlKey && "Ctrl", e.altKey && "Alt", e.shiftKey && "Shift", e.key.length === 1 ? e.key.toUpperCase() : e.key.replace("Arrow", "")].filter(Boolean).join("+"), cancel: false });
    });
    let n = 4;
    return {
      get: async () => JSON.parse(JSON.stringify(f)),
      update: async (t) => { const i = f.timers.findIndex((x) => x.id === t.id); f.timers[i] = JSON.parse(JSON.stringify(t)); },
      add: async () => { const t = { ...base, id: `u${n++}`, name: "新計時器", builtin: false, duration_ms: 60000, sound: "tts", overlay: ov(0) }; f.timers.push(t); return JSON.parse(JSON.stringify(t)); },
      del: async (id) => { f.timers = f.timers.filter((t) => t.id !== id); },
      start: async (id) => { const t = f.timers.find((x) => x.id === id); t.enabled = true; rt[id] = { start: Date.now(), total: t.duration_ms }; },
      stop: async (id) => { delete rt[id]; },
      setAll: async (on) => { f.timers.forEach((t) => { t.enabled = on; t.overlay.show = on; }); return JSON.parse(JSON.stringify(f)); },
      gameOnly: async (on) => { f.game_only = on; },
      resetOverlay: async () => {}, preview: async () => {}, pickFile: async () => null,
      capture: async () => { cap = true; }, cancelCapture: async () => { cap = false; },
      onTick: (cb) => { tickCb = cb; }, onCaptured: (cb) => { capCb = cb; },
    };
  }
})();
