# rosamta

跨 shell、跨主機的 dotfile 管理工具：設定宣告在 repo 裡，render 成各 shell 載入的腳本，並持續把主機上出現的新變動接管回 repo。

術語後的括號是繁體中文介面使用的名稱。

## Language

### 設定與產物

**Repo**:
存放所有 module 與 dotfile 的 git repository，是設定的唯一真實來源。
_Avoid_: source state, source dir, dotfiles repo

**Module**（模組）:
一個獨立的設定單位，repo 裡一個 module 一個檔案。分為 provider module、structured module 與 raw module 三種。
_Avoid_: plugin, snippet, unit

**Provider**（整合）:
rosamta 內建、針對某個第三方工具（例如 starship、zoxide、oh-my-zsh）的知識：怎麼偵測、有哪些選項、各 shell 該產生什麼、跟誰互斥。啟用某個 provider 的 module 稱為 provider module。
_Avoid_: plugin, adapter

**Structured module**（結構化）:
只宣告環境變數、PATH 與 alias 的 module，由 rosamta 轉成每個 shell 的語法。
_Avoid_: env module, simple module

**Raw module**（片段）:
原樣保存一段 shell 語法的 module，只套用到語法相容的 shell。
_Avoid_: custom snippet, script

**Generated script**:
由 module render 出來、給特定 shell 載入的腳本。不手動編輯，不進 repo。
_Avoid_: artifact, output, build

**Render**:
從 module 產生 generated script 的動作。

**Phase**（載入階段）:
generated script 在 shell 啟動時的載入階段，依固定順序執行：env（環境）、early（前置）、framework（框架）、tools（工具）、prompt（提示字元）、aliases（別名）、late（收尾）。
_Avoid_: stage, order

**Hook**（掛載點）:
插在使用者 rc 檔裡、以 marker 標記的一小段，只負責載入 generated script。
_Avoid_: managed block, integration

**Dotfile**（設定檔）:
repo 裡要部署到 host 家目錄的檔案或目錄，例如 nvim、ghostty 的設定。shell 環境不算 dotfile，由 module 負責。
_Avoid_: target, managed file

**Deploy mode**（部署方式）:
dotfile 放到 host 上的方式：link（symlink 回 repo）、copy（複製）或 template（render 成實體檔案）。
_Avoid_: install mode, strategy

**Condition**（套用範圍）:
決定一個 module 或 dotfile 套用到哪些 host 的條件，依據 OS、架構、host ID 與 tag。
_Avoid_: filter, ignore rule

**Secret source**（秘密來源）:
提供 template 秘密值的外部工具，例如 rbw、bw、bws。
_Avoid_: vault, secret backend

### 主機與同步

**Host**（主機）:
一台執行 rosamta 的機器。正在操作的這台稱為本機。
_Avoid_: machine, device

**Host ID**（識別名稱）:
rosamta 用來指稱一台 host 的固定名稱，condition 以它為準，與系統的 hostname 無關。
_Avoid_: hostname, host name

**Tag**（標籤）:
host 自己設定的標籤（例如 work、server），供 condition 使用。
_Avoid_: group, role, label

**Daemon**（背景服務）:
在 host 上常駐背景、負責 Adopt、render、Publish 與 Pull 的程序。
_Avoid_: helper, watcher

**Watched file**:
daemon 監看的使用者 rc 檔，例如 `~/.zshrc`、`~/.bash_profile`、`~/.profile`。
_Avoid_: target

**Adopt**（接管）:
把 watched file 裡、hook 以外的新內容轉成 repo 裡的 module，並從 rc 檔移除原行。首次匯入既有 rc 檔也是 Adopt。
_Avoid_: capture, import, re-add, 收錄

**Adopt policy**（接管偏好）:
偵測到可接管的內容時的處理方式：自動接管、先詢問使用者，或關閉。
_Avoid_: capture policy, auto-sync

**Review queue**（待處理）:
等待使用者決定要不要接管的項目。詢問模式下的項目，以及無法發出通知的 host 上的項目，都放在這裡。
_Avoid_: inbox, pending list

**Snapshot**（快照）:
修改 watched file 之前保存的原文，用來還原任何一段被接管的內容。
_Avoid_: backup

**Publish**（推送）:
把 repo 的變更 commit 並 push 到遠端。
_Avoid_: upload

**Publish policy**（推送偏好）:
Adopt 或編輯之後，要自動 Publish 還是等使用者手動執行。

**Pull**（拉取）:
從遠端取得其他 host 的變更，並重新 render。
_Avoid_: fetch, update

**Sync**（同步）:
先 Pull 再 Publish 的一次完整往返。不包含 Adopt。
_Avoid_: 用「同步」指稱接管

**Doctor**（診斷）:
檢查 host 上 shell 設定的問題與可最佳化之處，並為每一項提供可直接套用的修正。
_Avoid_: health check, lint, 健檢

### 應用程式

**Package**（應用程式）:
repo 裡宣告某些 host 應該安裝的軟體，以 catalog 裡的名稱指稱。
_Avoid_: dependency, app, 套件

**Catalog**（應用程式目錄）:
隨 rosamta 發布的資料，把一個 package 名稱對應到各套件管理器裡的 ID。使用者可以在 repo 裡自行新增項目。
_Avoid_: registry, package database

**Bootstrap**（補齊應用程式）:
在 host 上補裝 repo 已宣告、但尚未安裝的 package。
_Avoid_: provision, setup
