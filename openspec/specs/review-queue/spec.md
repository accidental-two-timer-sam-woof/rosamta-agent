# review-queue Specification

## Purpose
保存等待使用者決定的項目：接管的 rc 檔變動、dotfile 的衝突與本機修改、等待秘密來源或變數的 template，以及等待執行的腳本；並在使用者忘記處理時主動提醒，避免變動再次累積。

## Requirements

### Requirement: Adopt policy
掃描到可接管的內容時，rosamta SHALL 依本機的接管偏好處理：「自動」立即接管（secret-guard 規定要先詢問的內容除外）；「詢問」放進待處理清單；「關閉」不做任何記錄。

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

### Requirement: Waiting items
template 因秘密來源鎖住、需要執行秘密指令或缺少變數而無法 render 時，待處理清單 SHALL 列出該路徑與在等待的對象。這類項目 SHALL NOT 能 accept、ignore 或 discard；嘗試時 SHALL 說明要在終端機執行 `rsmt apply` 並以 2 結束。該 template 成功 render 後，項目 SHALL 自動移除。`--json` SHALL 輸出等待的類型（`locked`、`command` 或 `unset`）與對象。

#### Scenario: Locked source
- **WHEN** rbw 鎖住，`~/.config/gh/hosts.yml` 的 template 需要 Bitwarden 的值
- **THEN** `rsmt review` 列出該路徑在等待 rbw 解鎖

#### Scenario: Resolved by apply
- **WHEN** 使用者在終端機執行 `rsmt apply` 並解鎖 rbw
- **THEN** template 部署完成，該項目從清單移除

### Requirement: Script items
待處理清單 SHALL 列出等待執行的腳本，包含 `source`、`run` 種類，以及從未執行、內容改變或上次執行失敗。`accept` SHALL 執行該腳本（在終端機中時接終端機），成功後移除項目，失敗時項目保留並顯示紀錄；`ignore` SHALL 把腳本記錄為已執行但不執行（`onchange` 到下次改變為止）；`discard` SHALL 被拒絕並以 2 結束。宣告移除、不再符合本機的 `when`，或腳本已在其他套用中執行後，項目 SHALL 自動移除；項目等待時腳本再改變，SHALL 沿用同一個 ID。`accept` 時腳本與列出時不同，SHALL 不執行並以 2 結束，項目更新為新的內容。

#### Scenario: Run from the queue
- **WHEN** 使用者對等待中的腳本項目執行 `rsmt review accept <id>`
- **THEN** 腳本在終端機中執行，成功後項目從清單移除

#### Scenario: Already installed by hand
- **WHEN** 使用者對 `once` 的安裝腳本執行 `rsmt review ignore <id>`
- **THEN** 腳本不執行，之後的套用也不再執行它
