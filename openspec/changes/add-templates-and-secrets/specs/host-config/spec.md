# Spec Delta

## ADDED Requirements

### Requirement: Template variables and secret source order
`host.toml` SHALL 可包含 `[vars]`（template 使用的本機變數）與 `[secrets] bitwarden`（Bitwarden 工具的嘗試順序，值為 `rbw`、`bw`、`bws` 組成的陣列）。這些設定 SHALL 只存在本機，不進 repo。`rsmt schema host` SHALL 包含這兩個區段。

#### Scenario: Unknown Bitwarden tool
- **WHEN** `host.toml` 設定 `[secrets] bitwarden = ["lastpass"]`
- **THEN** rosamta 讀取 `host.toml` 時回報錯誤與行號
