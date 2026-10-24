# 使用狀況收集端（Cloudflare Worker）

免費額度就夠用。設定一次就好：

1. 到 https://dash.cloudflare.com 登入（沒有帳號就免費註冊）。
2. 左邊「Workers & Pages」→「Create」→「Create Worker」，名稱取 `maboo-stats`，按 Deploy。
3. 按「Edit code」，把 `stats-worker.js` 的內容整個貼上去取代，按 Deploy。
4. 左邊「Storage & Databases」→「KV」→「Create a namespace」，名稱 `STATS`。
5. 回到 Worker →「Settings」→「Bindings」→「Add」→「KV namespace」：Variable name 填 `STATS`，選剛建的命名空間，儲存。
6. 同一頁「Variables and Secrets」→「Add」：名稱 `ADMIN_KEY`，值填一串自己想的密碼（看儀表板用），型別選 Secret，儲存。
7. Worker 的網址在 Overview 頁，長得像 `https://maboo-stats.xxxx.workers.dev`。把這個網址告訴我，我把它填進程式。

看儀表板：`https://maboo-stats.xxxx.workers.dev/admin?key=你的密碼`
