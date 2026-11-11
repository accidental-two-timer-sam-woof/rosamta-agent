## MODIFIED Requirements

### Requirement: Structured recognition
一段內容若只包含環境變數設定、PATH 的增加與 alias，rosamta SHALL 把它轉成結構化 module，並只套用到來源檔案所屬的 shell：`~/.profile` 為 sh、bash、zsh，bash 的檔案為 bash，zsh 的檔案為 zsh，fish 的檔案為 fish。只要有任一行無法辨識，整段 SHALL 保存為片段 module。

#### Scenario: PATH and alias
- **WHEN** `~/.zshrc` 中的一段內容是 `export PATH=/Users/me/.bun/bin:$PATH` 與 `alias cc="claude"`
- **THEN** 產生一個只套用到 zsh 的結構化 module，含一個 PATH prepend 與一個 alias

#### Scenario: Deduplicating PATH idiom
- **WHEN** 一段內容以 `case ":$PATH:" in` 判斷後才 prepend 某個目錄
- **THEN** 產生一個只含該 PATH prepend 的結構化 module

#### Scenario: Mixed content
- **WHEN** 一段內容同時有 `export FOO=1` 與 `source ~/.orbstack/shell/init.zsh`
- **THEN** 整段保存為一個片段 module

#### Scenario: Profile content
- **WHEN** 從 `~/.profile` 接管 `export EDITOR=vim`
- **THEN** 產生的結構化 module 套用到 sh、bash 與 zsh，不套用到 fish

### Requirement: Default scope is this host
接管產生的 module SHALL 預設只套用到本機：`when` 記錄本機的識別名稱與 OS，`shells` 記錄來源檔案所屬的 shell。使用者選擇所有主機時，SHALL 只去掉識別名稱與 OS 的限制。要套用到其他主機、OS 或 shell 時，由使用者修改套用範圍或使用 `rsmt include`。

#### Scenario: Default
- **WHEN** 使用者在 macOS 的 `mbp` 上接受從 `~/.zshrc` 接管的項目且沒有變更範圍
- **THEN** 產生的 module 宣告 `hosts = ["mbp"]`、`os = ["macos"]` 與 `shells = ["zsh"]`

#### Scenario: All hosts
- **WHEN** 使用者把同一個項目的範圍改為所有主機後接受
- **THEN** 產生的 module 只宣告 `shells = ["zsh"]`

### Requirement: Re-added content
安裝程式把已經接管過的內容再次寫回 rc 檔時，rosamta SHALL 認出它並直接依 host 設定移除，不產生新的 module，也不放進待處理清單。寫回原本的檔案時一律如此；寫進其他檔案時，只有對應的 module 在本機載入（已啟用且 `when` 符合本機）並且會在讀取該檔案的每個 shell 中生效，才 SHALL 視為已接管，否則照一般內容處理。內容與只套用到其他主機的 module 相同時，依「Same content on another host」處理。

#### Scenario: Installer runs again
- **WHEN** 已接管的 bun PATH 被 bun 的安裝程式再次寫進 `~/.zshrc`
- **THEN** 下一次掃描直接移除這段，repo 中仍只有原本的 bun module

#### Scenario: Same line in another shell's file
- **WHEN** `~/.profile` 與 `~/.zshrc` 都有 `. "$HOME/.cargo/env"`，使用者只接管 `~/.zshrc` 的那一行
- **THEN** `~/.profile` 的那一行留在原處，並列為可接管的內容

#### Scenario: Module for another host
- **WHEN** repo 中有一個只套用到其他主機、沒有限制 shell 的舊 module 設定 `FOO=1`，本機的 `~/.zshrc` 也有 `export FOO=1`
- **THEN** 本機的這一行不被視為已接管；接管時另建一個只套用到本機 zsh 的 module，因為兩者的 shell 不同

## ADDED Requirements

### Requirement: Same content on another host
接管到的內容與 repo 中一個只套用到其他主機的 module 相同時，rosamta SHALL 把本機的識別名稱加進該 module 的 `hosts`，`os` 有值且不含本機的 OS 時一併加入，不另建 module。相同的條件為：

- 該 module 的 `hosts` 有值，`shells` 與這次接管的來源 shell 相同，且加入後會在本機載入；
- 結構化 module 的環境變數、PATH 與 alias 在家目錄路徑改寫為 `$HOME` 後完全相同，不論順序；片段 module 的文字在改寫家目錄路徑、去掉註解行與行尾空白後完全相同。

不符合時 SHALL 照常另建 module，不做近似比對。內容看起來像秘密而留在本機的 module SHALL NOT 合併。這次接管的 commit 訊息 SHALL 為 `adopt(<shell>): include <module> from <來源檔> on <識別名稱>`；還原這次接管時，SHALL 把原行放回 rc 檔，並從 `hosts` 移除本機。

#### Scenario: Installed on two hosts
- **WHEN** repo 有 `modules/adopted/bun.toml`，宣告 `hosts = ["mbp"]`、`os = ["macos"]`、`shells = ["zsh"]`；Linux 主機 `box` 的 `~/.zshrc` 被 bun 的安裝程式寫入相同的 PATH
- **THEN** `bun.toml` 變成 `hosts = ["mbp", "box"]`、`os = ["macos", "linux"]`，repo 中沒有 `bun-2.toml`

#### Scenario: Different version
- **WHEN** `mbp` 的 module prepend `$HOME/.nvm/versions/node/v20/bin`，`box` 接管到的是 `v22`
- **THEN** `box` 另建一個只套用到 `box` 的 module

#### Scenario: Different shell
- **WHEN** `mbp` 的 module 只套用到 zsh，`box` 的相同內容來自 `~/.bashrc`
- **THEN** `box` 另建一個只套用到 bash 的 module

#### Scenario: Restore an include
- **WHEN** 使用者在 `box` 上還原這次合併
- **THEN** 原行回到 `box` 的 `~/.zshrc`，`bun.toml` 的 `hosts` 回到 `["mbp"]`
