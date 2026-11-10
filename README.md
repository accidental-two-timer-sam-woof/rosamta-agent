# rosamta

跨 shell、跨主機的 dotfile 管理工具：設定宣告在 git repo 裡，render 成各 shell 載入的腳本，並持續把主機上出現的新變動接管回 repo。目前仍在開發中。

命令列程式有 `rosamta` 與 `rsmt` 兩個名稱，用法完全相同。

## 安裝

從 [Releases](https://github.com/accidental-two-timer-sam-woof/rosamta-agent/releases) 下載 macOS 的 `.dmg`，或 Linux 的 `.deb`、`.rpm`、tarball；也可以從原始碼安裝：

```sh
cargo install --locked --git https://github.com/accidental-two-timer-sam-woof/rosamta-agent rosamta
```

## 開始使用

建立新的 repo：

```sh
rsmt init
```

repo 建在 `~/.local/share/rosamta/repo`，本機設定寫在 `~/.config/rosamta/host.toml`，並從主機名稱產生這台主機的識別名稱。這一步不會修改任何 shell 設定檔。

在另一台主機加入同一個 repo（換成你的 repo 網址）：

```sh
rsmt init git@github.com:<你的帳號>/dotfiles.git
```

## 檢查設定

```sh
rsmt validate
```

檢查 repo 裡每個 TOML 檔，錯誤會附上檔案與行號，例如：

```text
/home/me/.local/share/rosamta/repo/modules/bun.toml:2:1: error: unknown field `phse`, expected one of `enabled`, `phase`, `order`, `when`
```

加上 `--json` 時，stdout 只輸出一個 JSON 文件，方便其他程式讀取：

```sh
rsmt validate --json
```

## 撰寫 module

每個 module 是 repo 中 `modules/` 底下的一個 TOML 檔，例如 `modules/editor.toml`：

```toml
[env]
EDITOR = "nvim"

[path]
prepend = ["~/.local/bin"]

[aliases]
ll = "ls -lh"
```

同一份宣告會套用到 sh、bash、zsh 與 fish。只適用於特定 shell 的程式碼放在 `[raw]`：

```toml
[raw]
zsh = '''
bindkey -e
'''
```

## 套用

```sh
rsmt apply --dry-run
rsmt apply
```

`--dry-run` 只顯示產生的腳本與 shell 設定檔會有的差異，不寫入任何檔案。

第一次套用時，每個已安裝 shell 的設定檔會加入一段以 `# >>> rosamta >>>` 開頭、`# <<< rosamta <<<` 結尾的區塊，區塊外的內容不會變動。修改設定檔之前，原本的內容會先存到 `~/.local/state/rosamta/snapshots`。

套用前會先以新的設定啟動每個 shell；啟動時若出現新的錯誤，這次套用會取消並列出造成錯誤的 module，shell 繼續使用原本的設定。

## 接管設定檔的內容

```sh
rsmt adopt
```

把各 shell 設定檔中區塊以外的內容搬進 repo 的 `modules/adopted/`，搬完後設定檔只剩區塊。只有環境變數、PATH 與 alias 的段落會轉成套用到所有 shell 的 module；其他內容原樣保存，只套用到讀取該檔的 shell。看起來像 API key 或 token 的內容不會進 repo，而是存在 `~/.config/rosamta/local/modules/`。

修改前會先存 snapshot，每次接管都有一個 commit，輸出中的編號可用來還原：

```sh
rsmt restore <編號>
```

之後安裝程式或你自己加進設定檔的內容，會出現在待處理清單：

```sh
rsmt review
rsmt review accept <編號>
rsmt review ignore <編號>
```

`accept` 接管該項，預設只套用到這台主機，加上 `--scope all` 則套用到所有主機。`ignore` 讓內容留在設定檔，內容改變前不再列出。清單不是空的時候，開啟互動 shell 會顯示一行提醒。

接管方式寫在 `~/.config/rosamta/host.toml`：

| 設定 | 可用的值 |
|---|---|
| `adopt` | `ask`：列入待處理（預設）；`auto`：直接接管；`off`：不處理 |
| `adopt_original` | `delete`：刪除原內容（預設）；`comment`：改成註解 |
| `shell_notice` | `true`：開 shell 時提醒（預設）；`false` |

## 同步到其他主機

```sh
rsmt sync
```

先拉取其他主機推送的變更並套用，再把這台主機的變更推送到遠端。也可以分開執行：

```sh
rsmt pull
rsmt publish
```

`rsmt publish` 會先把你在 repo 中手動修改的內容建立成 commit。commit 依你的 git 設定簽章。

`rsmt init` 建立的 repo 沒有遠端，要先加上一個（換成你的 repo 網址）：

```sh
git -C ~/.local/share/rosamta/repo remote add origin git@github.com:<你的帳號>/dotfiles.git
rsmt publish
```

接管與還原建立 commit 後會立即推送。在 `~/.config/rosamta/host.toml` 設定 `publish = "manual"` 時，只在執行 `rsmt publish` 或 `rsmt sync` 時推送。

拉取時，這台主機還沒推送的 commit 會接在遠端的變更之後。兩台主機改到同一個檔案的同一處時，拉取會中止並列出這些檔案，repo 維持拉取前的狀態；在 repo 中用 git 合併後，再執行 `rsmt sync`。拉取到的 module 讓 shell 啟動時出現新的錯誤時，repo 保留新內容，shell 繼續使用原本的設定。

### 推送受阻

推送前會檢查要推送的內容。看起來像 API key 或 token 時，推送會停止並列出檔案、行號與符合的規則，手動修改的內容也不會被 commit；把該內容移出 repo 後再推送一次。內容如果已經在 commit 中，要先修改那個 commit。

| 情況 | 結果 |
|---|---|
| commit 無法簽章，例如背景中無法輸入 GPG 密碼 | 不建立 commit，變更留在 repo 中；解鎖金鑰後再執行 `rsmt publish` |
| 遠端拒絕存取 | 不推送；確認 SSH 金鑰或 GitHub 登入後再推送 |
| 連不上遠端 | commit 留在本機，下一次 `rsmt sync` 時推送 |

## 連結 GitHub

```sh
rsmt remote github login
```

依畫面上的網址開啟 GitHub 的驗證頁，輸入顯示的驗證碼並授權。token 存在系統的 keychain（macOS Keychain、Linux Secret Service）；沒有 keychain 時存在 `~/.config/rosamta/github-token`，只有你能讀取。

登入後，建立一個 private repo 作為遠端：

```sh
rsmt init --github dotfiles
```

或加入既有的 repo：

```sh
rsmt init --github <帳號>/dotfiles
```

SSH 金鑰能通過 GitHub 驗證時使用 SSH；否則以登入的 token 走 HTTPS，相關設定只寫在 repo 中，不修改 `~/.gitconfig`。

登出時會刪除 token：

```sh
rsmt remote github logout
```

## 背景服務

```sh
rsmt daemon enable
```

背景服務登入後自動啟動。rc 檔出現新內容時，依接管偏好接管或加入待處理清單；repo 有變動時立即套用；並依拉取偏好定時同步。

- macOS：背景服務是 rosamta.app 中的登入項目。系統要求核准時，到「系統設定 › 一般 › 登入項目」允許 rosamta。出現待處理的內容時會發出通知，可以直接選「接管」、「忽略」或「查看差異」。
- Linux：註冊為 systemd 使用者服務 `rosamta.service`。沒有 systemd 使用者服務的環境（例如容器），請在登入後的 session 中自行執行：

```sh
rsmt daemon run
```

查看狀態與停用：

```sh
rsmt daemon status
rsmt daemon disable
```

背景服務啟動時會讀取一次 login shell 的環境變數，例如 `SSH_AUTH_SOCK`；修改後先停用再啟用，才會生效。沒有通知服務時不發通知，只更新待處理清單並在開 shell 時提醒。執行紀錄寫在 `~/.local/state/rosamta/daemon.log`。

`~/.config/rosamta/host.toml` 中與背景服務有關的設定：

| 設定 | 可用的值 |
|---|---|
| `pull` | `auto`：定時同步，電腦從睡眠喚醒後也會同步一次（預設）；`manual`：只在執行 `rsmt sync` 或 `rsmt pull` 時拉取 |
| `pull_interval` | 定時同步的間隔，單位為分鐘（預設 15） |

## 移除 hook

```sh
rsmt unhook zsh
```

只移除該 shell 的區塊，之後的 `rsmt apply` 也不會再加回去。要重新加入：

```sh
rsmt hook zsh
```

## 停止使用

```sh
rsmt eject
```

把目前生效的設定直接寫進各 shell 的設定檔並移除區塊；之後即使移除 rosamta，shell 的 PATH、環境變數與 alias 都不變。

```sh
rsmt restore --original
```

把 rosamta 改過的每個設定檔還原成第一次修改前的內容，並刪除 rosamta 建立的設定檔。

兩個指令都會先顯示差異並要求確認；不在終端機中執行時，要加上 `--yes`。

## 編輯器支援

`rsmt schema` 輸出各種檔案的 JSON Schema，種類有 `repo`（`rosamta.toml`）、`module` 與 `host`（`host.toml`）：

```sh
rsmt schema module > module.schema.json
```

在 TOML 檔第一行加上 `#:schema ./module.schema.json`，Taplo 或 Tombi 等編輯器外掛就能提示欄位並即時檢查。

## 檔案位置

所有平台都使用 XDG 路徑；對應的環境變數未設定或不是絕對路徑時，使用括號中的預設值。

| 內容 | 位置 |
|---|---|
| 本機設定 | `$XDG_CONFIG_HOME/rosamta/host.toml`（`~/.config`） |
| repo | `$XDG_DATA_HOME/rosamta/repo`（`~/.local/share`） |
| 產生的腳本 | `$XDG_DATA_HOME/rosamta/generated`（`~/.local/share`） |
| 狀態 | `$XDG_STATE_HOME/rosamta`（`~/.local/state`） |

## 結束碼

| 結束碼 | 意義 |
|---|---|
| 0 | 成功 |
| 2 | 指令用法或設定有誤 |
| 1 | 其他錯誤，例如無法讀取檔案 |

## 授權

[Apache-2.0](LICENSE)。第三方套件的授權見 [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md)。
