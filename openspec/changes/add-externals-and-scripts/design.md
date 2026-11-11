# Design

## Context

`add-dotfile-deploy` 建立了 `dotfiles.toml` 的明確宣告（ADR 0007）、部署紀錄、待處理清單的 dotfile 項目與 snapshot；`add-templates-and-secrets` 確立「只有在終端機執行的 `rsmt apply` 會提示使用者」。本 change 沿用這些機制加入 externals、scripts 與 `create` 部署方式。套用流程目前是：render、smoke test、切換 generated script，然後部署 dotfile；`rsmt apply`、GUI 的套用、拉取後的套用與背景服務都走這條路。

## Goals / Non-Goals

**Goals:**
- 在 repo 中宣告 oh-my-zsh、zsh 插件、tmux 插件、字型等外部內容，每台 host 下載到同一個鎖定的版本。
- 新 host 第一次 `rsmt apply` 就執行完設定腳本；之後只在需要時再執行。
- 背景服務不會在使用者不知情時執行腳本（預設），也不覆蓋使用者在 host 上修改過的外部內容。

**Non-Goals:**
- 單一檔案的下載（chezmoi 的 `file` 類型）與 `.tar.xz`、`.tar.bz2` 等其他壓縮格式；需要時再加。
- 腳本作為 template render、腳本的秘密值。
- 每次套用都執行的腳本（chezmoi 的 `run_`）；需要時以 `onchange` 搭配 `watch` 表達。
- 重新執行已完成的 `once` 腳本的指令、`rsmt add --create`。
- GUI 中 externals 的列表。

## Decisions

### externals 的宣告

repo 根目錄的 `externals.toml`，每筆 `[[external]]`：

```toml
[[external]]
path = ".oh-my-zsh"                                 # 相對家目錄
git = "https://github.com/ohmyzsh/ohmyzsh.git"
rev = "3c2f4a1…"                                    # 完整 commit SHA，由 rsmt externals update 寫入
branch = "master"                                   # 可選：update 追蹤的分支，預設遠端 HEAD

[[external]]
path = ".local/share/fonts/JetBrainsMono"
archive = "https://github.com/…/JetBrainsMono.zip"
sha256 = "9a0c…"                                    # 由 rsmt externals update 寫入
strip = 0                                           # 可選：去掉開頭幾層目錄
when = { os = ["linux"] }
```

- `git` 與 `archive` 恰好一個。鎖定值（`rev`、`sha256`）缺少時 `rsmt validate` 回報、套用略過該路徑並以 1 結束；`rsmt externals update` 補上。不在套用時自動鎖定，因為套用不應改 repo。
- 壓縮檔格式依內容判斷（gzip 的 tar、tar、zip），不看網址副檔名。
- 路徑互相包含時，只允許「在 `git` external 之內」的 external 或 dotfile（例如 `.oh-my-zsh/custom/plugins/zsh-autosuggestions`，oh-my-zsh 的 `custom/` 不受 git 追蹤）；其他情況 `rsmt validate` 回報、兩者都不部署。理由：external 寫進 `link` 部署的目錄會直接改到 repo，寫進壓縮檔或 copy 的目錄會被當成本機修改。
- 外層 `git` external 還不在目標時（下載失敗、衝突），其中的 external 與 dotfile 這次略過，避免先建立出一個普通目錄，讓外層之後永遠衝突。外層先於內層處理。
- 同一路徑多筆宣告符合本機時，沿用 dotfile 的優先順序。

### externals 的下載與更新

- 暫存目錄建在目標的上層目錄（同一個檔案系統，改名是原子的），名稱帶固定前綴，下次套用時清掉中斷留下的。上層目錄不存在時先建立。
- git：在暫存目錄 `git init`、`fetch --depth 1 origin <rev>`，失敗時（伺服器不允許以 SHA 取得）改為完整 fetch，`checkout --detach <rev>`，完成後改名到目標。使用使用者的 `git` 與憑證設定，沿用既有 git 呼叫的 `GIT_TERMINAL_PROMPT=0`，stdin 為 `/dev/null`。
- 壓縮檔：用既有的 HTTP client 下載到暫存檔（逾時、上限 1 GiB），先驗證 SHA-256 再解析。解壓時：
  - `strip` 去掉每個路徑開頭的幾層，層數不足的項目略過，什麼都不剩時失敗。
  - 拒絕 `..`、絕對路徑、hard link 與裝置、FIFO 等特殊項目；symlink 只接受相對且不跳出目標的，並在所有一般檔案之後才建立，所以不會有檔案經由 symlink 寫到外面。
  - 去掉 setuid、setgid 與 sticky 位元。
