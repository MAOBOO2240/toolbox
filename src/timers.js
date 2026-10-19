"use strict";

/* 計時器頁面：卡片直接編輯、全局設定面板、按鍵指派、即時顯示 */
(() => {
  const tapi = TAURI ? {
    get: () => invoke("get_timers"),
    update: (timer) => invoke("update_timer", { timer }),
    updatePrefs: (prefs) => invoke("update_prefs", { prefs }),
    add: () => invoke("add_timer"),
    del: (id) => invoke("delete_timer", { id }),
    start: (id) => invoke("timer_start", { id }),
    stop: (id) => invoke("timer_stop", { id }),
    setMaster: (on) => invoke("set_master", { on }),
    gameOnly: (on) => invoke("set_game_only", { on }),
    resetOverlay: () => invoke("reset_overlay"),
    preview: (prefs, name) => invoke("preview_sound", { prefs, name }),
    pickFile: () => invoke("pick_sound_file"),
    onTick: (cb) => listen("timer-tick", (e) => cb(e.payload)),
  } : mockApi();

  const MODE_TIPS = {
    normal: "一般：時間到通知後停止，按「開始」可再次計時。",
    count: "次數：時間到自動重新倒數，跑完設定的次數後停止（0 = 無限重複）。",
    key: "指派按鍵：時間到後進入超時並向上計時，按下指派按鍵才重新計時。",
  };
  const COMP_TIP = "超逾回補：超時期間按下指派按鍵，會以「原本時間 − 超時時間」重新計時；超時超過原本時間則用完整時間。";
  const KEY_LABEL = {
    Left: "←", Right: "→", Up: "↑", Down: "↓",
    Mouse4: "滑鼠側鍵(後)", Mouse5: "滑鼠側鍵(前)", MouseMid: "滑鼠中鍵",
    Space: "空白鍵", NumAdd: "Num+",
  };
  const keyLabel = (k) => k.split("+").map((p) => KEY_LABEL[p] || p).join(" + ");

  let file = { game_only: true, master_on: true, prefs: {}, timers: [] };
  let snaps = {};
  let editing = null; // 正在直接編輯時間的計時器 id
  let loaded = false;

  const byId = (id) => file.timers.find((t) => t.id === id);
  const card = (id) => document.querySelector(`.tcard[data-id="${CSS.escape(id)}"]`);

  /* ---------- 即時儲存（節流：立即送出，之後最多每 60ms 一次） ---------- */
  function throttled(fn, wait = 60) {
    const state = {};
    return (key, arg) => {
      const s = (state[key] ||= { timer: null, pending: null });
      if (s.timer) { s.pending = arg; return; }
      run(() => fn(arg));
      s.timer = setTimeout(function flush() {
        if (s.pending !== null) {
          const a = s.pending;
          s.pending = null;
          run(() => fn(a));
          s.timer = setTimeout(flush, wait);
        } else {
          s.timer = null;
        }
      }, wait);
    };
  }
  const pushTimer = throttled((t) => tapi.update(JSON.parse(JSON.stringify(t))));
  const pushPrefs = throttled((p) => tapi.updatePrefs(JSON.parse(JSON.stringify(p))));
  const saveTimer = (t) => pushTimer(t.id, t);
  const savePrefs = () => pushPrefs("prefs", file.prefs);

  /* ---------- 格式 ---------- */
  const pad = (n) => String(n).padStart(2, "0");
  function fmt(totalSec) {
    totalSec = Math.max(0, totalSec);
    const h = Math.floor(totalSec / 3600);
    const m = Math.floor((totalSec % 3600) / 60);
    const s = totalSec % 60;
    return h > 0 ? `${h}:${pad(m)}:${pad(s)}` : `${pad(m)}:${pad(s)}`;
  }

  /** 剩餘 30% 起由原色漸變為紅色，回傳 0~1 */
  function heat(snap) {
    if (snap.phase !== "running" || !snap.total_ms) return 0;
    const frac = snap.remain_ms / snap.total_ms;
    return frac >= 0.3 ? 0 : Math.min(1, 1 - frac / 0.3);
  }

  /* ---------- 卡片 ---------- */
  const sw = (cls) => `<label class="switch sm"><input type="checkbox" class="${cls}" /><span class="slider"></span></label>`;
  const TRASH = '<svg viewBox="0 0 24 24"><path d="M4 7h16M10 11v6M14 11v6M6 7l1 12a2 2 0 0 0 2 2h6a2 2 0 0 0 2-2l1-12M9 7V4h6v3"/></svg>';

  function cardHtml(t) {
    return `
      <div class="tc-head">
        <input class="tc-name f-name" maxlength="20" spellcheck="false" title="點擊修改名稱" />
        <span class="spacer"></span>
        <button class="icon-btn f-del" title="刪除計時器">${TRASH}</button>
        ${sw("f-enabled")}
      </div>
      <div class="tc-clock">
        <div class="tc-time-wrap">
          <button class="tc-time f-time" title="點擊直接修改時間">00:00</button>
          <div class="tc-edit">
            <input type="number" class="f-min" min="0" max="999" /><span>:</span><input type="number" class="f-sec" min="0" max="59" />
          </div>
          <label class="tc-cd" title="冷卻時間：重新計時後的這段時間內按鍵無效">CD<input type="number" class="f-cd" min="0" max="3600" />s</label>
        </div>
        <div class="tc-actions">
          <button class="pill primary small f-start">開始</button>
          <button class="pill ghost danger small f-stop">結束</button>
        </div>
      </div>
      <div class="tc-opts">
        <div class="field"><span class="lbl">重新計時</span>
          <div class="seg f-mode">
            <button data-v="normal" data-tip="${MODE_TIPS.normal}">一般</button>
            <button data-v="count" data-tip="${MODE_TIPS.count}">次數</button>
            <button data-v="key" data-tip="${MODE_TIPS.key}">指派按鍵</button>
          </div>
        </div>
        <div class="field r-repeat"><span class="lbl">次數</span>
          <div class="inline"><input type="number" class="num-in f-repeat" min="0" max="9999" /><span>次</span><small class="muted">0 = 無限重複</small></div></div>
        <div class="field r-keys"><span class="lbl">指派按鍵</span><div class="keys f-keys"></div></div>
        <div class="field r-comp"><span class="lbl" data-tip="${COMP_TIP}">超逾回補 <i class="info">i</i></span>${sw("f-comp")}</div>
      </div>`;
  }

  function fillCard(el, t) {
    if (!el) return;
    const q = (s) => el.querySelector(s);
    const focused = document.activeElement;
    if (focused !== q(".f-name")) q(".f-name").value = t.name;
    if (focused !== q(".f-cd")) q(".f-cd").value = Math.round(t.cooldown_ms / 1000);
    fitCd(q(".f-cd"));
    if (focused !== q(".f-repeat")) q(".f-repeat").value = t.repeat;
    q(".f-enabled").checked = t.enabled;
    q(".f-comp").checked = t.overflow_comp;
    el.querySelectorAll(".f-mode button").forEach((b) => b.classList.toggle("on", b.dataset.v === t.mode));
    q(".r-repeat").hidden = t.mode !== "count";
    q(".r-keys").hidden = t.mode !== "key"; // 只有「指派按鍵」模式才使用按鍵
    q(".r-comp").hidden = t.mode !== "key";
    el.classList.toggle("disabled", !t.enabled);
    el.classList.toggle("editing", editing === t.id);
    renderKeys(el, t);
  }

  /** CD 輸入框寬度跟著數字位數 */
  function fitCd(inp) {
    inp.style.width = `${Math.max(1, String(inp.value).length) + 0.3}ch`;
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
      b.textContent = waiting ? "按新按鍵…（Esc 取消指派）" : keyLabel(k);
      b.title = "點擊後按新按鍵替換；按 Esc 取消這個指派";
      b.addEventListener("click", (e) => { e.stopPropagation(); beginCapture(t.id, i); });
      box.appendChild(b);
    });
    const add = document.createElement("button");
    add.className = "keycap add";
    const waitingNew = cap && cap.index === null;
    add.classList.toggle("waiting", !!waitingNew);
    add.textContent = waitingNew ? "請按按鍵…（Esc 取消）" : "＋ 指派按鍵";
    add.addEventListener("click", (e) => { e.stopPropagation(); beginCapture(t.id, null); });
    box.appendChild(add);
  }

  /* ---------- 直接修改時間 ---------- */
  function openEditor(id) {
    if (editing && editing !== id) closeEditor();
    const t = byId(id);
    const el = card(id);
    editing = id;
    el.classList.add("editing");
    const mi = el.querySelector(".f-min"), si = el.querySelector(".f-sec");
    mi.value = Math.floor(t.duration_ms / 60000);
    mi.style.width = `${Math.max(2, String(mi.value).length) + 0.3}ch`;
    si.value = pad(Math.floor((t.duration_ms % 60000) / 1000));
    mi.focus();
    mi.select();
  }

  function closeEditor() {
    if (!editing) return;
    const el = card(editing);
    editing = null;
    if (el) el.classList.remove("editing");
  }

  /* ---------- 按鍵指派（在程式視窗內擷取） ---------- */
  let capturing = null; // { id, index }
  let pendingMod = null;

  const CODE = {
    Space: "Space", Enter: "Enter", NumpadEnter: "Enter", Tab: "Tab", Escape: "Esc", Backspace: "Backspace",
    Insert: "Insert", Delete: "Delete", Home: "Home", End: "End", PageUp: "PageUp", PageDown: "PageDown",
    ArrowLeft: "Left", ArrowRight: "Right", ArrowUp: "Up", ArrowDown: "Down",
    CapsLock: "CapsLock", Pause: "Pause", ScrollLock: "ScrollLock", NumLock: "NumLock", PrintScreen: "PrintScreen",
    ShiftLeft: "Shift", ShiftRight: "Shift", ControlLeft: "Ctrl", ControlRight: "Ctrl",
    AltLeft: "Alt", AltRight: "Alt", MetaLeft: "Win", MetaRight: "Win",
    NumpadMultiply: "Num*", NumpadAdd: "NumAdd", NumpadSubtract: "Num-", NumpadDecimal: "Num.", NumpadDivide: "Num/",
    Semicolon: ";", Equal: "=", Comma: ",", Minus: "-", Period: ".", Slash: "/", Backquote: "`",
    BracketLeft: "[", Backslash: "\\", BracketRight: "]", Quote: "'",
  };
  const MODS = ["Ctrl", "Alt", "Shift", "Win"];

  function codeName(code) {
    if (CODE[code]) return CODE[code];
    let m;
    if ((m = /^Key([A-Z])$/.exec(code))) return m[1];
    if ((m = /^Digit(\d)$/.exec(code))) return m[1];
    if ((m = /^Numpad(\d)$/.exec(code))) return `Num${m[1]}`;
    if ((m = /^F(\d{1,2})$/.exec(code))) return `F${m[1]}`;
    return null;
  }

  function combo(e, key) {
    const held = { Ctrl: e.ctrlKey, Alt: e.altKey, Shift: e.shiftKey, Win: e.metaKey };
    return [...MODS.filter((m) => held[m] && m !== key), key].join("+");
  }

  function beginCapture(id, index) {
    if (document.activeElement) document.activeElement.blur();
    const prev = capturing;
    capturing = { id, index };
    pendingMod = null;
    if (prev && prev.id !== id) fillCard(card(prev.id), byId(prev.id));
    renderKeys(card(id), byId(id));
  }

  function endCapture() {
    if (!capturing) return;
    const { id } = capturing;
    capturing = null;
    const t = byId(id);
    if (t) renderKeys(card(id), t);
  }

  function finishCapture(key, cancel) {
    const { id, index } = capturing;
    capturing = null;
    pendingMod = null;
    const t = byId(id);
    if (!t) return;
    if (cancel) {
      if (index !== null) { t.keys.splice(index, 1); toast("已取消指派按鍵"); saveTimer(t); }
    } else if (index !== null) {
      t.keys[index] = key;
      t.keys = [...new Set(t.keys)];
      saveTimer(t);
    } else if (!t.keys.includes(key)) {
      t.keys.push(key);
      saveTimer(t);
    }
    renderKeys(card(id), t);
  }

  document.addEventListener("keydown", (e) => {
    if (!capturing) return;
    e.preventDefault();
    e.stopPropagation();
    if (e.repeat) return;
    const name = codeName(e.code);
    if (!name) return;
    if (MODS.includes(name)) { pendingMod = name; return; } // 只按修飾鍵：放開時才算
    pendingMod = null;
    if (name === "Esc" && !e.ctrlKey && !e.altKey && !e.shiftKey && !e.metaKey) finishCapture("", true);
    else finishCapture(combo(e, name), false);
  }, true);

  document.addEventListener("keyup", (e) => {
    if (!capturing) return;
    e.preventDefault();
    const name = codeName(e.code);
    if (pendingMod && name === pendingMod) finishCapture(combo(e, name), false);
  }, true);

  // 滑鼠側鍵、中鍵
  document.addEventListener("mousedown", (e) => {
    if (!capturing || ![1, 3, 4].includes(e.button)) return;
    e.preventDefault();
    e.stopPropagation();
    finishCapture(combo(e, e.button === 3 ? "Mouse4" : e.button === 4 ? "Mouse5" : "MouseMid"), false);
  }, true);
  // 避免側鍵觸發上一頁／下一頁
  document.addEventListener("mouseup", (e) => { if ([3, 4].includes(e.button)) e.preventDefault(); }, true);

  document.addEventListener("click", (e) => {
    if (capturing && !e.target.closest(".keycap")) endCapture();
    if (editing && !e.target.closest(".tc-time-wrap")) closeEditor();
  });

  /* ---------- 卡片事件 ---------- */
  function wireCard(el, t) {
    const q = (s) => el.querySelector(s);
    const setNow = (fn) => { fn(); fillCard(el, t); saveTimer(t); };

    q(".f-name").addEventListener("input", (e) => { t.name = e.target.value.trim() || "計時器"; saveTimer(t); });
    q(".f-enabled").addEventListener("change", (e) => setNow(() => (t.enabled = e.target.checked)));

    q(".f-time").addEventListener("click", (e) => { e.stopPropagation(); openEditor(t.id); });
    const setDur = () => {
      const m = Math.max(0, parseInt(q(".f-min").value) || 0);
      const s = Math.min(59, Math.max(0, parseInt(q(".f-sec").value) || 0));
      t.duration_ms = Math.max(1, m * 60 + s) * 1000;
      saveTimer(t);
      paint(el, t, snaps[t.id]);
    };
    q(".f-min").addEventListener("input", (e) => { e.target.style.width = `${Math.max(2, e.target.value.length) + 0.3}ch`; setDur(); });
    q(".f-sec").addEventListener("input", setDur);
    el.querySelectorAll(".tc-edit input").forEach((inp) =>
      inp.addEventListener("keydown", (e) => { if (e.key === "Enter" || e.key === "Escape") closeEditor(); })
    );

    q(".f-cd").addEventListener("input", (e) => { t.cooldown_ms = Math.max(0, parseInt(e.target.value) || 0) * 1000; fitCd(e.target); saveTimer(t); });
    q(".f-repeat").addEventListener("input", (e) => { t.repeat = Math.max(0, parseInt(e.target.value) || 0); saveTimer(t); });
    el.querySelectorAll(".f-mode button").forEach((b) => b.addEventListener("click", () => setNow(() => (t.mode = b.dataset.v))));
    q(".f-comp").addEventListener("change", (e) => setNow(() => (t.overflow_comp = e.target.checked)));
    q(".f-del")?.addEventListener("click", () => run(async () => {
      if (!(await confirmBox(`確定要刪除「${t.name}」嗎？`))) return;
      await tapi.del(t.id);
      file.timers = file.timers.filter((x) => x.id !== t.id);
      renderAll();
    }));
    q(".f-start").addEventListener("click", () => run(async () => {
      await tapi.start(t.id);
      if (!t.enabled) { t.enabled = true; fillCard(el, t); }
      if (!file.master_on) { file.master_on = true; renderMaster(); }
    }));
    q(".f-stop").addEventListener("click", () => run(() => tapi.stop(t.id)));
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
      paint(el, t, snaps[t.id]);
    }
    // 最後一格：新增計時器
    const add = document.createElement("div");
    add.className = "tcard tadd";
    add.innerHTML = '<button class="tadd-circle" title="新增計時器"><svg viewBox="0 0 24 24"><path d="M12 5v14M5 12h14"/></svg></button>';
    add.querySelector("button").addEventListener("click", () => addTimer());
    grid.appendChild(add);
    // 和一張計時器卡片一樣高，圓圈落在那個位置的正中間
    const first = grid.querySelector(".tcard:not(.tadd)");
    if (first) add.style.minHeight = `${first.offsetHeight}px`;
    $("#gameOnly").checked = file.game_only;
    renderMaster();
    renderPrefs();
  }

  function renderMaster() {
    const btn = $("#masterBtn");
    btn.classList.toggle("off", !file.master_on);
    btn.querySelector("span").textContent = file.master_on ? "全部關閉" : "全部開啟";
    $("#timerGrid").classList.toggle("paused", !file.master_on);
  }

  /* ---------- 即時顯示 ---------- */
  function paint(el, t, s) {
    if (!el) return;
    s = s || { phase: "idle", remain_ms: t.duration_ms, over_ms: 0, total_ms: t.duration_ms, cooldown_left_ms: 0, round: 0 };
    const time = el.querySelector(".f-time");
    let text;
    if (s.phase === "over") text = `+${fmt(Math.floor(s.over_ms / 1000))}`;
    else if (s.phase === "running") text = fmt(Math.ceil(s.remain_ms / 1000));
    else if (s.phase === "done") text = "00:00";
    else text = fmt(Math.round(t.duration_ms / 1000));
    if (time.textContent !== text) time.textContent = text;
    time.className = `tc-time f-time ${s.phase}`;
    time.style.setProperty("--heat", heat(s).toFixed(3));
    const startBtn = el.querySelector(".f-start");
    const label = s.phase === "running" || s.phase === "over" ? "重新計時" : "開始";
    if (startBtn.textContent !== label) startBtn.textContent = label;
    el.querySelector(".f-stop").disabled = s.phase === "idle";
    const cdLabel = el.querySelector(".tc-cd");
    cdLabel.classList.toggle("cooling", s.cooldown_left_ms > 0);
  }

  function onTick(p) {
    for (const s of p.timers) {
      snaps[s.id] = s;
      const t = byId(s.id);
      if (t) paint(card(s.id), t, s);
    }
    if (p.master_on !== file.master_on) { file.master_on = p.master_on; renderMaster(); }
    const chip = $("#fgChip");
    chip.classList.toggle("game", p.fg.is_game);
    chip.classList.toggle("paused", !p.listening);
    const who = p.fg.is_self ? "本程式" : p.fg.exe || "—";
    chip.querySelector("span").textContent = `前景：${who} · ${p.listening ? "監聽中" : "暫停監聽"}`;
    $("#keyChip").textContent = `最後按鍵：${p.last_key ? keyLabel(p.last_key) : "—"}`;
  }

  /* ---------- 全局設定（上方：通知；左下：浮動視窗） ---------- */

  function renderPrefs() {
    const p = file.prefs;
    $$("#prefNotify button").forEach((b) => b.classList.toggle("on", b.dataset.v === p.notify));
    // 移除的音效（撞到木櫃）改回柔和木琴
    if (![...$("#prefSound").options].some((o) => o.value === p.sound)) p.sound = "beep1";
    $("#prefSound").value = p.sound;
    $("#prefFileRow").hidden = p.sound !== "file";
    $("#prefFileName").textContent = p.sound_file ? p.sound_file.split(/[\\/]/).pop() : "尚未選擇";
    $("#prefVolume").value = p.volume;
    if (document.activeElement !== $("#prefVolumeVal")) $("#prefVolumeVal").value = p.volume;
    $("#prefOvShow").checked = p.overlay.show;
    $("#prefOvLock").checked = p.overlay.locked;
    // 介面顯示「透明度」= 100% − 底色濃度
    const trans = Math.round((1 - p.overlay.bg) * 100);
    $("#prefOvTrans").value = trans;
    if (document.activeElement !== $("#prefOvTransVal")) $("#prefOvTransVal").value = trans;
  }
  const setPref = (fn) => { fn(file.prefs); renderPrefs(); savePrefs(); };

  $$("#prefNotify button").forEach((b) => b.addEventListener("click", () => setPref((p) => (p.notify = b.dataset.v))));
  $("#prefSound").addEventListener("change", (e) => setPref((p) => (p.sound = e.target.value)));
  $("#prefPick").addEventListener("click", () => run(async () => {
    const f = await tapi.pickFile();
    if (f) setPref((p) => (p.sound_file = f));
  }));
  const clampNum = (v, lo, hi) => Math.min(hi, Math.max(lo, Math.round(Number(v) || 0)));
  $("#prefVolume").addEventListener("input", (e) => setPref((p) => (p.volume = Number(e.target.value))));
  $("#prefVolumeVal").addEventListener("input", (e) => {
    if (e.target.value === "") return;
    setPref((p) => (p.volume = clampNum(e.target.value, 0, 200)));
  });
  $("#prefVolumeVal").addEventListener("blur", (e) => (e.target.value = file.prefs.volume));
  $("#prefPreview").addEventListener("click", () => run(() => tapi.preview(file.prefs, file.timers[0]?.name || "計時器")));
  $("#prefOvShow").addEventListener("change", (e) => setPref((p) => (p.overlay.show = e.target.checked)));
  $("#prefOvLock").addEventListener("change", (e) => setPref((p) => (p.overlay.locked = e.target.checked)));
  $("#prefOvTrans").addEventListener("input", (e) => setPref((p) => (p.overlay.bg = 1 - Number(e.target.value) / 100)));
  $("#prefOvTransVal").addEventListener("input", (e) => {
    if (e.target.value === "") return;
    setPref((p) => (p.overlay.bg = 1 - clampNum(e.target.value, 0, 100) / 100));
  });
  $("#prefOvTransVal").addEventListener("blur", (e) => (e.target.value = Math.round((1 - file.prefs.overlay.bg) * 100)));
  $("#prefOvReset").addEventListener("click", () => run(() => tapi.resetOverlay()));
  // 浮動視窗建立失敗時顯示原因
  if (TAURI) listen("overlay-error", (e) => toast(String(e.payload)));

  /* ---------- 上方與左下控制 ---------- */
  $("#masterBtn").addEventListener("click", () => run(async () => {
    const on = !file.master_on;
    await tapi.setMaster(on);
    file.master_on = on;
    // 總開關連動浮動視窗的顯示與鎖定（後端會記住關閉前的狀態）
    try { file.prefs = (await tapi.get()).prefs; } catch (_) { file.prefs.overlay.show = on; }
    renderMaster();
    renderPrefs();
    file.timers.forEach((t) => fillCard(card(t.id), t));
    toast(on ? "已開啟（只恢復原本有啟用的計時器）" : "已關閉所有計時器");
  }));
  $("#gameOnly").addEventListener("change", (e) => run(async () => {
    file.game_only = e.target.checked;
    await tapi.gameOnly(e.target.checked);
  }));
  const addTimer = () => run(async () => {
    const t = await tapi.add();
    file.timers.push(t);
    renderAll();
    const el = card(t.id);
    el?.scrollIntoView({ behavior: "smooth", block: "nearest" });
    el?.querySelector(".f-name").select();
  });

  async function load() {
    file = await tapi.get();
    renderAll();
    loaded = true;
  }

  tapi.onTick(onTick);
  load();

  window.Timers = {
    onShow() { if (loaded) file.timers.forEach((t) => fillCard(card(t.id), t)); },
  };

  /* ---------- 瀏覽器預覽用的模擬後端 ---------- */
  function mockApi() {
    const base = { cooldown_ms: 0, mode: "key", repeat: 0, keys: [], overflow_comp: false, enabled: true };
    const f = {
      game_only: true, master_on: true, next_id: 4,
      prefs: { notify: "short", sound: "beep1", sound_file: "", volume: 100, overlay: { show: false, locked: false, bg: 0.35, x: 120, y: 160, w: 340, h: 180 } },
      timers: [
        { ...base, id: "t1", name: "輪迴碑石", builtin: true, duration_ms: 590000, keys: ["S", "Left", "Right"] },
        { ...base, id: "t2", name: "燃燒戒指", builtin: true, duration_ms: 905000, keys: ["Ctrl+F1"] },
        { ...base, id: "t3", name: "靈魂亞努斯", builtin: true, duration_ms: 120000, cooldown_ms: 50000, keys: ["Mouse4"] },
      ],
    };
    const rt = {};
    let tickCb = () => {};
    setInterval(() => {
      const now = Date.now();
      tickCb({
        fg: { exe: "MapleStory.exe", is_game: true, is_self: false, is_main: false },
        listening: true, last_key: "Ctrl+S", master_on: f.master_on,
        overlay: { bg: f.prefs.overlay.bg, locked: f.prefs.overlay.locked },
        timers: f.timers.map((t) => {
          const r = rt[t.id];
          const base = { id: t.id, name: t.name, active: f.master_on && t.enabled, round: 1 };
          if (!r || !base.active) return { ...base, phase: "idle", remain_ms: t.duration_ms, over_ms: 0, total_ms: t.duration_ms, cooldown_left_ms: 0, round: 0 };
          const el = now - r.start;
          const cd = Math.max(0, t.cooldown_ms - el);
          if (el < r.total) return { ...base, phase: "running", remain_ms: r.total - el, over_ms: 0, total_ms: r.total, cooldown_left_ms: cd };
          return { ...base, phase: "over", remain_ms: 0, over_ms: el - r.total, total_ms: r.total, cooldown_left_ms: cd };
        }),
      });
    }, 100);
    let n = 4;
    const clone = (x) => JSON.parse(JSON.stringify(x));
    return {
      get: async () => clone(f),
      update: async (t) => { const i = f.timers.findIndex((x) => x.id === t.id); f.timers[i] = clone(t); },
      updatePrefs: async (p) => { f.prefs = clone(p); },
      add: async () => { const t = { ...base, id: `u${n++}`, name: "新計時器", builtin: false, duration_ms: 60000 }; f.timers.push(t); return clone(t); },
      del: async (id) => { f.timers = f.timers.filter((t) => t.id !== id); },
      start: async (id) => { const t = f.timers.find((x) => x.id === id); t.enabled = true; f.master_on = true; rt[id] = { start: Date.now(), total: t.duration_ms }; },
      stop: async (id) => { delete rt[id]; },
      setMaster: async (on) => { f.master_on = on; f.prefs.overlay.show = on; },
      gameOnly: async (on) => { f.game_only = on; },
      resetOverlay: async () => {}, preview: async () => {}, pickFile: async () => null,
      onTick: (cb) => { tickCb = cb; },
    };
  }
})();
