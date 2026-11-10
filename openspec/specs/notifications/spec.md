# notifications Specification

## Purpose
在需要使用者決定或注意的時候主動發出系統通知，讓使用者不必開啟 rosamta 也能處理接管，並及早知道同步出了問題。

## Requirements

### Requirement: Ask before adopting
接管偏好為「詢問」且出現新的待處理項目時，背景服務 SHALL 發出系統通知，說明來源檔與新增的行數，並提供「接管」、「忽略」、「查看差異」三個動作。

#### Scenario: Accept from the notification
- **WHEN** 使用者在通知上按「接管」
- **THEN** 該項目被接管，效果與 `rsmt review accept` 相同

#### Scenario: Show diff
- **WHEN** 使用者在通知上按「查看差異」
- **THEN** rosamta 的 GUI 開啟並顯示該項目

### Requirement: Batch notifications
短時間內出現多個待處理項目時，背景服務 SHALL 合併成一則通知，而不是逐項通知。

#### Scenario: Installer writes three files
- **WHEN** 某個安裝程式在 10 秒內修改了三個 rc 檔
- **THEN** 使用者只收到一則通知，說明有三個待處理項目

### Requirement: Problem notifications
推送受阻（簽章失敗或偵測到秘密）、拉取衝突、拉取後套用失敗，以及遠端 repo 需要較新版本的 rosamta 時，背景服務 SHALL 各發出一則通知；同一個問題在解決之前 SHALL 不重複通知。

#### Scenario: Signing blocked
- **WHEN** 推送因 GPG 簽章失敗而受阻
- **THEN** 使用者收到一則說明推送受阻與原因的通知，之後的同步不再重複通知同一個原因

### Requirement: Fallback without notifications
系統沒有通知服務、或使用者拒絕通知權限時，背景服務 SHALL 不發出通知，只依靠待處理清單與開 shell 時的提醒。

#### Scenario: SSH-only server
- **WHEN** 背景服務在沒有桌面環境的 Linux 伺服器上偵測到新的 rc 內容
- **THEN** 項目出現在待處理清單中，下一次開互動 shell 時顯示提醒

### Requirement: Notification language
通知文字 SHALL 跟隨系統語言，支援英文與繁體中文。

#### Scenario: zh-TW system
- **WHEN** 系統語言為繁體中文
- **THEN** 通知的標題、內容與動作按鈕都是繁體中文