- 替換壓縮檔：新版解壓完成後，舊目標改名到暫存名稱、新版改名到目標、再刪除舊的，中間沒有目標不存在的時刻以外的空窗。
- 本機紀錄（狀態目錄 `externals.json`）：路徑、種類、網址、鎖定值；壓縮檔另記解壓後的內容雜湊（沿用 dotfile 的目錄雜湊：排序的相對路徑、類型、內容與 symlink 目標）。

套用時每個 external 的處理：

| 本機狀態 | 鎖定值未變 | 鎖定值改變 |
|---|---|---|
| 目標不存在 | 下載 | 下載 |
| git，工作目錄乾淨 | HEAD 不是 `rev` 時 checkout | fetch 後 checkout |
| git，有未 commit 的修改 | 不動、回報「本機已修改」 | 同左 |
| 壓縮檔，內容與紀錄相同 | 不動 | 解壓新版、換掉舊版 |
| 壓縮檔，內容被修改 | 不動 | 不動、回報「本機已修改」 |
| 沒有紀錄、目標已存在 | git 且 `origin` 相同：視為已部署；壓縮檔且內容與新解壓的相同：記錄；其他：不動、回報衝突 |

- git 的「乾淨」不計未追蹤的檔案（checkout 不會刪除它們；oh-my-zsh 的 `custom/` 也是 git 忽略的）。HEAD 已在 `rev` 時不檢查工作目錄，套用不必每次跑 `git status`。
- checkout 讓 HEAD 從其他 commit 回到 `rev`（例如 oh-my-zsh 自己更新過）時，輸出說明移回鎖定的版本。
- git 的網址改變、有紀錄且工作目錄乾淨時，改 `origin` 再 fetch；沒有紀錄時依上表的 `origin` 比對。
- 舊版壓縮檔內容只在與紀錄相同時才刪除，所以不必存 snapshot。
- 不再宣告的 external 從紀錄移除，目標留在原地（與 dotfile 的 forget 一致）。
- 每個 external 的失敗只影響自己，記在套用結果中而不是讓整個套用失敗：`rsmt apply` 列出並以 1 結束，拉取仍記為成功，背景服務寫進紀錄。

`rsmt externals update [<path>...]`：對所有宣告（不看 `when`，因為鎖定值是所有 host 共用的）或指定路徑，git 以 `git ls-remote` 取 `branch`（預設 HEAD）最新的 commit，壓縮檔重新下載計算 SHA-256；以 toml_edit 寫回、一個 commit（依推送偏好推送），再部署本機適用的 external。

### scripts 的宣告

repo 根目錄的 `scripts.toml`，每筆 `[[script]]`：

```toml
[[script]]
source = "scripts/install-tools.sh"   # 相對 repo
run = "once"                          # once | onchange，必填
timing = "before"                     # before | after（部署 dotfile 之前或之後），預設 after
watch = ["Brewfile"]                  # 只用於 onchange：這些 repo 路徑改變也算改變
when = { os = ["macos"] }
```

- 腳本以 `source` 識別；同一個 `source` 只能宣告一次。改名視為新的腳本。
- `once`：本機成功執行過一次後就不再執行，之後修改內容也不再執行。失敗不算執行過。
- `onchange`：腳本內容與 `watch` 的內容（目錄沿用 dotfile 的目錄雜湊，symlink 不跟隨）的雜湊與上次成功執行時不同才執行；從沒執行過也算改變。`when`、`timing` 改變不算。
- 執行方式：讀第一行的 shebang，以該直譯器執行（不需要執行權限）；沒有 shebang 時用 `sh`。工作目錄為家目錄，另提供 `ROSAMTA_REPO` 與 `ROSAMTA_HOST_ID` 環境變數。終端機中繼承使用者的環境；背景服務的環境本來就取自使用者的 login shell，所以 `brew` 等在 PATH 上。
- 同一時機的腳本依宣告順序執行。某個腳本失敗不影響其他步驟：不記錄為已執行，下次再試，`rsmt apply` 以 1 結束；與 external 一樣記在套用結果中，不讓拉取失敗。
- 套用本來就持有 adopt 的鎖，所以背景服務與終端機不會同時執行同一個腳本。

### 套用順序

externals → `before` 腳本 → render、smoke test、切換 → dotfile → `after` 腳本。

