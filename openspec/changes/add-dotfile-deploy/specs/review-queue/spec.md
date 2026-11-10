# Spec Delta

## ADDED Requirements

### Requirement: Dotfile items
待處理清單 SHALL 也包含 dotfile 的衝突與本機修改，每項列出路徑、原因與偵測時間。`rsmt review accept <id>` SHALL 保留本機版本：`--scope host`（預設）存到 `hosts/<識別名稱>/<路徑>` 並新增限定本機的宣告，`--scope all` 取代原宣告的來源。`rsmt review discard <id>` SHALL 先存 snapshot 再部署 repo 版本，只適用於 dotfile 項目。`rsmt review ignore <id>` SHALL 保留本機內容且不收進 repo，兩邊都沒再改變前，套用時略過該路徑且不再列出。

#### Scenario: Discard local changes
- **WHEN** `~/.gitconfig` 因衝突列在待處理清單第 4 項，使用者執行 `rsmt review discard 4`
- **THEN** `~/.gitconfig` 換成 repo 版本，原內容保存在 snapshot 中，第 4 項從清單移除

#### Scenario: Discard an adopt item
- **WHEN** 使用者對 rc 檔段落的項目執行 `rsmt review discard`
- **THEN** 指令說明 discard 只適用於 dotfile 項目並以 2 結束
