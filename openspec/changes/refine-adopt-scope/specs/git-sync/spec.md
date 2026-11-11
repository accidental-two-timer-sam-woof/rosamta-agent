## MODIFIED Requirements

### Requirement: Offline
推送因網路或遠端無法連線而失敗時，rosamta SHALL 保留本機的 commit，並在下一次推送或同步時重試；推送偏好為「自動」時，背景服務 SHALL 每分鐘重試推送，直到推送成功或失敗的原因不再是無法連線。狀態 SHALL 顯示尚未推送的 commit 數量。

#### Scenario: No network
- **WHEN** 沒有網路時使用者接受一個接管項目，推送偏好為「自動」
- **THEN** commit 留在本機，狀態顯示有 1 個未推送的 commit，恢復連線後一分鐘內背景服務把它推送出去
