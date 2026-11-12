# adopt Specification

## Purpose
把 rc 檔中 hook 以外的內容轉成 repo 裡的 module 並從 rc 檔移除，讓安裝程式寫入或使用者手改的設定都回到 repo 管理，rc 檔只留下 hook。

## Requirements

### Requirement: Watched files
rosamta SHALL 掃描下列檔案中 hook 以外的內容：`~/.profile`、`~/.bash_profile`、`~/.bash_login`、`~/.bashrc`、`$ZDOTDIR` 下的 `.zshenv`、`.zprofile`、`.zshrc`、`.zlogin`、fish 的 `config.fish`，以及 fish `conf.d` 中 rosamta 以外的檔案。不是 UTF-8 文字的檔案 SHALL 略過且不被修改。`~/.bash_profile` 或 `~/.bash_login` 中執行 `~/.profile` 或 `~/.bashrc` 的段落是 login bash 讀到這兩個檔案的途徑，SHALL 留在原處、不列為可接管的內容。

#### Scenario: Installer adds a fish conf.d file
- **WHEN** 某個安裝程式新增 `conf.d/foo.fish`
- **THEN** 下一次掃描把整個檔案列為可接管的內容

#### Scenario: bash_profile that runs bashrc
- **WHEN** `~/.bash_profile` 是 Fedora 預設的版本，以 `if [ -f ~/.bashrc ]; then . ~/.bashrc; fi` 載入 `~/.bashrc`
- **THEN** 掃描不列出這一段，它留在原處；login bash 仍只執行 `~/.bashrc` 一次

#### Scenario: File that is not UTF-8
- **WHEN** `~/.zlogin` 含有不是 UTF-8 的位元組
- **THEN** 掃描不列出它的內容，檔案的每個位元組維持原樣

### Requirement: Segmentation
rosamta SHALL 把連續的非空白行切成一段（結構結尾之後另起一段的情況見 Segments end at complete code），並把緊接在上方的註解併入該段；以成對 marker 包住的區塊（例如 `# >>> conda initialize >>>` 到 `# <<< conda initialize <<<`）SHALL 整塊視為一段，即使它位在還沒結束的結構或 here-document 的內容之中。fish `conf.d` 的檔案 SHALL 整個檔案視為一段。

#### Scenario: Installer comment
- **WHEN** `~/.zprofile` 中有 `# Added by OrbStack` 註解，下一行是 `source ~/.orbstack/shell/init.zsh`
- **THEN** 兩行屬於同一段

#### Scenario: Marker block with blank lines
- **WHEN** 一個 marker 區塊內有空白行
- **THEN** 整個區塊仍是一段

### Requirement: Segments end at complete code
空白行 SHALL 只在這一段到它為止是完整的程式碼時才結束該段：還沒結束的 `if`、迴圈、函式等結構，以及 here-document 的內容，其中的空白行不切開。一段的最後一行只有 `fi`、`done`、`esac`、`}` 或 `end`（後面可以接 `;`），且這一段是完整的程式碼時，下一個非空白行 SHALL 開始新的一段，即使中間沒有空白行。

程式碼是否完整 SHALL 以該檔案所屬的 shell 檢查語法，`~/.profile` 以 sh 檢查。檢查不通過時（結構還沒結束、語法錯誤、逾時或 shell 無法執行），空白行不結束該段；一直不通過時，該段延續到下一個 marker 區塊、hook 區塊、已註解掉的接管段落或檔案結尾。找不到該 shell 時不檢查，一律視為完整：marker 區塊以外的空白行都結束一段，只有 here-document 內容中的空白行例外；結構結尾那一行之後的非空白行也開始新的一段，即使外層的結構還沒結束。

#### Scenario: Blank line inside an unfinished block
- **WHEN** zsh 已安裝，`~/.zshrc` 中一個 `if` 到 `fi` 的區塊內有空白行
- **THEN** 區塊的每一行屬於同一段

#### Scenario: Blank line inside a here-document
- **WHEN** 一個 here-document 的內容有空白行，內容中沒有成對的 marker
- **THEN** 從開始那一行到結束的分隔字屬於同一段

