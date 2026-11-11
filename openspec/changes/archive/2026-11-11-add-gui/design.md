# Design

## Context

ADR 0005：GPUI 透過 `gpui-kit`（含 gpui-component）引入並鎖定版本。頁面依使用者確認過的預覽（總覽、待處理、模組、診斷、應用程式、設定、設定精靈），間距規範與文案限制寫在 proposal 與使用者對預覽的意見中。core 已有：接管與待處理清單、dotfile 部署與其待處理項目、推送與拉取的狀態、背景服務的狀態與啟用、snapshot、host 設定。診斷（`add-providers-and-doctor`）、應用程式（`add-packages`）、從 chezmoi 遷移（`add-chezmoi-import`）尚未實作。

## Goals / Non-Goals

**Goals:**
- 第一版涵蓋 core 已支援的頁面：總覽、待處理、模組清單、設定檔、歷史、設定，以及這台主機尚未設定時的建立或加入 repo。
- GUI 與 CLI、背景服務同時使用時行為一致：所有寫入都經過 core 並取得同一把鎖。
- 淺色與深色主題、英文與繁體中文介面，跟隨系統。

**Non-Goals（之後的 change 補上）:**
- 診斷、應用程式頁面與設定精靈的掃描步驟：依賴尚未實作的 core。側欄在 core 完成前不顯示這些頁面。
- 模組的側邊面板（provider 選項、互斥確認）：依賴 provider。第一版的模組頁只有清單與開關。
- 總覽中各 shell 的啟動時間：由 doctor 量測。
- macOS `.app` 的 URL scheme 註冊與簽章：屬於打包；第一版處理以命令列參數傳入的 `rosamta://` URL。

## Decisions

### 結構

```
crates/rosamta-gui/src/
├── main.rs        # 開啟視窗、主題、URL
├── app.rs         # 根視圖：側欄、目前頁面、通知
├── state.rs       # 從 core 讀出的畫面資料與重新整理
├── actions.rs     # 呼叫 core 的寫入操作（取得鎖、背景執行）
├── text.rs        # 介面文字（讀 core 的 locales）
├── theme.rs       # 色彩與間距 token
└── pages/         # overview、review、modules、dotfiles、history、settings、setup
```

### 資料流

- 畫面資料（`state.rs`）是一份從 core 讀出的快照：host 設定、待處理清單、同步狀態、背景服務狀態、模組、dotfile 部署紀錄、repo 的 commit 與 snapshot。讀取在背景執行緒進行，結果回到主執行緒後替換整份快照。
- 重新整理的時機：視窗啟動、每 2 秒（只比對 state 目錄、repo 與 `host.toml` 的修改時間，有變才重讀）、每次操作完成後。背景服務與 CLI 的變動因此在 2 秒內出現在畫面上。
- 寫入（`actions.rs`）一律在背景執行緒呼叫 core，與 CLI 相同地先取得接管的鎖；失敗時以視窗內的通知顯示 core 的錯誤訊息，畫面資料不變。
- GUI 不自己解析或寫入任何設定檔。

### 頁面

| 頁面 | 內容 | 操作 |
|---|---|---|
| 總覽 | 需要處理的項目、狀態（背景服務、接管、推送、拉取、遠端）、各 shell 的掛載狀態、最近活動 | 同步 |
| 待處理 | 項目清單；選取的項目顯示 diff 與會產生的模組 | 接管（本機／所有主機）、忽略、採用 repo 版本（dotfile） |
| 模組 | 依載入階段分組；每列開關、名稱、類型；搜尋 | 啟用、停用 |
| 設定檔 | 每個 dotfile 的路徑、部署方式與狀態 | — |
| 歷史 | repo 的 commit 與 snapshot | — |
| 設定 | 識別名稱、標籤、偏好方法（接管、原始內容、推送、拉取）、拉取間隔、遠端、背景服務 | 修改、新增與移除標籤、啟用或停用背景服務 |
| 尚未設定 | 建立新的 repo、輸入 URL 加入既有的 repo | 建立、加入 |

最近活動取自 repo 的最近 commit 與同步狀態，不另存紀錄。歷史頁第一版只列出；還原單次接管需要對應 commit 與接管紀錄，之後再加。

### 文字與主題

- 介面文字放在 core 的 `locales/*.toml`，與背景服務的通知共用一份表，新增 `[gui]` 等區段；語言跟隨系統。
- 文字遵守使用者對預覽的意見：不出現 CLI 旗標、不寫建議或說明性的副標、同一欄只用一種語言、狀態標記固定寬度、背景服務只以狀態點表示。
- 色彩取自預覽的 token（淺色與深色各一組），以 gpui-component 的主題設定；跟隨系統的外觀切換。
- 字型使用系統字型（含中文字），等寬文字使用平台的等寬字型，不額外打包字型。
- 間距只用 4、8、12、16、20、24、32、40、48 px，依 proposal 的規範。

### URL

`rosamta://review/<id>` 開啟待處理頁並選取該項目。第一版接受命令列參數傳入的 URL，以及 GPUI 收到的 open URL 事件；`.app` 註冊 scheme 屬於打包。

### 驗證

- 畫面資料的讀取與操作在 core 已有測試；GUI 以 gpui 的 test-support 測試頁面狀態（選取、切換、操作後重新整理）。
- macOS 上以實際視窗截圖檢查淺色、深色與繁中、英文；Linux 只在 CI 編譯。

## Risks / Trade-offs

- 2 秒輪詢 → 只比對修改時間，成本低；之後若需要即時再改為檔案監看。
- 模組頁沒有側邊面板，看起來比預覽簡單 → provider 完成後補上。
- GPUI 沒有 semver 保證 → 版本鎖定，升級是獨立的 change（ADR 0005）。
