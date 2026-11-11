# Spec Delta

## Purpose

把 chezmoi 的 source 目錄轉成 rosamta repo 的宣告，讓 chezmoi 的使用者換到 rosamta 時，同一台主機上套用的結果不變，轉不了的部分逐項列出。

## ADDED Requirements

### Requirement: Import command
`rsmt import chezmoi [<source-dir>]` SHALL 讀取參數指定的 source，否則 `chezmoi source-path` 回報的位置，否則 `~/.local/share/chezmoi`，並遵循 `.chezmoiroot`。rosamta 的 repo 尚未初始化、有尚未推送的變更或找不到 source 時 SHALL 以 2 結束且不寫入任何東西。source 中的 symlink SHALL 不被讀取，列為需要手動處理。匯入 SHALL 不修改 chezmoi 的 source 與設定、不部署、不連網路。`--dry-run` SHALL 只顯示報告。匯入完成 SHALL 以一個 commit 記錄，commit 內容包含報告；有需要手動處理的項目時仍以 0 結束。

#### Scenario: Dry run
- **WHEN** 使用者執行 `rsmt import chezmoi --dry-run`
- **THEN** 顯示會匯入、略過與需要手動處理的項目，repo 與 `host.toml` 都沒有改變

#### Scenario: Not initialized
- **WHEN** 還沒有執行 `rsmt init` 就執行匯入
- **THEN** 以 2 結束並說明要先初始化

### Requirement: Same result as chezmoi
匯入後在同一台主機執行 `rsmt apply`，每個匯入的目標 SHALL 與 `chezmoi apply` 的結果有相同的內容與權限，部署方式 SHALL 依 chezmoi 的 `mode`（預設複製，`symlink` 時連結）。內容只有空白且沒有 `empty_` 的檔案 SHALL 不匯入。轉換後的 template SHALL 以本機的值 render 並與 chezmoi render 同一份 template 的結果比較，不同、或 chezmoi 的結果只有空白時列為需要手動處理。

#### Scenario: Private file
- **WHEN** source 有 `private_dot_netrc`
- **THEN** `dotfiles.toml` 宣告 `.netrc`，部署方式為 `copy`、`permissions = "0600"`，套用後 `~/.netrc` 的內容與權限與 chezmoi 相同

#### Scenario: OS-specific file
- **WHEN** `dot_yabairc.tmpl` 的整份內容被 `{{ if eq .chezmoi.os "darwin" }}…{{ end }}` 包住
- **THEN** 匯入為一般檔案並宣告 `when = { os = ["macos"] }`，Linux 主機不會建立 `~/.yabairc`

### Requirement: Template conversion
`.tmpl` 檔案 SHALL 只在全部動作都屬於支援的寫法時轉成 minijinja；任何其他寫法 SHALL 使整個檔案不被宣告，並在報告列出行號與寫法。`.chezmoi.os` 與 `.chezmoi.arch` 的比較 SHALL 換成 rosamta 的名稱；本機 hostname 的比較 SHALL 換成本機的主機識別名稱；資料 SHALL 依設計轉成常值或 `vars`，本機的值寫入 `host.toml`。

#### Scenario: Unsupported function
- **WHEN** template 第 12 行使用 `{{ range .packages }}`
- **THEN** 該檔案不被宣告，原檔複製到 `chezmoi-import/`，報告列出檔案、第 12 行與 `range`

#### Scenario: Hostname on this host
- **WHEN** chezmoi 回報的 hostname 是 `mbp-m5`，rosamta 的主機識別名稱是 `xinshoutw-macbook-pro`，template 寫 `{{ if eq .chezmoi.hostname "mbp-m5" }}`
- **THEN** 轉換結果比較 `host.id == "xinshoutw-macbook-pro"`，在本機 render 的結果與 chezmoi 相同

### Requirement: Scripts and externals
`run_` 腳本 SHALL 轉成 `onchange` 的 `[[script]]`，`before_` 轉成 `timing = "before"`，依 chezmoi 的執行順序宣告，並記為本機已執行過。`include "<path>" | sha256sum` SHALL 轉成 `watch`。`git-repo` 與 `archive` 類型的 external SHALL 轉成 `externals.toml` 的宣告；git 的 `rev` SHALL 鎖定在目標中已有 clone 的 HEAD。其他類型與欄位 SHALL 列為需要手動處理。

#### Scenario: Brewfile script
- **WHEN** `run_onchange_install.sh.tmpl` 包含 `# {{ include "dot_Brewfile" | sha256sum }}`
- **THEN** `scripts.toml` 宣告該腳本 `run = "onchange"`、`watch = ["home/.Brewfile"]`，匯入後第一次 `rsmt apply` 不會執行它

#### Scenario: oh-my-zsh clone
- **WHEN** `.chezmoiexternal.toml` 宣告 `[".oh-my-zsh"] type = "git-repo"`，且 `~/.oh-my-zsh` 已是 chezmoi clone 的 repo
- **THEN** `externals.toml` 以 `rev` 鎖定在目前的 HEAD，第一次 `rsmt apply` 不改動 `~/.oh-my-zsh`

### Requirement: Encrypted files
`encrypted_` 檔案 SHALL 以 `chezmoi decrypt` 解密，再以 rosamta 的金鑰加密並宣告 `encrypted = true`；repo 還沒有金鑰時 SHALL 在終端機詢問新密碼。沒有終端機或解密失敗時 SHALL 列為需要手動處理。

#### Scenario: First encrypted import
- **WHEN** source 有 `encrypted_private_dot_ssh/config.age`，rosamta 的 repo 還沒有金鑰，使用者在終端機執行匯入
- **THEN** 詢問新密碼兩次，`home/.ssh/config` 以 rosamta 的金鑰加密，宣告 `encrypted = true`、`permissions = "0600"`

### Requirement: Import report
報告 SHALL 列出匯入、略過與需要手動處理的項目，每項包含 source 路徑與原因；`--json` SHALL 以原因代碼輸出。App 的設定精靈 SHALL 在偵測到 chezmoi 的 source 時提供匯入，並以 App 的文字顯示數量與需要手動處理的清單。

#### Scenario: Report in JSON
- **WHEN** 使用者執行 `rsmt import chezmoi --json`
- **THEN** 輸出每個項目的 source、目標、結果與原因代碼
