# Design

## Context

`add-dotfile-deploy`、`add-templates-and-secrets` 與 `add-externals-and-scripts` 已經提供 chezmoi 各種功能在 rosamta 的對應：`dotfiles.toml` 的明確宣告（ADR 0007）、`link`／`copy`／`template`／`create` 部署、`permissions`、加密（ADR 0012）、minijinja template 與 `host`／`vars`／`bitwarden()`／`secret()`、`when` 套用範圍、`scripts.toml` 與 `externals.toml`。本 change 把一個 chezmoi source 目錄轉成這些宣告。

chezmoi 的 source 目錄把屬性編在檔名（`dot_`、`private_`、`executable_`、`.tmpl` 等，前綴有固定順序）、以 Go template 產生內容、以 `.chezmoiignore`（本身是 template）排除目標、以 `.chezmoiexternal.*` 宣告外部內容、以 `run_` 前綴宣告腳本。本機有 chezmoi 時，`chezmoi data` 與 `chezmoi dump-config` 提供這台機器的資料與 age 金鑰位置。

## Goals / Non-Goals

**Goals:**
- 一個指令把常見的 chezmoi repo 轉進 rosamta repo，轉換後在同一台主機 `rsmt apply` 的結果與 `chezmoi apply` 相同。
- 轉不了的部分不猜：不部署，並在報告中逐項列出原因與位置，使用者可以照著手動處理。
- 不修改 chezmoi 的 source 目錄與 chezmoi 的設定；匯入後兩者可以並存，直到使用者確認。

**Non-Goals:**
- 完整的 Go template 與 sprig 函式；只轉換下方列出的寫法。
- `modify_`、`remove_`、`symlink_`、`exact_`、`.chezmoiremove`、`.chezmoitemplates`、gpg 加密、YAML 格式的資料與 externals、`file` 與 `archive-file` 類型的 external；這些列在報告中。
- 匯入後刪除 chezmoi 的 source 或設定。
- 從其他 dotfile 管理工具匯入。

## Decisions

### 指令與流程

`rsmt import chezmoi [<source-dir>] [--dry-run] [--json]`：

1. 找到 source：參數，否則 `chezmoi source-path`，再否則 `~/.local/share/chezmoi`；有 `.chezmoiroot` 時改用它指向的子目錄。
2. 本機有 chezmoi 時讀取 `chezmoi data --format json` 與 `chezmoi dump-config --format json`（唯讀），取得這台機器的資料、`mode`、`umask` 與設定中的 `[data]`；沒有時不讀，依賴這些資料的項目列為需要手動處理。
3. 掃描 source，產生匯入計畫與報告。`--dry-run` 到此為止，不寫任何東西、不連網路。
4. 轉換後的 template 以本機的值 render，和 `chezmoi execute-template` render 原檔的結果比較（只 render 這份內容，不讀取 externals 與其他 template，也不連網路）；不同、或 chezmoi 的結果只有空白（chezmoi 不建立目標）時列為需要手動處理（用到秘密值的 template 不比較，避免解鎖密碼管理工具）。
5. 寫入 repo 的 `home/`、`scripts/`、`dotfiles.toml`、`scripts.toml`、`externals.toml`、`secrets.toml`，轉不了的 source 原樣複製到 repo 的 `chezmoi-import/`（不宣告、不部署），以及本機 `host.toml` 的 `[vars]`，建立一個 commit `import(chezmoi): …`，報告寫在 commit 內容中。匯入的腳本記為本機已執行過，因為 chezmoi 已在這台機器執行過它們。
6. 不部署、不連網路；使用者之後執行 `rsmt apply`。chezmoi 已經放好的檔案內容相同，不會變成衝突；例外見 externals。

repo 必須已經初始化（`rsmt init`），且上述路徑沒有尚未推送的變更（否則會一起進入匯入的 commit）。寫入或 commit 失敗時，這些路徑回到匯入前的內容；`[vars]` 與腳本的執行紀錄在 commit 成功後才寫入。source 中的 symlink 不讀取、不複製，列為需要手動處理；名稱是 `.` 或 `..` 的項目與指向 source 以外的 `.chezmoiroot` 使匯入以 2 結束，與 chezmoi 相同。`chezmoi-import/` 中已有的檔案不覆蓋。git 忽略的目標路徑（例如使用者全域 excludes 中的 `.env`）無法 commit，列為需要手動處理。匯入時才建立的 repo 金鑰與匯入一起 commit。App 匯入失敗時，移除這次建立的 repo 與 `host.toml`，回到設定頁。沒有終端機時 chezmoi 以 `--no-tty` 執行並有時間上限。

