# Design

## Context

ADR 0007：dotfile 的屬性寫在 `dotfiles.toml`，`home/` 使用實際檔名。ADR 0012：age 私鑰以密碼加密後放進 repo。module 的 `when`、host 的接管偏好、snapshot、待處理清單、背景服務與自動推送都已存在，dotfile 沿用這些機制，不另做一套。已決定的細節：

- `dotfiles.toml` 是 `[[dotfile]]` 陣列，每筆以 `path` 指定目標，同一個路徑可以有多筆宣告。
- 「保留本機版本」把內容收進 repo 的 `hosts/<識別名稱>/<路徑>`，加一筆 `when.hosts = [本機]` 的宣告。
- 「採用 repo 版本」的指令是 `rsmt review discard <編號>`。

## Goals / Non-Goals

**Goals:**
- 新 host 執行 `rsmt apply` 就得到所有 dotfile；不會在使用者不知情時覆蓋本機的內容。
- app 自己改了設定（包括把 symlink 換成一般檔案），變更會回到 repo。
- 任何覆蓋前都有 snapshot，`rsmt eject`、`rsmt restore --original` 之後的 home 與沒用過 rosamta 時一樣可用。

**Non-Goals:**
- template 部署（`add-templates-and-secrets`）、`create` 部署方式（`add-externals-and-scripts`）。
- 家目錄以外的檔案。
- 更改加密密碼、多把金鑰、Windows。
- 回報 `home/` 中沒有宣告的檔案：由 `add-providers-and-doctor` 的 doctor 處理。

## Decisions

### 宣告格式

```toml
[[dotfile]]
path = ".config/nvim"           # 相對於家目錄

[[dotfile]]
path = ".ssh/config"
deploy = "copy"                 # link（預設）| copy | template
permissions = "0600"
encrypted = true

[[dotfile]]
path = ".gitconfig"
source = "hosts/mbp/.gitconfig" # 相對於 repo，預設 home/<path>
when = { hosts = ["mbp"] }
```

- `path` 必須是相對路徑、不含 `..`；`source` 必須在 repo 內。一筆宣告可以是檔案或目錄。
- `encrypted` 只能搭配 copy（之後也可搭配 template）：symlink 指向密文沒有意義。
- `permissions` 是八進位字串，設定在該檔案或目錄本身。link 時設定在 repo 中的來源，因為 git 只保存執行權限，新 host clone 下來的檔案要重新設定。
- 不同路徑之間不可互相包含（例如同時宣告 `.config/nvim` 與 `.config/nvim/init.lua`）。這項與格式錯誤由 `rsmt validate` 檢查；`rsmt schema dotfiles` 輸出 JSON Schema。
- `deploy = "template"` 在本 change 可以通過驗證，套用時略過並回報尚不支援。

### 選擇套用的宣告

先以 `when` 篩出符合本機的宣告。同一個路徑有多筆符合時，`when.hosts` 只列本機識別名稱的那一筆最優先，其次是列出本機識別名稱的那一筆；仍有多筆時，該路徑不部署並回報重複宣告，其他路徑照常部署。重複只能在套用時判斷，因為 tag 與 OS 的組合在別台 host 上才知道。

### 部署紀錄

狀態目錄的 `deployed.json` 記錄每個部署出去的路徑：部署方式、來源、部署當時 repo 內容的雜湊，以及寫到目標的內容的雜湊（加密時兩者不同）。目錄的雜湊涵蓋排序後的相對路徑、內容與執行權限。這份紀錄用來分辨「repo 變了」與「本機被改了」：

| repo 與紀錄相同 | 目標與紀錄相同 | 處理 |
|---|---|---|
| 是 | 是 | 不動 |
| 否 | 是 | 更新目標（先存 snapshot） |
| 是 | 否 | 本機被改：依下節處理 |
| 否 | 否 | 衝突 |

沒有紀錄（第一次部署）時：目標不存在就直接部署；內容相同就記錄並接手；不同就是衝突。

### 三種情況的處理

- **link 被換成一般檔案**：不論接管偏好，repo 沒變時把內容寫回 repo 再重建 symlink。對 link 而言，編輯目標就是編輯 repo，app 的「寫暫存檔再改名」不該讓變更消失。repo 也變了時列為衝突。
- **copy 的目標被改**：依接管偏好；「自動」時 repo 沒變就寫回 repo，「詢問」放進待處理清單，「關閉」不處理，之後的套用也不覆蓋它。
- **衝突**：互動終端機中的 `rsmt apply` 逐一顯示 diff，讓使用者選採用 repo 版本、保留本機版本或稍後處理；非互動執行與背景服務一律放進待處理清單。

