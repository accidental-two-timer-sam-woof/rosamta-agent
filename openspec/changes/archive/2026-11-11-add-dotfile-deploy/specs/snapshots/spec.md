# Spec Delta

## ADDED Requirements

### Requirement: Dotfiles in snapshots
rosamta SHALL 在覆蓋或取代任何 dotfile 目標之前保存 snapshot；目標是目錄時保存其中的每個檔案。`rsmt restore --original` SHALL 也把 dotfile 目標還原成第一次修改前的內容，當時不存在的目標 SHALL 被刪除。

#### Scenario: Restore an adopted directory
- **WHEN** `~/.config/ghostty` 原本是一般目錄，部署後成為 symlink，使用者執行 `rsmt restore --original` 並確認
- **THEN** `~/.config/ghostty` 恢復成原本內容的一般目錄
