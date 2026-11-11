# Spec Delta

## ADDED Requirements

### Requirement: Template deploy
`deploy = "template"` 的 dotfile SHALL render 後以一般檔案部署到目標，並依部署紀錄判斷 template 的輸入與目標是否改變；`encrypted = true` 的 template SHALL 先解密再 render。template SHALL 只能是檔案。render 失敗時 SHALL 只略過該路徑並回報，其他 dotfile 照常部署。

#### Scenario: Encrypted template
- **WHEN** 加密的 template `.config/gh/hosts.yml` 引用 `host.id`，本機已解開金鑰
- **THEN** 目標是解密並 render 後的內容，repo 中只有密文

#### Scenario: Template directory
- **WHEN** `deploy = "template"` 的宣告指向一個目錄
- **THEN** `rsmt validate` 回報只有檔案能使用 template

## REMOVED Requirements

### Requirement: Template deploy is declared only
**Reason**: template 部署已實作，由 Template deploy 取代。
**Migration**: 不需要；原本略過的 template 宣告在下一次套用時部署。