#### Scenario: Line right after a finished block
- **WHEN** 一個完整的 `if` 到 `fi` 區塊，`fi` 的下一行是 `. x`，中間沒有空白行
- **THEN** `. x` 開始新的一段

#### Scenario: Shell that cannot be found
- **WHEN** `~/.zshrc` 有 rosamta 的 hook 區塊但找不到 zsh，檔案中一個 `if` 到 `fi` 的區塊內有空白行
- **THEN** 空白行之前與之後的內容分屬不同的段

### Requirement: Structured recognition
一段內容若只包含環境變數設定、PATH 的增加與 alias，rosamta SHALL 把它轉成結構化 module，套用到所有支援的 shell；只要有任一行無法辨識，整段 SHALL 保存為片段 module。

#### Scenario: PATH and alias
- **WHEN** 一段內容是 `export PATH=/Users/me/.bun/bin:$PATH` 與 `alias cc="claude"`
- **THEN** 產生一個結構化 module，含一個 PATH prepend 與一個 alias

#### Scenario: Deduplicating PATH idiom
- **WHEN** 一段內容以 `case ":$PATH:" in` 判斷後才 prepend 某個目錄
- **THEN** 產生一個只含該 PATH prepend 的結構化 module

#### Scenario: Mixed content
- **WHEN** 一段內容同時有 `export FOO=1` 與 `source ~/.orbstack/shell/init.zsh`
- **THEN** 整段保存為一個片段 module

### Requirement: Raw module scope
片段 module SHALL 只套用到實際讀取來源檔的 shell：`~/.profile` 的內容套用到 sh，以及會執行它的 bash 與 zsh（login bash 沒有 `~/.bash_profile` 與 `~/.bash_login`，或讀取的那一個、或 `~/.bashrc` 執行 `~/.profile`；zsh 的啟動檔執行 `~/.profile`；執行 `~/.profile` 的那一行已接管進該 shell 的 module 時也算）；bash 檔案的內容只套用到 bash；zsh 檔案的內容只套用到 zsh；fish 檔案的內容只套用到 fish。

#### Scenario: Snippet from zprofile
- **WHEN** 從 `~/.zprofile` 接管一段無法辨識的內容
- **THEN** 產生的片段 module 只套用到 zsh

#### Scenario: Profile that only sh reads
- **WHEN** `~/.bash_profile` 沒有執行 `~/.profile`，zsh 的啟動檔也沒有，從 `~/.profile` 接管一行 `. "$HOME/.cargo/env"`
- **THEN** 產生的片段 module 只套用到 sh

### Requirement: Phase from source file
接管產生的片段 module SHALL 依來源檔決定載入階段：`.profile`、`.bash_profile`、`.bash_login`、`.zshenv` 的內容放在 env 階段，`.zprofile` 的內容放在 login 階段，其他檔案的內容放在 late 階段，並保留原本在檔案中的先後順序。

#### Scenario: Two snippets from zshrc
- **WHEN** 從 `~/.zshrc` 先後接管兩段片段
- **THEN** 兩者都在 late 階段，且載入順序與原本在檔案中的順序相同

#### Scenario: Homebrew in zprofile on macOS
- **WHEN** 接管 `~/.zprofile` 中的 `eval "$(/opt/homebrew/bin/brew shellenv)"`
- **THEN** 新開的 login zsh 中，`/opt/homebrew/bin` 仍排在 `/usr/bin` 之前，與接管前相同

### Requirement: Path normalization
接管時 rosamta SHALL 把位於使用者家目錄下的絕對路徑改寫成 `$HOME` 開頭。

#### Scenario: Absolute home path
- **WHEN** 接管的內容含有 `/Users/me/.bun/bin`
- **THEN** module 中記錄為 `$HOME/.bun/bin`

### Requirement: Default scope is this host
接管產生的 module SHALL 預設只套用到本機（以識別名稱表示）；使用者可在接管時或之後改為所有主機。

#### Scenario: Default
- **WHEN** 使用者接受一個接管項目且沒有變更範圍
- **THEN** 產生的 module 宣告 `hosts` 只含本機的識別名稱

