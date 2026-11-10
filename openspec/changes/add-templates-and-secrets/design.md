# Design

## Context

ADR 0006：template 用 minijinja，在沙箱中執行，可用的函式由 rosamta 明確提供。`add-dotfile-deploy` 已建立 `deploy = "template"` 的宣告（目前套用時略過）、`deployed.json` 部署紀錄、待處理清單的 dotfile 項目、加密與 snapshot；template 沿用這些機制。已決定的細節：

- 以指令取值時只能執行 repo 中宣告過、有名稱的指令。
- 背景服務永遠不跳出解鎖或輸入提示。

## Goals / Non-Goals

**Goals:**
- 同一份 template 在每台 host 上 render 出該 host 需要的內容，例如工作電腦的 git email。
- template 能取得密碼管理工具中的秘密，但秘密不進 repo、rosamta 也不保存秘密的值。
- 新 host 第一次套用時，缺少的變數與鎖住的秘密來源都在終端機中一次問完。

**Non-Goals:**
- 相容 chezmoi 的 Go template（ADR 0006）；匯入時的轉換由 `add-chezmoi-import` 處理。
- template 之間的 include、import 與 extends：沙箱不提供讀檔。
- 設定 `SSH_AUTH_SOCK` 的 module：由 `add-providers-and-doctor` 的 provider 使用本 change 的路徑偵測。
- 除錯用的單獨 render 指令（類似 `chezmoi execute-template`）。

## Decisions

### Template 的位置與宣告

template 就是 `deploy = "template"` 的 dotfile，來源仍在 `home/<path>`（或 `source`），檔名不加副檔名（ADR 0007）。`encrypted = true` 可以搭配 template：先解密再 render。template 只能是檔案。`rsmt add --template <path>` 把現有檔案原樣收進 repo 並宣告為 template，之後由使用者把要變動的部分改成變數。

### 變數

| 變數 | 內容 |
|---|---|
| `host.id` | 識別名稱 |
| `host.os` | `macos` 或 `linux` |
| `host.arch` | `aarch64` 或 `x86_64` |
| `host.tags` | 標籤陣列 |
| `host.home` | 家目錄的絕對路徑 |
| `vars.<name>` | 本機 `host.toml` 的 `[vars]` |

變數值只存在本機的 `host.toml`，repo 不另外宣告變數。render 前以 minijinja 的靜態分析列出 template 引用的 `vars.*`；本機沒有的變數：

- 終端機中的 `rsmt apply`：以變數名稱逐一詢問，答案寫進 `host.toml` 的 `[vars]`（保留檔案中的註解）。空白答案存成空字串，之後不再詢問。
- 非互動執行與背景服務：不 render 這個 template，目標維持原樣，放進待處理清單。

未定義的變數一律視為錯誤（minijinja 的 strict 模式），打錯字時 render 失敗並指出行號，而不是輸出空字串。

### 函式

只提供以下三個，加上 minijinja 內建的 filter 與 test：

```jinja
{{ bitwarden("GitHub") }}                 {# 密碼 #}
{{ bitwarden("GitHub", "username") }}     {# username、notes、totp 或自訂欄位名稱 #}
{{ secret("op", "op://Private/GitHub/token") }}
{% if bitwarden_ssh_socket() %}IdentityAgent {{ bitwarden_ssh_socket() }}{% endif %}
```

同一次套用中，相同參數的秘密只取一次；值只留在記憶體，不寫入任何檔案或紀錄。

### Bitwarden

`bitwarden(item, field)` 依序嘗試 rbw → bw → bws，使用第一個已安裝且已登入的工具；順序可用 `host.toml` 的 `[secrets] bitwarden = ["bw", "rbw"]` 覆寫，GUI 不顯示這項設定。

| 工具 | 已登入 | 鎖住的判斷 | 取值 |
|---|---|---|---|
| rbw | 已設定 email | `rbw unlocked` 失敗 | `rbw get [--field <f>] <item>` |
| bw | `bw status` 不是 `unauthenticated` | 狀態是 `locked` | `bw get item <item>` 的 JSON |
| bws | keychain 或環境變數中有 access token | 不會鎖住 | `bws secret list` 中 key 等於 item 的值 |

- 鎖住時，終端機中的 `rsmt apply` 執行該工具的解鎖指令（`rbw unlock`、`bw unlock --raw`），讓工具自己詢問主密碼；bw 回傳的 session key 只用於這次套用。非互動執行與背景服務不解鎖。
- bws 的 `field` 只接受省略（值）或 `note`。
- rbw 與 bw 找不到完全同名的項目時會改用部分符合；rosamta 只接受名稱或 ID 完全相同的項目，bw 以 `bw list items --search` 列出後挑出同名者，同名的有多個時失敗。bws 同一個 key 有多個秘密時也失敗。
- `totp` 取的是存在項目中的種子（base32 或 `otpauth://`），不是當下的驗證碼。
- `rsmt secrets bws login` 以不回顯的方式讀入 access token，存進系統 keychain；`rsmt secrets bws logout` 刪除它。執行 bws 時以環境變數傳入 token，不寫進命令列。測試環境以 `ROSAMTA_NO_KEYCHAIN=1` 改用檔案。

### 以指令取值

repo 根目錄的 `secrets.toml` 宣告可用的指令：

```toml
[commands.op]
run = ["op", "read", "{arg}"]

[commands.pass]
run = ["pass", "show", "{arg}"]
```

