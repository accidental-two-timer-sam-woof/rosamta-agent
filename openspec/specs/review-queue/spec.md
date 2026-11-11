# review-queue Specification

## Purpose
保存等待使用者決定的接管項目，並在使用者忘記處理時主動提醒，避免 rc 檔的變動再次累積。

## Requirements

### Requirement: Adopt policy
掃描到可接管的內容時，rosamta SHALL 依本機的接管偏好處理：「自動」立即接管；「詢問」放進待處理清單；「關閉」不做任何記錄。

#### Scenario: Ask
- **WHEN** 接管偏好為「詢問」，掃描發現新的段落
- **THEN** 該段落出現在待處理清單中，rc 檔不變

#### Scenario: Off
- **WHEN** 接管偏好為「關閉」
- **THEN** 掃描不產生待處理項目，也不修改任何檔案

### Requirement: Review command
`rsmt review` SHALL 列出所有待處理項目，每項包含編號、來源檔、行號範圍、偵測時間與預計產生的 module；`rsmt review accept <id>` 接管該項，`rsmt review ignore <id>` 忽略該項。`accept` SHALL 接受 `--scope host|all` 指定套用範圍。

#### Scenario: Accept for all hosts
- **WHEN** 使用者執行 `rsmt review accept 3 --scope all`
- **THEN** 第 3 項被接管，產生的 module 沒有 `hosts` 限制

### Requirement: Ignored content stays ignored
被忽略的段落 SHALL 留在 rc 檔中，之後內容沒有改變時 SHALL 不再列入待處理清單。

#### Scenario: Ignored segment unchanged
- **WHEN** 使用者忽略一段內容，之後再次掃描
- **THEN** 該段不會再出現在待處理清單中

#### Scenario: Ignored segment edited
- **WHEN** 被忽略的段落內容之後被修改
- **THEN** 修改後的段落重新出現在待處理清單中

### Requirement: Shell start reminder
待處理清單不是空的、且本機開啟了提醒時，互動 shell 啟動時 SHALL 印出一行英文提醒，包含待處理的數量與 `rsmt review` 指令；非互動 shell SHALL 不輸出任何提醒。

#### Scenario: Pending items
- **WHEN** 有 2 個待處理項目，使用者開啟新的互動 zsh
- **THEN** 終端機顯示一行提醒，指出有 2 個待處理項目並建議執行 `rsmt review`

#### Scenario: Reminder disabled
- **WHEN** 使用者關閉提醒
- **THEN** 開啟互動 shell 時不顯示提醒

### Requirement: Machine-readable queue
`rsmt review --json` SHALL 以 JSON 輸出完整的待處理清單，供 GUI 與 AI 使用。

#### Scenario: JSON queue
- **WHEN** 使用者執行 `rsmt review --json`
- **THEN** stdout 是一個包含所有待處理項目的 JSON 陣列

### Requirement: Dotfile items
待處理清單 SHALL 也包含 dotfile 的衝突與本機修改，每項列出路徑、原因與偵測時間。`rsmt review accept <id>` SHALL 保留本機版本：`--scope host`（預設）存到 `hosts/<識別名稱>/<路徑>` 並新增限定本機的宣告，`--scope all` 取代原宣告的來源。`rsmt review discard <id>` SHALL 先存 snapshot 再部署 repo 版本，只適用於 dotfile 項目。`rsmt review ignore <id>` SHALL 保留本機內容且不收進 repo，兩邊都沒再改變前，套用時略過該路徑且不再列出。

#### Scenario: Discard local changes
- **WHEN** `~/.gitconfig` 因衝突列在待處理清單第 4 項，使用者執行 `rsmt review discard 4`
- **THEN** `~/.gitconfig` 換成 repo 版本，原內容保存在 snapshot 中，第 4 項從清單移除

#### Scenario: Discard an adopt item
- **WHEN** 使用者對 rc 檔段落的項目執行 `rsmt review discard`
- **THEN** 指令說明 discard 只適用於 dotfile 項目並以 2 結束
