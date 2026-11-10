# Design

## Context

依 ADR 0003，設定預先 render 成檔案，shell 啟動時只 source 檔案、不呼叫 rosamta 執行檔。依 ADR 0002，rosamta 會修改 rc 檔，因此修改前一律存 snapshot。交接文件整理的各 shell 細節（`ZDOTDIR`、`path_helper`、bash 只讀第一個存在的 profile 檔、fish 的 universal 變數）在此落實。

## Goals / Non-Goals

**Goals:**
- 一份 module 宣告在四種 shell 上產生相同效果。
- rosamta 出錯、被刪除或產物損毀時，shell 都能正常開啟。
- 每次套用都能回滾。

**Non-Goals:**
- provider 的 render（`add-providers-and-doctor`）。本 change 只處理結構化與片段 module，但 renderer 留好 provider 的介面。
- 接管 rc 檔中既有的內容（`add-adopt`）。
- 背景自動套用（`add-daemon`）。

## Decisions

### generated 目錄採版本化並以 symlink 切換

```
generated/
├── current -> v-20261110T143000-3f9a
├── v-20261110T143000-3f9a/
│   ├── sh/init.sh   sh/00-env-path.sh …
│   ├── bash/init.bash …
│   ├── zsh/init.zsh …
│   └── fish/init.fish …
└── v-20261110T120000-91c2/   # 保留最近 3 個版本供回滾
```

套用時先寫好新版本目錄，smoke test 通過後，建立新的 symlink 再以 `rename(2)` 覆蓋 `current`，達成原子切換。替代方案是直接覆寫檔案，但 shell 可能在中途讀到新舊混合的內容。

### hook 內容

hook 只檢查檔案存在後 source loader，例如 zsh：

```zsh
# >>> rosamta >>>
[ -r "${XDG_DATA_HOME:-$HOME/.local/share}/rosamta/generated/current/zsh/init.zsh" ] && . "${XDG_DATA_HOME:-$HOME/.local/share}/rosamta/generated/current/zsh/init.zsh"
# <<< rosamta <<<
```

loader（`init.<ext>`）依檔名順序 source 該 shell 的片段，每個片段的錯誤只印警告，迴圈繼續。env 階段以匯出變數 `ROSAMTA_ENV=<版本 id>` 防止同一程序重複執行。

### 各 shell 的 hook 位置

| Shell | 位置 | 載入內容 |
| --- | --- | --- |
| sh | `~/.profile` | env |
| bash | bash 實際讀取的 profile 檔（`~/.bash_profile`、`~/.bash_login`、`~/.profile` 中第一個存在的）＋ `~/.bashrc` | profile 檔：env，互動時再 source `~/.bashrc`；`~/.bashrc`：env（有防重）＋互動階段 |
| zsh | `$ZDOTDIR/.zshenv`、`.zshrc`；macOS 另加 `.zprofile` | `.zshenv`：env；`.zprofile`：重新排序 PATH；`.zshrc`：互動階段 |
| fish | `conf.d/rosamta.fish` | env 一律載入，其餘包在 `status is-interactive` 中 |

`ZDOTDIR` 以 `zsh -c 'echo ${ZDOTDIR:-$HOME}'` 詢問 zsh 本身；fish 的設定目錄以 `fish -c 'echo $__fish_config_dir'` 詢問。

### macOS 的 path_helper

macOS 的 `/etc/zprofile` 在 `~/.zshenv` 之後執行 `path_helper`，會把系統路徑排到最前面。`.zprofile` 的 hook 把 module 宣告的 PATH 項目重新移到正確位置（先移除再按順序 prepend）。bash 的 `/etc/profile` 也會執行 `path_helper`，但它在使用者的 profile 檔之前執行，所以不受影響。

### 語法輸出規則

- POSIX（sh、bash、zsh 共用）：`export NAME="..."`，值中的 `$NAME`、`${NAME}` 保留給 shell 展開，`"`、`` ` ``、`\` 與其他 `$` 一律跳脫；開頭的 `~` 轉成 `$HOME`。
- PATH 去重：`case ":$PATH:" in *":$dir:"*) ;; *) PATH="$dir:$PATH" ;; esac`。
- fish：`set -gx NAME "..."`；PATH 用 `fish_add_path --global --path`（不寫入 universal 變數）。
- bash 腳本只用 3.2 能執行的語法；CI 在 macOS runner 的內建 bash 3.2 上執行全部 bash 測試。

### 版本偵測

以 `bash -c 'echo ${BASH_VERSINFO[0]}.${BASH_VERSINFO[1]}'`、`zsh -c 'echo $ZSH_VERSION'`、`fish -c 'echo $version'` 取得版本。結果依執行檔路徑與修改時間快取在狀態目錄。

### Smoke test

對每個已安裝 hook 的 shell，以 `TERM=dumb` 執行 `<shell> -i -c exit`，逾時 5 秒；以環境變數讓 loader 改讀候選版本。先用目前版本跑一次取得基準 stderr，再跑候選版本；候選版本多出來的 stderr 行視為新錯誤，並依 loader 印出的 module 名稱定位問題來源。

### Snapshot 存放

`$XDG_STATE_HOME/rosamta/snapshots/` 是一個獨立的本機 git repo，從不設定 remote。每次修改前把 rc 檔複製到對應的家目錄相對路徑並 commit，commit 訊息記錄原因。第一次修改前的版本打上 `original` tag，供 `rsmt restore --original` 使用。

### Eject

依載入階段把 generated script 串接後寫進各 rc 檔原本 hook 的位置，並在開頭加一行註解標明 eject 的日期，接著移除 hook 與 fish 的 `conf.d/rosamta.fish`。寫入前存 snapshot。

## Risks / Trade-offs

- [`<shell> -i -c exit` 可能因使用者既有的設定而卡住，例如等待輸入] → 逾時 5 秒並視為失敗，訊息中說明是逾時，而不是錯誤輸出。
- [使用者自己的 rc 內容在 hook 之後又改了 PATH] → 不處理；交給 `add-providers-and-doctor` 的診斷規則偵測。
- [5 ms 預算在慢速機器上不易量測] → 以 CI 的固定機型量測中位數，並與沒有 hook 的基準比較，而不是看絕對值。
