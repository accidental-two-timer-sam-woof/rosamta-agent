## MODIFIED Requirements

### Requirement: Offline
推送因網路或遠端無法連線而失敗時，rosamta SHALL 保留本機的 commit，並在下一次推送或同步時重試；推送偏好為「自動」時，背景服務 SHALL 每分鐘重試推送，直到推送成功或失敗的原因不再是無法連線。狀態 SHALL 顯示尚未推送的 commit 數量。

#### Scenario: No network
- **WHEN** 沒有網路時使用者接受一個接管項目，推送偏好為「自動」
- **THEN** commit 留在本機，狀態顯示有 1 個未推送的 commit，恢復連線後一分鐘內背景服務把它推送出去

#### Scenario: Connection that stalls
- **WHEN** 背景服務推送時連線建立後不再有任何回應
- **THEN** 這次推送在 10 分鐘後被中止，狀態顯示連不上遠端，之後照常重試

#### Scenario: Offline for an hour
- **WHEN** 有 commit 等待推送，而電腦一小時都沒有網路
- **THEN** 背景服務前幾次每分鐘重試一次，之後間隔逐次拉長，最長 15 分鐘重試一次
