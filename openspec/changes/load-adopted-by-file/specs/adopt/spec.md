## MODIFIED Requirements

### Requirement: Phase from source file
接管產生的片段 module SHALL 以 `file` 記錄來源檔，並只在 shell 讀取那個檔時載入，保留原本在檔案中的先後順序。

#### Scenario: Two snippets from zshrc
- **WHEN** 從 `~/.zshrc` 先後接管兩段片段
- **THEN** 兩者都宣告 `file = ".zshrc"`，載入順序與原本在檔案中的順序相同

#### Scenario: Homebrew in zprofile on macOS
- **WHEN** 接管 `~/.zprofile` 中的 `eval "$(/opt/homebrew/bin/brew shellenv)"`
- **THEN** 新開的 login zsh 中，`/opt/homebrew/bin` 仍排在 `/usr/bin` 之前，與接管前相同

#### Scenario: Login-only content
- **WHEN** 從 `~/.bash_profile` 接管一行 `ssh-add -q ~/.ssh/id_ed25519`
- **THEN** 這一行只在 login bash 執行，`bash -c` 與非 login 的互動 bash 都不執行

#### Scenario: zlogin
- **WHEN** 從 `~/.zlogin` 接管一段片段
- **THEN** 非 login 的 zsh 不執行它

### Requirement: Raw module scope
片段 module SHALL 只套用到來源檔所屬的 shell：`~/.profile` 的內容屬於 sh，bash 與 zsh 讀取 `~/.profile` 時經由它的 hook 載入同一份；bash 檔案的內容只套用到 bash；zsh 檔案的內容只套用到 zsh；fish 檔案的內容只套用到 fish。

#### Scenario: Snippet from zprofile
- **WHEN** 從 `~/.zprofile` 接管一段無法辨識的內容
- **THEN** 產生的片段 module 只套用到 zsh

#### Scenario: Profile that only sh reads
- **WHEN** `~/.bash_profile` 沒有執行 `~/.profile`，zsh 的啟動檔也沒有，從 `~/.profile` 接管一行 `. "$HOME/.cargo/env"`
- **THEN** 產生的片段 module 只套用到 sh

#### Scenario: Profile read by zsh through emulate
- **WHEN** `~/.zprofile` 以 `emulate sh -c '. ~/.profile'` 執行 `~/.profile`，使用者接管 `~/.profile` 的一行
- **THEN** login zsh 執行這一行一次

## ADDED Requirements

### Requirement: Startup wiring stays in place
執行其他啟動檔的行（例如 `. ~/.bashrc`、`. ~/.profile`）SHALL 不被接管，也不列入待處理清單。

#### Scenario: Fedora bash_profile
- **WHEN** `~/.bash_profile` 含有 `. ~/.bashrc`，使用者執行 `rsmt adopt`
- **THEN** 這一行留在 `~/.bash_profile`，login bash 執行 `~/.bashrc` 的內容一次
