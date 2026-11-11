# Spec Delta

## ADDED Requirements

### Requirement: Watch deployed dotfiles
背景服務 SHALL 監看部署出去的 dotfile：`link` 的目標是否被換掉，`copy` 的目標內容是否改變，並在數秒內依 dotfile 的規則處理。背景服務自己寫入目標 SHALL 不再觸發一次處理。

#### Scenario: App rewrites a copied file
- **WHEN** 接管偏好為「自動」，某個 app 修改了 `copy` 部署的設定檔，repo 的來源沒有改變
- **THEN** 數秒內 repo 的來源更新為新內容
