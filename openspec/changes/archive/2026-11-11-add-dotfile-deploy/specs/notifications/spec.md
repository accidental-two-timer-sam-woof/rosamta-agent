# Spec Delta

## ADDED Requirements

### Requirement: Dotfile notifications
新的 dotfile 待處理項目 SHALL 與其他項目一樣合併通知，「接管」等同以預設範圍保留本機版本。需要輸入密碼才能部署加密的 dotfile 時，背景服務 SHALL 發出一則問題通知，直到本機有私鑰前不重複通知。

#### Scenario: Conflict found in the background
- **WHEN** 背景服務拉取後發現 `~/.gitconfig` 與 repo 版本衝突
- **THEN** 使用者收到一則指出該檔案與 repo 不同的通知，可以直接選擇接管、忽略或查看差異
