## ADDED Requirements

### Requirement: Offline notification
推送偏好為「自動」、推送因無法連線而失敗且有尚未推送的 commit 時，背景服務 SHALL 發出一則通知「沒有網路，{count} 個變更等待推送」；推送成功之前 SHALL 不再通知，即使等待的數量增加。只有拉取無法連線、沒有等待推送的 commit 時 SHALL 不通知。

#### Scenario: First offline push
- **WHEN** 沒有網路時自動接管了一段內容
- **THEN** 使用者收到「沒有網路，1 個變更等待推送」

#### Scenario: More changes while offline
- **WHEN** 仍然沒有網路時又接管了一段內容
- **THEN** 不再發出通知；網路恢復並推送成功後，下一次離線會再通知

#### Scenario: Pull only
- **WHEN** 沒有尚未推送的 commit，定期拉取因沒有網路而失敗
- **THEN** 不發出通知
