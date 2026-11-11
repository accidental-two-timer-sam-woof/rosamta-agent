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
_rosamta="${ROSAMTA_CURRENT:-${_rosamta_version:-$HOME/.local/share/rosamta/generated/current}}/zsh"
[ -r "$_rosamta/rc.zsh" ] && . "$_rosamta/rc.zsh"
unset _rosamta _rosamta_version
# <<< rosamta <<<
```

`ROSAMTA_CURRENT` 讓 smoke test 把 shell 指向候選版本，使用者不需要設定。generated 目錄的位置在安裝 hook 時寫進區塊（在 HOME 底下時寫成 `$HOME/...`），因為 shell 啟動時不一定看得到 rosamta 執行時的 `XDG_DATA_HOME`。env loader 把自己的版本目錄記在 `_rosamta_version`，同一個 shell 之後的 hook（zsh 的 `.zprofile`、`.zshrc`，bash 的 `~/.bashrc`）沿用它，所以套用途中啟動的 shell 不會混用兩個版本；最後一個階段的 hook 把它刪掉，之後重新載入 rc 檔時讀的是最新版本。

### Loader

每個 shell 有三種 loader：`env.<ext>`（所有模式）、`rc.<ext>`（互動 shell）、`profile.<ext>`（sh、bash、zsh 的 login shell 重新排序 PATH）。loader 以絕對路徑逐一列出片段，不在啟動時列目錄。

- 片段直接 source，不包錯誤處理：shell 自己的錯誤訊息會帶上片段的檔名，也就是 module 名稱。dash 與 BusyBox ash 遇到語法錯誤會中止整個 shell，所以 `raw` 片段在套用時先以 `<shell> -n`（fish 為 `--no-execute`）檢查語法，有錯就不套用。
- env 階段以匯出變數 `ROSAMTA_ENV=<版本 id>:<shell>` 防止同一程序樹重複執行；依 shell 區分，因為 sh 先跑過 env 後，再啟動的 bash 仍需要載入自己的 env。login bash 也會讀 `~/.profile`，所以 sh 的 loader 在 bash 中（`$BASH` 的檔名是 bash，且不是 posix 模式）不執行，以免同一個程序跑兩次 env；以 sh 啟動的 bash 讀 `~/.profile` 時 `$BASH` 是 sh，posix 模式也要等讀完啟動檔才開啟。
- `ROSAMTA_PROFILE=1` 時，shell 有 `EPOCHREALTIME`（bash 5、zsh）就記錄每個 module 的毫秒數，否則以 perl 記錄整體時間，寫入 `$XDG_STATE_HOME/rosamta/profile.log`。

### 各 shell 的 hook 位置

| Shell | 位置 | 載入內容 |
| --- | --- | --- |
| sh | `~/.profile` | env，接著重新排序 PATH |
| bash | bash 實際讀取的 profile 檔（`~/.bash_profile`、`~/.bash_login` 中第一個存在的；兩者都沒有就新建 `~/.bash_profile`，開頭先 source `~/.profile`）＋ `~/.bashrc` | profile 檔：env、重新排序 PATH，原本（自己或透過它 source 的 `~/.profile`）沒有載入 `~/.bashrc` 時，互動時再 source；`~/.bashrc`：env（有防重）＋互動階段 |
| zsh | `.zshenv`（全域 zshenv 執行後的 `$ZDOTDIR`，或 HOME）、`$ZDOTDIR/.zshrc`、`$ZDOTDIR/.zprofile` | `.zshenv`：env；`.zprofile`：重新排序 PATH；`.zshrc`：互動階段 |
| fish | `conf.d/rosamta.fish`（整個檔案屬於 rosamta） | env 一律載入，互動階段包在 `status is-interactive` 中 |

`ZDOTDIR` 以 `zsh -c 'printf %s "${ZDOTDIR:-$HOME}"'` 詢問 zsh 本身，因此 `~/.zshenv` 中設定的 `ZDOTDIR` 也會生效。`.zshenv` 的位置改用 `zsh -f` 詢問：`-f` 仍會執行全域的 zshenv，但不讀使用者的 `.zshenv`。rosamta 自己繼承的 `ZDOTDIR` 只有在新開的 zsh 不會自己設成同樣的值時，也就是來自登入 session 時，才帶進這兩次詢問：在 `~/.zshenv` 匯出 `ZDOTDIR` 的 zsh 中執行 rosamta（背景服務也會從 login shell 繼承）時，新開的 zsh 仍先讀 `~/.zshenv`。Alpine 的全域 zshenv 會在 HOME 沒有任何 zsh 啟動檔時，把 `ZDOTDIR` 設為 `~/.config/zsh`，這時四個檔案都在 `~/.config/zsh`。fish 的設定目錄以 `fish -c 'printf %s $__fish_config_dir'` 詢問。

`rsmt unhook <shell>` 只移除該 shell 的區塊，並把它記在 `$XDG_STATE_HOME/rosamta/unhooked`，之後的 `rsmt apply` 不再替它安裝 hook；`rsmt hook [shell...]` 重新安裝並移出清單。`eject` 與 `restore --original` 會把所有 shell 記入清單。

### 系統的 profile 改動 PATH

login zsh 在 `.zshenv` 之後才執行全域的 zprofile：macOS 的 `/etc/zprofile` 執行 `path_helper`，把系統路徑排到最前面；Alpine 的 `/etc/zsh/zprofile` 執行 `/etc/profile`，直接重設 PATH。sh 與 bash 的 `/etc/profile` 雖然在使用者的 profile 檔之前執行，但從另一個 shell 啟動的 login shell（tmux、編輯器、`bash -l`）繼承了 `ROSAMTA_ENV`，不會再跑 env，PATH 就停在 `/etc/profile` 改過的樣子。因此 sh、bash、zsh 都在 login 時載入 `profile.<ext>`（zsh 從 `.zprofile`，sh 從 `~/.profile`，bash 從它的 profile 檔），不受防重限制，把 module 宣告的 PATH 項目放回正確位置：依 module 的載入順序，prepend 的項目先移除再放到最前面，append 的項目不存在時加到最後、已存在時留在原處。所以和 env 一樣，同一個 module 的 prepend 依宣告順序排列，後載入的 module 排在前面。

### 語法輸出規則

- POSIX（sh、bash、zsh 共用）：`export NAME="..."`，值中的 `$NAME`、`${NAME}` 保留給 shell 展開，`"`、`` ` ``、`\` 與其他 `$` 一律跳脫；開頭的 `~` 轉成 `$HOME`。
- PATH 去重：`[ -n "$dir" ] && case ":$PATH:" in *":$dir:"*) ;; *) PATH="$dir:$PATH" ;; esac`。展開後是空字串的項目略過，因為 PATH 中的空項目代表目前目錄。
- fish：`set -gx NAME "..."`；PATH 用 `test -n <dir>; and not contains -- <dir> $PATH; and set -gx PATH <dir> $PATH`，不寫入 universal 變數。不用 `fish_add_path`，因為它會略過不存在的目錄，與其他 shell 的結果不同。
- bash 腳本只用 3.2 能執行的語法；CI 在 macOS runner 的內建 bash 3.2 上執行全部 bash 測試。

