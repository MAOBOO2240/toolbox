"use strict";

/* ==========================================================
 * 送禮買賣
 * - 買送禮：原價 × 折扣 = 要付多少
 * - 賣送禮：客戶付 原價 × 折扣；成本看儲值方式；送禮會拿到 VIP 點數，
 *   VIP 點數 → 楓點 → 楓幣 → 台幣 算回饋價值
 * - 買賣楓幣：楓幣、台幣互換（w = 萬、e = 億）
 * - GiftCore：計算（可獨立測試）
 * ========================================================== */

const GiftCore = (() => {
  // 官方 VIP：每送禮 1 樂豆點獲得的 VIP 點數
  const VIP = {
    silver: { name: "VIP 銀牌", gift: 16 },
    gold: { name: "VIP 金牌", gift: 24 },
    diamond: { name: "VIP 鑽石", gift: 32 },
    royal: { name: "VIP 皇家", gift: 40 },
  };
  const READER_BONUS = 0.05; // 讀卡機儲值 +5%

  /** 折扣：8 → 8 折（0.8）、82 → 82 折（0.82）、0.85 → 0.85 */
  function discount(v) {
    const n = parseFloat(String(v).replace(/[,\s折%]/g, ""));
    if (!isFinite(n) || n <= 0) return null;
    if (n <= 1) return n;
    if (n < 10) return n / 10;
    if (n <= 100) return n / 100;
    return null;
  }

  /**
   * 數字可以帶單位：w = 萬、e = 億（也接受「萬」「億」），例如 3e2000w、5000w、1.5e。
   * 沒有單位的數字乘上 defUnit。
   */
  function amount(v, defUnit = 1) {
    const s = String(v).toLowerCase().replace(/[,\s]/g, "").replace(/億/g, "e").replace(/萬/g, "w");
    if (!s) return null;
    if (/^\d+(\.\d+)?$/.test(s)) return parseFloat(s) * defUnit;
    const m = /^(?:(\d+(?:\.\d+)?)e)?(?:(\d+(?:\.\d+)?)w)?$/.exec(s);
    if (!m || (!m[1] && !m[2])) return null;
    return (m[1] ? parseFloat(m[1]) * 1e8 : 0) + (m[2] ? parseFloat(m[2]) * 1e4 : 0);
  }

  /**
   * 算式：支援 + - * / 與括號，數字可以帶 w（萬）、e（億），例如 1000+390*2、3e+5000w、(10-2)*3。
   * 沒有單位的數字視為 defUnit；結果換回實際數值。算不出來回傳 null。
   */
  function calc(v, defUnit = 1) {
    const src = String(v).toLowerCase().replace(/[,\s]/g, "").replace(/億/g, "e").replace(/萬/g, "w")
      .replace(/[×x]/g, "*").replace(/÷/g, "/").replace(/（/g, "(").replace(/）/g, ")");
    if (!src) return null;
    const toks = [];
    const re = /(\d+(?:\.\d+)?|\.\d+)(?:e(\d+(?:\.\d+)?)w|([ew]))?|([-+*/()])/y;
    let m;
    re.lastIndex = 0;
    while (re.lastIndex < src.length) {
      m = re.exec(src);
      if (!m) return null;
      if (m[4]) { toks.push(m[4]); continue; }
      let n = parseFloat(m[1]);
      if (m[2] !== undefined) n = n * 1e8 + parseFloat(m[2]) * 1e4;
      else if (m[3] === "e") n *= 1e8;
      else if (m[3] === "w") n *= 1e4;
      else n *= defUnit;
      toks.push(n / defUnit); // 以預設單位計算，乘除時不會重複套用單位
    }
    let i = 0;
    const peek = () => toks[i];
    function factor() {
      const t = toks[i++];
      if (typeof t === "number") return t;
      if (t === "-") return -factor();
      if (t === "+") return factor();
      if (t === "(") { const v2 = expr(); if (toks[i++] !== ")") throw 0; return v2; }
      throw 0;
    }
    function term() {
      let v2 = factor();
      while (peek() === "*" || peek() === "/") { const op = toks[i++]; const r = factor(); v2 = op === "*" ? v2 * r : v2 / r; }
      return v2;
    }
    function expr() {
      let v2 = term();
      while (peek() === "+" || peek() === "-") { const op = toks[i++]; const r = term(); v2 = op === "+" ? v2 + r : v2 - r; }
      return v2;
    }
    try {
      const out = expr();
      if (i !== toks.length || !isFinite(out)) return null;
      return out * defUnit;
    } catch (_) { return null; }
  }
  const isFormula = (v) => /[-+*/×÷()（）]/.test(String(v).replace(/^\s*-/, ""));

  /** 每 1 點樂豆點的成本（台幣） */
  function pointCost(pay, card) {
    if (pay === "reader") return 1 / (1 + READER_BONUS);
    if (pay === "card") { const d = discount(card); return d ?? 1; }
    return 1;
  }

  /**
   * 賣送禮試算
   * s = { price, disc, pay, card, vip, fengPerE（1 億楓幣 = 幾楓點）, ratio（幾萬楓幣 = 1 台幣）, vipPer（幾 VIP 點數 = 1 楓點） }
   */
  function sell(s) {
    const d = discount(s.disc);
    const price = s.price;
    if (!(price > 0) || d === null) return null;
    const receive = price * d;
    const cost = price * pointCost(s.pay, s.card);
    const vipPts = price * (VIP[s.vip] || VIP.silver).gift;
    const feng = s.vipPer > 0 ? vipPts / s.vipPer : 0;
    const meso = s.fengPerE > 0 ? (feng * 1e8) / s.fengPerE : 0;
    const rebate = s.ratio > 0 ? meso / (s.ratio * 1e4) : 0;
    const profit = receive - cost + rebate;
    return { receive, cost, vipPts, feng, meso, rebate, profit, margin: receive > 0 ? profit / receive : 0, breakEven: (cost - rebate) / price, disc: d };
  }

  function buy(s) {
    const d = discount(s.disc);
    if (!(s.price > 0) || d === null) return null;
    return { pay: s.price * d, save: s.price * (1 - d), disc: d };
  }

  return { VIP, READER_BONUS, discount, amount, calc, isFormula, pointCost, sell, buy };
})();

