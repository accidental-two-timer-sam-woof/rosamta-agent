## ADDED Requirements

### Requirement: Items other hosts added
有其他主機最近新增、本機尚未套用的項目時，待處理頁的「需要決定」SHALL 列出這些項目，使用者 SHALL 能把本機加進它們的套用範圍，結果與 `rsmt include` 相同。

#### Scenario: Include recent items
- **WHEN** 新主機的待處理頁列出其他主機最近新增的 3 項，使用者把其中 2 項加入本機
- **THEN** 結果與對這 2 項執行 `rsmt include --yes` 相同，待處理頁剩下 1 項

## MODIFIED Requirements

### Requirement: Review page
待處理頁 SHALL 把清單分成三個各自有外框的區塊：問題、需要決定、等待中；每列第一行是名稱與時間，第二行是發生了什麼。選取的列 SHALL 只改變底色。問題 SHALL 與背景服務通知的問題來自同一份清單，並以相同的文字顯示。

選取的項目 SHALL 顯示帶行號的 diff，rc 檔段落另顯示會加入 repo 的模組，與其他主機的 module 相同時改為顯示該 module 套用範圍的變更；顯示既有檔案的區塊 SHALL 有開啟該檔案的按鈕：檔案不存在時開啟所在資料夾，資料夾也不存在時先建立。使用者 SHALL 能接管（套用範圍為本機或所有主機）、忽略，dotfile 項目另可採用 repo 版本，採用前 SHALL 先確認。選取的問題 SHALL 顯示完整的錯誤訊息與相關檔案，重試有用時提供重試。`rosamta://review/<id>` SHALL 開啟此頁並選取該項目。

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

#### Scenario: Same as another host
- **WHEN** 選取的項目與 `mbp` 的 `bun` module 相同
- **THEN** 頁面顯示 `bun` 的 `hosts` 會加入本機，而不是新的 module
