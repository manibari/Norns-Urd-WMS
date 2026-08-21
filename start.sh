#!/usr/bin/env bash
#
# Urd-WMS 一鍵啟動 —— 缺什麼補什麼，然後把前後端一起拉起來。
#
#   ./start.sh                    後端 :8071（僅本機）＋ 前端 :3071
#   ./start.sh --host 0.0.0.0     後端也開給區網／WSL 外面連
#   ./start.sh --reset-db         砍掉 urdwms.db 重新 seed
#   ./start.sh --setup            只做環境準備，不啟動
#
# Ctrl+C 會把兩個 server 一起收掉。
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$ROOT"

NODE_HOME="$HOME/.local/node"
NODE_VERSION="v22.14.0"
BACKEND_PORT=8071
FRONTEND_PORT=3071
BACKEND_HOST="127.0.0.1"      # 預設只給本機；要區網連得到請用 --host 0.0.0.0
BACKEND_LOG="/tmp/urdwms-backend.log"
FRONTEND_LOG="/tmp/urdwms-frontend.log"
RESET_DB=0
SETUP_ONLY=0

while [[ $# -gt 0 ]]; do
  case "$1" in
    --host)     BACKEND_HOST="$2"; shift 2 ;;
    --reset-db) RESET_DB=1; shift ;;
    --setup)    SETUP_ONLY=1; shift ;;
    -h|--help)  sed -n '2,12p' "${BASH_SOURCE[0]}" | sed 's/^# \?//'; exit 0 ;;
    *) echo "不認得的參數：$1（--help 看用法）" >&2; exit 2 ;;
  esac
done

say()  { printf '\033[1;36m▸\033[0m %s\n' "$*"; }
warn() { printf '\033[1;33m!\033[0m %s\n' "$*"; }
die()  { printf '\033[1;31m✗\033[0m %s\n' "$*" >&2; exit 1; }

export PATH="$NODE_HOME/bin:$PATH"

# ── 環境準備（都是 idempotent，裝過就跳過）──────────────────────────

# Python：這台機器的 python3 沒有 pip，venv 也建不出 pip，所以用 get-pip bootstrap
if [[ ! -x .venv/bin/python3 ]]; then
  say "建立 .venv"
  python3 -m venv .venv 2>/dev/null || python3 -m venv --without-pip .venv
fi
if [[ ! -x .venv/bin/pip ]]; then
  say "bootstrap pip（系統沒有 python3-pip）"
  curl -sSL https://bootstrap.pypa.io/pip/get-pip.py -o /tmp/get-pip.py
  .venv/bin/python3 /tmp/get-pip.py -q
fi
# google-genai 是 recognition.py 裡的延遲 import，掃頂層 import 看不到它，
# 少了它整條辨識路徑會 500（不是優雅降級）
if ! .venv/bin/python3 -c "import fastapi, uvicorn, pydantic, multipart; from google import genai" 2>/dev/null; then
  say "安裝後端套件"
  .venv/bin/pip install -q fastapi uvicorn pydantic python-multipart google-genai
fi

# Node：沒有 sudo，抓官方 tarball 裝進 ~/.local
if ! command -v node >/dev/null 2>&1; then
  say "安裝 Node $NODE_VERSION 到 $NODE_HOME"
  mkdir -p "$NODE_HOME"
  curl -sSL "https://nodejs.org/dist/$NODE_VERSION/node-$NODE_VERSION-linux-x64.tar.xz" \
    | tar -xJ -C "$NODE_HOME" --strip-components=1
fi
if ! command -v pnpm >/dev/null 2>&1; then
  say "啟用 pnpm（corepack）"
  corepack enable --install-directory "$NODE_HOME/bin" >/dev/null
  corepack prepare pnpm@latest --activate >/dev/null
fi

# .env：沒有 key 流程仍可跑，只是影像辨識會退回人工挑批次
if [[ ! -f .env ]]; then
  warn "沒有 .env —— 影像辨識會停用，其餘功能照常"
  warn "  要開啟：echo 'GEMINI_API_KEY=...' > .env"
