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
| `docs/_inherited/` | 前身 M7 的 requirement v0.2 + 系統架構（唯讀參考） |

## 狀態

需求階段 v1.1。**下一步是辨識 PoC** —— 驗收章確定改不掉（Q1=no），
必須先量出「候選集命中率 / 誤命中率」才知道整套能不能成立。PoC 有結論再進 `/user-flow` → mockup → plan。
