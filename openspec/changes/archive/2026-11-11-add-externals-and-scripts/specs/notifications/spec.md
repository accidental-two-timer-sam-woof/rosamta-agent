# Spec Delta

## ADDED Requirements

### Requirement: Script notifications
新的腳本項目出現時，背景服務 SHALL 通知一次：只有一個時指出腳本，同一批有多個時指出數量。`ask` 時通知內容為等待執行，`auto` 時為執行失敗。通知 SHALL 只提供開啟待處理清單的「查看」動作；背景服務重新啟動後同一個項目 SHALL NOT 再通知。

#### Scenario: Two new scripts
- **WHEN** `scripts = "ask"`，拉取後有兩個腳本需要執行
- **THEN** 背景服務以一則通知指出有 2 個腳本等待執行
