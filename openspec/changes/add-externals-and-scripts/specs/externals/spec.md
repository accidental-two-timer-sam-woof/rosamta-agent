# Spec Delta

## Purpose

把 repo 中宣告的外部內容（git repo 或壓縮檔）下載到各 host 的家目錄，所有 host 使用同一個鎖定的版本，且不覆蓋使用者在 host 上修改過的內容。

## ADDED Requirements

### Requirement: External declarations
每個 external SHALL 在 repo 的 `externals.toml` 中以一筆 `[[external]]` 宣告，包含 `path`（相對於家目錄）、`git`（git repo 的網址）或 `archive`（壓縮檔的網址）其中之一、鎖定值（`git` 為完整 commit SHA 的 `rev`，`archive` 為 `sha256`），以及可選的 `branch`（僅 `git`）、`strip`（僅 `archive`）與 `when`。`rsmt validate` SHALL 回報格式錯誤、缺少鎖定值，以及與其他 external 或 dotfile 互相包含的路徑，但位於 `git` external 之內的 external 與 dotfile 除外；有問題的宣告 SHALL 只讓該路徑不部署。外層的 `git` external 不在目標時，其中的 external 與 dotfile SHALL 在該次套用略過。

#### Scenario: Inside a linked directory
- **WHEN** `dotfiles.toml` 以 `link` 宣告 `.config/zsh`，`externals.toml` 宣告 `.config/zsh/plugins/fzf-tab`
- **THEN** `rsmt validate` 回報這兩筆路徑互相包含並以 2 結束

#### Scenario: oh-my-zsh plugin
- **WHEN** `externals.toml` 宣告 `.oh-my-zsh`（`git`）與 `.oh-my-zsh/custom/plugins/zsh-autosuggestions`
- **THEN** `rsmt validate` 通過，套用時先下載 oh-my-zsh 再下載插件

#### Scenario: Not locked yet
- **WHEN** 一筆 `git` external 沒有 `rev`，使用者執行 `rsmt apply`
- **THEN** 該路徑回報尚未鎖定，其他設定照常套用，指令以 1 結束

### Requirement: Download the locked version
套用時，目標不存在的 external SHALL 下載鎖定的版本：`git` 為 checkout 到 `rev` 的 git 工作目錄，`archive` 為驗證 SHA-256 後解壓的內容，`strip` 指定去掉開頭幾層目錄。下載 SHALL 先完成在暫存位置再放到目標，中斷時目標不存在或保持原樣。壓縮檔中跳出目標的路徑 SHALL 使該 external 失敗。

#### Scenario: New host
- **WHEN** 在沒有 `~/.oh-my-zsh` 的 host 上執行 `rsmt apply`
- **THEN** `~/.oh-my-zsh` 是 checkout 在 `rev` 的 oh-my-zsh 工作目錄

#### Scenario: Checksum mismatch
- **WHEN** 壓縮檔的內容與 `sha256` 不符
- **THEN** 目標不建立，該路徑回報雜湊不符，指令以 1 結束

#### Scenario: Path traversal
- **WHEN** 壓縮檔中有 `../.zshrc` 這樣的路徑
- **THEN** 該 external 失敗，家目錄中目標以外的檔案都沒有變動

### Requirement: Follow lock changes
鎖定值改變後套用時，rosamta SHALL 把 external 更新到新的版本：`git` 在工作目錄沒有未 commit 的修改時 fetch 並 checkout（未追蹤的檔案不算修改）；`archive` 在內容與上次解壓時相同時換成新版本。`git` 的 HEAD 不在 `rev`、但工作目錄乾淨時 SHALL checkout 回 `rev`。

#### Scenario: Updated on another host
- **WHEN** 另一台 host 執行 `rsmt externals update` 並推送，本機拉取後套用
- **THEN** 本機的 external 是新的版本

### Requirement: Local changes to externals are never overwritten
`git` 工作目錄有未 commit 的修改、或 `archive` 的內容自上次解壓後被修改時，套用 SHALL 不動該 external 並回報本機已修改。第一次部署時目標已存在，且不是 `origin` 相同的 git repo 時，SHALL 不動目標並回報衝突。不再宣告的 external SHALL 留在原地、不再管理。

#### Scenario: Edited plugin
- **WHEN** 使用者修改了 `~/.oh-my-zsh` 中的檔案，之後鎖定值改變
- **THEN** 套用不更新 `~/.oh-my-zsh`，輸出指出它在本機被修改，指令以 1 結束

#### Scenario: Existing clone
- **WHEN** 新 host 已有從同一個網址 clone 的 `~/.oh-my-zsh`，工作目錄乾淨
- **THEN** 套用把它 checkout 到 `rev`，不重新下載

### Requirement: Update externals
`rsmt externals update [<path>...]` SHALL 把所有（或指定路徑的）external 的鎖定值更新為最新版本，不論 `when`：`git` 取 `branch`（預設為遠端的預設分支）最新的 commit，`archive` 重新下載並計算 SHA-256。有改變時 SHALL 寫回 `externals.toml`、建立一個 commit 並依推送偏好推送，再套用本機適用的 external。

#### Scenario: New oh-my-zsh commits
- **WHEN** oh-my-zsh 的 master 有新的 commit，使用者執行 `rsmt externals update`
- **THEN** `externals.toml` 的 `rev` 變成新的 commit，repo 多一個 commit，`~/.oh-my-zsh` 是新的版本

#### Scenario: Already up to date
- **WHEN** 所有 external 都已是最新版本
- **THEN** 指令說明沒有改變，repo 不變
