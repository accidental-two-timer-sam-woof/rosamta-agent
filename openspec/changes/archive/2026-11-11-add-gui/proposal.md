# Proposal

## Why

接管、診斷、應用程式安裝與設定都有 CLI，但檢視 diff、在多個值中做選擇、調整偏好這些操作，用圖形介面更直接；從通知按「查看差異」也需要有地方開啟。GUI 是專題 demo 的主要畫面。

## What Changes

- 以 GPUI 透過 `gpui-component` 建立桌面 app（ADR 0005），支援 macOS 與 Linux。
- 頁面依已確認的預覽（https://claude.ai/artifact/P1ymLYrq559Yb4e1UpdAzA）：
  - 總覽：上次同步時間、需要處理的項目、狀態、各 shell 的掛載與啟動時間、最近活動。
  - 待處理：每項的 rc 檔 diff 與產生的模組，可切換套用範圍；接管、忽略、在編輯器開啟。
  - 模組：依載入階段列出，每列只有開關、名稱、類型與載入時間；選取後在側邊調整 provider 選項、shell 與套用範圍；啟用互斥的 provider 時先確認。
  - 設定檔：每個 dotfile 的部署方式與狀態。
  - 應用程式：篩選、選擇來源、顯示將執行的指令、執行與複製；每列只有名稱，以及安裝來源或「安裝」按鈕。
  - 診斷：問題與最佳化，一鍵修正、還原；多個值時列出讓使用者選。
  - 歷史：repo 的 commit 與 snapshot，可還原。
  - 設定：識別名稱、標籤、偏好方法（接管、原始內容、推送、拉取）、開 shell 提醒、遠端與 GitHub 連結、安裝來源、背景服務。
  - 設定精靈：選擇起點（全新開始、加入既有的 repo、從 chezmoi 遷移）、掃描與診斷、掛載 hook、啟用背景服務。
- 側欄固定為視窗高度，底部以狀態點顯示背景服務與識別名稱。
- 間距規範：以 4px 為基準，padding、gap、margin 只能使用 4、8、12、16、20、24、32、40、48 px。
  - 主內容區：上 32、左右 40、下 48；區塊之間 32；區塊標題到內容 12。
  - 側欄：內距 16／12；導覽項目 8／12，項目之間 4。
  - 卡片：內距 20；卡片內的群組之間 16，標題列到說明 12；卡片之間 16。
  - 清單列：可操作的列 12／16，緊湊的列 8／16；欄位之間 16。
  - 控制項：高度 36（緊湊 32、分段按鈕 28），左右內距 12 至 16；按鈕之間 8；圖示與文字 8；標籤與欄位 8。
  - 標記（badge）：高 20、左右 8；需要對齊時固定寬 56。
- 排序類設定（例如安裝來源）以垂直拖曳把手調整，把手也可用上下方向鍵操作。
- 介面語言跟隨系統（英文、繁體中文），支援淺色與深色主題。
- GUI 不自己寫設定檔的邏輯：所有操作都呼叫 `rosamta-core`，與 CLI、背景服務以檔案溝通。
- 支援 `rosamta://review/<id>` 等 URL，供通知開啟指定頁面。
- 所有介面文字在實作完成、推送之前先列出給使用者審閱。

## Capabilities

### New Capabilities

- `gui`: 各頁面的內容、互動、語言、主題與從通知開啟的行為。

### Modified Capabilities

（無）

## Impact

- `crates/rosamta-gui` 由空殼擴充為完整 app；macOS 的 `.app` 同時包含 helper app（`add-daemon`）。
- 依賴 `add-adopt`、`add-git-sync`、`add-daemon`、`add-providers-and-doctor`、`add-packages`、`add-dotfile-deploy` 提供的 core 操作；各頁面可在對應的 core 完成後逐步加入。
- 本 change 實作 core 已支援的頁面；診斷、應用程式頁面、模組的側邊面板與設定精靈的掃描和遷移步驟，由 `add-providers-and-doctor`、`add-packages`、`add-chezmoi-import` 各自加入 GUI。
