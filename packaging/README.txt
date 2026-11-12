Urd-WMS 包材批次管理 —— Windows 版
====================================

拍箱子取代抄表單：收貨建批、拍照領用（FIFO 判定）、紀錄追溯、提醒。
單一執行檔，不需要安裝 Python、Node 或瀏覽器。

系統需求
--------
- Windows 10 / 11（64 位元）
- 顯示卡支援 OpenGL 2.0 以上（Intel HD 620 等內顯都可以）
- 記憶體 4 GB 以上

第一次使用
----------
1. 把整個資料夾放到想放的位置（例如 C:\Urd-WMS），執行 Urd-WMS.exe。
   Windows 若跳出「已保護您的電腦」，按「其他資訊」→「仍要執行」。
2. 登入帳號 admin，密碼 urdwms-admin。系統會要求立刻改成只有你知道的密碼。
3. 到「系統設定 → 人員與權限」建立其他人的帳號（倉管、廠長、作業員）。
4. 到「系統設定 → 影像設定」：
   - 貼上 Google Gemini API 金鑰（沒有也能用，只是拍照不辨識，改人工選批次）
   - 設定影像來源：網路相機（HTTP snapshot / 原始 TCP）或相機存檔資料夾，按「測試連線」確認
5. 到「基本資料」建立品項、機台、產品名稱，就可以開始收貨與領用。

想先看看示範資料：在命令提示字元執行
    Urd-WMS.exe --demo --data-dir C:\Urd-WMS-demo
示範帳號：peter / peter0821（系統管理者）、kuo / urdwms2026（倉管）、operator / urdwms2026（作業員）

資料放在哪
----------
預設 %LOCALAPPDATA%\UrdWMS\（系統設定頁上方有「開啟資料夾」「看日誌」）
  urdwms.db   資料庫（SQLite）
  uploads\    領用照片（追溯用，請勿刪除）
  urdwms.log  運作日誌：每次辨識、相機擷取、畫面上跳過的錯誤（回報問題時請附上）
  crash.log   程式當掉時的紀錄

備份：關掉程式後複製整個 UrdWMS 資料夾即可。
要放到別的位置：Urd-WMS.exe --data-dir D:\UrdWMS（可建立捷徑帶這個參數）

忘記密碼
--------
一般人員忘記密碼，請系統管理者到「系統設定 → 人員與權限」重設。
系統管理者忘記密碼，請另一位系統管理者重設。建議保留兩個系統管理者帳號。
如果只有一位系統管理者，請先關閉 Urd-WMS，在程式所在資料夾開啟「命令提示字元」，執行：
    Urd-WMS.exe --reset-admin
如果有多位系統管理者，請加上帳號，例如 Urd-WMS.exe --reset-admin peter。
資料放在別的位置時，請再加上 --data-dir D:\UrdWMS。
記下顯示的臨時密碼，用它登入，然後依提示立刻更改密碼。
這個指令只能在存放資料的電腦執行。任何能登入該 Windows 帳號的人都能執行，請鎖定該 Windows 帳號。

從舊版（Python + 網頁）搬過來
------------------------------
把舊版的 backend\urdwms.db 複製成 %LOCALAPPDATA%\UrdWMS\urdwms.db，
backend\uploads\ 整個複製到 %LOCALAPPDATA%\UrdWMS\uploads\。
資料表格式相同，帳號密碼照舊可以登入。

授權
----
介面字型 Noto Sans TC 採 SIL Open Font License 1.1，見 LICENSE-font-NotoSansTC-OFL.txt。