寫回 repo 的內容和手動編輯 repo 一樣：由下一次推送或拉取 commit（先做秘密掃描）。`edit(...)` commit 訊息新增 `home` 範圍：變更都在 `home/`、`hosts/` 或 `dotfiles.toml` 時寫成 `edit(home): update .config/nvim/lazy-lock.json`。

### 待處理清單的 dotfile 項目

`QueueItem` 加上類型（`adopt` 或 `dotfile`）。dotfile 項目記錄路徑、原因（衝突或本機被改）與當時兩邊的雜湊；任一邊再改變時，項目以新內容更新。

- `rsmt review accept <id> [--scope host|all]`：保留本機版本。`host`（預設，與接管一致）寫到 `hosts/<識別名稱>/<路徑>` 並新增限定本機的宣告，部署方式、權限與加密沿用原宣告；`all` 直接取代原宣告的來源。建立 commit 並依推送偏好推送。
- `rsmt review discard <id>`：先存 snapshot，再部署 repo 版本。只適用於 dotfile 項目。
- `rsmt review ignore <id>`：保留本機內容、不收進 repo；兩邊都沒再改變前，套用時略過這個路徑，也不再列出。

通知沿用「接管」、「忽略」、「查看差異」：接管等同 `accept`（限定本機）。

### `rsmt add` 與 `rsmt forget`

- `rsmt add <path>... [--copy] [--encrypt]`：路徑必須在家目錄內、存在、未被管理，也不在 rosamta 自己的目錄裡。內容移到 `home/<路徑>`，寫入宣告；link 時把原位置換成 symlink。檔案沒有群組與其他人的權限時自動記錄 `permissions`。內容看起來像秘密、又沒有 `--encrypt` 時拒絕並說明。`--encrypt` 隱含 `--copy`。
- `rsmt forget <path>`：移除該路徑的所有宣告與 repo 中的內容；link 的目標換成內容相同的一般檔案。
- 兩者各建立一個 commit（`add(home): <path>`、`forget(home): <path>`），依推送偏好推送。
- 其他 host 拉取後，紀錄中有、宣告中已沒有的路徑：copy 留在原地不再管理；link 換成一般檔案，來源已從 repo 刪除時，從刪除它的 commit 的上一版以 `git archive` 取回內容。這樣在一台 host 上 forget，不會讓其他 host 的設定消失。

### 加密

- repo 根目錄的 `key.age`：age 的 X25519 私鑰，以使用者的密碼經 scrypt 加密，armor 格式。不另存公鑰：能加密的 host 一定已有本機私鑰，公鑰從私鑰算出。
- 本機私鑰：`$XDG_CONFIG_HOME/rosamta/age.key`，權限 0600。
- 加密的 dotfile 以 armor 格式存在原本的來源路徑，檔名不變（ADR 0007）。
- 第一次 `rsmt add --encrypt` 時 repo 還沒有 `key.age`：在終端機中要求設定密碼（輸入兩次），產生金鑰並與該 dotfile 一起 commit。
- repo 有 `key.age`、本機沒有私鑰時：終端機中的 `rsmt apply` 要求輸入一次密碼（最多三次）；非互動執行與背景服務略過加密的 dotfile，背景服務以問題通知一次。core 提供以密碼解開金鑰的函式，GUI 之後直接呼叫。
- 密碼輸入用 `rpassword`，不回顯。

### 套用流程

`rsmt apply` 先完成 generated script 的 render、檢查與切換，再部署 dotfile；shell 的檢查失敗時整次套用中止，dotfile 也不動。`--dry-run` 列出每個 dotfile 的動作與 copy 的 diff。輸出與 `--json` 列出部署、更新、寫回、衝突與略過的路徑。覆蓋或取代任何目標前，先以 snapshot 保存（目錄逐一保存其中的檔案）。

- `rsmt eject`：link 的目標換成內容相同的一般檔案，清空部署紀錄。
- `rsmt restore --original`：dotfile 目標也還原成第一次修改前的內容；當時不存在的目標刪除。

### 背景服務

依部署紀錄監看：link 監看目標的上層目錄（非遞迴，不解析目標本身的 symlink，否則會解析到 repo）；copy 的檔案監看上層目錄，copy 的目錄遞迴監看。事件到來時以內容雜湊判斷是否需要處理，背景服務自己寫入的不會再觸發。repo 變動觸發的套用已包含 dotfile 部署。

## Risks / Trade-offs

- 有些 app 不接受 symlink 或總是以新檔案取代 → 寫回 repo 再重建 symlink 可以處理；若某個 app 一直取代，使用者可改用 copy。
- 大型目錄的雜湊成本 → 只在事件到來時計算該路徑；之後若有需要再以 mtime 與大小先篩選。
- scrypt 解開金鑰約需一秒 → 只在本機沒有私鑰時執行一次。
- 遺失密碼就無法在新 host 解開加密的 dotfile → 已有私鑰的 host 仍能運作；README 說明密碼無法找回。
