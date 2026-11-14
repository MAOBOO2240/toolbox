// 查問題用：網頁（與內嵌框架）跟程式互傳訊息通不通；chrome.webview 能不能從網頁上拿掉
(() => {
  const who = window.top === window ? 'top' : (/lvl=2/.test(location.search) ? 'nested' : 'frame');
  const log = (m) => fetch('http://127.0.0.1:8765/log?m=' + encodeURIComponent('[gm ' + who + '] ' + m)).catch(() => {});
  const wv = window.chrome && window.chrome.webview;
  log('webview=' + typeof wv + ' desc=' + JSON.stringify((window.chrome && Object.getOwnPropertyDescriptor(window.chrome, 'webview')) || null, (k, v) => (typeof v === 'function' || (typeof v === 'object' && v && k) ? typeof v : v))
    + ' chromeDesc=' + JSON.stringify(Object.getOwnPropertyDescriptor(window, 'chrome') || null, (k, v) => (typeof v === 'function' || (typeof v === 'object' && v && k) ? typeof v : v)));
  if (!wv) return;
  // 最上層的網頁：用程式加的「綁定函式」傳（名稱每次不同）；內嵌框架用 chrome.webview
  const bn = Object.getOwnPropertyNames(window).find((n) => /^__mb[0-9a-f]{16}$/.test(n));
  const bind = bn ? window[bn] : null;
  log('binding=' + (bn ? 'yes ' + typeof bind : 'no'));
  const post = who === 'top' && bind ? (m) => bind(JSON.stringify(m)) : wv.postMessage.bind(wv);
  wv.addEventListener('message', (e) => log('got ' + JSON.stringify(e.data)));
  post({ mb: 'test', who });
  if (bn) {
    let del = 'n/a';
    try { del = String(delete window[bn]); } catch (e) { del = 'ERR ' + e.message; }
    log('binding delete=' + del + ' after=' + typeof window[bn]);
  }
  setTimeout(() => {
    let del = 'n/a';
    try { del = String(delete window.chrome.webview); } catch (e) { del = 'ERR ' + e.message; }
    log('delete=' + del + ' after=' + typeof (window.chrome && window.chrome.webview));
    if (window.chrome && window.chrome.webview) {
      try { Object.defineProperty(window.chrome, 'webview', { value: undefined, configurable: true }); } catch (e) { log('defineProperty ERR ' + e.message); }
      log('defineProperty after=' + typeof window.chrome.webview);
    }
    if (window.chrome && window.chrome.webview) {
      try {
        const c = {};
        for (const k of Object.keys(window.chrome)) if (k !== 'webview') c[k] = window.chrome[k];
        window.chrome = c;
      } catch (e) { log('replace ERR ' + e.message); }
      log('replace after=' + typeof (window.chrome && window.chrome.webview));
    }
    // 拿掉之後，先前抓好的還能不能用
    post({ mb: 'test', who: who + '-after' });
  }, 1200);
})();