### 版本偵測

以 `bash -c 'echo ${BASH_VERSINFO[0]}.${BASH_VERSINFO[1]}'`、`zsh -c 'echo $ZSH_VERSION'`、`fish -c 'echo $version'` 取得版本。結果依執行檔路徑與修改時間快取在狀態目錄。

### Smoke test

對每個已安裝 hook 的 shell，以 `TERM=dumb` 執行 `<shell> -i -c exit`，bash 與 zsh 另執行 `<shell> -l -i -c exit`（sh 為 `sh -l -c exit`），逾時 5 秒；以 `ROSAMTA_CURRENT` 讓 hook 改讀候選版本。shell 在自己的 session 中執行，沒有終端機可搶；逾時時整個程序群組一起結束，rc 檔啟動的程序不會讓 rosamta 卡住。先用目前版本跑一次取得基準 stderr，再跑候選版本；候選版本多出來的 stderr 行視為新錯誤。錯誤行含有片段檔名時指出該 module；沒有時列出這次有變動的 module。

### Snapshot 存放

`$XDG_STATE_HOME/rosamta/snapshots/` 是一個獨立的本機 git repo，從不設定 remote，也不簽章。每次修改前把 rc 檔的內容存到 `files/<路徑>` 並 commit，commit 訊息記錄原因；HOME 底下的檔案存在 `home/` 之下，其他存在 `root/` 之下。

`rsmt restore --original` 需要的是每個檔案第一次被修改前的樣子，而不同檔案第一次被修改的時間不同，所以另外存在 `original/<路徑>`，之後不再覆寫；當時不存在的檔案記在 `original-absent`，還原時刪除。第一個 commit 打上 `original` tag。

### Eject

依載入階段把 generated script 串接後寫進各 rc 檔原本 hook 的位置，並在開頭加一行註解標明 eject 的日期，接著移除 hook 與 fish 的 `conf.d/rosamta.fish`。寫入前存 snapshot。env 階段保留與 loader 相同的防重條件；login 的 PATH 排序照原樣寫進 profile 檔。片段原本各自是一個被 source 的檔案，`return` 只結束該片段，所以含有 `return` 的片段包在一個呼叫後就刪除的函式中；其他片段不包，因為函式中的 `typeset`、`declare` 會變成區域變數。沒有互動階段的內容時不寫空的條件區塊。目前版本中已沒有腳本的 shell（例如已解除安裝）只移除它的 hook。

## Risks / Trade-offs

- [`<shell> -i -c exit` 可能因使用者既有的設定而卡住，例如等待輸入] → 逾時 5 秒並視為失敗，訊息中說明是逾時，而不是錯誤輸出；基準也逾時時不算新錯誤。
- [使用者自己的 rc 內容在 hook 之後又改了 PATH] → 不處理；交給 `add-providers-and-doctor` 的診斷規則偵測。
- [Debian 與 Ubuntu 預設的 `~/.bashrc` 開頭在非互動 shell 中直接 `return`，區塊在檔尾，`ssh 主機 指令` 讀不到 env] → 不把 env 拆到檔案開頭的第二個區塊，因為每個檔案只有一個區塊是 hook 與 adopt 共同的前提，而整個區塊放到開頭又會讓互動階段早於使用者自己的設定；README 說明可以自行把區塊移到該段之前，之後的套用會在原處更新。
- [5 ms 預算在慢速機器上不易量測] → 以 CI 的固定機型量測中位數，並與沒有 hook 的基準比較，而不是看絕對值。
