"use strict";

/* ==========================================================
 * web：置頂的小瀏覽器（浮動視窗）。這一頁管理有哪些視窗、開關與設定；
 * 視窗本身由 Rust 建立，上面的工具列是 web.html。
 * ========================================================== */
if (typeof document !== "undefined") (() => {
  const wapi = TAURI ? {
    list: () => invoke("web_list"),
    add: () => invoke("web_add"),
    open: (id) => invoke("web_open", { id }),
    close: (id) => invoke("web_close", { id }),
    del: (id) => invoke("web_delete", { id }),
    set: (id, o) => invoke("web_set", { id, opacity: o.opacity ?? null, locked: o.locked ?? null, gameOnly: o.game_only ?? null }),
    profiles: () => invoke("web_chrome_profiles"),
    importChrome: (dir) => invoke("web_chrome_import", { dir }),
    onChange: (cb) => listen("web-changed", cb),
  } : mockApi();

  let file = { next_id: 0, wins: [], bookmarks: { bar: [], other: [] } };
  const countLinks = (list) => (list || []).reduce((n, b) => n + (b.url ? 1 : countLinks(b.children)), 0);

  const host = (u) => { try { return new URL(u).host; } catch { return u; } };
  const sw = (cls, on) => `<label class="switch sm"><input type="checkbox" class="${cls}"${on ? " checked" : ""} /><span class="slider"></span></label>`;

  function render() {
    const list = $("#webList");
    list.innerHTML = "";
    if (!file.wins.length) {
      list.innerHTML = '<div class="g-empty">還沒有任何視窗，按右上角「新增視窗」開一個（預設首頁是 Google）</div>';
      return;
    }
    file.wins.forEach((w, i) => {
      const el = document.createElement("div");
      el.className = `card web-card${w.open ? " open" : ""}`;
      el.innerHTML = `
        <div class="web-head">
          <span class="web-no">${i + 1}</span>
          <div class="web-name">
            <strong></strong>
            <span class="muted"></span>
          </div>
          ${w.open && w.min ? '<button class="cbtn small c-blue solid web-show">顯示視窗</button>' : ""}
          <button class="cbtn small ${w.open ? "c-silver" : "c-green solid"} web-toggle">${w.open ? "關閉視窗" : "打開視窗"}</button>
          <button class="cbtn small c-red web-del">刪除</button>
        </div>
        <div class="web-opts">
          <div class="web-opt"><span>不透明度</span>
            <span class="web-range"><input type="range" class="range web-alpha" min="30" max="100" step="5" value="${w.opacity}" /><b>${w.opacity}%</b></span></div>
          <label class="web-opt" title="鎖定後滑鼠會直接穿過視窗點到遊戲，視窗本身就不能點了；要操作時回到這裡解除"><span>鎖定（滑鼠穿透）</span>${sw("web-lock", w.locked)}</label>
          <label class="web-opt" title="切到其他程式時自動隱藏，回到遊戲再出現"><span>只在遊戲畫面時顯示</span>${sw("web-game", w.game_only)}</label>
        </div>`;
      el.querySelector(".web-name strong").textContent = w.title || "新視窗";
      const tabs = (w.tabs || []).length;
      el.querySelector(".web-name .muted").textContent = [host(w.url), tabs > 1 ? `${tabs} 個分頁` : "", w.open && w.min ? "已縮小" : ""].filter(Boolean).join("・");
      el.querySelector(".web-name").title = w.url;

      el.querySelector(".web-show")?.addEventListener("click", () => run(async () => {
        await wapi.open(w.id);
        await reload();
      }));
      el.querySelector(".web-toggle").addEventListener("click", () => run(async () => {
        if (w.open) await wapi.close(w.id); else await wapi.open(w.id);
        await reload();
      }));
      el.querySelector(".web-del").addEventListener("click", () => run(async () => {
        if (!(await confirmBox(`確定要刪除「${w.title || "新視窗"}」嗎？`))) return;
        await wapi.del(w.id);
        await reload();
      }));
      const alpha = el.querySelector(".web-alpha");
      alpha.addEventListener("input", () => {
        w.opacity = +alpha.value;
        alpha.nextElementSibling.textContent = `${w.opacity}%`;
        run(() => wapi.set(w.id, { opacity: w.opacity }));
      });
      // 游標放在滑桿上可以用滾輪調整
      alpha.addEventListener("wheel", (e) => {
        e.preventDefault();
        alpha.value = Math.max(30, Math.min(100, +alpha.value + (e.deltaY < 0 ? 5 : -5)));
        alpha.dispatchEvent(new Event("input"));
      }, { passive: false });
      el.querySelector(".web-lock").addEventListener("change", (e) => {
        w.locked = e.target.checked;
        run(() => wapi.set(w.id, { locked: w.locked }));
        if (w.locked) toast("已鎖定：滑鼠會穿過視窗，要操作時回到這裡解除");
      });
      el.querySelector(".web-game").addEventListener("change", (e) => {
        w.game_only = e.target.checked;
        run(() => wapi.set(w.id, { game_only: w.game_only }));
      });
      list.append(el);
    });
  }

  function renderBm() {
    const b = file.bookmarks || { bar: [], other: [] };
    const n = countLinks(b.bar) + countLinks(b.other);
    $("#webBmCount").textContent = n ? `${n} 個書籤（所有 web 視窗共用）` : "還沒有書籤";
  }

  async function reload() {
    file = await wapi.list();
    render();
    renderBm();
  }

  /* ---------- 從 Chrome 匯入書籤 ---------- */
  const COLORS = ["c-blue", "c-purple", "c-teal", "c-pink", "c-indigo", "c-orange", "c-green"];
  async function importFrom(p) {
    const n = await wapi.importChrome(p.dir);
    $("#webProfiles").hidden = true;
    toast(n ? `已匯入 ${n} 個書籤` : "沒有新的書籤（都已經匯入過了）");
    await reload();
  }
  $("#webImport").addEventListener("click", () => run(async () => {
    const box = $("#webProfiles");
    if (!box.hidden) { box.hidden = true; return; }
    const list = await wapi.profiles();
    if (list.length === 1) { await importFrom(list[0]); return; }
    // 有好幾個 Chrome 使用者：選要匯入哪一個帳號的書籤
    box.innerHTML = '<span class="muted">Chrome 有好幾個使用者，選要匯入哪一個帳號的書籤：</span>';
    list.forEach((p, i) => {
      const b = document.createElement("button");
      b.className = `cbtn small ${COLORS[i % COLORS.length]}`;
      b.textContent = `${p.name}${p.account ? `（${p.account}）` : ""}・${p.count} 個書籤`;
      b.addEventListener("click", () => run(() => importFrom(p)));
      box.append(b);
    });
    box.hidden = false;
  }));

  $("#webAdd").addEventListener("click", () => run(async () => {
    await wapi.add();
    await reload();
  }));

  // 視窗標題、網址改變或被關掉時更新（只在這一頁顯示時才重畫）
  wapi.onChange(() => { if ($("#view-web").classList.contains("active")) run(reload); });
  document.addEventListener("viewchange", (e) => { if (e.detail === "web") run(reload); });
  run(reload);

  /* ---------- 瀏覽器預覽用的模擬後端 ---------- */
  function mockApi() {
    const f = {
      next_id: 1,
      wins: [{ id: "w1", url: "https://www.youtube.com/watch?v=x", title: "YouTube", opacity: 100, locked: false, game_only: false, open: true, min: true, tabs: [{ id: "t1" }, { id: "t2" }] }],
      bookmarks: { bar: [], other: [] },
    };
    const clone = (x) => JSON.parse(JSON.stringify(x));
    return {
      list: async () => clone(f),
      add: async () => { f.next_id++; const w = { id: `w${f.next_id}`, url: "https://www.google.com/", title: "", opacity: 100, locked: false, game_only: false, open: true }; f.wins.push(w); return clone(w); },
      open: async (id) => { Object.assign(f.wins.find((w) => w.id === id), { open: true, min: false }); },
      close: async (id) => { f.wins.find((w) => w.id === id).open = false; },
      del: async (id) => { f.wins = f.wins.filter((w) => w.id !== id); },
      set: async (id, o) => { Object.assign(f.wins.find((w) => w.id === id), o); },
      profiles: async () => [{ dir: "Default", name: "個人", account: "", count: 24 }, { dir: "Profile 1", name: "工作", account: "", count: 7 }],
      importChrome: async () => { f.bookmarks.bar.push({ url: "https://www.youtube.com/" }, { url: "https://forum.gamer.com.tw/" }); return 2; },
      onChange: () => {},
    };
  }
})();