目標路徑已經在 `dotfiles.toml` 宣告、`home/` 中已有同名內容、`scripts/` 中已有同名腳本或 externals 已宣告同一路徑時，該項目略過並列在報告中，不覆蓋。

### 檔名屬性

依 chezmoi 的規則解析每個 source 項目：

- 以 `.` 開頭的項目忽略，`.chezmoi*` 特殊檔案另外處理。
- 目錄：前綴依序 `remove_`、`external_`、`exact_`、`private_`、`readonly_`、`dot_`。`external_` 目錄之下的名稱不再解析。
- 檔案：第一個前綴決定類型（`create_`、`modify_`、`remove_`、`run_`、`symlink_`，否則是一般檔案），之後依序 `encrypted_`、`private_`、`readonly_`、`empty_`、`executable_`、`dot_`；`literal_` 之後不再解析前綴。後綴 `.literal` 之後不再解析後綴；`.tmpl` 表示 template；加密的檔案去掉 `.age`。

每個檔案轉成一筆 dotfile 宣告（不合併目錄），因為 chezmoi 只管理列出的檔案，以目錄宣告會把目錄中其他檔案也納入 repo。例外是 `exact_` 目錄且其中都是一般檔案：整個目錄一筆 `copy`，rosamta 的 copy 會整個替換目錄，正是 exact 的效果。

部署方式依 chezmoi 的 `mode`：預設的 `file` 用 `copy`，`symlink` 用 `link`；目標與 `chezmoi apply` 的結果一樣是一般檔案，在主機上的修改照 copy 的規則進入待處理清單，不會直接改到 repo。`permissions` 依 chezmoi 的 `umask`（預設 022）計算：一般檔案 0644、`executable_` 加上執行權限、`private_` 去掉 group 與 other、`readonly_` 去掉寫入權限；與 rosamta 的預設不同時（template 與加密檔案預設 0600，copy 沿用 repo 中檔案的權限）寫入宣告。

| chezmoi | rosamta |
|---|---|
| 一般檔案 | `home/<目標>`，部署方式如上；`executable_` 時 repo 中的檔案為 0755 |
| `encrypted_` | 以 `chezmoi decrypt` 解密（支援 chezmoi 的各種加密設定），再以 rosamta 的金鑰加密，`encrypted = true` |
| `encrypted_` 加 `.tmpl` | 轉換後 `deploy = "template"`、`encrypted = true` |
| `create_` | `deploy = "create"`；`create_` 加 `.tmpl` 需要手動處理（rosamta 的 create 不 render） |
| `.tmpl` | 轉換後 `deploy = "template"`；見下方 |
| `empty_` | 照常匯入空檔案 |
| 內容只有空白、沒有 `empty_` | 不匯入：chezmoi 不建立這種目標（只有空白也算空）；列為略過 |
| `private_`、`readonly_` 目錄 | 檔案照常匯入；目錄權限列為需要手動處理（rosamta 建立的上層目錄是 0755） |
| `exact_` 目錄（含 template 等） | 檔案照常匯入；「移除未管理的檔案」列為需要手動處理 |
| 只有 `.keep` 的目錄 | 需要手動處理：chezmoi 會建立空目錄 |
| `symlink_`、`modify_`、`remove_`、`.chezmoiremove` | 不匯入，需要手動處理 |

### 套用範圍

每個匯入的項目帶一個「適用的平台」集合，元素是 {macos, linux} × {x86_64, aarch64} 的組合，起始為全部四個：

- `.chezmoiignore`（根目錄與子目錄的，模式相對於所在目錄的目標）：條件外的模式，符合的項目不匯入、列為略過；在 `{{ if <條件> }}` 區塊中的模式，從集合中去掉條件成立的組合。支援 `*`、`**`、`?` 與 `!` 例外。
- 整份內容被單一 `{{ if <條件> }}…{{ end }}` 包住的 template 或腳本：去掉外層，集合與條件取交集；剩下的內容沒有其他 template 寫法時改為一般檔案。這也保留了 chezmoi「內容為空就不建立」在這種寫法下的效果。

集合為空時不匯入（列為略過）；是全部時不寫 `when`；能寫成 `os` 與 `arch` 兩個清單的乘積時寫成 `when`；否則列為需要手動處理。條件支援下方 template 中的 OS 與架構比較；另外整份 template 被 `{{ if eq .chezmoi.hostname "a" … }}` 包住時轉成 `when.hosts`，`.chezmoiignore` 中 `{{ if ne .chezmoi.hostname "a" }}` 區塊的模式也轉成 `when.hosts`。其他用到 hostname 的 `.chezmoiignore` 區塊列為需要手動處理。

