## MODIFIED Requirements

### Requirement: Periodic sync
拉取偏好為「自動」時，背景服務 SHALL 依設定的間隔執行同步；電腦從睡眠喚醒後 SHALL 立即同步一次，連不上遠端時 SHALL 在約 30 秒與 2 分鐘後各重試一次，之後回到設定的間隔。推送偏好為「手動」時，這些同步 SHALL 只拉取，不推送，也不 commit 使用者手動的修改。推送偏好為「自動」、有尚未推送的 commit，且上一次推送因無法連線而失敗時，背景服務 SHALL 每分鐘重試推送（拉取偏好為「手動」時只推送），推送成功後回到設定的間隔。

#### Scenario: Interval
- **WHEN** 拉取間隔為 15 分鐘
- **THEN** 背景服務大約每 15 分鐘執行一次同步

#### Scenario: Network not back after waking
- **WHEN** 電腦從睡眠喚醒時網路還沒恢復
- **THEN** 背景服務在約 30 秒與 2 分鐘後再同步，不必等到下一個間隔

#### Scenario: Unpushed while offline
- **WHEN** 有 2 個 commit 因沒有網路而沒有推送，10 分鐘後網路恢復
- **THEN** 背景服務在網路恢復後一分鐘內推送這 2 個 commit，之後回到設定的間隔
