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
├── current -> v-1794307833-0830a630
├── v-1794307833-0830a630/
│   ├── sh/env.sh   sh/0-0050-path-env.sh …
│   ├── bash/env.bash   bash/rc.bash …
│   ├── zsh/env.zsh   zsh/rc.zsh   zsh/profile.zsh …
│   └── fish/env.fish   fish/rc.fish …
└── v-1794300000-91c2e0a1/   # 保留最近 3 個版本供回滾
```

版本 id 是 `v-<秒>-<奈秒的十六進位>`。片段檔名是 `<階段序號>-<order 四位數>-<module>-<種類>.<副檔名>`，種類為 `env`、`alias` 或 `raw`，所以錯誤訊息中的檔名就能看出 module。

套用時先寫好新版本目錄，smoke test 通過後，建立新的 symlink 再以 `rename(2)` 覆蓋 `current`，達成原子切換。替代方案是直接覆寫檔案，但 shell 可能在中途讀到新舊混合的內容。

### hook 內容

hook 只檢查檔案存在後 source loader，例如 zsh 的 `.zshrc`：

```zsh
# >>> rosamta >>>
_rosamta="${ROSAMTA_CURRENT:-${XDG_DATA_HOME:-$HOME/.local/share}/rosamta/generated/current}/zsh"
[ -r "$_rosamta/rc.zsh" ] && . "$_rosamta/rc.zsh"
unset _rosamta
# <<< rosamta <<<
```

`ROSAMTA_CURRENT` 讓 smoke test 把 shell 指向候選版本，使用者不需要設定。

### Loader

每個 shell 有三種 loader：`env.<ext>`（所有模式）、`rc.<ext>`（互動 shell）、`zsh/profile.zsh`（macOS login zsh 重新排序 PATH）。loader 以絕對路徑逐一列出片段，不在啟動時列目錄。

- 片段直接 source，不包錯誤處理：shell 自己的錯誤訊息會帶上片段的檔名，也就是 module 名稱。dash 與 BusyBox ash 遇到語法錯誤會中止整個 shell，所以 `raw` 片段在套用時先以 `<shell> -n`（fish 為 `--no-execute`）檢查語法，有錯就不套用。
- env 階段以匯出變數 `ROSAMTA_ENV=<版本 id>:<shell>` 防止同一程序樹重複執行；依 shell 區分，因為 sh 先跑過 env 後，再啟動的 bash 仍需要載入自己的 env。
- `ROSAMTA_PROFILE=1` 時，shell 有 `EPOCHREALTIME`（bash 5、zsh）就記錄每個 module 的毫秒數，否則以 perl 記錄整體時間，寫入 `$XDG_STATE_HOME/rosamta/profile.log`。

### 各 shell 的 hook 位置

| Shell | 位置 | 載入內容 |
| --- | --- | --- |
| sh | `~/.profile` | env |
| bash | bash 實際讀取的 profile 檔（`~/.bash_profile`、`~/.bash_login` 中第一個存在的；兩者都沒有就新建 `~/.bash_profile`，開頭先 source `~/.profile`）＋ `~/.bashrc` | profile 檔：env，原本沒有載入 `~/.bashrc` 時，互動時再 source；`~/.bashrc`：env（有防重）＋互動階段 |
| zsh | `.zshenv`（全域 zshenv 執行後的 `$ZDOTDIR`，或 HOME）、`$ZDOTDIR/.zshrc`、`$ZDOTDIR/.zprofile` | `.zshenv`：env；`.zprofile`：重新排序 PATH；`.zshrc`：互動階段 |
| fish | `conf.d/rosamta.fish`（整個檔案屬於 rosamta） | env 一律載入，互動階段包在 `status is-interactive` 中 |

`ZDOTDIR` 以 `zsh -c 'printf %s "${ZDOTDIR:-$HOME}"'` 詢問 zsh 本身，因此 `~/.zshenv` 中設定的 `ZDOTDIR` 也會生效。`.zshenv` 的位置改用 `zsh -f` 詢問：`-f` 仍會執行全域的 zshenv，但不讀使用者的 `.zshenv`。Alpine 的全域 zshenv 會在 HOME 沒有任何 zsh 啟動檔時，把 `ZDOTDIR` 設為 `~/.config/zsh`，這時四個檔案都在 `~/.config/zsh`。fish 的設定目錄以 `fish -c 'printf %s $__fish_config_dir'` 詢問。

`rsmt unhook <shell>` 只移除該 shell 的區塊，並把它記在 `$XDG_STATE_HOME/rosamta/unhooked`，之後的 `rsmt apply` 不再替它安裝 hook；`rsmt hook [shell...]` 重新安裝並移出清單。`eject` 與 `restore --original` 會把所有 shell 記入清單。

### 全域 zprofile 改動 PATH

login zsh 在 `.zshenv` 之後才執行全域的 zprofile：macOS 的 `/etc/zprofile` 執行 `path_helper`，把系統路徑排到最前面；Alpine 的 `/etc/zsh/zprofile` 執行 `/etc/profile`，直接重設 PATH。因此每個平台都在 `.zprofile` 安裝 hook，把 module 宣告的 PATH 項目放回正確位置：prepend 的項目先移除再按順序放到最前面，append 的項目不存在時加到最後。bash 的 `/etc/profile` 也會執行 `path_helper`，但它在使用者的 profile 檔之前執行，所以不受影響。

### 語法輸出規則

- POSIX（sh、bash、zsh 共用）：`export NAME="..."`，值中的 `$NAME`、`${NAME}` 保留給 shell 展開，`"`、`` ` ``、`\` 與其他 `$` 一律跳脫；開頭的 `~` 轉成 `$HOME`。
- PATH 去重：`case ":$PATH:" in *":$dir:"*) ;; *) PATH="$dir:$PATH" ;; esac`。
- fish：`set -gx NAME "..."`；PATH 用 `contains -- <dir> $PATH; or set -gx PATH <dir> $PATH`，不寫入 universal 變數。不用 `fish_add_path`，因為它會略過不存在的目錄，與其他 shell 的結果不同。
- bash 腳本只用 3.2 能執行的語法；CI 在 macOS runner 的內建 bash 3.2 上執行全部 bash 測試。

