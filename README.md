# Urd-WMS

**輕量包材批次管理 —— 拍箱子取代抄表單，FIFO 稽核、追溯、提醒。**

不需要買一整套 ERP，就能回答「哪一批包材、什麼時候、被誰、用在哪個產品上」。

## 定位

Norns 產品線 **Urd（記錄）** 層。前身是 Norns-ERP 的 M7 模組，2026-08-21 抽出成獨立可配置產品。

| 項目 | 內容 |
|------|------|
| 核心價值 | **紀錄** —— FIFO 擋單是疊在紀錄之上的提醒，不是主角 |
| 首發客戶 | 金軒揚食品（美珍香）成品包裝區 —— 取代紙本表單 P-4-P-01-07 |
| 資料 SoT | 自持批次庫存（`inventory_lot`），不依賴外部 ERP |
| 可配置 | Day 1 設定驅動：表單欄位 / 機台清單 / 正規化對映 / 提醒門檻 / 報表格式。**每廠一套部署**（非 SaaS 多租戶） |
| 明確不做 | 會計分錄、儲位/揀貨/盤點、成品出貨、與 Norns-ERP 同步（M7 各走各的） |

## 文件

| 文件 | 內容 |
|------|------|
| [`docs/requirements/packaging-lot-issue-fifo.md`](docs/requirements/packaging-lot-issue-fifo.md) | **v1 需求**（12 個 user story、範圍邊界、風險、待答問題） |
| `docs/requirements/assets/packaging-material-fifo/` | 現場實照 4 張（2026-08-20 拍攝） |
| [`docs/poc/recognition-poc-spec.md`](docs/poc/recognition-poc-spec.md) | **辨識 PoC 規格** — v1 第一優先，排在 mockup 之前 |
| `docs/_inherited/` | 前身 M7 的 requirement v0.2 + 系統架構（唯讀參考） |
| `core/urdwms_core/` | **共用核心**：日期/料號正規化、OCR 混淆距離、候選集比對、辨識 provider |
| `poc/` | PoC 工具與測試（見 [`poc/README.md`](poc/README.md)） |
| `backend/` · `frontend/` | 垂直切片 demo |

## 跑起來

```bash
echo 'GEMINI_API_KEY=...' > .env          # 辨識用；沒有的話流程仍可跑，會退人工挑批次

# 後端 :8071
python3 backend/seed.py
set -a && . ./.env && set +a
python3 -m uvicorn app.main:app --app-dir backend --port 8071

# 前端 :3071
cd frontend && pnpm install && pnpm dev
```

開 http://localhost:3071 。四個畫面：領用登錄 / 收貨建批 / 紀錄與追溯 / 提醒。

## 狀態

**垂直切片 demo 可跑**（US-1/2/3/4/5 核心路徑 + US-6/8 提醒）。範圍見下表。

| | 已做 | 未做 |
|---|---|---|
| 收貨 | 建批、料號可自由新增、期初補登 | 採購單、供應商 |
| 領用 | 拍照辨識、候選集比對、FIFO 硬擋、人工挑批、同 transaction 扣帳 | 離線暫存、連拍擇優 |
| 稽核 | 覆核放行（必填原因）、audit_log、影像永久留存 | RBAC（目前單一 demo 身分） |
| 提醒 | 效期 / 呆滯 / 低水位 / 明細待補 | 稽核異常告警（US-9） |
| 追溯 | 紀錄列表 + 影像 drill-down | 正反向查詢頁、報表匯出 |
| 設定 | — | 廠別配置（US-11，目前欄位寫死） |

辨識 PoC 初步結果（n=2 有章樣本，`gemini-pro-latest`）：命中 100%、誤命中 0%、
兩張陷阱樣本正確回 null。**撐不起結論**，真 PoC 仍需現場實拍 100–200 張分層樣本。

技術棧：FastAPI + SQLite（:8071）· Next.js 15 + antd v6（:3071）· 設計沿用
[ChimesFlow design system](../ChimesFlow/docs/design-system.md)，領用畫面宣告 mode-b 局部 override
（touch target 64px、大字、整片色塊回饋 —— 戴手套、包裝線、可能逆光）。
