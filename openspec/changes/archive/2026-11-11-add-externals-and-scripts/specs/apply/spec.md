# Spec Delta

## ADDED Requirements

### Requirement: Apply order with externals and scripts
套用 SHALL 依序處理 external、`before` 腳本、generated script 的檢查與切換、dotfile、`after` 腳本；shell 的檢查失敗而中止時，dotfile 與 `after` 腳本 SHALL 不處理。external 與腳本的失敗 SHALL 只影響自己，拉取仍算成功。`--dry-run` SHALL 列出將下載或更新的 external 與將執行或放進待處理清單的腳本，不下載也不執行；`--json` SHALL 包含每個 external 與腳本的結果。

#### Scenario: Dry run on a new host
- **WHEN** 新 host 執行 `rsmt apply --dry-run`，repo 有一個 external 與一個 `once` 腳本
- **THEN** 輸出列出將下載的 external 與將執行的腳本，家目錄沒有改變，腳本沒有執行

#### Scenario: oh-my-zsh on a new host
- **WHEN** 新 host 的 module 載入 `~/.oh-my-zsh`，該目錄由 external 宣告
- **THEN** `rsmt apply` 先下載 external，shell 的檢查通過
