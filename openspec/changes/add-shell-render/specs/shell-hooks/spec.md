# Spec Delta

## Purpose

定義 rosamta 在各 shell 的 rc 檔中放置 hook 的位置與規則，讓 rosamta 只占用 rc 檔中一個明確標記的區塊。

## ADDED Requirements

### Requirement: Hook locations
rosamta SHALL 在下列位置安裝 hook：sh 為 `~/.profile`；bash 為 `~/.bashrc` 與 login 時實際讀取的 profile 檔；zsh 為 `$ZDOTDIR` 下的 `.zshenv`、`.zshrc`，macOS 另加 `.zprofile`；fish 為 fish 設定目錄下 `conf.d/rosamta.fish` 這個獨立檔案。

#### Scenario: zsh with ZDOTDIR
- **WHEN** 使用者在 `~/.zshenv` 把 `ZDOTDIR` 設為 `~/.config/zsh`
- **THEN** rosamta 把 `.zshrc` 的 hook 裝在 `~/.config/zsh/.zshrc`

#### Scenario: fish
- **WHEN** 使用者安裝 fish 的 hook
- **THEN** rosamta 建立 `conf.d/rosamta.fish`，不修改 `config.fish`

### Requirement: Marked block only
hook SHALL 是以 `# >>> rosamta >>>` 與 `# <<< rosamta <<<` 包住的區塊。rosamta 安裝或更新 hook 時 SHALL 只改動這個區塊，重複安裝 SHALL 不產生第二個區塊。

#### Scenario: Install twice
- **WHEN** 使用者連續執行兩次 hook 安裝
- **THEN** 每個 rc 檔中只有一個 rosamta 區塊，區塊外的內容不變

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
