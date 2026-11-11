# Design

## Context

module 目前只有結構化與片段兩種（`config::Module`），支援 sh、bash、zsh、fish，平台為 macOS 與 Linux。render 依載入階段（env、early、framework、tools、prompt、aliases、late、login）產生 generated script；背景服務預先 render（ADR 0003），並已能擷取 login shell 的環境（`daemon/env.rs`）。接管把 rc 檔的段落轉成 module，套用範圍與跨主機合併由 `refine-adopt-scope` 決定。

使用量依據：以 GitHub 程式碼搜尋統計公開 `.zshrc`、`.bashrc`、`config.fish` 中各工具 init 寫法的出現次數，並對照 Homebrew 一年的安裝數（調查於 2026-11-11，110 個工具）。第一批涵蓋 `.zshrc` 前 25 名中符合條件的工具。

## Goals / Non-Goals

**Goals:**
- 常見工具的載入順序與互斥由 rosamta 保證，使用者不必知道 compinit、syntax-highlighting 必須最後這類規則。
- 開 shell 時不再為 init 啟動外部程式。
- 接管既有 rc 檔時，認得的 init 寫法變成帶選項的 provider module。

**Non-Goals:**
- repo 自訂 provider（列入 README 的展望）。
- 診斷與最佳化（`add-doctor`）、安裝缺少的工具（`add-packages`）。
- 插件管理器（zinit、antidote）、通用的指令補全 provider、rbenv、fnm、SDKMAN、fish 的 fisher 與 tide：留給之後的批次。
- powerlevel10k 轉 starship 的專屬流程與 starship preset 選擇。
- 編輯工具自己的設定檔（`starship.toml` 等由 dotfile 管理）。

## Decisions

### Provider 定義

每個 provider 是 rosamta 原始碼中的一個 TOML 資料檔，以 `include_str!` 編進執行檔；資料檔描述：

- `id`、中英文名稱、分類（提示字元、框架與插件、環境、工具）。
- 支援的 shell 與各 shell 的 init：執行指令（例如 `zoxide init {shell} {args}`）或要 source 的檔案（例如 `$ZSH/oh-my-zsh.sh`），以及偵測用的執行檔名稱與常見位置。
- 載入階段，以及相對其他 provider 的 `after`／`before`；同一階段內依這些限制排序，限制之外依 `id`。
- 選項：型別（開關、單選、文字、清單）、中英文名稱、預設值，以及對應方式（init 參數、啟動時讀取的環境變數、shell 變數或 `zstyle`）。
- 互斥群組，可宣告在整個 provider 或單一選項上。
- 接管時要辨識的寫法。

無法以資料描述的部分（跨段落的接管辨識、oh-my-zsh 的插件清單、Powerlevel10k 的兩個載入點）以 Rust 實作，掛在對應的 `id` 上。只有需要 init、載入順序或互斥的工具做成 provider；只設環境變數與 PATH 的工具（rustup、uv、bun、Bitwarden SSH agent 等）仍是結構化 module。

替代方案：開放 repo 的 `providers/` 自訂 provider。要把資料格式當公開介面維護（schema、版本相容），而使用者的工具用片段 module 已能載入，因此不做。

### Provider module

```toml
# modules/zoxide.toml
provider = "zoxide"
when = { os = ["macos"] }

[options]
cmd = "cd"
```

- 只接受 `provider`、`options`、`enabled`、`when`、`shells`；`shells` 只能縮小 provider 支援的範圍。`env`、`path`、`aliases`、`raw`、`phase`、`order` 在 provider module 中是驗證錯誤：階段與先後由 provider 決定。
- 同一個 provider 在一台 host 上最多一個 module 生效，不同 host 可以各有一個（選項不同）；檢查方式與互斥相同。
- 未知的 provider、未知或型別不符的選項是驗證錯誤。

### Init 輸出

render 時執行 init 指令，把輸出直接寫進 generated script；開 shell 時不再啟動外部程式。

- 執行檔先以 login shell 的 PATH 尋找，再找 provider 列出的常見位置（例如 `/opt/homebrew/bin`、`~/.nvm/nvm.sh`、`~/miniconda3`）；找到的絕對路徑寫進 generated script 與 render 紀錄。
- init 會再呼叫自己的工具取完整輸出，例如 starship 用 `starship init <shell> --print-full-init`。
- 背景服務監看各 provider 執行檔的路徑、大小與修改時間，改變時重新 render；`rsmt apply` 也會重新執行。
- 找不到執行檔時不產生這個 provider 的內容，GUI 標示「未安裝」，`rsmt apply` 印一行提示，結束碼不受影響。
- init 指令失敗或逾時（10 秒）時沿用上一次成功的輸出並通知；從未成功過則視同未安裝。
- eject 寫回 rc 檔的是 provider 的標準 init 寫法（例如 `eval "$(zoxide init zsh --cmd cd)"`），不是快取的輸出。

