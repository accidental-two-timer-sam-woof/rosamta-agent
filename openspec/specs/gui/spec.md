# gui Specification

## Purpose
以桌面 app 呈現 rosamta 的狀態與待處理項目，讓使用者用圖形介面檢視 diff、做選擇與調整偏好；所有操作都經由 core，與 CLI、背景服務一致。

## Requirements

### Requirement: Shared core and lock
GUI SHALL 只透過 core 讀寫設定與狀態，寫入前 SHALL 取得與 CLI、背景服務相同的鎖。操作失敗時 SHALL 在視窗內顯示錯誤，畫面資料維持不變。CLI 或背景服務造成的變動 SHALL 在數秒內反映在畫面上。

#### Scenario: Change made from the CLI
- **WHEN** GUI 開著時，使用者在終端機執行 `rsmt review ignore 2`
- **THEN** 數秒內第 2 項從 GUI 的待處理清單消失

### Requirement: Sidebar
側欄 SHALL 固定為視窗高度，列出已支援的頁面，待處理頁 SHALL 顯示項目數量；底部 SHALL 以狀態點表示背景服務是否執行，並顯示識別名稱。尚未實作的頁面 SHALL NOT 出現在側欄。

#### Scenario: Daemon stopped
- **WHEN** 背景服務沒有執行
- **THEN** 側欄底部的狀態點改為停止的顏色，不顯示說明文字

### Requirement: Overview
總覽 SHALL 顯示需要處理的項目、背景服務、接管、推送、拉取與遠端的狀態、各 shell 是否已掛載，以及最近活動，並提供立即同步。

#### Scenario: Sync now
- **WHEN** 使用者在總覽按「同步」
- **THEN** rosamta 執行拉取與推送，完成後上次同步時間更新

### Requirement: Review page
待處理頁 SHALL 列出所有項目；選取的項目 SHALL 顯示 diff，rc 檔段落另顯示會產生的模組。使用者 SHALL 能接管（套用範圍為本機或所有主機）、忽略，dotfile 項目另可採用 repo 版本。`rosamta://review/<id>` SHALL 開啟此頁並選取該項目。

#### Scenario: Open from a notification
- **WHEN** 使用者點通知的「查看差異」，通知對應第 3 項
- **THEN** GUI 開啟待處理頁並選取第 3 項

#### Scenario: Adopt for all hosts
- **WHEN** 使用者把第 1 項的套用範圍切到所有主機並按「接管」
- **THEN** 結果與執行 `rsmt review accept 1 --scope all` 相同，項目從清單移除

### Requirement: Modules page
模組頁 SHALL 依載入階段分組列出 repo 與本機的模組，每列顯示開關、名稱與類型，不顯示欄位標題列；SHALL 可依名稱搜尋。切換開關 SHALL 更新該模組的 `enabled` 並重新套用。

#### Scenario: Disable a module
- **WHEN** 使用者關閉 `bun` 模組的開關
- **THEN** `bun` 的 `enabled` 變為 false、新開的 shell 不再載入它，畫面上開關維持關閉

### Requirement: Dotfiles page
設定檔頁 SHALL 列出套用到本機的每個 dotfile 的路徑、部署方式與狀態（已部署、待處理、略過）。

#### Scenario: A conflict is listed
- **WHEN** `~/.gitconfig` 在待處理清單中因衝突等待決定
- **THEN** 設定檔頁中該列的狀態顯示待處理

### Requirement: History page
歷史頁 SHALL 列出 repo 最近的 commit（時間、訊息、短 hash）與 snapshot（時間、原因、檔案）。

#### Scenario: After adopting
- **WHEN** 使用者接管了一段 rc 檔內容
- **THEN** 歷史頁出現該次接管的 commit，以及接管前的 snapshot

### Requirement: Settings page
設定頁 SHALL 顯示並可修改標籤、偏好方法（接管、原始內容、推送、拉取）與拉取間隔，顯示識別名稱與遠端，並可啟用或停用背景服務。修改 SHALL 寫入 `host.toml` 並保留其中的註解。

#### Scenario: Change the adopt policy
- **WHEN** 使用者把接管改成「自動」
- **THEN** `host.toml` 的 `adopt` 變為 `auto`，背景服務之後直接接管（secret-guard 規定要先詢問的內容除外）

### Requirement: Host not set up
這台主機還沒有 repo 或 `host.toml` 時，GUI SHALL 只顯示建立新的 repo 與以 URL 加入既有 repo 兩個選項。

#### Scenario: Join from the app
- **WHEN** 使用者在尚未設定的主機上輸入 repo 的 URL 並按「加入」
- **THEN** 結果與執行 `rsmt init <url>` 相同，之後顯示總覽

### Requirement: Language and theme
介面語言 SHALL 跟隨系統（英文、繁體中文），主題 SHALL 跟隨系統的淺色或深色。介面 SHALL NOT 顯示 CLI 旗標或說明性的副標；同一欄的值 SHALL 使用同一種語言。

#### Scenario: Dark mode
- **WHEN** 系統切換到深色外觀
- **THEN** GUI 不需重新啟動就改用深色主題
