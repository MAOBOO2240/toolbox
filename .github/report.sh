#!/usr/bin/env bash
# 編譯或測試失敗時：把錯誤變成註解，並把完整紀錄推到 ci-log 分支（方便遠端讀取）
# 用法：report.sh <標題> <紀錄檔>
title="$1"
log="$2"
msg=$(grep -E -A14 '^(error|warning: unused)|panicked at|^test .* FAILED' "$log" | head -200)
[ -z "$msg" ] && msg=$(tail -80 "$log")
enc=$(printf '%s' "$msg" | sed ':a;N;$!ba;s/%/%25/g;s/\r//g;s/\n/%0A/g')
echo "::error title=$title::$enc"

cp "$log" "$RUNNER_TEMP/ci.log"
cd "$GITHUB_WORKSPACE" || exit 0
git config user.name "ci"
git config user.email "ci@users.noreply.github.com"
git checkout -q --orphan ci-log
git rm -rq --cached . > /dev/null
cp "$RUNNER_TEMP/ci.log" ci.log
git add ci.log
git commit -qm "$title log"
git push -qf origin ci-log || true
