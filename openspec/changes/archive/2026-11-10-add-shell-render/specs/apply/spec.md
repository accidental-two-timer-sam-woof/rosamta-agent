# Spec Delta

## Purpose

定義把 repo 的設定套用到 host 的流程，確保壞掉的設定不會進入使用者的 shell，並讓使用者能完整退出 rosamta。

## ADDED Requirements

### Requirement: Preview before applying
`rsmt apply --dry-run` SHALL 顯示 generated script 與 rc 檔 hook 區塊將會出現的差異，且不寫入任何檔案。

#### Scenario: Dry run
- **WHEN** 使用者新增一個 module 後執行 `rsmt apply --dry-run`
- **THEN** 輸出顯示新增的腳本內容，generated 目錄與 rc 檔都沒有變動

### Requirement: Smoke test and rollback
`rsmt apply` SHALL 在切換之前，以新的 generated script 啟動每個已安裝的互動 shell；若 stderr 出現套用前沒有的新錯誤，SHALL 放棄這次套用、保留原本的版本，並指出造成錯誤的 module。

#### Scenario: Module breaks zsh startup
- **WHEN** 新的片段 module 讓 zsh 啟動時輸出錯誤
- **THEN** `rsmt apply` 以 1 結束並列出該 module，新開的 zsh 仍使用套用前的設定

### Requirement: Atomic switch
套用 SHALL 一次切換整組 generated script；在套用期間啟動的 shell SHALL 只會載入完整的舊版本或完整的新版本。

#### Scenario: Shell starts during apply
- **WHEN** 使用者在 `rsmt apply` 執行到一半時開啟新的 shell
- **THEN** 該 shell 載入的腳本全部屬於同一個版本

### Requirement: Eject
`rsmt eject` SHALL 把目前生效的設定直接寫入各 rc 檔、移除所有 hook，並停用 rosamta 的背景服務；repo 與 snapshot SHALL 保留。執行前 SHALL 顯示差異並要求確認，`--yes` 可略過確認。

#### Scenario: Uninstall after eject
- **WHEN** 使用者執行 `rsmt eject` 後移除 rosamta 程式
- **THEN** 新開的 shell 中 PATH、環境變數與 alias 與 eject 之前相同