if (typeof module !== "undefined") module.exports = GiftCore;

/* ==========================================================
 * 介面
 * ========================================================== */
if (typeof document !== "undefined") (() => {
  const G = GiftCore;
  const KEY = "gift-v1";
  const defaults = () => ({
    page: "gift", mode: "buy", disc: "", price: "", pay: "direct", card: "92", vip: "silver",
    fengPerE: 4, ratio: 2800, vipPer: 300, fxMode: "meso", fxAmount: "",
  });
  let st = (() => {
    try { return { ...defaults(), ...(JSON.parse(localStorage.getItem(KEY)) || {}) }; } catch (_) { return defaults(); }
  })();
  const save = () => { try { localStorage.setItem(KEY, JSON.stringify(st)); } catch (_) {} };

  /* ---------- 數字 ---------- */
  const ntFmt = new Intl.NumberFormat("zh-TW", { maximumFractionDigits: 2 });
  const intFmt = new Intl.NumberFormat("zh-TW", { maximumFractionDigits: 0 });
  const nt = (n) => `${n < 0 ? "-" : ""}NT$ ${ntFmt.format(Math.abs(n) >= 100 ? Math.round(Math.abs(n)) : Math.abs(n))}`;
  const discText = (d) => `${+(d * (Number.isInteger(d * 10) ? 10 : 100)).toFixed(2)} 折`;
  /** 楓幣：主要顯示 e／w，另附中文 */
  function meso(n) {
    const e = n / 1e8;
    const main = n >= 1e8
      ? `${e >= 1000 ? intFmt.format(Math.round(e)) : +e.toFixed(e >= 100 ? 1 : 2)}e`
      : `${n >= 1e6 ? intFmt.format(Math.round(n / 1e4)) : +(n / 1e4).toFixed(2)}w`;
    return { main, zh: zhNum(Math.round(n / 1e4) * 1e4) };
  }
  /** 中文讀法：154286 → 15萬4286、1209600000000 → 1兆2096億（萬以下的零頭四捨五入到小數兩位） */
  function zhNum(n) {
    if (!isFinite(n)) return "";
    const neg = n < 0;
    let x = Math.abs(n);
    const units = [[1e12, "兆"], [1e8, "億"], [1e4, "萬"]];
    let out = "";
    for (const [u, name] of units) {
      const q = Math.floor(x / u + 1e-9);
      if (q > 0) { out += `${q}${name}`; x -= q * u; }
    }
    const rest = Math.round(x * 100) / 100;
    if (rest > 0 || !out) out += `${rest}`;
    return (neg ? "-" : "") + out;
  }
  const num = (v) => { const n = parseFloat(String(v).replace(/[,\s]/g, "")); return isFinite(n) ? n : 0; };

  /* ---------- 分頁 ---------- */
  function renderPage() {
    $$("#fPages button").forEach((b) => b.classList.toggle("on", b.dataset.page === st.page));
    $("#fPageGift").hidden = st.page !== "gift";
    $("#fPageFx").hidden = st.page !== "fx";
  }

  /* ---------- 送禮 ---------- */
  function syncInputs() {
    const set = (id, v) => { const el = $(id); if (document.activeElement !== el) el.value = v ?? ""; };
    set("#fDisc", st.disc);
    set("#fPrice", st.price);
    set("#fCard", st.card);
    set("#fFengPerE", st.fengPerE);
    set("#fRatio", st.ratio);
    set("#fRatio2", st.ratio);
    set("#fVipPer", st.vipPer);
    set("#fFxAmount", st.fxAmount);
    $$("#fMode button").forEach((b) => b.classList.toggle("on", b.dataset.v === st.mode));
    $$("#fPay button").forEach((b) => b.classList.toggle("on", b.dataset.v === st.pay));
    $$("#fVip button").forEach((b) => b.classList.toggle("on", b.dataset.v === st.vip));
    $$("#fFxMode button").forEach((b) => b.classList.toggle("on", b.dataset.v === st.fxMode));
    $("#fCardRow").hidden = st.pay !== "card";
    $$(".f-sell-only").forEach((el) => (el.hidden = st.mode !== "sell"));
  }

  function row(label, value, sub = "", cls = "") {
    return `<div class="f-line ${cls}"><span>${label}${sub ? `<small>${sub}</small>` : ""}</span><b>${value}</b></div>`;
  }

  /* ---------- 圖示：樂豆點、VIP 點數、楓幣、遊戲幣（楓幣袋） ---------- */
  const COIN_SVG = '<svg viewBox="0 0 24 24"><circle cx="12" cy="12" r="9.5" fill="#f5b52e" stroke="#a86b00" stroke-width="1.5"/><circle cx="12" cy="12" r="6.2" fill="none" stroke="#fde68a" stroke-width="1.4"/><path d="M12 8.2l1.1 2.3 2.4.3-1.8 1.6.5 2.4-2.2-1.2-2.2 1.2.5-2.4-1.8-1.6 2.4-.3z" fill="#a86b00"/></svg>';
  const BAG_SVG = '<svg viewBox="0 0 24 24"><path d="M8.5 6.5h7l-1.4 2.2c3.4 1.2 5.6 4.4 5.6 7.7 0 3-2.4 4.6-7.7 4.6S4.3 19.4 4.3 16.4c0-3.3 2.2-6.5 5.6-7.7z" fill="#d6c49a" stroke="#6b5a3a" stroke-width="1.3" stroke-linejoin="round"/><path d="M9 8.7h6" stroke="#6b5a3a" stroke-width="1.3"/><circle cx="12" cy="4.6" r="2.3" fill="#f5b52e" stroke="#a86b00" stroke-width="1"/><path d="M12 12.3l.9 1.7 1.8.1-1.3 1.3.4 1.9-1.8-.9-1.8.9.4-1.9-1.3-1.3 1.8-.1z" fill="#a0602a"/></svg>';
  const ICON = {
    bean: () => '<img class="f-ic" src="assets/bean-point.png" alt="" draggable="false" />',
    vip: () => '<img class="f-ic" src="assets/vip-point.png" alt="" draggable="false" />',
    meso: () => `<span class="f-ic" data-ic="@meso1">${COIN_SVG}</span>`,
    bag: () => `<span class="f-ic" data-ic="@meso3">${BAG_SVG}</span>`,
  };
  /** 遊戲裡的楓幣圖示讀得到就換掉預設的圖 */
  function fillIcons(root = document.getElementById("view-gift")) {
    if (!window.ItemIcons) return;
    root.querySelectorAll("[data-ic]").forEach((el) => {
      ItemIcons.get(el.dataset.ic).then((png) => {
        if (!png || !el.isConnected) return;
        el.innerHTML = `<img src="${png}" alt="" draggable="false" />`;
        el.removeAttribute("data-ic");
      });
    });
  }

  /** 算式的結果顯示在輸入框裡、文字後面（灰色，不能改） */
  const measure = document.createElement("canvas").getContext("2d");
  function ghost(input, text) {
    const wrap = input.parentElement;
    let g = wrap.querySelector(".f-ghost");
    if (!g) { g = document.createElement("span"); g.className = "f-ghost"; wrap.append(g); }
    if (!text) { g.hidden = true; return; }
    const cs = getComputedStyle(input);
    measure.font = `${cs.fontStyle} ${cs.fontWeight} ${cs.fontSize} ${cs.fontFamily}`;
    const w = measure.measureText(input.value).width;
    const left = input.offsetLeft + w + 8 - input.scrollLeft;
    g.textContent = text;
    g.style.left = `${left}px`;
    g.hidden = left + 20 > input.offsetLeft + input.clientWidth;
  }

  function renderGift() {
    const d = G.discount(st.disc);
    $("#fDiscHint").textContent = st.disc === "" ? "" : d === null ? "看不懂這個折扣" : `${discText(d)}（${+(d * 100).toFixed(2)}%）`;
    const cd = G.discount(st.card);
    $("#fCardHint").textContent = cd === null ? "" : `每 1 點成本 ${ntFmt.format(cd)} 元`;
    const price = G.calc(st.price);
    const pf = st.price && G.isFormula(st.price);
    $("#fPriceHint").textContent = pf && price === null ? "算式看不懂" : "";
    ghost($("#fPrice"), pf && price !== null ? `= ${ntFmt.format(price)}` : "");
    const out = $("#fResult");
    if (st.mode === "buy") {
      const r = G.buy({ price, disc: st.disc });
      $("#fBigLabel").textContent = "要付多少錢";
      if (!r) { $("#fBig").textContent = "—"; out.innerHTML = '<p class="f-empty">輸入折扣與道具原價就會算出來</p>'; return; }
      $("#fBig").textContent = nt(r.pay);
      out.innerHTML = row("道具原價", `${ICON.bean()}${intFmt.format(price)} 點`) + row("折扣", discText(r.disc)) + row("比原價省下", nt(r.save), "", "pos");
      return;
    }
    const r = G.sell({ price, disc: st.disc, pay: st.pay, card: st.card, vip: st.vip, fengPerE: num(st.fengPerE), ratio: num(st.ratio), vipPer: num(st.vipPer) });
    $("#fBigLabel").textContent = "客戶要給多少錢";
    if (!r) { $("#fBig").textContent = "—"; out.innerHTML = '<p class="f-empty">輸入折扣與道具原價就會算出來</p>'; return; }
    $("#fBig").textContent = nt(r.receive);
    const payName = { direct: "直購", reader: "讀卡機", card: "點數卡" }[st.pay];
    const m = meso(r.meso);
    const vip = G.VIP[st.vip];
    out.innerHTML =
      row("你的成本", nt(r.cost), `${payName}・每點 ${ntFmt.format(G.pointCost(st.pay, st.card))} 元`) +
      row("VIP 回饋點數", `${ICON.vip()}${intFmt.format(r.vipPts)} 點`, `${vip.name}・送禮每 1 點拿 ${vip.gift} 點`) +
      row("換成楓點", `${ntFmt.format(r.feng)} 楓點`, `${intFmt.format(num(st.vipPer))} VIP 點數 = 1 楓點`) +
      row("換成楓幣", `${ICON.meso()}${m.main}`, m.zh) +
      row("回饋價值", nt(r.rebate), `${intFmt.format(num(st.ratio))} 萬遊戲幣 = 1 台幣`) +
      row("淨賺", nt(r.profit), `利潤率 ${(r.margin * 100).toFixed(1)}%`, r.profit >= 0 ? "pos total" : "neg total") +
      row("最低可以賣", r.breakEven > 0 ? discText(Math.ceil(r.breakEven * 1000) / 1000) : "不收錢也不虧", "低於這個折扣就會虧");
    fillIcons(out);
  }

  /* ---------- 買賣楓幣 ---------- */
  function renderFx() {
    const ratio = num(st.ratio); // 幾萬楓幣 = 1 台幣
    const perNt = ratio * 1e4;
    const out = $("#fFxBig"), sub = $("#fFxSub");
    const isMeso = st.fxMode === "meso";
    $("#fFxUnit").textContent = isMeso ? "e" : "元";
    $("#fFxAmount").placeholder = isMeso ? "例如 10（= 10e），也可以打 5000w、3e2000w" : "例如 1000，也可以打 300*3";
    $("#fFxLabel").textContent = isMeso ? "要買賣多少楓幣" : "要買賣多少台幣";
    const v = G.calc(st.fxAmount, isMeso ? 1e8 : 1);
    const ff = st.fxAmount && G.isFormula(st.fxAmount);
    $("#fFxHint").textContent = ff && v === null ? "算式看不懂" : "w = 萬、e = 億；可以輸入算式，例如 3e+5000w、1000*3";
    ghost($("#fFxAmount"), ff && v !== null ? `= ${isMeso ? meso(v).main : ntFmt.format(v)}` : "");
    if (!(perNt > 0) || !(v > 0)) {
      out.textContent = "—";
      sub.textContent = perNt > 0 ? "" : "請先輸入目前楓幣比值";
    } else if (isMeso) {
      const twd = v / perNt;
      // 大字後面用灰色小字寫中文讀法
      out.innerHTML = `${nt(twd)}<small class="f-zh">${zhNum(twd >= 100 ? Math.round(twd) : twd)}元</small>`;
      sub.innerHTML = `${ICON.meso()}${meso(v).main} 楓幣`;
    } else {
      const m = meso(v * perNt);
      out.innerHTML = `${ICON.meso()}${m.main}<small class="f-zh">${m.zh}</small>`;
      sub.textContent = nt(v);
    }
    // 常用換算
    const box = $("#fFxTable");
    box.innerHTML = "";
    if (!(perNt > 0)) return;
    const list = isMeso ? [1e8, 5e8, 10e8, 50e8, 100e8, 500e8] : [100, 300, 500, 1000, 3000, 5000];
    for (const x of list) {
      const el = document.createElement("button");
      el.className = "f-chip";
      el.innerHTML = isMeso
        ? `<b>${ICON.meso()}${meso(x).main}</b><span>${nt(x / perNt)}</span>`
        : `<b>${nt(x)}</b><span>${ICON.meso()}${meso(x * perNt).main}</span>`;
      el.addEventListener("click", () => { st.fxAmount = isMeso ? meso(x).main : String(x); save(); syncInputs(); renderFx(); });
      box.append(el);
    }
    fillIcons($("#fPageFx"));
  }

  function renderAll() {
    renderPage();
    syncInputs();
    renderGift();
    renderFx();
  }

  // 輸入框、選項前面的圖示（只放一次）
  $$("[data-icon]").forEach((el) => el.insertAdjacentHTML("afterbegin", ICON[el.dataset.icon]()));
  $$("#fVip button").forEach((b) => b.insertAdjacentHTML("afterbegin", `<img class="f-badge" src="assets/vip-${b.dataset.v}.png" alt="" draggable="false" />`));
  fillIcons();

  /* ---------- 事件 ---------- */
  $$("#fPages button").forEach((b) => b.addEventListener("click", () => { st.page = b.dataset.page; save(); renderAll(); }));
  const bindSeg = (id, field) => $$(`${id} button`).forEach((b) => b.addEventListener("click", () => { st[field] = b.dataset.v; save(); renderAll(); }));
  bindSeg("#fMode", "mode");
  bindSeg("#fPay", "pay");
  bindSeg("#fVip", "vip");
  bindSeg("#fFxMode", "fxMode");
  const bindInput = (id, field) => $(id).addEventListener("input", (e) => { st[field] = e.target.value.trim(); save(); syncInputs(); renderGift(); renderFx(); });
  bindInput("#fDisc", "disc");
  bindInput("#fPrice", "price");
  bindInput("#fCard", "card");
  bindInput("#fFengPerE", "fengPerE");
  bindInput("#fRatio", "ratio");
  bindInput("#fRatio2", "ratio");
  bindInput("#fVipPer", "vipPer");
  bindInput("#fFxAmount", "fxAmount");
  /* 游標放在輸入框上用滾輪調整：折扣類一次 ±1（8 折會先換成 80 再加減），其他依各自的單位 */
  function wheel(id, field, step, kind) {
    const el = $(id);
    el.dataset.wheel = "";
    el.addEventListener("wheel", (e) => {
      if (e.deltaY === 0) return;
      e.preventDefault();
      const dir = e.deltaY < 0 ? 1 : -1;
      let next;
      if (kind === "disc") {
        const d = G.discount(st[field]);
        const pct = d === null ? 90 : Math.round(d * 100);
        next = String(Math.min(100, Math.max(1, pct + dir * step)));
      } else {
        const cur = num(st[field]);
        next = String(Math.max(0, +(cur + dir * step).toFixed(4)));
      }
      st[field] = next;
      el.value = next; // 游標所在的框可能正在輸入中，直接更新
      if (id === "#fRatio") $("#fRatio2").value = next;
      if (id === "#fRatio2") $("#fRatio").value = next;
      save(); syncInputs(); renderGift(); renderFx();
    }, { passive: false });
  }
  wheel("#fDisc", "disc", 1, "disc");
  wheel("#fCard", "card", 1, "disc");
  wheel("#fFengPerE", "fengPerE", 1);
  wheel("#fRatio", "ratio", 50);
  wheel("#fRatio2", "ratio", 50);
  wheel("#fVipPer", "vipPer", 10);

  $("#fReset").addEventListener("click", () => {
    st = { ...defaults(), page: st.page, mode: st.mode };
    save(); renderAll();
    toast("已恢復預設值");
  });

  renderAll();
  window.Gift = { onShow: renderAll };
})();
