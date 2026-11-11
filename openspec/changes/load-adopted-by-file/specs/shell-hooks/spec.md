## MODIFIED Requirements

### Requirement: Hook locations
rosamta SHALL 在下列位置安裝 hook：sh 為 `~/.profile`；bash 為 `~/.bashrc` 與已存在的 `~/.bash_profile`、`~/.bash_login`，兩者都不存在時建立 `~/.bash_profile`；zsh 為 `$ZDOTDIR` 下的 `.zshenv`、`.zprofile`、`.zshrc`，以及有片段來自它時的 `.zlogin`；fish 為 fish 設定目錄下 `conf.d/rosamta.fish` 這個獨立檔案。

#### Scenario: zsh with ZDOTDIR
- **WHEN** 使用者在 `~/.zshenv` 把 `ZDOTDIR` 設為 `~/.config/zsh`
- **THEN** rosamta 把 `.zshrc` 的 hook 裝在 `~/.config/zsh/.zshrc`

#### Scenario: ZDOTDIR inherited from zsh
- **WHEN** `~/.zshenv` 匯出 `ZDOTDIR`，使用者在這個 zsh 中執行 `rsmt apply`
- **THEN** rosamta 仍把 `.zshenv` 的 hook 裝在 `~/.zshenv`

#### Scenario: fish
- **WHEN** 使用者安裝 fish 的 hook
- **THEN** rosamta 建立 `conf.d/rosamta.fish`，不修改 `config.fish`

#### Scenario: zlogin
- **WHEN** 使用者接管 `~/.zlogin` 的一段片段
- **THEN** `~/.zlogin` 中有 rosamta 的區塊，片段由它載入

### Requirement: bash login and non-login shells
rosamta 自己產生的互動設定（結構化 module 與使用者寫的 module）SHALL 在 bash 的 login 互動 shell 與非 login 互動 shell 都生效。rosamta SHALL 不改變既有的 `~/.bash_profile` 是否執行 `~/.bashrc`；rosamta 需要新建 `~/.bash_profile` 時 SHALL 讓它同時載入 `~/.profile` 與 `~/.bashrc`，避免遮蔽原本的設定。

#### Scenario: macOS Terminal
- **WHEN** 在 macOS Terminal 開啟 bash（login shell）
- **THEN** rosamta 的 alias 與 PATH 都已生效

#### Scenario: New bash_profile
- **WHEN** host 原本只有 `~/.profile`，沒有 `~/.bash_profile`
- **THEN** rosamta 建立 `~/.bash_profile`，login bash 仍會執行 `~/.profile` 的內容，`~/.bashrc` 只執行一次

#### Scenario: Existing bash_profile that does not read bashrc
- **WHEN** 既有的 `~/.bash_profile` 沒有執行 `~/.bashrc`，使用者執行 `rsmt apply`
- **THEN** `~/.bash_profile` 除了 rosamta 的區塊之外沒有變動，login bash 仍然不執行 `~/.bashrc` 中沒有接管的內容
