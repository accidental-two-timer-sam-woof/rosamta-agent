# Proposal

## Why

接管把 rc 檔的內容搬進「階段」之後，「shell 在什麼情況下讀那個檔」這個資訊就不見了，rosamta 只能靠靜態分析猜回來，猜錯的地方就是接管前後行為不同的地方。目前有五個 issue 與一個還沒開的問題都是同一個原因：

- #34：`.bash_profile` 的內容在非 login、甚至非互動的 bash 執行（這個變更只解決片段；接管成結構化 module 的 `export` 等行見 design 的 Open Questions 3）。
- #35：巢狀 shell 失去 `.zshenv`、`.bash_profile` 中沒有 export 的設定。
- #36：`.zprofile` 以 `emulate sh` 執行 `.profile` 時，內容跑兩次。
- #37：Fedora、Arch 的 login bash 把 `.bashrc` 跑兩次。
- #38：套用後 login bash 多讀了 `.bashrc`。
- `.zlogin` 的內容放在互動階段，所以在非 login 的 zsh 也會執行。

逐一修補會讓每個 issue 各長出一個特例。這個變更改掉根因：接管來的片段記住它來自哪個檔，並且只在 shell 讀那個檔時載入。

## What Changes

- 接管來的片段 module 以 `file` 記錄來源檔（例如 `file = ".zprofile"`），取代 `phase`。它只從那個檔的 hook 載入，順序與原本在檔案中的順序相同。
- 每個 watched file 都有自己的 hook，包含 `.zlogin` 與 `.bash_login`。
- `ROSAMTA_ENV` 只擋結構化的環境變數（引用自己的值不能重複套用）；片段在每個 shell 程序都執行，和接管前一樣。
- `.profile` 的片段只產生在 sh 的 script 中，由 `.profile` 的 hook 載入；bash 與 zsh 讀 `.profile` 時自然會載入，不再各自複製一份。
- 執行其他啟動檔的那幾行（例如 `. ~/.bashrc`、`. ~/.profile`）不接管，留在原處。
- **BREAKING** rosamta 不再替既有的 `.bash_profile` 加上執行 `.bashrc` 的那一行。login bash 的互動設定改由 `.bash_profile` 的 hook 直接載入 rosamta 自己的互動 script；使用者原本的 `.bashrc` 是否在 login bash 執行，維持接管前的樣子。
- **BREAKING** repo 格式升為 2：既有的接管片段由 migration 補上 `file`。較舊的 rosamta 會拒絕格式 2 的 repo。

## Capabilities

### Modified Capabilities

- `adopt`: 片段依來源檔載入；接線的那幾行不接管。
- `shell-hooks`: 每個 watched file 一個 hook；不再修改使用者的接線。
- `shell-loader`: 巢狀 shell 重新執行 env 階段的片段（已完成）。
- `shell-render`: 片段 module 的 `file` 欄位與對應的 generated script。
- `config-repo`: 格式 2 與 migration。

## Impact

- `crates/core/src/adopt/segment.rs`、`adopt/mod.rs`：來源檔取代階段；`profile_readers` 的靜態分析可以刪除。
- `crates/core/src/render.rs`、`generated.rs`：每個來源檔一份 script。
- `crates/core/src/hooks.rs`：hook 的位置與內容。
- `crates/core/src/config.rs`、`repo.rs`：`file` 欄位、格式 2、migration。
- 你自己的 Mac 有 36 個接管的 module，升級時會被 migration 改寫一次，並產生一個 commit。
