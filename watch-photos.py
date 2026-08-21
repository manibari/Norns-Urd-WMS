#!/usr/bin/env python3
"""盯著相機的存圖資料夾，出現新照片就送進 /api/recognize 並印出結果。

    ./watch-photos.py                     用預設資料夾，每 2 秒看一次
    ./watch-photos.py --dir /mnt/c/...    換一個資料夾
    ./watch-photos.py --all               連現有的舊照片也跑一遍
    ./watch-photos.py --json              印完整 JSON（預設只印一行摘要）

為什麼用輪詢而不是 inotify：相機存在 Windows 磁碟上，/mnt/c 是 drvfs，
inotify 在上面不會觸發 —— 訂了事件卻永遠等不到，比輪詢更難查。

只讀不寫：辨識端點不會動到庫存，看到的就是操作員按下辨識時會看到的東西。
"""
from __future__ import annotations

import argparse
import json
import mimetypes
import sys
import time
import urllib.error
import urllib.request
import uuid
from datetime import datetime
from pathlib import Path

DEFAULT_DIR = "/mnt/c/Users/HP/SCMVS/0821/MV-SC3016C-06M-WBN (DA9442483)"
DEFAULT_API = "http://127.0.0.1:8071"
SUFFIXES = {".jpg", ".jpeg", ".png", ".bmp"}


def login(api: str, username: str, password: str) -> str:
    body = json.dumps({"username": username, "password": password}).encode()
    req = urllib.request.Request(
        f"{api}/api/auth/login", data=body,
        headers={"Content-Type": "application/json"}, method="POST")
    with urllib.request.urlopen(req, timeout=15) as resp:
        return json.load(resp)["token"]


def recognize(api: str, token: str, path: Path) -> dict:
    """自己組 multipart —— 產線上不該為了傳一張圖去裝套件庫。"""
    boundary = uuid.uuid4().hex
    ctype = mimetypes.guess_type(path.name)[0] or "image/jpeg"
    body = b"".join([
        f'--{boundary}\r\n'.encode(),
        f'Content-Disposition: form-data; name="image"; filename="{path.name}"\r\n'.encode(),
        f"Content-Type: {ctype}\r\n\r\n".encode(),
        path.read_bytes(),
        f"\r\n--{boundary}--\r\n".encode(),
    ])
    req = urllib.request.Request(
        f"{api}/api/recognize", data=body, method="POST",
        headers={"Content-Type": f"multipart/form-data; boundary={boundary}",
                 "Authorization": f"Bearer {token}"})
    with urllib.request.urlopen(req, timeout=180) as resp:
        return json.load(resp)


def summarise(name: str, elapsed_ms: int, d: dict) -> str:
    r = d.get("recognition") or {}
    m = d.get("item_match") or {}
    model = r.get("model_code") or "讀不到型號"
    mfg = r.get("manufacture_date") or "—"
    stamp = r.get("receipt_date") or "無驗收章"
    item = d.get("item_label") or "—"

    # 品項對不對得上，比讀到什麼字更重要 —— 對不上就是選錯型號或拿錯箱
    if m.get("decision") == "lock":
        head = f"\033[1;32m✓\033[0m {item}"
    elif m.get("decision") == "defer":
        head = f"\033[1;33m?\033[0m 需人工確認"
    else:
        head = f"\033[1;31m✗\033[0m {m.get('decision') or '無法判定'}"

    lot = d.get("decision") or "—"
    reason = d.get("defer_reason")
    lot_txt = f"{lot}（{reason}）" if reason else lot
    fifo = d.get("fifo_target_lot_id")
    fifo_txt = f" · FIFO 應領 lot {fifo}" if fifo else ""

    return (f"{head}  {name}  {elapsed_ms}ms\n"
            f"    型號 {model} · 製造日 {mfg} · {stamp}\n"
            f"    批次 {lot_txt}{fifo_txt}")


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__,
                                 formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--dir", default=DEFAULT_DIR)
    ap.add_argument("--api", default=DEFAULT_API)
    ap.add_argument("--user", default="peter")
    ap.add_argument("--password", default="peter0821")
    ap.add_argument("--interval", type=float, default=2.0)
    ap.add_argument("--all", action="store_true", help="連現有舊照片也跑")
    ap.add_argument("--json", action="store_true", help="印完整 JSON")
    args = ap.parse_args()

    folder = Path(args.dir)
    if not folder.is_dir():
        print(f"找不到資料夾：{folder}", file=sys.stderr)
        return 1

    try:
        token = login(args.api, args.user, args.password)
    except Exception as exc:
        print(f"登入失敗（後端沒起來？）：{exc}", file=sys.stderr)
        return 1

    def shots() -> set[Path]:
        return {p for p in folder.iterdir()
                if p.is_file() and p.suffix.lower() in SUFFIXES}

    seen = set() if args.all else shots()
    print(f"▸ 盯著 {folder}")
    print(f"▸ 每 {args.interval}s 掃一次 · 已存在 {len(shots())} 張"
          f"{'（會一併處理）' if args.all else '（略過，只等新的）'}")
    print("▸ Ctrl+C 停止\n")

    while True:
        try:
            fresh = sorted(shots() - seen, key=lambda p: p.stat().st_mtime)
        except OSError as exc:          # 磁碟掉了、Windows 那邊搬走了
            print(f"讀不到資料夾：{exc}", file=sys.stderr)
            time.sleep(args.interval)
            continue

        for path in fresh:
            seen.add(path)
            # 相機可能還在寫檔，等大小穩定再送，否則會拿到半張圖
            last = -1
            for _ in range(20):
                try:
                    size = path.stat().st_size
                except OSError:
                    break
                if size == last and size > 0:
                    break
                last = size
                time.sleep(0.2)

            stamp = datetime.now().strftime("%H:%M:%S")
            print(f"[{stamp}] 新照片 {path.name} → 辨識中…")
            began = time.time()
            try:
                result = recognize(args.api, token, path)
            except urllib.error.HTTPError as exc:
                if exc.code == 401:      # token 過期就換一張再試一次
                    try:
                        token = login(args.api, args.user, args.password)
                        result = recognize(args.api, token, path)
                    except Exception as retry_exc:
                        print(f"    辨識失敗：{retry_exc}\n", file=sys.stderr)
                        continue
                else:
                    print(f"    辨識失敗 HTTP {exc.code}："
                          f"{exc.read()[:200].decode(errors='replace')}\n", file=sys.stderr)
                    continue
            except Exception as exc:
                print(f"    辨識失敗：{exc}\n", file=sys.stderr)
                continue

            elapsed = int((time.time() - began) * 1000)
            if args.json:
                print(json.dumps(result, ensure_ascii=False, indent=2))
            else:
                print(summarise(path.name, elapsed, result))
            print()

        time.sleep(args.interval)


if __name__ == "__main__":
    try:
        sys.exit(main())
    except KeyboardInterrupt:
        print("\n收工")
