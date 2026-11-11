# shell-hooks Specification

## Purpose
定義 rosamta 在各 shell 的 rc 檔中放置 hook 的位置與規則，讓 rosamta 只占用 rc 檔中一個明確標記的區塊。

## Requirements

### Requirement: Hook locations
rosamta SHALL 在下列位置安裝 hook：sh 為 `~/.profile`；bash 為 `~/.bashrc` 與 login 時實際讀取的 profile 檔；zsh 為 `$ZDOTDIR` 下的 `.zshenv`、`.zshrc` 與 `.zprofile`；fish 為 fish 設定目錄下 `conf.d/rosamta.fish` 這個獨立檔案。

#### Scenario: zsh with ZDOTDIR
- **WHEN** 使用者在 `~/.zshenv` 把 `ZDOTDIR` 設為 `~/.config/zsh`
- **THEN** rosamta 把 `.zshrc` 的 hook 裝在 `~/.config/zsh/.zshrc`

#### Scenario: ZDOTDIR inherited from zsh
- **WHEN** `~/.zshenv` 匯出 `ZDOTDIR`，使用者在這個 zsh 中執行 `rsmt apply`
- **THEN** rosamta 仍把 `.zshenv` 的 hook 裝在 `~/.zshenv`

#### Scenario: fish
- **WHEN** 使用者安裝 fish 的 hook
- **THEN** rosamta 建立 `conf.d/rosamta.fish`，不修改 `config.fish`

### Requirement: Marked block only
hook SHALL 是以 `# >>> rosamta >>>` 與 `# <<< rosamta <<<` 包住的區塊。rosamta 安裝或更新 hook 時 SHALL 只改動這個區塊，重複安裝 SHALL 不產生第二個區塊。檔案中的開始與結束標記不成對時，rosamta SHALL 不修改該檔，並指出該檔。寫入 rc 檔的最後一刻，rosamta SHALL 再確認檔案仍是先前讀到的內容；內容已不同時 SHALL 不寫入，並指出該檔。

#### Scenario: Install twice
- **WHEN** 使用者連續執行兩次 hook 安裝
- **THEN** 每個 rc 檔中只有一個 rosamta 區塊，區塊外的內容不變

#### Scenario: End marker deleted
- **WHEN** 使用者刪掉 `# <<< rosamta <<<` 這一行後執行 `rsmt apply`
- **THEN** `rsmt apply` 以 2 結束並指出該檔，檔案內容不變

#### Scenario: Installer appends during the write
- **WHEN** 安裝程式在 rosamta 寫入 `~/.zshrc` 的同時附加一行
- **THEN** 該行保留在 `~/.zshrc`，rosamta 不寫入並指出該檔

### Requirement: bash login and non-login shells
bash 的 login 互動 shell 與非 login 互動 shell SHALL 都載入完整的設定。rosamta 需要新建 `~/.bash_profile` 時 SHALL 讓它同時載入 `~/.profile` 與 `~/.bashrc`，避免遮蔽原本的設定。

#### Scenario: macOS Terminal
- **WHEN** 在 macOS Terminal 開啟 bash（login shell）
- **THEN** alias 與 PATH 都已生效

#### Scenario: New bash_profile
- **WHEN** host 原本只有 `~/.profile`，沒有 `~/.bash_profile`
- **THEN** rosamta 若建立 `~/.bash_profile`，login bash 仍會執行 `~/.profile` 的內容

### Requirement: Remove hooks
`rsmt unhook <shell>` SHALL 只移除該 shell 的 rosamta 區塊（fish 則刪除 `conf.d/rosamta.fish`），其餘內容維持原樣。

#### Scenario: Unhook zsh
- **WHEN** 使用者執行 `rsmt unhook zsh`
- **THEN** zsh 的 rc 檔中不再有 rosamta 區塊，其他內容不變

### Requirement: Block removed by another tool
`rsmt apply` 在 rc 檔安裝或改寫 rosamta 區塊後，若 chezmoi 也管理該檔，SHALL 在 stderr 警告該檔會在下次 `chezmoi apply` 時失去區塊；沒有改動區塊的 `rsmt apply` SHALL 不執行 chezmoi。目前版本服務的 shell 有 rc 檔失去區塊時，背景服務 SHALL 發出一則列出這些檔案的通知，App 的「需要處理」SHALL 列出同一件事；區塊恢復之前 SHALL 不重複通知。

#### Scenario: chezmoi manages .zshrc
- **WHEN** chezmoi 管理 `~/.zshrc`，`rsmt apply` 在其中安裝 rosamta 區塊
- **THEN** `rsmt apply` 成功，並警告 chezmoi 也管理 `~/.zshrc`

#### Scenario: Nothing to hook
- **WHEN** rc 檔的區塊都已是最新的，使用者執行 `rsmt apply`
- **THEN** chezmoi 沒有被執行

#### Scenario: chezmoi apply writes over .zshrc
- **WHEN** `chezmoi apply` 以沒有 rosamta 區塊的版本覆寫 `~/.zshrc`
- **THEN** 使用者收到一則通知，說明 shell 不再載入 rosamta，以及受影響的檔案
