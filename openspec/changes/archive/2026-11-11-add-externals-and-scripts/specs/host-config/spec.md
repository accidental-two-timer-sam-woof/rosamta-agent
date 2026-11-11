# Spec Delta

## ADDED Requirements

### Requirement: Script preference
`host.toml` SHALL 可包含 `scripts`，值為 `ask`（預設）或 `auto`，決定不在終端機中的套用如何處理需要執行的腳本。這個設定 SHALL 只存在本機；`rsmt schema host` SHALL 包含它，App 的設定頁 SHALL 可以切換。

#### Scenario: Unknown value
- **WHEN** `host.toml` 設定 `scripts = "always"`
- **THEN** rosamta 讀取 `host.toml` 時回報錯誤與行號
