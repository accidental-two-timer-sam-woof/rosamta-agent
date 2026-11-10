# Spec Delta

## ADDED Requirements

### Requirement: Re-render on variable changes
`host.toml` 改變時，背景服務 SHALL 重新部署 dotfile，讓 `[vars]` 的修改反映到 template；只有輸入改變的 template 會重新 render。

#### Scenario: Change an email
- **WHEN** 使用者把 `host.toml` 的 `vars.email` 改成新的值
- **THEN** 引用 `vars.email` 的 template 重新 render，其他 template 不動
