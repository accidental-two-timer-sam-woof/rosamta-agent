# config-repo Specification

## Purpose
定義所有 host 共用的 repo：目錄結構、檔案格式、格式版本、驗證規則，以及建立或加入 repo 的方式，讓設定可以被驗證，也能被程式安全地寫回。

## Requirements

### Requirement: Repo layout
repo SHALL 在根目錄包含 `rosamta.toml`，並可包含 `modules/`、`home/`、`dotfiles.toml`、`packages.toml`。`modules/` 底下（含子目錄）每個 `.toml` 檔是一個 module，檔名去掉副檔名即為 module 名稱；名稱以 `.` 開頭的檔案與目錄（例如編輯器的鎖定檔）SHALL 略過。

#### Scenario: Minimal repo
- **WHEN** repo 只有 `rosamta.toml`
- **THEN** `rsmt validate` 通過，且 repo 中沒有任何 module

#### Scenario: Module in a subdirectory
- **WHEN** repo 有 `modules/adopted/bun.toml`
- **THEN** rosamta 把它載入為名稱 `bun` 的 module

#### Scenario: Editor lock file
- **WHEN** Emacs 在 `modules/` 留下鎖定檔 `.#bun.toml`
- **THEN** rosamta 不把它當成 module，`rsmt validate` 與 `rsmt apply` 照常執行

### Requirement: Unique module names
module 名稱 SHALL 在整個 repo 內唯一；`modules/` 的子目錄只用來整理檔案，不構成命名空間。

#### Scenario: Duplicate name
- **WHEN** repo 同時有 `modules/bun.toml` 與 `modules/adopted/bun.toml`
- **THEN** `rsmt validate` 失敗，並列出兩個檔案的路徑

### Requirement: Format version
`rosamta.toml` SHALL 宣告正整數 `format`。rosamta 讀到較舊的格式時 SHALL 自動遷移 repo 並以一個 commit 記錄；讀到比自己支援的更新的格式時 SHALL 拒絕讀取 repo。

#### Scenario: Older format
- **WHEN** repo 的 `format` 小於目前支援的版本
- **THEN** rosamta 遷移 repo 內容、更新 `format`，並建立一個說明遷移的 commit

#### Scenario: Newer format
- **WHEN** repo 的 `format` 大於目前支援的版本
- **THEN** rosamta 不 render、不寫入 repo，並提示使用者升級 rosamta

### Requirement: Host registry
`rosamta.toml` SHALL 記錄曾經加入這個 repo 的所有 host 識別名稱。host 第一次加入 repo 時，rosamta SHALL 把它的識別名稱加進清單。

#### Scenario: Second host joins
- **WHEN** 一台識別名稱為 `lab-server` 的 host 加入既有的 repo
- **THEN** `rosamta.toml` 的 host 清單新增 `lab-server`，原有的名稱保留

### Requirement: Strict validation
`rsmt validate` SHALL 檢查 repo 中所有 TOML 檔的語法與欄位，把未定義的欄位視為錯誤，並回報每個錯誤的檔案、行號與原因；驗證過程 SHALL 不修改任何檔案。

#### Scenario: Unknown field
- **WHEN** 某個 module 檔含有拼錯的欄位 `phse`
- **THEN** `rsmt validate` 失敗，並指出該檔案、行號與欄位名稱

### Requirement: Comment-preserving writes
rosamta 修改使用者已有的 TOML 檔時 SHALL 只改動目標欄位，保留其他欄位、註解與排列順序。

#### Scenario: Disable a module
- **WHEN** 使用者停用一個檔案內含註解的 module
- **THEN** 該檔只有表示啟用狀態的欄位改變，註解與其他欄位維持原樣

### Requirement: Schema export
rosamta SHALL 能輸出 repo 中每種檔案的 JSON Schema，供編輯器與 AI 驗證設定。

#### Scenario: Export module schema
- **WHEN** 使用者執行 `rsmt schema module`
- **THEN** stdout 輸出 module 檔的 JSON Schema

### Requirement: Create a new repo
`rsmt init` 不帶參數時 SHALL 在 repo 位置建立只含 `rosamta.toml` 的 git repo、建立 `host.toml`，並把本機加進 host 清單。repo 位置已經有 repo 時 SHALL 拒絕執行。

#### Scenario: First run
- **WHEN** 本機沒有 repo，使用者執行 `rsmt init`
- **THEN** repo 位置出現一個含 `rosamta.toml` 的 git repo，`host.toml` 也已建立

#### Scenario: Already initialized
- **WHEN** repo 位置已經有 repo，使用者執行 `rsmt init`
- **THEN** 指令以 2 結束，現有的 repo 與 `host.toml` 都不被修改

### Requirement: Join an existing repo
`rsmt init <git-url>` SHALL 把該 repo clone 到 repo 位置、建立 `host.toml`，並把本機加進 host 清單。在使用者確認套用之前，rosamta SHALL 不修改任何 rc 檔或 dotfile。

#### Scenario: Join from GitHub
- **WHEN** 使用者執行 `rsmt init git@github.com:me/dotfiles.git`
- **THEN** repo 被 clone 到 repo 位置，`host.toml` 已建立，所有 rc 檔與 dotfile 維持原狀
