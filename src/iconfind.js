/* 點裝辨識：丟一張截圖（裝備欄、背包、商城），把裡面的道具格子拿去跟遊戲資料的點裝圖示比對，依欄位列出最像的幾件。
   左鍵：道具說明（跟更新道具一樣的視窗，名稱旁有收藏）；右鍵：複製名稱並加到變更點裝的收藏。 */
(() => {
  const { invoke, toast, $, TAURI } = window.App;
  const gameDir = () => { try { return JSON.parse(localStorage.getItem("beauty-v1"))?.gamePath || null; } catch (_) { return null; } };
  const SLOT_NAME = { Cap: "帽子", Face: "臉飾", Eye: "眼飾", Ear: "耳環", Coat: "上衣", Longcoat: "套服", Pants: "褲裙", Shoes: "鞋子", Glove: "手套", Cape: "披風", Shield: "輔助武器", Weapon: "武器", RingTag: "名牌戒指", RingChat: "聊天戒指" };
  const SLOT_COLOR = { Cap: "c-pink", Face: "c-purple", Eye: "c-blue", Ear: "c-amber", Coat: "c-orange", Longcoat: "c-red", Pants: "c-teal", Shoes: "c-green", Glove: "c-silver", Cape: "c-purple", Shield: "c-blue", Weapon: "c-orange", RingTag: "c-indigo", RingChat: "c-pink" };
  /** 欄位 → 資料夾（更新道具的說明視窗要用） */
  const folderOf = (slot) => `Character/${{ Face: "Accessory", Eye: "Accessory", Ear: "Accessory", RingTag: "Ring", RingChat: "Ring" }[slot] || slot}`;

  const drop = $("#ifDrop"), preview = $("#ifPreview"), dropText = $("#ifDropText"), status = $("#ifStatus"), results = $("#ifResults");
  let active = false, busy = false, lastPng = null, pollTimer = null;

  /* ---------- 圖示索引（第一次要建） ---------- */
  async function waitIndex() {
    // 回傳 true = 可以比對；false = 還在建（會顯示進度並稍後重試）
    let st;
    try { st = await invoke("iconidx_status", { gameDir: gameDir(), start: true }); } catch (e) { showStatus(String(e), true); return false; }
    if (st.ready) return true;
    if (st.error) { showStatus(`圖示索引建立失敗：${st.error}`, true); return false; }
    const pct = st.total ? Math.round((st.done / st.total) * 100) : 0;
    showStatus(`第一次使用要先建立圖示索引（讀取遊戲裡所有點裝的圖示，約一分鐘）… ${st.total ? `${st.done} / ${st.total}（${pct}%）` : "準備中"}`, false, pct);
    return false;
  }
  function showStatus(text, isErr, pct) {
    status.hidden = false;
    status.className = "card if-status" + (isErr ? " err" : "");
    status.innerHTML = "";
    const t = document.createElement("div"); t.textContent = text; status.append(t);
    if (pct != null) { const bar = document.createElement("div"); bar.className = "if-bar"; const fill = document.createElement("i"); fill.style.width = `${pct}%`; bar.append(fill); status.append(bar); }
  }
  function hideStatus() { status.hidden = true; }

  /* ---------- 讀圖：檔案、拖曳、貼上 → PNG ---------- */
  async function toPng(blobOrUrl) {
    const url = typeof blobOrUrl === "string" ? blobOrUrl : URL.createObjectURL(blobOrUrl);
    try {
      const img = await new Promise((res, rej) => { const i = new Image(); i.onload = () => res(i); i.onerror = () => rej(new Error("圖片讀不出來")); i.src = url; });
      const cv = document.createElement("canvas");
      cv.width = img.naturalWidth; cv.height = img.naturalHeight;
      cv.getContext("2d").drawImage(img, 0, 0);
      return cv.toDataURL("image/png");
    } finally { if (typeof blobOrUrl !== "string") URL.revokeObjectURL(url); }
  }
  async function handle(blob) {
    if (!blob || !/^image\//.test(blob.type)) { toast("這不是圖片"); return; }
    let dataUrl;
    try { dataUrl = await toPng(blob); } catch (e) { toast(String(e.message || e)); return; }
    preview.src = dataUrl; preview.hidden = false; dropText.hidden = true;
    lastPng = dataUrl.split(",")[1];
    run();
  }
  async function run() {
    if (!TAURI || !lastPng || busy) return;
    busy = true;
    results.innerHTML = "";
    try {
      if (!(await waitIndex())) {
        // 還在建索引：每秒看一次，好了就自動比對
        clearTimeout(pollTimer);
        pollTimer = setTimeout(() => { busy = false; if (active && lastPng) run(); }, 1000);
        return;
      }
      showStatus("比對中…");
      const out = await invoke("icon_match", { gameDir: gameDir(), pngB64: lastPng, cashOnly: $("#ifCash").checked });
      render(out);
    } catch (e) { showStatus(String(e), true); }
    finally { busy = false; }
  }

  /* ---------- 結果 ---------- */
  const iconCache = new Map();
  async function loadIcons(ids) {
    const need = ids.filter((id) => !iconCache.has(id));
    if (!need.length) return;
    try {
      const pngs = await invoke("beauty_icons", { gameDir: gameDir(), ids: need });
      need.forEach((id, i) => iconCache.set(id, pngs[i] || null));
    } catch (_) { need.forEach((id) => iconCache.set(id, null)); }
  }
  function detail(hit) {
    window.ItemUpdates?.openDetail?.({ id: hit.id, folder: folderOf(hit.slot), name: hit.name, cash: hit.cash });
  }
  async function favCopy(hit) {
    const item = { id: hit.id, name: hit.name, slot: hit.slot };
    try { await invoke("copy_text", { text: hit.name }); } catch (_) {}
    if (window.Beauty?.isFav?.(hit.id)) toast(`已複製「${hit.name}」（已經在收藏裡）`);
    else { window.Beauty?.addFav?.(item); toast(`已複製「${hit.name}」並加到變更點裝的收藏`); }
  }
  async function render(out) {
    hideStatus();
    results.innerHTML = "";
    if (!out.groups.length) {
      showStatus(out.candidates ? `找到 ${out.candidates} 個格子，但都對不到夠像的點裝。試試只截道具格子的部分，或關掉「只找點裝」。` : "圖裡找不到像道具格子的東西。請截裝備欄或背包的畫面（格子要看得清楚）。", true);
      return;
    }
    const head = document.createElement("div");
    head.className = "muted if-note";
    head.textContent = `圖裡找到 ${out.candidates} 個格子${out.big ? "，另外有大塊的圖（例如整隻角色）只能大概猜，那部分不一定準" : ""}。每一排最像的在最左邊；左鍵看說明、右鍵複製名稱並收藏。`;
    results.append(head);
    await loadIcons(out.groups.flatMap((g) => g.items.map((h) => h.id)));
    for (const g of out.groups) {
      const row = document.createElement("div");
      row.className = "card if-row";
      const tag = document.createElement("span");
      tag.className = `cbtn small solid ${SLOT_COLOR[g.slot] || "c-silver"} if-tag`;
      tag.textContent = SLOT_NAME[g.slot] || g.slot;
      const list = document.createElement("div");
      list.className = "if-list";
      for (const hit of g.items) {
        const el = document.createElement("button");
        el.className = "b-icon if-icon";
        el.title = `${hit.name}（差異 ${hit.score}）`;
        const png = iconCache.get(hit.id);
        if (png) { const img = new Image(); img.src = png; img.alt = ""; img.draggable = false; el.append(img); } else el.classList.add("none");
        if (window.Beauty?.isFav?.(hit.id)) el.classList.add("fav");
        el.addEventListener("click", () => detail(hit));
        el.addEventListener("contextmenu", (e) => { e.preventDefault(); favCopy(hit).then(() => el.classList.add("fav")); });
        list.append(el);
      }
      row.append(tag, list);
      results.append(row);
    }
  }

  /* ---------- 事件 ---------- */
  $("#ifPick").addEventListener("click", () => $("#ifFile").click());
  $("#ifFile").addEventListener("change", (e) => { const f = e.target.files?.[0]; e.target.value = ""; if (f) handle(f); });
  drop.addEventListener("click", () => { if (preview.hidden) $("#ifFile").click(); });
  for (const ev of ["dragenter", "dragover"]) drop.addEventListener(ev, (e) => { e.preventDefault(); drop.classList.add("over"); });
  for (const ev of ["dragleave", "drop"]) drop.addEventListener(ev, (e) => { e.preventDefault(); drop.classList.remove("over"); });
  drop.addEventListener("drop", (e) => { const f = e.dataTransfer?.files?.[0]; if (f) handle(f); });
  // 整個頁面都接受拖曳（拖到結果區也可以）
  document.addEventListener("dragover", (e) => { if (active && e.dataTransfer?.types?.includes("Files")) e.preventDefault(); });
  document.addEventListener("drop", (e) => { if (!active) return; e.preventDefault(); const f = e.dataTransfer?.files?.[0]; if (f) handle(f); });
  document.addEventListener("paste", (e) => {
    if (!active) return;
    const item = [...(e.clipboardData?.items || [])].find((x) => x.type.startsWith("image/"));
    if (item) { e.preventDefault(); handle(item.getAsFile()); }
  });
  $("#ifCash").addEventListener("change", () => { if (lastPng) run(); });
  document.addEventListener("viewchange", (e) => { active = e.detail === "iconfind"; if (!active) clearTimeout(pollTimer); });

  window.IconFind = {
    onShow: () => { active = true; if (TAURI && !lastPng) waitIndex().then((ok) => { if (ok) hideStatus(); else { clearTimeout(pollTimer); const tick = async () => { if (!active || lastPng) return; if (await waitIndex()) hideStatus(); else pollTimer = setTimeout(tick, 1000); }; pollTimer = setTimeout(tick, 1000); } }); },
  };
})();
