# Spec Delta

## ADDED Requirements

### Requirement: Re-render on variable changes
`host.toml` 改變時，背景服務 SHALL 重新部署 dotfile，讓 `[vars]` 的修改反映到 template；只有輸入改變的 template 會重新 render。

#### Scenario: Change an email
- **WHEN** 使用者把 `host.toml` 的 `vars.email` 改成新的值
- **THEN** 引用 `vars.email` 的 template 重新 render，其他 template 不動

### Requirement: Retry after an rbw unlock
有 template 在等待 rbw 解鎖時，背景服務 SHALL 每分鐘重新部署一次，rbw 在其他地方解鎖後 SHALL 自動 render 並移除等待項目。bw 的解鎖只存在於執行解鎖的 shell，秘密指令只在終端機執行，因此這兩種等待項目 SHALL NOT 重試。

#### Scenario: Unlock in another terminal
- **WHEN** `~/.npmrc` 在等待 rbw 解鎖，使用者在另一個終端機執行 `rbw unlock`
- **THEN** 一分鐘內 `~/.npmrc` 依 template 寫入，待處理清單不再有該項目
