# Spec Delta

## ADDED Requirements

### Requirement: Waiting notifications
新的等待項目出現時，背景服務 SHALL 通知一次，內容指出路徑與在等待的對象，只提供開啟待處理清單的動作。同一個項目在背景服務重新啟動後 SHALL NOT 再通知。

#### Scenario: Locked after pull
- **WHEN** 拉取後有兩個 template 都在等待 rbw 解鎖
- **THEN** 背景服務各通知一次，之後不再重複通知
