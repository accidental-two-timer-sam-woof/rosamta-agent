# apply Specification

## Purpose
定義把 repo 的設定套用到 host 的流程，確保壞掉的設定不會進入使用者的 shell，並讓使用者能完整退出 rosamta。

## Requirements

### Requirement: Preview before applying
`rsmt apply --dry-run` SHALL 顯示 generated script 與 rc 檔 hook 區塊將會出現的差異，且不寫入任何檔案。

#### Scenario: Dry run
- **WHEN** 使用者新增一個 module 後執行 `rsmt apply --dry-run`
- **THEN** 輸出顯示新增的腳本內容，generated 目錄與 rc 檔都沒有變動

### Requirement: Smoke test and rollback
`rsmt apply` SHALL 在切換之前，以新的 generated script 啟動每個已安裝的互動 shell，bash 與 zsh 另以互動 login shell 啟動一次；若 stderr 出現套用前沒有的新錯誤，或套用前能在時限內啟動的 shell 逾時，SHALL 放棄這次套用、保留原本的版本，並指出造成錯誤的 module。套用前就已逾時的 shell 再次逾時 SHALL 不算新錯誤。

#### Scenario: Module breaks zsh startup
- **WHEN** 新的片段 module 讓 zsh 啟動時輸出錯誤
- **THEN** `rsmt apply` 以 1 結束並列出該 module，新開的 zsh 仍使用套用前的設定

#### Scenario: Login snippet breaks login zsh
- **WHEN** 新的 login 階段片段讓 login zsh 啟動時輸出錯誤
- **THEN** `rsmt apply` 以 1 結束並列出該 module

#### Scenario: Shell already slow
- **WHEN** 使用者的 `.zshrc` 在套用前就讓 zsh 無法在時限內啟動
- **THEN** `rsmt apply` 不把這個逾時當成新錯誤，照常套用

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

#### Scenario: Snippet that returns early
- **WHEN** 片段在工具不存在時執行 `return`，使用者執行 `rsmt eject`
- **THEN** rc 檔中其後的設定照常載入，與 eject 之前相同

### Requirement: Apply deploys dotfiles
`rsmt apply` SHALL 在 generated script 切換成功後部署 dotfile；shell 的檢查失敗而中止時 SHALL 不部署 dotfile。`--dry-run` SHALL 列出每個 dotfile 的動作與 copy 的 diff，`--json` SHALL 包含每個 dotfile 的路徑與結果。

#### Scenario: Dry run
- **WHEN** 使用者執行 `rsmt apply --dry-run`，repo 新增了一筆 dotfile 宣告
- **THEN** 輸出列出將建立的 symlink，家目錄沒有任何改變

### Requirement: Eject keeps dotfiles
`rsmt eject` SHALL 把每個 `link` 的目標換成內容相同的一般檔案或目錄，並清除部署紀錄。

#### Scenario: Uninstall keeps nvim config
- **WHEN** 使用者執行 `rsmt eject` 後移除 rosamta 與 repo
- **THEN** `~/.config/nvim` 仍是內容相同的一般目錄
