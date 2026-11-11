# Spec Delta

## ADDED Requirements

### Requirement: Apply deploys dotfiles
`rsmt apply` SHALL 在 generated script 切換成功後部署 dotfile；shell 的檢查失敗而中止時 SHALL 不部署 dotfile。`--dry-run` SHALL 列出每個 dotfile 的動作與 copy 的 diff，`--json` SHALL 包含每個 dotfile 的路徑與結果。

#### Scenario: Dry run
- **WHEN** 使用者執行 `rsmt apply --dry-run`，repo 新增了一筆 dotfile 宣告
- **THEN** 輸出列出將建立的 symlink，家目錄沒有任何改變

### Requirement: Eject keeps dotfiles
`rsmt eject` SHALL 把每個 `link` 的目標換成內容相同的一般檔案或目錄，並清除部署紀錄。

#### Scenario: Uninstall keeps nvim config
- **WHEN** 使用者執行 `rsmt eject` 後移除 rosamta 與 repo
- **THEN** `~/.config/nvim` 仍是內容相同的一般目錄
