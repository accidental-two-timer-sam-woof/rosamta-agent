# Proposal

## Why

使用者最大的問題是「忘記自己在用 dotfile 管理工具」。只要接管、套用、推送、拉取還需要手動執行，repo 就會再次落後。rosamta 需要一個常駐背景的服務：rc 檔一變動就依偏好接管或詢問，repo 一變動就套用，並定時與遠端同步。

## What Changes

- 背景服務：macOS 為主程式內簽章的 helper app，以 SMAppService 註冊為登入項目（ADR 0004）；Linux 為 systemd 使用者服務。兩者都執行 `rsmt daemon run`。
- 啟動時從使用者的 login shell 讀取環境變數，以取得 `SSH_AUTH_SOCK`、完整的 PATH 等設定。
- 監看 watched file：有變動時掃描，並依接管偏好自動接管、加入待處理清單或忽略。
- 監看 repo：使用者手動編輯或拉取後自動驗證與套用；依推送偏好推送。
- 依拉取偏好與間隔定時同步。
- 系統通知：詢問模式下通知有新的可接管內容，提供「接管」、「忽略」、「查看差異」三個動作；推送受阻、衝突、套用失敗、需要升級時也發出通知。沒有通知服務的 host 只使用待處理清單與開 shell 提醒。
- CLI：`rsmt daemon run`、`enable`、`disable`、`status`。
- 實作前先做 spike，驗證 helper app 發出的通知能顯示按鈕並收到點擊。

## Capabilities

### New Capabilities

- `daemon`: 背景服務的生命週期、環境、監看與觸發規則。
- `notifications`: 系統通知的時機、內容、動作與無法通知時的退路。

### Modified Capabilities

（無）

## Impact

- `rosamta` 新增 `daemon` 子指令；macOS 的 `.app` 內新增 helper app 與 LaunchAgent plist；Linux 新增 systemd unit。
- 新依賴：`notify` 與 `notify-debouncer-full`（檔案監看）、macOS 通知 API 的 binding、Linux 的 D-Bus 通知。
- 依賴 `add-adopt`、`add-git-sync`、`add-shell-render` 提供的操作；本 change 只負責何時觸發它們。
