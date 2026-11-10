# rosamta

跨 shell、跨主機的 dotfile 管理工具：設定宣告在 repo 裡，render 成各 shell 載入的腳本，並持續把主機上出現的新變動收回 repo。

## Language

### 設定與產物

**Repo**:
存放所有 module 與 dotfile 的 git repository，是設定的唯一真實來源。
_Avoid_: source state, source dir, dotfiles repo

**Module**:
一個獨立的設定單位：一個 provider 的啟用設定，或一段自訂片段。repo 裡一個 module 一個檔案。
_Avoid_: plugin, snippet, unit

**Provider**:
工具內建、針對某個第三方工具（例如 starship、zoxide、oh-my-zsh）的知識：怎麼偵測、有哪些選項、各 shell 該產生什麼、跟誰互斥。
_Avoid_: plugin, integration

**Generated script**:
由 module render 出來、給特定 shell 載入的腳本。不手動編輯，不進 repo。
_Avoid_: artifact, output, build

**Render**:
從 module 產生 generated script 的動作。

**Phase**:
generated script 在 shell 啟動時的載入階段，依固定順序執行：env、early、framework、tools、prompt、aliases、late。
_Avoid_: stage, order

**Hook**:
插在使用者 rc 檔裡、以 marker 標記的一小段，只負責載入 generated script。
_Avoid_: managed block, integration

**Dotfile**:
repo 裡要部署到 host 家目錄的檔案或目錄，例如 nvim、ghostty 的設定。shell 環境不算 dotfile，由 module 負責。
_Avoid_: target, managed file

**Deploy mode**:
dotfile 放到 host 上的方式：link（symlink 回 repo）、copy（複製）或 template（render 成實體檔案）。
_Avoid_: install mode, strategy

**Condition**:
決定一個 module 或 dotfile 套用到哪些 host 的條件，依據 OS、架構、host 名稱與 tag。
_Avoid_: filter, ignore rule

**Secret source**:
提供 template 秘密值的外部工具，例如 rbw、bw、bws。
_Avoid_: vault, secret backend

### 套件

**Package**:
repo 裡宣告某些 host 應該安裝的軟體，以 catalog 裡的名稱指稱。
_Avoid_: dependency, app

**Catalog**:
隨 rosamta 發布的資料，把一個 package 名稱對應到各套件管理器的套件 ID。
_Avoid_: registry, package database

**Bootstrap**:
在 host 上補裝 repo 已宣告、但尚未安裝的 package。
_Avoid_: provision, setup

### 主機與同步

「同步」不是正式用語。依方向分別說 Capture、Publish 或 Pull。

**Host**:
一台執行本工具的機器。
_Avoid_: machine, device

**Tag**:
host 自己設定的標籤（例如 work、server），供 condition 使用。
_Avoid_: group, role, label

**Daemon**:
在 host 上常駐背景、負責 Capture、render、Publish 與 Pull 的程序。
_Avoid_: helper, service, watcher

**Watched file**:
daemon 監看的使用者 rc 檔，例如 `~/.zshrc`、`~/.bash_profile`、`~/.profile`。
_Avoid_: target, rc file（泛指時可用）

**Capture**:
把 watched file 裡、hook 以外的新內容轉成 repo 裡的 module，並把原行註解掉或刪除。首次匯入既有 rc 檔也是 Capture。
_Avoid_: import, sync, re-add

**Capture policy**:
偵測到可收錄的內容時的處理方式：自動 Capture、先詢問使用者，或關閉。
_Avoid_: auto-sync

**Review queue**:
等待使用者決定要不要 Capture 的項目。詢問模式下的項目，以及無法發出通知的 host 上的項目，都放在這裡。
_Avoid_: inbox, pending list

**Snapshot**:
修改 watched file 之前保存的原文，用來還原任何一段被 Capture 的內容。
_Avoid_: backup

**Publish**:
把 repo 的變更 commit 並 push 到遠端。
_Avoid_: sync, upload

**Publish policy**:
Capture 或編輯之後，要自動 Publish 還是等使用者手動執行。

**Pull**:
從遠端取得其他 host 的變更，並重新 render。
_Avoid_: sync, fetch, update
