/* 角色名稱歷史：所有「輸入角色名稱」的欄位共用一份（查詢成功過的才會記），點欄位就列出來，可以一筆一筆刪。
   取代 WebView2 內建那個「已儲存資訊」的自動填入視窗（樣式醜、而且只記表單送出過的）。 */
window.NameHistory = (() => {
  const KEY = "char-names-v1", MAX = 30;
  const read = () => { try { return (JSON.parse(localStorage.getItem(KEY)) || []).filter((n) => typeof n === "string" && n); } catch (_) { return []; } };
  const write = (l) => { try { localStorage.setItem(KEY, JSON.stringify(l.slice(0, MAX))); } catch (_) {} };
  /** 查詢成功後記下來（最近用的排前面） */
  function add(name) {
    const n = String(name || "").trim();
    if (!n) return;
    write([n, ...read().filter((x) => x !== n)]);
  }
  function remove(name) { write(read().filter((x) => x !== name)); }

  let pop = null, cur = null, picking = false; // cur = { input, onPick, items, sel }
  function ensure() {
    if (pop) return pop;
    pop = document.createElement("div");
    pop.className = "nh-pop";
    pop.hidden = true;
    // 用 mousedown 擋掉 blur，點名稱才接得到
    pop.addEventListener("mousedown", (e) => e.preventDefault());
    document.body.appendChild(pop);
    window.addEventListener("resize", () => place(), { passive: true });
    document.addEventListener("scroll", () => place(), true);
    // 點到欄位和清單以外的地方、或按 Esc：一定關掉（不靠欄位的 blur）。
    // pointerdown、mousedown、touchstart 都接（有些地方的 pointerdown 會把 mousedown 吃掉），在 window 的捕獲階段最先收到
    const outside = (e) => { if (cur && !pop.hidden && !pop.contains(e.target) && e.target !== cur.input) hide(); };
    for (const ev of ["pointerdown", "mousedown", "touchstart"]) window.addEventListener(ev, outside, true);
    // 切到別的視窗也關
    window.addEventListener("blur", () => hide());
    document.addEventListener("keydown", (e) => { if (e.key === "Escape" && cur && !pop.hidden) hide(); }, true);
    // 換頁一定關掉
    document.addEventListener("viewchange", hide);
    return pop;
  }
  function place() {
    if (!pop || pop.hidden || !cur) return;
    const r = cur.input.getBoundingClientRect();
    pop.style.left = `${Math.round(r.left)}px`;
    pop.style.top = `${Math.round(r.bottom + 4)}px`;
    pop.style.minWidth = `${Math.round(r.width)}px`;
  }
  function hide() { if (pop) pop.hidden = true; cur = null; }
  function render() {
    const p = ensure();
    p.innerHTML = "";
    const title = document.createElement("div");
    title.className = "nh-title";
    title.textContent = "用過的角色";
    p.appendChild(title);
    const list = document.createElement("div");
    list.className = "nh-list";
    p.appendChild(list);
    cur.items.forEach((name, i) => {
      const row = document.createElement("div");
      row.className = "nh-row" + (i === cur.sel ? " on" : "");
      const t = document.createElement("span");
      t.className = "nh-name";
      t.textContent = name;
      const x = document.createElement("button");
      x.type = "button";
      x.className = "nh-x";
      x.title = "從記錄移除";
      x.textContent = "×";
      x.addEventListener("click", (e) => { e.stopPropagation(); remove(name); show(cur.input, cur.onPick); });
      row.addEventListener("click", () => pick(name));
      row.append(t, x);
      list.appendChild(row);
    });
    // 鍵盤選到的那列要在可見範圍內
    list.querySelector(".nh-row.on")?.scrollIntoView({ block: "nearest" });
  }
  function pick(name) {
    const { input, onPick } = cur;
    hide();
    picking = true;
    input.value = name;
    input.dispatchEvent(new Event("input", { bubbles: true }));
    picking = false;
    if (onPick) onPick(name);
  }
  function show(input, onPick) {
    if (picking || input.readOnly || input.disabled) return;
    // 全部列出（不照欄位裡的字過濾，才不用先把名稱刪掉才能選別的）；超過 5 個用滾輪
    const items = read();
    if (!items.length) { hide(); return; }
    cur = { input, onPick, items, sel: -1 };
    render();
    ensure().hidden = false;
    place();
  }
  /** 掛到一個輸入框上：點／打字時列出用過的名稱，Enter 選取，× 移除 */
  function attach(input, { onPick } = {}) {
    if (!input || input.dataset.nh) return;
    input.dataset.nh = "1";
    input.setAttribute("autocomplete", "off");
    input.setAttribute("spellcheck", "false");
    input.addEventListener("focus", () => show(input, onPick));
    input.addEventListener("click", () => { if (!cur) show(input, onPick); });
    input.addEventListener("input", () => { if (cur && cur.input === input) render(); });
    input.addEventListener("blur", () => setTimeout(() => { if (cur && cur.input === input && document.activeElement !== input) hide(); }, 120));
    input.addEventListener("keydown", (e) => {
      if (!cur || cur.input !== input || pop.hidden) return;
      if (e.key === "ArrowDown" || e.key === "ArrowUp") {
        e.preventDefault();
        const n = cur.items.length;
        cur.sel = (cur.sel + (e.key === "ArrowDown" ? 1 : -1) + n) % n;
        render();
      } else if (e.key === "Enter" && cur.sel >= 0) {
        e.preventDefault();
        e.stopImmediatePropagation();
        pick(cur.items[cur.sel]);
      } else if (e.key === "Enter" || e.key === "Escape") { hide(); }
    }, true);
  }
  return { attach, add, remove, list: read, hide };
})();