### Template 轉換

以一個小的解析器讀 Go template 的動作（`{{ }}`，含 `{{-`、`-}}` 去空白與 `{{/* */}}` 註解），只接受下列寫法，其他任何寫法都使整個檔案列為需要手動處理（報告附行號與寫法）：

| Go template | minijinja |
|---|---|
| `if`、`else if`、`else`、`end` | `{% if %}`、`{% elif %}`、`{% else %}`、`{% endif %}`，去空白標記照搬 |
| `eq a b c`、`ne`、`and`、`or`、`not`、括號 | `a == b or a == c`、`!=`、`and`、`or`、`not` |
| `.chezmoi.os` | 比較時 `host.os`，`"darwin"` 換成 `"macos"`；輸出時換回 chezmoi 的值 |
| `.chezmoi.arch` | 同上，`amd64`／`arm64` 對應 `x86_64`／`aarch64` |
| `.chezmoi.hostname` | 比較時 `host.id`：等於本機 hostname 的值換成本機的識別名稱（rosamta 的識別名稱可能和 hostname 不同），其他值以識別名稱的規則整理（`host::sanitize`），報告列出換過的值；輸出 hostname 需要手動處理 |
| `.chezmoi.homeDir` | `host.home` |
| `.<名稱>`（資料） | `.chezmoidata.toml`／`.json` 與 `.chezmoidata/` 中、設定的 `[data]` 沒有覆蓋的值直接代入（各主機相同）；其他是 `vars.<名稱>`，並以 `chezmoi data` 中這台機器的值寫入本機 `host.toml` 的 `[vars]`。`vars` 只有字串：布林值寫成 `"true"`／`"false"`，在條件中轉成 `vars.<名稱> == "true"`；其他型別需要手動處理 |
| `(bitwarden "item" "x").login.password`、`.login.username`、`.notes` | `bitwarden("x", "password")`、`"username"`、`"notes"` |
| `(bitwardenFields "item" "x").<欄位>.value` | `bitwarden("x", "<欄位>")` |
| `onepasswordRead "ref"` | `secret("op", "ref")`，並在 `secrets.toml` 宣告 `[commands.op] run = ["op", "read", "{arg}"]` |
| 字串、數字、布林常值 | 同樣的常值 |

- 文字中出現 `{{`、`{%`、`{#` 的片段包進 `{% raw %}`，避免被 minijinja 解讀（例如 shell 的 `${#array}`）。
- 以 `chezmoi:template:` 改了分隔符號或選項的檔案、巢狀的非靜態資料（`.a.b`）、變數（`$x :=`）、`range`、`with`、`template`、`include`、`output`、`lookPath` 與其他函式都列為需要手動處理。
- 轉換結果以 rosamta 的 template 檢查（`rsmt validate` 用的同一個）確認可以解析。

### 腳本

`run_[once_|onchange_][before_|after_]<名稱>[.tmpl]`（含 `.chezmoiscripts/` 中的）轉成 `scripts/<所在目錄>/<名稱>` 與一筆 `[[script]]`：

- 一律 `run = "onchange"`：chezmoi 的 `run_once_` 以內容判斷是否執行過（內容改變就再執行），與 rosamta 的 `onchange` 相同，而不是 rosamta 只執行一次的 `once`；沒有 `once_`／`onchange_`（chezmoi 每次套用都執行）也轉成 `onchange`，報告提醒。
- 匯入時記為本機已執行過，否則第一次 `rsmt apply` 會重新執行每個腳本。
- chezmoi 在腳本所在目錄對應的目標目錄執行它，rosamta 在家目錄執行；不在 source 根目錄的腳本報告提醒。用到 `CHEZMOI_` 環境變數的腳本需要手動處理。
- `before_` → `timing = "before"`，其餘 `after`。
- 依 chezmoi 的執行順序（source 名稱排序）宣告。
- template 腳本：整份被單一條件包住時轉成 `when`；`{{ include "<source 路徑>" | sha256sum }}` 是 chezmoi 讓腳本跟著檔案重新執行的慣用寫法，轉成 `watch`（指向該檔案匯入後在 repo 的位置）並刪去該動作；其他 template 寫法列為需要手動處理，因為 rosamta 的腳本不是 template。

### Externals

讀取 `.chezmoiexternal.toml`、`.chezmoiexternal.json` 與 `.chezmoiexternals/` 中同格式的檔案，路徑相對於所在目錄的目標。內容含 template 動作的檔案整份列為需要手動處理。寫入前以 rosamta 的規則檢查路徑互相包含（例如 archive external 中的 external 或 dotfile），不符合的列為需要手動處理。

