## MODIFIED Requirements

### Requirement: Restore one adoption
`rsmt restore <adoption-id>` SHALL 把該次接管移除的內容放回 rc 檔原本的位置，並讓對應的 module 不再於本機載入：本機另一筆接管仍使用該 module 時保留不動；這次接管是以所有主機範圍加入其他主機的 module 時，放回它原本的主機與 OS 限制；`hosts` 還列著其他主機時，只移除本機；其餘情況刪除該 module。兩者的變更 SHALL 各自記錄（rc 檔存 snapshot，repo 建立 commit）。執行前 SHALL 顯示差異並要求確認。

#### Scenario: Undo an adoption
- **WHEN** 使用者還原 OrbStack 那次接管並確認
- **THEN** OrbStack 的三行回到 `~/.zprofile` 原本的位置，`modules/adopted/orbstack.toml` 被刪除，repo 多一個 commit 記錄這次還原

#### Scenario: Another host joined
- **WHEN** `mbp` 接管產生的 `bun.toml` 已列出 `hosts = ["mbp", "box"]`，使用者在 `mbp` 上還原這次接管
- **THEN** 原行回到 `mbp` 的 rc 檔，`bun.toml` 保留並宣告 `hosts = ["box"]`
