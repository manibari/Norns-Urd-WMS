#!/usr/bin/env bash
#
# 盯著 GitHub，有新 push 就自動 pull 並重啟。
#
#   ./watch.sh                    每 60 秒問一次 origin/main
#   ./watch.sh --interval 300     改成 5 分鐘
#   ./watch.sh --branch dev       盯別的分支
#   ./watch.sh --host 0.0.0.0     轉給 start.sh 的參數
#
# 它是 start.sh 的外層：start.sh 負責跑，這支負責在有新 commit 時把它換掉。
# Ctrl+C 會連同底下的 server 一起收掉。
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$ROOT"

INTERVAL=60
BRANCH="main"
REMOTE="origin"
LOG="/tmp/urdwms-watch.log"
START_ARGS=()

while [[ $# -gt 0 ]]; do
  case "$1" in
    --interval) INTERVAL="$2"; shift 2 ;;
    --branch)   BRANCH="$2"; shift 2 ;;
    --remote)   REMOTE="$2"; shift 2 ;;
    --host)     START_ARGS+=(--host "$2"); shift 2 ;;
    -h|--help)  sed -n '2,11p' "${BASH_SOURCE[0]}" | sed 's/^# \?//'; exit 0 ;;
    *) echo "不認得的參數：$1（--help 看用法）" >&2; exit 2 ;;
  esac
done

# 只准跑一份 —— 兩個 watcher 會互相把對方的 server 清掉，症狀是 server 莫名其妙消失
exec 9>/tmp/urdwms-watch.lock
if ! flock -n 9; then
  echo "已經有一份 watch.sh 在跑了（pgrep -af watch.sh 看看）" >&2
  exit 1
fi

ts()   { date '+%H:%M:%S'; }
say()  { printf '\033[1;36m[%s]\033[0m %s\n' "$(ts)" "$*" | tee -a "$LOG"; }
warn() { printf '\033[1;33m[%s]\033[0m %s\n' "$(ts)" "$*" | tee -a "$LOG"; }

APP_PID=""
start_app() {
  say "啟動 start.sh"
  ./start.sh "${START_ARGS[@]}" &
  APP_PID=$!
}
port_busy() { (exec 3<>"/dev/tcp/127.0.0.1/$1") 2>/dev/null && { exec 3<&-; return 0; } || return 1; }

stop_app() {
  [[ -z "$APP_PID" ]] && return 0
  # start.sh 自己有 EXIT trap，收到 TERM 會把前後端一起帶走
  kill -TERM "$APP_PID" 2>/dev/null || true
  wait "$APP_PID" 2>/dev/null || true
  APP_PID=""
  # 等 port 真的釋放再往下 —— 固定 sleep 猜不準，猜短了下一輪 start.sh
  # 會判定「已經有東西在跑」直接死掉
  for _ in $(seq 40); do
    port_busy 8071 || port_busy 3071 || return 0
    sleep 0.25
  done
  warn "port 過了 10 秒還沒釋放，硬清"
  pkill -f "uvicorn app.main:app" 2>/dev/null || true
  pkill -f "next-server"          2>/dev/null || true
  sleep 2
}

cleanup() { echo; say "收工"; stop_app; }
trap cleanup EXIT INT TERM

say "監看 $REMOTE/$BRANCH，每 ${INTERVAL}s 一次 · 日誌 $LOG"
start_app

while true; do
  sleep "$INTERVAL"

  # ls-remote 不動工作目錄，純問遠端現在的 HEAD 是什麼
  remote_sha="$(git ls-remote "$REMOTE" "refs/heads/$BRANCH" 2>>"$LOG" | cut -f1)"
  if [[ -z "$remote_sha" ]]; then
    warn "問不到遠端（網路？），下一輪再試"
    continue
  fi

  local_sha="$(git rev-parse HEAD)"
  [[ "$remote_sha" == "$local_sha" ]] && continue

  say "偵測到新 commit ${remote_sha:0:7}（本地 ${local_sha:0:7}）"

  git fetch "$REMOTE" "$BRANCH" >>"$LOG" 2>&1 || { warn "fetch 失敗，跳過"; continue; }

  # 工作目錄髒的時候什麼都別做 —— rebase 會直接拒絕，而且未存檔的修改
  # 比上游那幾個 commit 重要
  if ! git diff --quiet || ! git diff --cached --quiet; then
    warn "工作目錄有未 commit 的修改，這輪跳過（存檔或 stash 後會自動繼續）"
    continue
  fi

  stop_app

  if git merge-base --is-ancestor HEAD "$REMOTE/$BRANCH" 2>/dev/null; then
    # 本地沒有自己的 commit，單純快轉
    git merge --ff-only "$REMOTE/$BRANCH" >>"$LOG" 2>&1 || {
      warn "快轉失敗，維持舊版重啟"; start_app; continue; }
  else
    # 本地有上游沒有的 commit（例如這幾支腳本）——重播到新 commit 之上，
    # 這樣自動更新和自己的東西可以並存
    say "本地有自己的 commit，改用 rebase 重播"
    if ! git rebase "$REMOTE/$BRANCH" >>"$LOG" 2>&1; then
      git rebase --abort >/dev/null 2>&1 || true
      warn "rebase 衝突，已還原 —— 請手動處理（git log $REMOTE/$BRANCH 看上游改了什麼）"
      start_app
      continue
    fi
  fi

  new_sha="$(git rev-parse --short HEAD)"
  say "已更新到 $new_sha：$(git log -1 --pretty=%s)"

  # 依這次改了什麼決定要不要重裝／重 seed —— 不是每次 push 都要付這個代價
  changed="$(git diff --name-only "$local_sha" HEAD)"
  if grep -q "^frontend/\(package.json\|pnpm-lock.yaml\)$" <<<"$changed"; then
    say "前端依賴有變，重新安裝"
    (cd frontend && PATH="$HOME/.local/node/bin:$PATH" pnpm install >>"$LOG" 2>&1) \
      || warn "pnpm install 失敗，照舊啟動看看"
  fi
  if grep -q "^frontend/next.config" <<<"$changed"; then
    say "next.config 有變，清掉 .next 快取"
    rm -rf frontend/.next
  fi
  if grep -q "^backend/\(db.py\|seed.py\)$" <<<"$changed"; then
    warn "schema 或 seed 有變 —— 資料庫沒有自動重建（會清掉現有資料）"
    warn "  要重建請停下來跑：./start.sh --reset-db"
  fi

  start_app
done
