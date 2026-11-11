# Spec Delta

## ADDED Requirements

### Requirement: Create deploy
`deploy = "create"` 的 dotfile SHALL 只在目標不存在時，把來源（`encrypted` 的先解密）寫到目標並設定 `permissions`；目標存在時 SHALL 不讀取、不修改、不監看，也不進待處理清單。`create` SHALL 不能搭配 template。`rsmt forget` SHALL 不動 `create` 的目標。

#### Scenario: Starter config
- **WHEN** `.config/foo/config.toml` 以 `create` 宣告，host 上還沒有這個檔案
- **THEN** 套用後檔案內容與 repo 相同；之後 app 修改它，套用與背景服務都不再理會

#### Scenario: Deleted by the user
- **WHEN** 使用者刪除了以 `create` 部署的檔案後執行 `rsmt apply`
- **THEN** 檔案以 repo 的內容重新建立
