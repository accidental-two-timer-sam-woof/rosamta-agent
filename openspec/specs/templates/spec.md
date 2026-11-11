# templates Specification

## Purpose
讓同一份 dotfile 在每台 host 上 render 出該 host 需要的內容：依 host 的屬性與本機變數產生不同內容，必要時填入秘密來源提供的值。

## Requirements

### Requirement: Template syntax and sandbox
`deploy = "template"` 的 dotfile SHALL 以 Jinja 語法 render 成實體檔案。template SHALL NOT 能讀取檔案、include 其他 template 或執行任意指令；可用的函式只有 `bitwarden`、`secret`、`bitwarden_ssh_socket` 與內建的 filter 和 test。引用未定義的變數 SHALL 讓 render 失敗並指出行號。`rsmt validate` SHALL 回報 template 的語法錯誤與行號。

#### Scenario: Undefined variable
- **WHEN** template 寫了 `{{ host.ids }}`，使用者執行 `rsmt apply`
- **THEN** 該 dotfile 不部署，輸出指出檔案與行號，其他 dotfile 照常部署

#### Scenario: No file access
- **WHEN** template 使用 `{% include "other" %}`
- **THEN** render 失敗，目標維持原樣

### Requirement: Host variables
template SHALL 可使用 `host.id`、`host.os`（`macos` 或 `linux`）、`host.arch`（`aarch64` 或 `x86_64`）、`host.tags` 與 `host.home`（家目錄的絕對路徑）。

#### Scenario: Different email per host
- **WHEN** template 內容為 `{% if "work" in host.tags %}me@work.com{% else %}me@home.com{% endif %}`，本機標籤含 `work`
- **THEN** render 結果為 `me@work.com`

### Requirement: Host-specific variables
template SHALL 可使用 `vars.<name>`，值來自本機 `host.toml` 的 `[vars]`。template 引用本機沒有的變數時：終端機中的 `rsmt apply` SHALL 以變數名稱詢問並寫進 `host.toml` 且保留其中的註解，空白答案 SHALL 存成空字串；非互動執行與背景服務 SHALL 不 render 該 template、維持目標原樣並放進待處理清單。

#### Scenario: First apply asks
- **WHEN** template 引用 `vars.email`，本機 `host.toml` 沒有它，使用者在終端機執行 `rsmt apply` 並輸入 `me@work.com`
- **THEN** `host.toml` 的 `[vars]` 多了 `email = "me@work.com"`，template 以這個值 render，下次套用不再詢問

#### Scenario: Missing variable in the background
- **WHEN** 拉取到一個引用 `vars.email` 的新 template，本機沒有這個變數
- **THEN** 背景服務不部署它，待處理清單列出該路徑在等待變數 `email`

### Requirement: Render timing
終端機中的 `rsmt apply` SHALL render 每個套用到本機的 template。非互動執行、背景服務與待處理清單的處理 SHALL 只 render 輸入（template 內容、引用的 `host.*` 值、引用的變數值與引用的秘密指令宣告）改變的 template。render 結果與目標相同時 SHALL NOT 寫入；覆蓋前 SHALL 存 snapshot。沒有宣告權限時，新建的目標 SHALL 為 0600。

#### Scenario: Rotated token
- **WHEN** Bitwarden 中的 token 更新了，template 本身沒變，使用者在終端機執行 `rsmt apply`
- **THEN** 目標換成含新 token 的內容，舊內容保存在 snapshot 中

#### Scenario: Unchanged inputs in the background
- **WHEN** 背景服務因 repo 中其他檔案變動而套用，某個 template 的輸入沒有改變
- **THEN** 該 template 不重新 render，也不取用任何秘密

### Requirement: Local edits to rendered files
render 出來的目標被修改、偏好不是「關閉」時，SHALL 放進待處理清單（不論偏好是「自動」或「詢問」），因為內容無法寫回 template。`rsmt review accept` SHALL 把目前內容存成本機專用的 copy 宣告（`hosts/<識別名稱>/<路徑>`），沿用權限與加密，且只接受 `--scope host`；template 使用秘密來源、或內容看起來像秘密，而 template 沒有加密時 SHALL 拒絕。`rsmt review discard` SHALL 重新 render 並覆蓋。偏好為「關閉」時 SHALL 不處理，之後的套用也不覆蓋它。

#### Scenario: Keep a hand edit
- **WHEN** 使用者手動修改了 render 出來的 `~/.gitconfig`，再對該項目執行 `rsmt review accept`
- **THEN** repo 多了 `hosts/<識別名稱>/.gitconfig` 與限定本機的 copy 宣告，本機之後不再 render 這個 template

#### Scenario: Scope all on a template
- **WHEN** 使用者對 template 的項目執行 `rsmt review accept --scope all`
- **THEN** 指令說明 template 只能保留在本機並以 2 結束

### Requirement: Add as a template
`rsmt add --template <path>` SHALL 把檔案原樣收進 repo 並宣告為 `deploy = "template"`；可與 `--encrypt` 一起使用。目錄 SHALL 被拒絕。

#### Scenario: Start a template
- **WHEN** 使用者執行 `rsmt add --template ~/.gitconfig`
- **THEN** `home/.gitconfig` 的內容與原檔相同，`dotfiles.toml` 新增 `deploy = "template"` 的宣告，`~/.gitconfig` 是 render 出來的一般檔案
