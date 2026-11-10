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
