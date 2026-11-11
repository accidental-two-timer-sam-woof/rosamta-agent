# Spec Delta

## ADDED Requirements

### Requirement: Waiting items
template 因秘密來源鎖住、需要執行秘密指令或缺少變數而無法 render 時，待處理清單 SHALL 列出該路徑與在等待的對象。這類項目 SHALL NOT 能 accept、ignore 或 discard；嘗試時 SHALL 說明要在終端機執行 `rsmt apply` 並以 2 結束。該 template 成功 render 後，項目 SHALL 自動移除。`--json` SHALL 輸出等待的類型（`locked`、`command` 或 `unset`）與對象。

#### Scenario: Locked source
- **WHEN** rbw 鎖住，`~/.config/gh/hosts.yml` 的 template 需要 Bitwarden 的值
- **THEN** `rsmt review` 列出該路徑在等待 rbw 解鎖

#### Scenario: Resolved by apply
- **WHEN** 使用者在終端機執行 `rsmt apply` 並解鎖 rbw
- **THEN** template 部署完成，該項目從清單移除
