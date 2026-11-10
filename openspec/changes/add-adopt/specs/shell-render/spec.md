# Spec Delta

## MODIFIED Requirements

### Requirement: Phase order
generated script SHALL 依載入階段 env、early、framework、tools、prompt、aliases、late 的順序載入。同一階段內 SHALL 依 module 的 `order`（預設 50）由小到大，再依名稱排序。另有只供 zsh 片段使用的 login 階段：它 SHALL 只在 login zsh 中、系統的 zprofile 之後由 `.zprofile` 的 hook 載入，且 SHALL 在 module 宣告的 PATH 重新排序之前載入；module 宣告 login 階段卻含有 zsh 片段以外的內容時，SHALL 視為設定錯誤。

#### Scenario: Same phase
- **WHEN** tools 階段有 `zoxide`（未設 order）與 `fzf`（`order = 10`）
- **THEN** `fzf` 先於 `zoxide` 載入

#### Scenario: Login phase
- **WHEN** module 宣告 `phase = "login"` 且只有 `raw.zsh`
- **THEN** 該片段只在 login zsh 中於系統 zprofile 之後載入，非 login 的 zsh 不載入

#### Scenario: Login phase with other content
- **WHEN** module 宣告 `phase = "login"` 且含有環境變數
- **THEN** `rsmt validate` 回報錯誤