### 版本偵測

以 `bash -c 'echo ${BASH_VERSINFO[0]}.${BASH_VERSINFO[1]}'`、`zsh -c 'echo $ZSH_VERSION'`、`fish -c 'echo $version'` 取得版本。結果依執行檔路徑與修改時間快取在狀態目錄。

### Smoke test

對每個已安裝 hook 的 shell，以 `TERM=dumb` 執行 `<shell> -i -c exit`（sh 為 `sh -l -c exit`），逾時 5 秒；以 `ROSAMTA_CURRENT` 讓 hook 改讀候選版本。先用目前版本跑一次取得基準 stderr，再跑候選版本；候選版本多出來的 stderr 行視為新錯誤。錯誤行含有片段檔名時指出該 module；沒有時列出這次有變動的 module。

### Snapshot 存放

`$XDG_STATE_HOME/rosamta/snapshots/` 是一個獨立的本機 git repo，從不設定 remote，也不簽章。每次修改前把 rc 檔的內容存到 `files/<路徑>` 並 commit，commit 訊息記錄原因；HOME 底下的檔案存在 `home/` 之下，其他存在 `root/` 之下。

`rsmt restore --original` 需要的是每個檔案第一次被修改前的樣子，而不同檔案第一次被修改的時間不同，所以另外存在 `original/<路徑>`，之後不再覆寫；當時不存在的檔案記在 `original-absent`，還原時刪除。第一個 commit 打上 `original` tag。

### Eject

依載入階段把 generated script 串接後寫進各 rc 檔原本 hook 的位置，並在開頭加一行註解標明 eject 的日期，接著移除 hook 與 fish 的 `conf.d/rosamta.fish`。寫入前存 snapshot。

## Risks / Trade-offs

- [`<shell> -i -c exit` 可能因使用者既有的設定而卡住，例如等待輸入] → 逾時 5 秒並視為失敗，訊息中說明是逾時，而不是錯誤輸出。
- [使用者自己的 rc 內容在 hook 之後又改了 PATH] → 不處理；交給 `add-providers-and-doctor` 的診斷規則偵測。
- [5 ms 預算在慢速機器上不易量測] → 以 CI 的固定機型量測中位數，並與沒有 hook 的基準比較，而不是看絕對值。
