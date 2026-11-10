# snapshots Specification

## Purpose
在 rosamta 修改使用者的 rc 檔之前保存原文，讓任何修改都能還原，且原文不會離開這台 host。

## Requirements

### Requirement: Snapshot before every change
rosamta 修改任何 rc 檔之前 SHALL 先把該檔的完整內容存成 snapshot，並記錄時間與原因。

#### Scenario: Hook install
- **WHEN** rosamta 第一次在 `~/.zshrc` 安裝 hook
- **THEN** snapshot 中有安裝前的 `~/.zshrc` 原文，原因標示為安裝 hook

### Requirement: Snapshots stay on the host
snapshot SHALL 存在本機的狀態目錄，不在 repo 內，也 SHALL 不被推送到任何遠端。

#### Scenario: After publishing
- **WHEN** 使用者推送 repo 到 GitHub
- **THEN** 遠端 repo 中沒有任何 snapshot 的內容

### Requirement: Restore the original files
`rsmt restore --original` SHALL 把每個被 rosamta 修改過的 rc 檔還原成第一次修改前的內容。執行前 SHALL 顯示差異並要求確認，`--yes` 可略過確認。

#### Scenario: Restore original zshrc
- **WHEN** 使用者執行 `rsmt restore --original` 並確認
- **THEN** `~/.zshrc` 的內容與安裝 rosamta 之前相同
