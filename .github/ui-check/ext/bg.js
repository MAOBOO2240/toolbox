const log = (m) => fetch('http://127.0.0.1:8765/log?m=' + encodeURIComponent('[extbg] ' + m)).catch(() => {});
const make = () => {
  try {
    chrome.contextMenus.create({ id: 'mbtest', title: '測試擴充項目', contexts: ['all'] }, () => log('menu created err=' + (chrome.runtime.lastError ? chrome.runtime.lastError.message : '')));
  } catch (e) { log('menu threw ' + e); }
};
chrome.runtime.onInstalled.addListener(make);
chrome.runtime.onStartup.addListener(make);
try { chrome.contextMenus.onClicked.addListener((info) => log('menu clicked ' + info.menuItemId)); } catch (e) { log('onClicked threw ' + e); }
log('bg started');