- `secret(name, arg)` 把 `run` 每個元素中的 `{arg}` 換成參數後直接執行（不經 shell），取 stdout 並去掉結尾的一個換行。指令失敗時 render 失敗，訊息包含指令名稱與 stderr 的最後一行。
- 只在終端機中的 `rsmt apply` 執行，並讓指令使用終端機（例如 `op` 要求 Touch ID）。背景服務與非互動執行不執行任何秘密指令，因為無法保證指令不跳出提示。
- `rsmt validate` 檢查 `secrets.toml` 的格式；`rsmt schema secrets` 輸出 JSON Schema。template 呼叫不存在的名稱時 render 失敗。

### Bitwarden SSH agent 的 socket

`bitwarden_ssh_socket()` 依安裝方式回傳 socket 路徑，沒裝 Bitwarden 桌面版時回傳空值：

| 安裝方式 | 判斷 | socket |
|---|---|---|
| Mac App Store | `~/Library/Containers/com.bitwarden.desktop` 存在 | 該目錄下 `Data/.bitwarden-ssh-agent.sock` |
| snap | `~/snap/bitwarden` 存在 | `~/snap/bitwarden/current/.bitwarden-ssh-agent.sock` |
| flatpak | `~/.var/app/com.bitwarden.desktop` 存在 | `~/.var/app/com.bitwarden.desktop/data/.bitwarden-ssh-agent.sock` |
| brew、dmg、deb、rpm、AppImage | 找得到 Bitwarden 桌面版 | `~/.bitwarden-ssh-agent.sock` |

偵測放在 core 的獨立函式，之後的 provider 直接使用。

### 何時 render

部署紀錄中，template 的「repo 雜湊」改為輸入的雜湊：template 內容（加密時為解密後）、`host.*`、引用到的 `vars.*` 的值，以及引用到的 `secrets.toml` 指令宣告。秘密的值不在其中。

- 終端機中的 `rsmt apply`：每次都 render 所有套用到本機的 template，秘密在密碼管理工具中更新後，下次套用就會生效。
- 非互動執行與背景服務：只 render 輸入改變的 template；需要的秘密來源鎖住、需要執行秘密指令或缺少變數時，保留目標原本的內容，放進待處理清單。
- render 結果與目標相同時不寫入。寫入前先存 snapshot；權限照宣告設定，沒有宣告時新檔案為 0600，因為 render 結果可能含有秘密。

背景服務在 `host.toml` 改變時重新部署 dotfile，讓修改 `[vars]` 立即生效；repo 變動觸發的套用本來就包含 dotfile 部署。

### 本機修改 render 結果

目標與紀錄中的 render 結果不同，表示有人改了它。內容無法自動寫回 template，因此不論接管偏好是「自動」或「詢問」都放進待處理清單；偏好為「關閉」時不處理，之後的套用也不覆蓋它。

- `accept`：把目前的內容存成本機專用的 copy（`hosts/<識別名稱>/<路徑>`，加上 `when.hosts = [本機]` 的宣告，沿用權限與加密）。內容看起來像秘密而 template 沒有加密時拒絕，與 `rsmt add` 相同。只接受 `--scope host`。
- `discard`：重新 render 並覆蓋（先存 snapshot）。
- `ignore`：與其他 dotfile 項目相同。

輸入也改變時列為衝突，三種處理相同。

### 待處理清單的「等待輸入」項目

dotfile 項目新增原因 `waiting`，並記錄在等什麼：哪個秘密來源鎖住、哪些變數沒有值，或需要執行哪個秘密指令。這類項目不能 accept、ignore 或 discard，指令說明要在終端機執行 `rsmt apply`；套用成功後自動移除。背景服務對新出現的等待項目通知一次，沿用 dotfile 項目的通知方式，但只有「查看」動作。

## Risks / Trade-offs

- 每次終端機中的 `rsmt apply` 都取一次秘密 → rbw 與已解鎖的 bw 很快；鎖住時才詢問。若之後覺得太慢，再加上只 render 輸入改變的選項。
- 靜態分析會列出寫在 `{% if %}` 裡、或以 `default`、`is defined` 保護的變數 → 第一次會詢問；答空白會存成空字串，此後 `default` 不再生效，因此需要預設值的 template 應以 `vars.x or "預設"` 判斷空字串。只有 `vars.<name>` 的寫法會被詢問，`vars["name"]` 等寫法在 render 時才以錯誤指出。
- template 可能寫出極長的迴圈 → render 以 minijinja 的 fuel 限制指令數，超過時該 template render 失敗。
- bws 以 key 名稱查詢需要列出所有秘密 → 一次套用只列一次。
- 不同安裝方式的 socket 路徑依 Bitwarden 文件，Mac App Store 與 flatpak 版本的 SSH agent 支援狀況可能改變 → 只在 doctor 中回報 socket 不存在，不擋套用。
- 背景服務不執行秘密指令，使用 `secret()` 的 template 只會在終端機套用時更新 → 等待項目與通知會提醒使用者。
- rbw 在 `rbw unlocked` 與 `rbw get` 之間剛好自動鎖住時，`rbw get` 會自己叫出 pinentry；rbw 沒有關閉這個行為的選項 → 機率低，只記錄在此。bw 以 `BW_NOINTERACTION` 保證不詢問。
- 鎖住的工具在終端機中解鎖時仍持有套用的鎖，背景服務要等密碼輸入完成。
- 秘密指令的 `run` 第一個元素只能是 PATH 中的名稱或絕對路徑，不能來自 `{arg}`，避免依執行目錄或 template 參數改變要執行的程式。
