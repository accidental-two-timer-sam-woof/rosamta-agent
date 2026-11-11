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
短時間內出現多個待處理項目時，背景服務 SHALL 合併成一則通知，而不是逐項通知。通知服務沒有接下這則通知時，背景服務 SHALL 稍後再試，且這些項目在顯示前不算已通知，重新啟動後也會再通知。

#### Scenario: Installer writes three files
- **WHEN** 某個安裝程式在 10 秒內修改了三個 rc 檔
- **THEN** 使用者只收到一則通知，說明有三個待處理項目

#### Scenario: Review items found before the notification service starts
- **WHEN** 背景服務在桌面的通知服務啟動前發現待處理項目
- **THEN** 通知服務啟動後，使用者會收到這則通知

### Requirement: Problem notifications
推送或拉取受阻（簽章失敗、偵測到秘密或遠端拒絕存取）、推送或拉取失敗、拉取衝突、拉取後套用失敗，以及遠端 repo 需要較新版本的 rosamta 時，背景服務 SHALL 各發出一則通知，並說明是推送還是拉取出了問題；同一個問題在解決之前 SHALL 不重複通知。通知沒能交給系統的通知服務時（例如通知服務比背景服務晚啟動），該問題 SHALL 不算已通知，之後 SHALL 再試。

#### Scenario: Signing blocked
- **WHEN** 推送因 GPG 簽章失敗而受阻
- **THEN** 使用者收到一則說明推送受阻與原因的通知，之後的同步不再重複通知同一個原因

#### Scenario: Notification service starts later
- **WHEN** 背景服務啟動時發現離線，但桌面的通知服務還沒啟動
- **THEN** 通知服務啟動後，使用者仍會收到這則通知

#### Scenario: Access refused on pull
- **WHEN** 拉取時遠端拒絕存取
- **THEN** 通知說明拉取受阻與原因，而不是推送受阻

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

### Requirement: Dotfile notifications
新的 dotfile 待處理項目 SHALL 與其他項目一樣合併通知，「接管」等同以預設範圍保留本機版本。需要輸入密碼才能部署加密的 dotfile 時，背景服務 SHALL 發出一則問題通知，直到本機有私鑰前不重複通知。

#### Scenario: Conflict found in the background
- **WHEN** 背景服務拉取後發現 `~/.gitconfig` 與 repo 版本衝突
- **THEN** 使用者收到一則指出該檔案與 repo 不同的通知，可以直接選擇接管、忽略或查看差異

### Requirement: Waiting notifications
新的等待項目出現時，背景服務 SHALL 通知一次：只有一個項目時內容指出路徑與在等待的對象，同一批有多個時指出數量；通知 SHALL 只提供開啟待處理清單的「查看」動作，點通知本身也一樣。同一批也有需要決定的項目時，通知 SHALL 只關於那些項目，動作不套用到等待項目。同一個項目在背景服務重新啟動後 SHALL NOT 再通知；只有在等待的對象改變（例如工具鎖住又解鎖）時，項目不算新的。

#### Scenario: Locked after pull
- **WHEN** 拉取後有兩個 template 都在等待 rbw 解鎖
- **THEN** 背景服務以一則通知指出有 2 個設定檔等待中，之後不再重複通知

### Requirement: Script notifications
新的腳本項目出現時，背景服務 SHALL 通知一次：只有一個時指出腳本，同一批有多個時指出數量。`ask` 時通知內容為等待執行，`auto` 時為執行失敗。通知 SHALL 只提供開啟待處理清單的「查看」動作；背景服務重新啟動後同一個項目 SHALL NOT 再通知。

#### Scenario: Two new scripts
- **WHEN** `scripts = "ask"`，拉取後有兩個腳本需要執行
- **THEN** 背景服務以一則通知指出有 2 個腳本等待執行
