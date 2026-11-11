# dotfile-deploy Specification

## Purpose
把 repo 中的 dotfile 部署到各 host 的家目錄，並讓 app 對這些檔案的修改回到 repo，同時不在使用者不知情時覆蓋本機的內容。

## Requirements

### Requirement: Explicit declarations
每個 dotfile SHALL 在 repo 的 `dotfiles.toml` 中以一筆 `[[dotfile]]` 宣告，包含 `path`（相對於家目錄）與可選的 `source`（相對於 repo，預設 `home/<path>`）、`deploy`（`link`、`copy`、`template`，預設 `link`）、`permissions`（八進位字串）、`encrypted` 與 `when`。沒有宣告的檔案 SHALL 不被部署。`rsmt validate` SHALL 回報格式錯誤、不存在的來源、互相包含的路徑、shell 的啟動檔（例如 `.zshrc`，shell 設定由 module 負責），以及搭配 `link` 的 `encrypted`。有問題的宣告 SHALL 只讓該路徑不部署，也不釋放它先前部署的內容；其他 dotfile 照常部署。

#### Scenario: Undeclared file
- **WHEN** repo 的 `home/` 中有 `.config/foo`，但 `dotfiles.toml` 沒有它的宣告
- **THEN** `rsmt apply` 不會在家目錄建立 `.config/foo`

#### Scenario: Nested declarations
- **WHEN** `dotfiles.toml` 同時宣告 `.config/nvim` 與 `.config/nvim/init.lua`
- **THEN** `rsmt validate` 回報這兩筆宣告互相包含並以 2 結束

### Requirement: Host-specific declarations win
同一個路徑有多筆宣告符合本機時，`when.hosts` 只列本機識別名稱的宣告 SHALL 最優先，其次是 `when.hosts` 列出本機識別名稱的宣告；仍有多筆時，該路徑 SHALL 不部署並回報重複宣告，其他路徑照常部署。

#### Scenario: Host variant
- **WHEN** `.gitconfig` 有一筆沒有條件的宣告，和一筆 `when.hosts = ["mbp"]`、來源為 `hosts/mbp/.gitconfig` 的宣告，在 mbp 上套用
- **THEN** mbp 的 `~/.gitconfig` 是 `hosts/mbp/.gitconfig` 的內容，其他 host 使用 `home/.gitconfig`

### Requirement: Link deploy
`link` 的 dotfile SHALL 在目標位置建立指向 repo 來源的 symlink；來源是目錄時，整個目錄以一個 symlink 連結。目標已存在且內容與來源相同時，SHALL 先存 snapshot 再換成 symlink。

#### Scenario: New host
- **WHEN** 在沒有 `~/.config/nvim` 的 host 上執行 `rsmt apply`
- **THEN** `~/.config/nvim` 是指向 repo 中 `home/.config/nvim` 的 symlink

### Requirement: Link replaced by a file
`link` 的目標被換成一般檔案或目錄、且 repo 的來源自上次部署後沒有改變時，rosamta SHALL 不論接管偏好，把目標的內容寫回 repo 的來源並重建 symlink；repo 的來源也改變時 SHALL 視為衝突。

#### Scenario: Editor saves by rename
- **WHEN** 編輯器以「寫暫存檔再改名」儲存 `~/.gitconfig`，把 symlink 換成一般檔案
- **THEN** repo 的 `home/.gitconfig` 變成新內容，`~/.gitconfig` 再次是指向它的 symlink

### Requirement: Copy deploy
`copy` 的 dotfile SHALL 複製到目標位置。repo 的來源改變、而目標自上次部署後沒有被修改時，SHALL 先存 snapshot 再更新目標。

#### Scenario: Repo updated
- **WHEN** 另一台 host 修改了 `copy` 的 `.ssh/config` 並推送，本機拉取後套用
- **THEN** 本機的 `~/.ssh/config` 更新為新內容，舊內容保存在 snapshot 中

### Requirement: Copy changed on the host
`copy` 的目標自上次部署後被修改時，rosamta SHALL 依接管偏好處理：「自動」且 repo 的來源沒有改變時寫回 repo；「詢問」放進待處理清單；「關閉」不處理。被修改的目標 SHALL 不會被之後的套用覆蓋。

#### Scenario: Ask
- **WHEN** 接管偏好為「詢問」，某個 app 修改了 `copy` 部署的設定檔
- **THEN** 該檔案出現在待處理清單中，repo 不變，之後的 `rsmt apply` 也不覆蓋它