| chezmoi | rosamta |
|---|---|
| `type = "git-repo"` | `git = <url>`；`clone.args` 中的 `--branch`／`-b` 成為 `branch`；`rev` 鎖定在目標中 chezmoi 已 clone 的 HEAD（不連網路，也不會在第一次套用時移動），沒有 clone 時不鎖定，報告提醒執行 `rsmt externals update` |
| `type = "archive"` | `archive = <url>`，`stripComponents` → `strip`，`checksum.sha256` → `sha256`；目標中 chezmoi 解壓的內容沒有 rosamta 的紀錄，第一次套用會列為衝突，報告提醒 |
| `refreshPeriod` | 忽略；rosamta 以 `rsmt externals update` 更新鎖定的版本，報告提醒 |
| `file`、`archive-file`、`include`、`exclude`、`filter`、`encrypted`、`decompress`、`urls`、tar.gz／tar／zip 以外的 `format` | 列為需要手動處理 |
| `exact` | 忽略，報告提醒 |

### 加密

`encrypted_` 檔案以 `chezmoi decrypt` 解密，再以 rosamta 的金鑰加密：

- 由 chezmoi 解密，所以 chezmoi 支援的設定（age 金鑰、以密碼保護的金鑰、對稱加密、gpg）都可以匯入；chezmoi 需要密碼時在終端機詢問。
- repo 還沒有金鑰時，在終端機詢問新密碼（兩次），流程與 `rsmt add --encrypt` 相同；已有金鑰但本機未解開時，詢問密碼解開。
- 不沿用 chezmoi 的金鑰（proposal 原本的做法）：rosamta 的 repo 只有一把金鑰，可能已經存在；重新加密後只需要 rosamta 的金鑰，chezmoi 的金鑰不再需要。
- 沒有終端機、沒有 chezmoi 或解密失敗時，這些檔案列為需要手動處理。

### 報告

每個項目有 source 路徑、結果與原因代碼（不是句子），CLI 與 App 各自以自己的文字顯示，`--json` 輸出代碼：

- 匯入：dotfile（含部署方式）、腳本、external。
- 略過：`.chezmoiignore` 排除、內容為空、不適用任何平台、已存在。
- 需要手動處理：原因代碼與細節（例如 template 的行號與寫法、hostname 的值）。
- 提醒：例如 hostname 改為主機識別名稱、每次執行的腳本改為 `onchange`、`refreshPeriod`。

CLI 以 2 結束代表 source 或 repo 有問題（找不到、未初始化）；有需要手動處理的項目仍以 0 結束，因為匯入本身完成了。

### App

設定精靈在偵測到 chezmoi 的 source 時多一個「從 chezmoi 匯入」：建立新的 repo 後執行匯入（不詢問密碼，加密的檔案列為需要手動處理），顯示匯入、略過與需要手動處理的數量，以及需要手動處理的清單（路徑與原因）。

### 驗證

- 屬性解析、`.chezmoiignore`、平台集合、template 轉換以單元測試涵蓋。
- 整合測試以固定的 chezmoi source fixture 匯入，檢查產生的宣告、腳本、externals、`[vars]` 與報告。
- 開發機有 chezmoi 時，另一組測試在暫存的家目錄分別執行 `chezmoi apply` 與匯入後的 `rsmt apply`，比較兩邊的檔案內容與權限；沒有 chezmoi 時略過。

## Risks / Trade-offs

- [只支援常見的 template 寫法] → 其他寫法整份列為需要手動處理，不產生半對的結果；報告附行號。
- [hostname 換成主機識別名稱] → 本機的值換成本機的識別名稱；其他主機的值是推測，報告列出，使用者在那台主機設定相同的識別名稱或修改 template。
- [靜態資料直接代入] → `.chezmoidata` 的值之後要改時需要改每個 template；換來不必為共用資料新增一種設定。
- [重新加密需要密碼] → 只在終端機的匯入詢問；App 列為需要手動處理。
- [只有在某些主機為空的 template] → chezmoi 在內容為空時不建立目標，rosamta 會寫入空檔案；整份單一條件的寫法已轉成 `when`，其他所有內容都在條件中的 template 報告提醒。
- [archive external 第一次套用是衝突] → 目標已有 chezmoi 解壓的內容，rosamta 不覆蓋沒有紀錄的內容；報告提醒，使用者移除後由 rosamta 下載。

## Migration Plan

新功能，沒有既有資料要遷移。使用者匯入、檢查報告與 `rsmt apply --dry-run` 後，自行決定何時停用 chezmoi。

## Open Questions

（無）
