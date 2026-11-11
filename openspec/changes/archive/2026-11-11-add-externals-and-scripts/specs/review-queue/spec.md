# Spec Delta

## ADDED Requirements

### Requirement: Script items
待處理清單 SHALL 列出等待執行的腳本，包含 `source`、`run` 種類，以及從未執行、內容改變或上次執行失敗。`accept` SHALL 執行該腳本（在終端機中時接終端機），成功後移除項目，失敗時項目保留並顯示紀錄；`ignore` SHALL 把腳本記錄為已執行但不執行（`onchange` 到下次改變為止）；`discard` SHALL 被拒絕並以 2 結束。宣告移除、不再符合本機的 `when`，或腳本已在其他套用中執行後，項目 SHALL 自動移除；項目等待時腳本再改變，SHALL 沿用同一個 ID。`accept` 時腳本與列出時不同，SHALL 不執行並以 2 結束，項目更新為新的內容。

#### Scenario: Run from the queue
- **WHEN** 使用者對等待中的腳本項目執行 `rsmt review accept <id>`
- **THEN** 腳本在終端機中執行，成功後項目從清單移除

#### Scenario: Already installed by hand
- **WHEN** 使用者對 `once` 的安裝腳本執行 `rsmt review ignore <id>`
- **THEN** 腳本不執行，之後的套用也不再執行它
