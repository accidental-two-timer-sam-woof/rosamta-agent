# Spec Delta

## ADDED Requirements

### Requirement: Externals and scripts in the background
背景服務套用時（包括拉取之後）SHALL 下載與更新 external，與 `rsmt apply` 相同；腳本 SHALL 依 `host.toml` 的 `scripts` 偏好處理，`ask` 時 SHALL NOT 執行任何腳本。背景服務執行的腳本 SHALL 沒有終端機，stdin 為 `/dev/null`。

#### Scenario: Auto on a server
- **WHEN** `scripts = "auto"`，背景服務拉取到修改過的 `onchange` 腳本
- **THEN** 背景服務執行該腳本，輸出寫進紀錄檔
