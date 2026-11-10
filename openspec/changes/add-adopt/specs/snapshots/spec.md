# Spec Delta

## ADDED Requirements

### Requirement: Restore one adoption
`rsmt restore <adoption-id>` SHALL 把該次接管移除的內容放回 rc 檔原本的位置，並刪除對應的 module；兩者的變更 SHALL 各自記錄（rc 檔存 snapshot，repo 建立 commit）。執行前 SHALL 顯示差異並要求確認。

#### Scenario: Undo an adoption
- **WHEN** 使用者還原 OrbStack 那次接管並確認
- **THEN** OrbStack 的三行回到 `~/.zprofile` 原本的位置，`modules/adopted/orbstack.toml` 被刪除，repo 多一個 commit 記錄這次還原