### Requirement: Module naming
rosamta SHALL 依序以 marker 名稱、`Added by <名稱>` 註解、內容中的工具路徑推得 module 名稱，轉成小寫並以 `-` 連接；與既有名稱重複時加上數字後綴。產生的 module 放在 `modules/adopted/`。

#### Scenario: Name from installer comment
- **WHEN** 段落含有 `# Added by OrbStack`
- **THEN** module 名稱為 `orbstack`，檔案為 `modules/adopted/orbstack.toml`

### Requirement: Remove original lines
接管後 rosamta SHALL 從 rc 檔移除該段：host 設定為「刪除」時直接刪除，設定為「註解掉」時在每行前加上 `#` 並在上方標註 module 路徑。修改前 SHALL 存 snapshot。

#### Scenario: Delete mode
- **WHEN** host 設定為「刪除」，使用者接管 `~/.zprofile` 中的 OrbStack 段落
- **THEN** 該段從 `~/.zprofile` 消失，snapshot 中保有原文

#### Scenario: Comment mode
- **WHEN** host 設定為「註解掉」
- **THEN** 原段落每行前加上 `#`，上方多一行指出 module 檔案的位置，之後的掃描不再把它列為可接管

### Requirement: Failed adoption leaves nothing half adopted
接管後 shell 的啟動檢查失敗時，rosamta SHALL 把接管的內容放回每個 rc 檔，並移除為它產生的 module；rc 檔在 rosamta 寫入後又被其他程式修改時，SHALL 保留那些修改，把接管的內容放回原處。接管的內容 SHALL 不會同時存在於 rc 檔與 module，也 SHALL 不會兩邊都沒有。

#### Scenario: Installer appends during the check
- **WHEN** 接管 `~/.zshrc` 的一段內容後啟動檢查失敗，而檢查期間有安裝程式在 `~/.zshrc` 附加了一行
- **THEN** `~/.zshrc` 同時有原本的內容與安裝程式附加的那一行，repo 中沒有新的 module

### Requirement: One commit per adoption
每次接管 SHALL 在 repo 建立一個 commit，訊息格式為 `chore(adopt): <摘要> from <來源檔> (<shell>) on <識別名稱>`，符合 Conventional Commits。

#### Scenario: Commit message
- **WHEN** 在 `mbp` 上接管 `~/.zshrc` 中 bun 的 PATH
- **THEN** repo 新增一個 commit，訊息為 `chore(adopt): add bun PATH from ~/.zshrc (zsh) on mbp`

### Requirement: Re-added content
安裝程式把已經接管過的內容再次寫回 rc 檔時，rosamta SHALL 認出它並直接依 host 設定移除，不產生新的 module，也不放進待處理清單。寫回原本的檔案時一律如此；寫進其他檔案時，只有對應的 module 在本機載入（已啟用且 `when` 符合本機）並且會在讀取該檔案的每個 shell 中生效，才 SHALL 視為已接管，否則照一般內容處理。

#### Scenario: Installer runs again
- **WHEN** 已接管的 bun PATH 被 bun 的安裝程式再次寫進 `~/.zshrc`
- **THEN** 下一次掃描直接移除這段，repo 中仍只有原本的 bun module

#### Scenario: Same line in another shell's file
- **WHEN** `~/.profile` 與 `~/.zshrc` 都有 `. "$HOME/.cargo/env"`，使用者只接管 `~/.zshrc` 的那一行
- **THEN** `~/.profile` 的那一行留在原處，並列入待處理清單

#### Scenario: Module for another host
- **WHEN** repo 中有一個只套用到其他主機的 module 設定 `FOO=1`，本機的 `~/.zshrc` 也有 `export FOO=1`
- **THEN** 本機的這一行不被視為已接管，列入待處理清單

### Requirement: Adopt everything at once
`rsmt adopt` SHALL 掃描並接管所有 watched file 中 hook 以外的內容，執行前列出將產生的 module 與 rc 檔的差異並要求確認，`--yes` 可略過確認。

#### Scenario: First adoption on a new host
- **WHEN** 使用者在安裝 hook 後執行 `rsmt adopt` 並確認
- **THEN** 所有 rc 檔只剩 hook，原本的設定都以 module 的形式存在 repo 中，新開的 shell 行為與接管前相同
