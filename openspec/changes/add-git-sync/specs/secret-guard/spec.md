# Spec Delta

## ADDED Requirements

### Requirement: Scan before pushing
每次推送前，rosamta SHALL 以相同的秘密規則掃描所有即將推送的 commit 內容；發現疑似秘密時 SHALL 停止推送、不修改任何 commit，並回報檔案、行號與命中的規則，推送狀態標記為受阻。

#### Scenario: Secret in a hand-edited module
- **WHEN** 使用者手動在 `modules/env.toml` 寫入一個 API key，並執行 `rsmt publish`
- **THEN** 遠端沒有收到任何 commit，指令以 1 結束並指出 `modules/env.toml` 的行號