替代方案：generated script 中寫 `eval "$(zoxide init zsh)"`，每次開 shell 執行。較簡單，但每個工具都增加數毫秒到數十毫秒的啟動時間；rosamta 本來就預先 render，預設快取最自然。

### 互斥

- 啟用 provider 時，若本機已有同一互斥群組的另一個 provider 生效，CLI 與 GUI 先確認；同意後把另一個 module 的 `enabled` 設為 false（所有主機都停用），兩者在同一個 commit。
- 互斥只在兩個 module 的 `when` 都符合的 host 上成立。
- 選項層級的互斥：fzf 的 Ctrl-R 歷史搜尋與 atuin 互斥。啟用 atuin 時若 fzf 的 Ctrl-R 是開的，確認後關閉該選項，fzf 其他功能照常。
- repo 仍出現衝突（手動編輯、從其他 host 拉取）時，比照 module 名稱重複：`rsmt apply` 以 2 結束、不 render，原本的 generated script 不變，背景服務通知。
- 第一批的群組：提示字元（starship、Powerlevel10k、oh-my-posh）、補全初始化（compinit、oh-my-zsh）、語法標示（zsh-syntax-highlighting、fast-syntax-highlighting）、Ctrl-R（fzf 的選項、atuin）。

### compinit

`compinit` 是內建 provider（只有 zsh），放在 framework 階段、所有 `fpath` 設定之後；zsh-completions 排在它之前，fzf-tab 在它之後、zsh-autosuggestions 之前。oh-my-zsh 自己會呼叫 compinit，因此兩者互斥。接管時認出常見寫法（`autoload -Uz compinit && compinit` 與其變形）就啟用它。

### 主題設定檔

`starship.toml`、`~/.p10k.zsh`、oh-my-posh 主題檔以一般 dotfile（link 部署）管理，不設定 `STARSHIP_CONFIG`；provider 只負責 init。

### 接管

- 一段內容只有 provider 的寫法與它認得的選項設定時，轉成 provider module；有不認得的參數或行時，整段照現有規則存成片段 module。
- 跨段落的寫法由 provider 合併成一個 module（例如 Powerlevel10k 的 instant prompt 與 `source ~/.p10k.zsh`）。
- 本機已有同一 provider 的 module 生效時，不論選項是否相同都視為重新寫回的內容（多半是安裝程式寫回預設行），依 host 設定刪除或註解掉，原文留在 snapshot。
- 接管後會與本機生效中的其他 provider 互斥時，不論接管偏好都放進待處理清單；接受時新增 module 並依互斥規則停用另一個。
- 跨主機的合併依 `refine-adopt-scope`：provider 與選項都相同才算同一內容。

### CLI

```
rsmt providers                              # ID、名稱、是否安裝、本機是否啟用、互斥群組
rsmt providers enable <id> [--set key=value]... [--host] [--yes]
rsmt providers disable <id>
```

`enable` 新增或啟用 module，預設套用到所有 host，`--host` 只套用到本機；遇到互斥時在終端機確認，`--yes` 直接停用另一個。兩個指令各建立一個 commit，依推送偏好推送；都支援 `--json`。

### App

- 側欄新增「整合」（在模組與設定檔之間），依分類列出所有內建 provider：開關、支援的 shell、「未安裝」與「部分主機」徽章。
- 詳細頁：狀態（載入階段、Shell、位置、不能同時啟用）、套用範圍（所有主機／本機）、選項；右上角顯示 module 檔案路徑。
- 互斥確認：「停用 {old}？」「{new} 與 {old} 不能同時啟用，所有主機都會停用 {old}。」按鈕「取消」「改用 {new}」。
- 待處理項目：接管會停用另一個 provider 時，顯示「接管後，所有主機都會停用 {name}」。
- 模組頁：provider module 帶「整合」徽章，點擊進入詳細頁。
- 介面不顯示 CLI 參數；文字已審閱（2026-11-11）。

## Risks / Trade-offs

- [init 輸出與工具版本不一致] 背景服務沒在跑的 host 升級工具後，要等下次 `rsmt apply` 才更新 → 監看執行檔並在 apply 時重新產生；init 失敗時沿用上一次的輸出。
- [工具自己改 rc 檔] conda、rbenv、Docker Desktop 等升級時會再寫回 init → 接管把它們當成重新寫回的內容移除。
- [資料檔與工具實際行為脫節] 工具改了 init 參數 → 每個 provider 有以真實執行檔驗證輸出的測試（CI 安裝工具），未安裝時跳過。
- [第一批 20 個 provider 的維護量] 只收需要 init、順序或互斥的工具，其餘延後。

## Migration Plan

新增功能，不改既有 module 的格式；既有片段 module 中的 provider 寫法不自動轉換，使用者可用 `rsmt restore` 還原後重新接管，或手動改寫。

## Open Questions

- 各 provider 選項的完整清單與中英文名稱在實作每個 provider 時列出審閱。
