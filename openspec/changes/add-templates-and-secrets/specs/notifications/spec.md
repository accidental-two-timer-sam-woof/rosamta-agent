# Spec Delta

## ADDED Requirements

### Requirement: Waiting notifications
新的等待項目出現時，背景服務 SHALL 通知一次：只有一個項目時內容指出路徑與在等待的對象，同一批有多個時指出數量；通知 SHALL 只提供開啟待處理清單的「查看」動作，點通知本身也一樣。同一批也有需要決定的項目時，通知 SHALL 只關於那些項目，動作不套用到等待項目。同一個項目在背景服務重新啟動後 SHALL NOT 再通知；只有在等待的對象改變（例如工具鎖住又解鎖）時，項目不算新的。

#### Scenario: Locked after pull
- **WHEN** 拉取後有兩個 template 都在等待 rbw 解鎖
- **THEN** 背景服務以一則通知指出有 2 個設定檔等待中，之後不再重複通知