- externals 在 smoke test 之前：新 host 上 oh-my-zsh 的 module 需要 `~/.oh-my-zsh` 已存在，否則 smoke test 會把它當成新錯誤而中止套用。
- `before` 腳本也在 smoke test 之前：安裝 shell 會用到的工具（例如 starship）的腳本，讓第一次套用就通過檢查；這也與 chezmoi 的 `before_` 在寫入檔案之前執行一致。
- smoke test 失敗而中止時，dotfile 與 `after` 腳本都不處理（與現有規則一致）；已下載的 external 與已執行的 `before` 腳本保留。
- `rsmt apply`、GUI 套用與背景服務的套用走 `apply`；拉取在 `apply_after` 之前先處理 externals 與 `before` 腳本、之後在 `with_dotfiles` 部署 dotfile 與 `after` 腳本。接管（adopt）造成的重新 render 不處理 externals 與腳本。

### 誰會執行腳本

- 在終端機執行的 `rsmt apply`：需要執行的腳本直接執行，執行前印出腳本與原因，stdin 接終端機（可以 sudo、可以詢問）。理由：與 template 一致，終端機的 `rsmt apply` 是唯一可以互動的執行，也是新 host 的第一步。不另外確認：repo 的片段 module 本來就在每次 shell 啟動時執行，腳本沒有擴大能執行的程式碼範圍，確認只會讓新 host 的第一次套用多一步。
- 其他情況（沒有終端機的 `rsmt apply`、GUI、拉取、背景服務）依 `host.toml` 的 `scripts`：
  - `ask`（預設）：放進待處理清單，背景服務通知一次。
  - `auto`：直接執行，stdin 為 `/dev/null`；失敗的腳本放進待處理清單並通知。
- 待處理清單的腳本項目：`accept` 執行（在終端機時接終端機），成功後移除；`ignore` 記錄為已執行但不執行（`onchange` 到下次改變為止）；`discard` 拒絕。項目以 `source` 識別，內容再改變時沿用同一個 ID。
- 項目記下列出時的雜湊；`accept` 時腳本已不同（例如之間拉取了新版本），拒絕執行並更新項目，讓使用者看到的就是會執行的內容。
- 每次套用重新整理腳本項目：宣告移除、`when` 不再符合、已在其他套用中執行的項目都移除。

### 輸出紀錄

每次執行把 stdout 與 stderr（同一個 pipe，保留順序）寫進狀態目錄 `scripts/<source>.log`，覆寫上一次；終端機中同時顯示。開頭記錄時間與指令，結尾記錄結束碼。目錄 0700、檔案 0600（輸出可能含秘密）；超過 1 MiB 的部分不寫入、註明已截斷。GUI 待處理項目的詳細內容：還沒執行過時顯示腳本內容，執行失敗後顯示紀錄。

終端機中的輸出經過 pipe，所以腳本的 stdout 不是終端機（顏色、進度條可能改變）；sudo 從 `/dev/tty` 讀密碼，不受影響。用 pty 需要處理 raw mode 與視窗大小，不值得。

### `create` 部署方式

`deploy = "create"`：目標不存在時把來源（加密的先解密）寫到目標並設定 `permissions`；目標存在時完全不碰。沒有部署紀錄、不監看、不進待處理清單；使用者刪掉目標後，下次套用會再建立。不能搭配 `template`（`create` 原樣複製）。`rsmt forget` 不動目標。

### `host.toml`

新增 `scripts = "ask" | "auto"`，預設 `ask`；GUI 設定頁可切換。

## Risks / Trade-offs

- [以 SHA fetch 不被所有 git 伺服器支援] → 失敗時改為完整 fetch。
- [oh-my-zsh 等工具自己更新會移動 HEAD] → 工作目錄乾淨時套用會 checkout 回鎖定的版本並說明；README 建議關閉工具的自動更新、改用 `rsmt externals update`。
- [external 的問題在背景時只寫進紀錄] → 下一次 `rsmt apply` 會列出；GUI 的 externals 列表不在本 change。
- [`before` 腳本失敗時，dotfile 仍照常部署] → 與「單項失敗不影響其他」一致；需要時再加 `required`。
- [壓縮檔每次套用不重新雜湊] → 鎖定值改變時才比對，套用保持快速；鎖定值不變時本機的修改不會被發現，但也不會被覆蓋。
- [背景服務下載大型 external 會佔住背景服務一段時間] → 只在鎖定值改變時下載，與拉取本身的網路操作相同等級。
- [`auto` 的腳本在背景執行很久] → 不設逾時；stdin 為 `/dev/null`，需要輸入的程式（例如 sudo）會立刻失敗而不是卡住。
- [腳本是遠端拉來的程式] → 預設 `ask`，項目詳細內容顯示腳本本身，使用者看過才執行。