fi

if [[ ! -d frontend/node_modules ]]; then
  say "安裝前端套件"
  # pnpm 11 把「build script 被忽略」當硬錯誤，sharp 得先放行否則 pnpm dev 起不來
  (cd frontend && pnpm install && pnpm approve-builds sharp)
fi

if [[ $RESET_DB -eq 1 ]]; then
  say "重置資料庫"
  rm -f backend/urdwms.db
fi
if [[ ! -f backend/urdwms.db ]]; then
  say "seed 資料庫"
  .venv/bin/python3 backend/seed.py
fi

[[ $SETUP_ONLY -eq 1 ]] && { say "環境就緒（--setup，不啟動）"; exit 0; }

# ── 啟動 ────────────────────────────────────────────────────────

port_busy() { (exec 3<>"/dev/tcp/127.0.0.1/$1") 2>/dev/null && { exec 3<&-; return 0; } || return 1; }
for p in $BACKEND_PORT $FRONTEND_PORT; do
  port_busy "$p" && die "port $p 已經有東西在跑了（舊的 server？先關掉再試）"
done

# 開 job control：背景工作各自成為 process group leader（PGID == PID），
# 這樣才收得掉孫程序 —— pnpm 會再生一個 next-server，只殺子 shell 的話
# 它會活下來繼續佔著 3071。
set -m

BACKEND_PID=""; FRONTEND_PID=""
kill_group() {   # $1=PID，連同它底下整個 process group 一起送訊號
  local pid="$1" sig="${2:-TERM}"
  [[ -z "$pid" ]] && return 0
  kill -"$sig" -- -"$pid" 2>/dev/null || kill -"$sig" "$pid" 2>/dev/null || true
}
cleanup() {
  echo
  say "收工中…"
  kill_group "$FRONTEND_PID"
  kill_group "$BACKEND_PID"
  # 給它們幾秒好好結束，賴著不走的就 KILL
  for _ in $(seq 20); do
    port_busy "$FRONTEND_PORT" || port_busy "$BACKEND_PORT" || break
    sleep 0.25
  done
  port_busy "$FRONTEND_PORT" && kill_group "$FRONTEND_PID" KILL
  port_busy "$BACKEND_PORT"  && kill_group "$BACKEND_PID"  KILL
  wait 2>/dev/null || true
}
trap cleanup EXIT INT TERM

say "後端 :$BACKEND_PORT（$BACKEND_HOST）→ $BACKEND_LOG"
set -a; [[ -f .env ]] && . ./.env; set +a
.venv/bin/python3 -m uvicorn app.main:app \
  --app-dir backend --port "$BACKEND_PORT" --host "$BACKEND_HOST" \
  > "$BACKEND_LOG" 2>&1 &
BACKEND_PID=$!

for _ in $(seq 60); do
  port_busy "$BACKEND_PORT" && break
  kill -0 "$BACKEND_PID" 2>/dev/null || { tail -20 "$BACKEND_LOG"; die "後端沒起來"; }
  sleep 0.5
done
port_busy "$BACKEND_PORT" || { tail -20 "$BACKEND_LOG"; die "後端逾時"; }

say "前端 :$FRONTEND_PORT → $FRONTEND_LOG"
(cd frontend && exec pnpm dev > "$FRONTEND_LOG" 2>&1) &
FRONTEND_PID=$!

for _ in $(seq 120); do
  grep -q "Ready in" "$FRONTEND_LOG" 2>/dev/null && break
  kill -0 "$FRONTEND_PID" 2>/dev/null || { tail -20 "$FRONTEND_LOG"; die "前端沒起來"; }
  sleep 0.5
done

GREEN_DOT="$(printf '\033[1;32m●\033[0m')"
cat <<EOF

  $GREEN_DOT Urd-WMS 跑起來了

    前端   http://localhost:$FRONTEND_PORT
    API    http://localhost:$BACKEND_PORT/docs

    peter / peter0821        admin
    kuo · huang / urdwms2026 manager
    operator / urdwms2026    user

  Ctrl+C 停止。
EOF

wait
