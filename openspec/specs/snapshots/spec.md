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
`rsmt restore --original` SHALL 把每個被 rosamta 修改過的 rc 檔還原成第一次修改前的內容，並停用 rosamta 的背景服務，以免它再掛載 shell。執行前 SHALL 顯示差異並要求確認，`--yes` 可略過確認。

#### Scenario: Restore original zshrc
- **WHEN** 使用者執行 `rsmt restore --original` 並確認
- **THEN** `~/.zshrc` 的內容與安裝 rosamta 之前相同

### Requirement: Restore one adoption
`rsmt restore <adoption-id>` SHALL 把該次接管移除的內容放回 rc 檔原本的位置，並刪除對應的 module；兩者的變更 SHALL 各自記錄（rc 檔存 snapshot，repo 建立 commit）。執行前 SHALL 顯示差異並要求確認。

#### Scenario: Undo an adoption
- **WHEN** 使用者還原 OrbStack 那次接管並確認
- **THEN** OrbStack 的三行回到 `~/.zprofile` 原本的位置，`modules/adopted/orbstack.toml` 被刪除，repo 多一個 commit 記錄這次還原

### Requirement: Dotfiles in snapshots
rosamta SHALL 在覆蓋或取代任何 dotfile 目標之前保存 snapshot；目標是目錄時保存其中的每個檔案。`rsmt restore --original` SHALL 也把 dotfile 目標還原成第一次修改前的內容，當時不存在的目標 SHALL 被刪除。

#### Scenario: Restore an adopted directory
- **WHEN** `~/.config/ghostty` 原本是一般目錄，部署後成為 symlink，使用者執行 `rsmt restore --original` 並確認
- **THEN** `~/.config/ghostty` 恢復成原本內容的一般目錄
