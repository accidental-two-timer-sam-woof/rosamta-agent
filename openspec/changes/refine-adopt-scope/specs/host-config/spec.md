## MODIFIED Requirements

### Requirement: Host settings stay local
`host.toml` SHALL 只存在本機、不進 repo。它記錄識別名稱、標籤、接管偏好、rc 檔原內容的處理方式、推送偏好、拉取偏好與間隔、開 shell 提醒。拉取間隔以分鐘為單位，SHALL 至少為 1。

#### Scenario: Defaults
- **WHEN** rosamta 第一次建立 `host.toml`
- **THEN** 接管偏好為「自動」、rc 檔原內容為「刪除」、推送偏好為「自動」、拉取偏好為「自動」且間隔 15 分鐘、開 shell 提醒為開啟

#### Scenario: Existing setting kept
- **WHEN** 既有的 `host.toml` 寫著接管偏好為「詢問」
- **THEN** 升級 rosamta 後接管偏好仍為「詢問」

#### Scenario: Zero pull interval
- **WHEN** `host.toml` 的拉取間隔為 0
- **THEN** rosamta 拒絕讀取 `host.toml`，並指出該行與「至少 1 分鐘」
