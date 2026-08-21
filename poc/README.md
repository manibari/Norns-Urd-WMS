# 辨識 PoC

量測「封閉候選集辨識」撐不撐得起 v1。規格見 [`../docs/poc/recognition-poc-spec.md`](../docs/poc/recognition-poc-spec.md)。

## 怎麼跑

```bash
export ANTHROPIC_API_KEY=...        # 或 ant auth login

# 1. 辨識並記錄（會呼叫 API，計費）
python3 poc/run_poc.py \
  --annotations poc/annotations-smoke.csv \
  --images docs/requirements/assets/packaging-material-fifo \
  --record poc/out/run1.json \
  --out poc/out

# 2. 重新評分不同門檻（不呼叫 API，免費）
python3 poc/run_poc.py \
  --annotations poc/annotations-smoke.csv \
  --replay poc/out/run1.json \
  --min-margin 1.5 --out poc/out

# 3. 掃描門檻組合，畫「誤命中率 vs 退人工率」取捨曲線
python3 poc/run_poc.py \
  --annotations poc/annotations-smoke.csv \
  --replay poc/out/run1.json --sweep --out poc/out
```

**辨識與評分是兩階段。** 辨識計費且結果固定，門檻可以免費重掃 —— 錄一次，掃無限次。

## 測試

```bash
python3 -m unittest discover -s poc/tests -t poc
```

`normalize.py` 與 `matching.py` 是純函式，**也是 v1 的正式程式碼**（requirement US-4 的人工輸入版要用同一套正規化），不是拋棄式 PoC 程式。

## 模組

| 檔案 | 責任 |
|------|------|
| `urdwms_poc/normalize.py` | 四種日期格式 + 料號簡寫 → 標準值；OCR 混淆感知距離。**讀不清的值降級成可比較的 key，絕不偽裝成乾淨值** |
| `urdwms_poc/matching.py` | 候選集比對。兩個旋鈕：`max_distance`（信不信這個讀值）、`min_margin`（最佳與次佳夠不夠拉開）。兩者都偏向拒答 |
| `urdwms_poc/recognition.py` | 影像 → 欄位。provider 可換，不外洩廠商 SDK 型別。prompt 的首要任務是讓模型回 null 而不是編一個 |
| `urdwms_poc/evaluate.py` | hit / false_hit / defer 判定 + 分層彙總 + 幻覺直接量測 |
| `run_poc.py` | CLI：辨識 → 評分 → 報告；支援 replay 與門檻掃描 |

## 標註檔欄位

| 欄位 | 說明 |
|------|------|
| `photo_id` | 檔名 |
| `item_code` | 料號真值 |
| `receipt_date_truth` | 進貨日真值（ISO）。**留空 = 此箱面沒有章**，正確行為是回 null |
| `manufacture_date_truth` | 製造日真值 |
| `stratum` | 分層標籤（章淡/歪斜/遮擋/逆光…），用於分層報告 |
| `candidate_lots` | 該料號當下在庫的所有進貨日，`\|` 分隔 |

> ⚠️ **ground truth 要對實體箱子抄，不是看照片標。** 人已經在放大的照片上把章面的「金軒揚」抄成「鑫軒揚」（requirement §2B）。

## `annotations-smoke.csv` 不是 PoC 資料集

那是 M7 留下的四張現場照，用來驗證管線與量測方法，**不是可下結論的樣本**：n=4、候選集是依文件構造的而非現場在庫清單。真正的 PoC 需要現場實拍 100–200 張並依規格 §5.1 分層。

不過這四張各有針對性：

| 照片 | 測什麼 | 正確行為 |
|------|--------|---------|
| `IMG_2928` / `IMG_2929` | 側面有章的正常情形 | 命中 `2026-08-12` |
| `IMG_2930` | 頂面無章 | **回 null**。若編出一個日期並命中候選 = 幻覺，正是本設計要消滅的 |
| `IMG_2921` | 拍到的是紙本表單不是箱子（表單上印有 `20260410`/`20260303`） | **回 null**。讀成箱子即為誤命中 |