### Requirement: Conflicts are never overwritten
部署時目標已存在且內容與 repo 不同（第一次部署，或兩邊自上次部署後都改變）時，rosamta SHALL 不覆蓋目標。在互動終端機中執行的 `rsmt apply` SHALL 逐一顯示 diff，讓使用者選擇採用 repo 版本、保留本機版本或稍後處理；非互動執行與背景服務 SHALL 把它放進待處理清單。

#### Scenario: Existing gitconfig on a new host
- **WHEN** 新 host 已有內容不同的 `~/.gitconfig`，使用者在終端機中執行 `rsmt apply` 並選擇保留本機版本
- **THEN** 本機內容存到 repo 的 `hosts/<識別名稱>/.gitconfig`，`dotfiles.toml` 新增一筆限定本機的宣告，其他 host 不受影響

#### Scenario: Background apply
- **WHEN** 背景服務套用時遇到同樣的衝突
- **THEN** `~/.gitconfig` 不變，衝突出現在待處理清單中

### Requirement: Permissions
宣告了 `permissions` 的 dotfile，套用時 SHALL 把該權限設定在目標（`copy`）或 repo 的來源（`link`）本身。

#### Scenario: SSH config on a fresh clone
- **WHEN** 新 host clone repo 後套用宣告 `permissions = "0600"` 的 `.ssh/config`
- **THEN** 部署出去的檔案權限為 0600

### Requirement: Add and forget
`rsmt add <path>...` SHALL 把家目錄中的檔案或目錄移進 repo 的 `home/`、寫入宣告並部署；`--copy` 以 copy 部署，`--encrypt` 加密並以 copy 部署。沒有群組與其他人權限的檔案 SHALL 自動記錄 `permissions`。內容看起來像秘密、又沒有 `--encrypt` 時 SHALL 拒絕；空目錄 SHALL 被拒絕，因為 git 不保存空目錄。`rsmt forget <path>` SHALL 移除該路徑的宣告與 repo 中的內容，並把 symlink 換成內容相同的一般檔案。兩者各建立一個 commit 並依推送偏好推送。

#### Scenario: Add a directory
- **WHEN** 使用者執行 `rsmt add ~/.config/ghostty`
- **THEN** 內容移到 repo 的 `home/.config/ghostty`，`dotfiles.toml` 新增一筆宣告，`~/.config/ghostty` 變成 symlink，並產生一個 commit

#### Scenario: Secret without encryption
- **WHEN** 使用者執行 `rsmt add ~/.config/gh/hosts.yml`，內容含有 token
- **THEN** 指令列出符合的規則並建議加上 `--encrypt`，repo 不變

### Requirement: Forgotten on another host
其他 host 拉取後，部署紀錄中有、但已不再宣告的路徑 SHALL 不被刪除：`copy` 的目標留在原地不再管理；`link` 的目標換成內容相同的一般檔案，來源已從 repo 刪除時，從刪除前的 commit 取回內容。

#### Scenario: Forget on one host
- **WHEN** 使用者在 A 上執行 `rsmt forget ~/.config/nvim` 並推送，B 拉取後套用
- **THEN** B 的 `~/.config/nvim` 是內容與之前相同的一般目錄

### Requirement: Template deploy is declared only
`deploy = "template"` SHALL 通過驗證；在 template 功能完成前，套用時 SHALL 略過這些 dotfile 並回報尚不支援。

#### Scenario: Template before support
- **WHEN** `dotfiles.toml` 有一筆 `deploy = "template"` 的宣告，使用者執行 `rsmt apply`
- **THEN** 輸出指出該路徑尚不支援 template 部署，其他 dotfile 照常部署

### Requirement: Shell startup files are not dotfiles
`rsmt add` SHALL 拒絕 shell 的啟動檔，以及包含啟動檔的目錄（例如 `~/.config/fish`），並說明 shell 設定應放在 module 中。

#### Scenario: Add .zshrc
- **WHEN** 使用者執行 `rsmt add ~/.zshrc`
- **THEN** 指令說明 `.zshrc` 是 shell 的啟動檔並以 2 結束，repo 不變

#### Scenario: One broken declaration
- **WHEN** 另一台 host 加入的 dotfile 來源在本機不存在，使用者執行 `rsmt apply`
- **THEN** 該路徑回報錯誤，其他 dotfile 照常部署，指令以 1 結束
