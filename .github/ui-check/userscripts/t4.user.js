// ==UserScript==
// @name         測試腳本四
// @version      1.0
// @description  整輪檢查用：設定值、選單、跨網站連線、開分頁
// @match        http://127.0.0.1:8765/us.html*
// @match        http://localhost:8765/usframe.html*
// @run-at       document-start
// @grant        GM_getValue
// @grant        GM_setValue
// @grant        GM_addValueChangeListener
// @grant        GM_registerMenuCommand
// @grant        GM_unregisterMenuCommand
// @grant        GM_xmlhttpRequest
// @grant        GM_openInTab
// @grant        GM_info
// @grant        unsafeWindow
// @connect      localhost
// ==/UserScript==
(function () {
  'use strict';
  const who = window.top === window ? 'top' : 'frame';
  const q = location.search || '-';
  const log = (m) => { fetch('/log?m=' + encodeURIComponent('[us4] ' + who + ' ' + q + ' ' + m)).catch(() => {}); };
  log('start pageRan=' + !!unsafeWindow.__pageRan + ' ready=' + document.readyState + ' webview=' + typeof (window.chrome && window.chrome.webview)
    + ' shared=' + JSON.stringify(GM_getValue('shared', null)) + ' handler=' + GM_info.scriptHandler + ' ver=' + GM_info.script.version
    + ' types=' + [typeof GM_setValue, typeof GM_deleteValue, typeof GM_notification].join(','));
  GM_addValueChangeListener('shared', (k, o, n, remote) => log('change new=' + JSON.stringify(n) + ' remote=' + remote));
  if (who !== 'top') return;
  let on = false;
  const ids = {};
  const draw = () => {
    for (const k of Object.keys(ids)) { GM_unregisterMenuCommand(ids[k]); delete ids[k]; }
    ids.set = GM_registerMenuCommand('測試：設值', () => { const v = 'v' + Date.now(); GM_setValue('shared', v); log('menu set ' + v); }, { autoClose: true, title: '把共用的值改掉' });
    ids.tog = GM_registerMenuCommand((on ? '【開】' : '【關】') + '開關', () => { on = !on; log('menu toggle on=' + on); draw(); }, { autoClose: false });
    ids.tab = GM_registerMenuCommand('測試：開分頁', () => { GM_openInTab('http://127.0.0.1:8765/snap.html?gmtab=1', { active: true, insert: true }); log('menu tab'); });
  };
  draw();
  if (/noxhr/.test(q)) return;
  const X = 'http://localhost:8765';
  // 跨網站連線（網頁自己連會被擋）：標頭、Cookie、回應
  GM_xmlhttpRequest({
    method: 'GET', url: X + '/xhr-echo?auto=1', headers: { Referer: 'https://ref.example/', 'X-Test': 'abc' }, timeout: 15000,
    onload: (r) => {
      let j = {};
      try { j = JSON.parse(r.responseText); } catch (_) {}
      log('xhr get status=' + r.status + ' method=' + j.method + ' referer=' + j.referer + ' xtest=' + j.xtest + ' cookie=' + j.cookie + ' ua=' + (j.ua === navigator.userAgent)
        + ' final=' + r.finalUrl + ' hdr=' + /content-type: application\/json/i.test(r.responseHeaders));
    },
    onerror: (e) => log('xhr get ERROR ' + (e && e.error)), ontimeout: () => log('xhr get TIMEOUT'),
  });
  GM_xmlhttpRequest({
    method: 'POST', url: X + '/xhr-echo', headers: { 'Content-Type': 'application/json' }, data: JSON.stringify({ a: '中文' }), responseType: 'arraybuffer',
    onload: (r) => {
      let j = {};
      try { j = JSON.parse(new TextDecoder().decode(r.response)); } catch (_) {}
      log('xhr post status=' + r.status + ' bytes=' + (r.response && r.response.byteLength > 0) + ' body=' + j.body + ' ctype=' + j.ctype);
    },
    onerror: () => log('xhr post ERROR'),
  });
  // 串流：伺服器一段一段送，這邊要一段一段收到
  let chunks = 0, text = '';
  GM_xmlhttpRequest({
    method: 'GET', url: X + '/xhr-stream', responseType: 'stream',
    onloadstart: async (r) => {
      try {
        const rd = r.response.getReader();
        const td = new TextDecoder();
        for (;;) { const { done, value } = await rd.read(); if (done) break; chunks++; text += td.decode(value, { stream: true }); }
        log('xhr stream chunks=' + chunks + ' text=' + text.replace(/\n/g, '|'));
      } catch (e) { log('xhr stream ERROR ' + e); }
    },
    onerror: () => log('xhr stream ERROR'),
  });
  // 腳本沒有允許的網域：連不了
  GM_xmlhttpRequest({ url: 'http://example.org/', onload: (r) => log('xhr denied?? status=' + r.status), onerror: () => log('xhr denied ok') });
  // 網頁自己連同一個網址（對方沒有允許跨網站）：應該失敗，證明上面那些不是網頁自己連的
  fetch(X + '/xhr-echo?page=1').then((r) => r.text()).then(() => log('pagefetch OK??'), () => log('pagefetch blocked'));
})();
