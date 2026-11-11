# Design

## Context

目前每個 watched file 對應一個階段（`adopt/segment.rs` 的 `watched_files`）：`.profile`、`.bash_profile`、`.bash_login`、`.zshenv` 是 env，`.zprofile` 是 login，`.bashrc`、`.zshrc`、`.zlogin` 是 late。每個 shell 產生 `env`、`profile`、`rc` 三份 script，bash 的 `.bashrc` hook 也載入 `env.bash`，env 階段再用 export 出去的 `ROSAMTA_ENV` 擋重複。沒有 `.bash_profile` 執行 `.bashrc` 的那一行時，rosamta 會自己加上。

這個模型對 rosamta 自己產生的內容（結構化 module、使用者寫的 module）是對的：它們要在每個互動 shell 生效。對接管來的片段是錯的：片段是使用者原本的啟動程式碼，它的語意就是「所在的檔被讀到時執行」。

## Goals / Non-Goals

**Goals**

- 接管一段片段之後，每一種啟動方式（login、非 login、互動、非互動、巢狀）下它被執行的次數與時機，和接管前相同。
- 不需要靜態分析「哪個 shell 會讀哪個檔」。

**Non-Goals**

- 不改變結構化 module 與使用者自己寫的 module 的載入方式。
- 不處理部分接管時，留在檔案裡的行與被接管的行之間的先後順序（現在也是 hook 所在的位置決定）。

## Decisions

### 1. 片段記錄來源檔，而不是階段

接管產生的片段 module 寫入 `file`，值是 `.profile`、`.bash_profile`、`.bash_login`、`.bashrc`、`.zshenv`、`.zprofile`、`.zshrc`、`.zlogin`、`config.fish` 之一。`file` 與 `phase` 不能同時出現；`file` 只能用在只有 `raw` 的 module。

為什麼不沿用階段再修正對應：階段是「在啟動流程的哪一步」，來源檔是「shell 讀到這個檔時」。兩者在 zsh 幾乎一對一，在 bash 不是（`.bash_profile` 與 `.bashrc` 何時被讀取決於使用者的接線），所以用階段表達一定要再猜一次接線。

### 2. 每個來源檔一份 generated script，由那個檔的 hook 載入

`generated/<版本>/<shell>/files/<檔名>.<副檔名>`，內容是 `file` 等於該檔的片段，依 `order` 排列。檔案 F 的 hook 在原本的階段 script 之後載入 F 的那一份，不加任何條件。

`.profile` 的片段只產生在 `sh/files/profile.sh`。bash 沒有 `.bash_profile` 時讀 `.profile`，zsh 以 `emulate sh` 讀 `.profile`，都會經過 `.profile` 的 hook，所以不需要 `NOT_BASH`，也不需要 `profile_readers`。

### 3. 拿掉 `ROSAMTA_ENV`

結構化內容重複載入沒有副作用：環境變數是覆寫，PATH 的 prepend 已經會跳過既有的項目，alias 是重新定義。片段則應該是檔案被讀幾次就執行幾次。兩者都不需要跨程序的防重複。

代價：巢狀 shell 會重新套用結構化的環境變數，蓋掉使用者在外層 shell 手動改過的值。這和沒有 rosamta 時 rc 檔的行為相同。

### 4. 接線留在檔案裡，rosamta 不替使用者加

執行其他啟動檔的行（`. ~/.bashrc`、`. ~/.profile`、`emulate sh -c '. ~/.profile'`）不接管。這是 `fix/bash-profile-wiring` 已經做的部分。

rosamta 不再替既有的 `.bash_profile` 加 `. ~/.bashrc`。為了讓 rosamta 自己的互動內容在 login bash 仍然生效，`.bash_profile` 的 hook 在互動 shell 中直接載入 `rc.bash`，並用一個**不 export** 的變數避免同一個程序在 `.bashrc` 的 hook 再載入一次。rosamta 新建 `.bash_profile` 時，仍然讓它執行 `.profile` 與 `.bashrc`，因為這時沒有既有的行為可以保留。

### 5. 格式 2 與 migration

既有的接管片段沒有 `file`。migration 從接管紀錄與 module 開頭的 `# Adopted from <檔案> on <主機>` 取回來源檔，寫入 `file` 並移除 `phase`；兩者都找不到時保留 `phase`，行為與現在相同，並在 `rsmt validate` 列出。

## Risks / Trade-offs

- **login bash 的行為改變**：原本靠 rosamta 加的那一行才在 login bash 執行 `.bashrc` 的使用者，升級後 `.bashrc` 中沒有接管的內容不再於 login bash 執行。這正是 #38 回報的問題的另一面，需要你決定（見 Open Questions）。
- **migration 改寫使用者的 repo**：在你的 Mac 上會改 36 個 module。migration 前後各比對一次每個 shell、每種啟動方式的環境與 alias，不同就停止並還原。
- **跨主機**：`file = ".zprofile"` 的 module 套用到沒有 `.zprofile` 的主機時，rosamta 會建立只含 hook 的 `.zprofile`，和現在相同。
- **測試量**：`crates/cli/tests/adopt.rs` 與 9 個發行版的 docker 測試都要重跑；#34–#38 各補一個重現的測試。

## Migration Plan

1. 新版讀到格式 1 的 repo 時，在第一次 `rsmt apply` 執行 migration，產生一個 commit。
2. 失敗時 repo 與 generated script 維持原狀，並回報無法判斷來源檔的 module。
3. 還原方式：`git revert` 那個 commit 並改回舊版。

## Open Questions

1. **login bash 要不要執行使用者的 `.bashrc`？** 建議：不要替使用者決定，維持接管前的樣子（上面的決定 4）。另一個選擇是保留現在「一律執行」的規則，並把同時出現在兩個檔的行視為同一行，#38 要另外處理。
2. **`file` 的值要不要包含 fish 的 `conf.d/<名稱>.fish`？** 建議：先只支援 `config.fish`，`conf.d` 的檔案維持整檔接管的現狀。
