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
側欄 SHALL 固定為視窗高度，只列出三個頁面：待處理、內容、歷史；待處理 SHALL 顯示問題與需要決定的項目的總數。底部 SHALL 以狀態點表示背景服務是否執行，顯示識別名稱與開啟設定的按鈕；其下 SHALL 顯示同步狀態（上次同步的時間、同步中、尚未同步或尚未設定遠端）與同步的按鈕。沒有遠端時同步的按鈕 SHALL 停用。只以圖示表示的按鈕 SHALL 在游標停留時顯示名稱。

#### Scenario: Daemon stopped
- **WHEN** 背景服務沒有執行
- **THEN** 側欄底部的狀態點改為停止的顏色，不顯示說明文字

#### Scenario: Sync now
- **WHEN** 使用者按側欄底部的同步
- **THEN** rosamta 執行拉取與推送，完成後上次同步時間更新

#### Scenario: No remote
- **WHEN** repo 沒有設定遠端
- **THEN** 側欄顯示尚未設定遠端，同步的按鈕不能按

### Requirement: Review page
待處理頁 SHALL 把清單分成三個各自有外框的區塊：問題、需要決定、等待中；每列第一行是名稱與時間，第二行是發生了什麼。選取的列 SHALL 只改變底色。問題 SHALL 與背景服務通知的問題來自同一份清單，並以相同的文字顯示。

選取的項目 SHALL 顯示帶行號的 diff，rc 檔段落另顯示會加入 repo 的模組；顯示既有檔案的區塊 SHALL 有開啟該檔案的按鈕：檔案不存在時開啟所在資料夾，資料夾也不存在時先建立。使用者 SHALL 能接管（套用範圍為本機或所有主機）、忽略，dotfile 項目另可採用 repo 版本，採用前 SHALL 先確認。選取的問題 SHALL 顯示完整的錯誤訊息與相關檔案，重試有用時提供重試。`rosamta://review/<id>` SHALL 開啟此頁並選取該項目。

#### Scenario: Open from a notification
- **WHEN** 使用者點通知的「查看差異」，通知對應第 3 項
- **THEN** GUI 開啟待處理頁並選取第 3 項

#### Scenario: Adopt for all hosts
- **WHEN** 使用者把第 1 項的套用範圍切到所有主機並按「接管」
- **THEN** 結果與執行 `rsmt review accept 1 --scope all` 相同，項目從清單移除

#### Scenario: A blocked publish
- **WHEN** 推送因為疑似秘密而受阻
- **THEN** 待處理頁的問題區塊出現一列，文字與背景服務的通知相同，側欄的數量加一

#### Scenario: Open a file that is gone
- **WHEN** 使用者按開啟檔案，而該檔案已被刪除
- **THEN** 系統開啟它所在的資料夾

#### Scenario: Use the repo version
- **WHEN** 使用者對 `~/.gitconfig` 按「採用 repo 版本」
- **THEN** GUI 先詢問是否取代，確認後才改寫本機的檔案

### Requirement: Library page
內容頁 SHALL 以分段控制切換模組與設定檔，並可依名稱搜尋目前的清單。

模組 SHALL 先列出各 shell 的掛載點與是否已掛載，再依載入階段分組列出 repo 與本機的模組，每列顯示開關、名稱與類型，不顯示欄位標題列。切換開關 SHALL 更新該模組的 `enabled` 並重新套用。

設定檔 SHALL 列出套用到本機的每個 dotfile 的路徑、部署方式、是否加密與狀態（已部署、待處理、略過、未部署）。待處理的列 SHALL 可以點選，並開啟待處理頁中對應的項目。類型、部署方式與加密 SHALL 以圖示表示，游標停留時顯示名稱。

#### Scenario: Disable a module
- **WHEN** 使用者關閉 `bun` 模組的開關
- **THEN** `bun` 的 `enabled` 變為 false、新開的 shell 不再載入它，畫面上開關維持關閉

#### Scenario: A conflict is listed
- **WHEN** `~/.gitconfig` 在待處理清單中因衝突等待決定
- **THEN** 設定檔中該列的狀態顯示待處理，點選後開啟待處理頁的該項目

### Requirement: History page
歷史頁 SHALL 以分段控制切換變更與快照：變更列出 repo 最近的 commit（時間、訊息、短 hash），快照列出時間、原因與檔案，原因以介面的語言顯示。沒有紀錄時 SHALL 顯示沒有紀錄。

#### Scenario: After adopting
- **WHEN** 使用者接管了一段 rc 檔內容
- **THEN** 變更出現該次接管的 commit，快照出現接管前的 snapshot

### Requirement: Settings window
設定 SHALL 開在自己的視窗，由側欄的按鈕或系統慣用的快速鍵開啟，同時只有一個；主視窗關閉時 SHALL 一併關閉。設定 SHALL 分為本機（識別名稱、標籤）、自動處理（接管、原本的行、執行腳本、開新終端機時提醒）、同步（遠端、推送、拉取、拉取間隔）與背景服務（登入時啟動、狀態）。拉取為手動時拉取間隔 SHALL 停用。修改 SHALL 寫入 `host.toml` 並保留其中的註解。

#### Scenario: Change the adopt policy
- **WHEN** 使用者把接管改成「自動」
- **THEN** `host.toml` 的 `adopt` 變為 `auto`，背景服務之後直接接管

#### Scenario: Opened twice
- **WHEN** 設定視窗開著時，使用者再按一次設定
- **THEN** 既有的設定視窗移到最前面，不開第二個

### Requirement: Host not set up
這台主機還沒有 repo 或 `host.toml` 時，GUI SHALL 只顯示建立新的 repo 與以 URL 加入既有 repo 兩個選項。

#### Scenario: Join from the app
- **WHEN** 使用者在尚未設定的主機上輸入 repo 的 URL 並按「加入」
- **THEN** 結果與執行 `rsmt init <url>` 相同，之後顯示待處理頁

### Requirement: Language and theme
介面語言 SHALL 跟隨系統（英文、繁體中文），主題 SHALL 跟隨系統的淺色或深色。介面 SHALL NOT 顯示 CLI 旗標或說明性的副標；同一欄的值 SHALL 使用同一種語言。

#### Scenario: Dark mode
- **WHEN** 系統切換到深色外觀
- **THEN** GUI 不需重新啟動就改用深色主題
