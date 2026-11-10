# Proposal

## Why

只靠結構化與片段 module，rosamta 不知道 zoxide、starship、oh-my-zsh 這些工具的載入順序限制、彼此的互斥關係與可調整的選項，也無法發現使用者既有設定中的問題。provider 讓 rosamta 理解常見工具；診斷則把問題與最佳化建議變成可以一鍵套用的修正。最佳化同時是專題的特色重點。

## What Changes

- provider：每個 provider 以資料檔描述偵測方式、選項（含中英文顯示名稱，介面不顯示 CLI 參數）、各 shell 的產生內容、載入階段、互斥關係、需要的套件，以及接管時要辨識的寫法；複雜的部分以 Rust 實作。
- 第一批 provider：
  - prompt：starship、oh-my-posh、powerlevel10k
  - 框架與插件：oh-my-zsh、zsh-autosuggestions、zsh-syntax-highlighting
  - 環境：Homebrew、rustup／cargo、mise、uv、bun、conda、nvm
  - 工具：zoxide、fzf、direnv、atuin
  - 安全：Bitwarden SSH agent、GnuPG
- starship：`starship.toml` 放在 repo，以 `STARSHIP_CONFIG` 指向它；提供從 powerlevel10k 切換到 starship 的流程，切換前先顯示差異。
- 互斥：啟用與既有 provider 互斥的 provider 時，先確認再停用另一個。
- 接管時辨識 provider 的寫法（例如 `eval "$(zoxide init zsh --cmd cd)"`），轉成帶選項的 provider module。
- 診斷：
  - 交接文件列出的衝突規則（PowerShell 相關的延後到 Windows 支援時）。
  - 新增四條規則：同一行執行多次、同一變數在不同檔案有不同值、PATH 寫在別的 shell 讀不到的檔案、指向不存在的路徑。
  - 每一項都提供修正，套用前顯示 diff，可還原；有多個可能值時列出所有偵測到的值讓使用者選擇。
- 最佳化：
  - 快取 init 與補全腳本的輸出，執行檔更新時才重建。
  - 整理 PATH。
  - `compinit` 只執行一次。
  - 以 `zcompile` 預先編譯 zsh 腳本。
  - nvm、conda 拖慢啟動超過 100 ms 時，建議改為延遲載入。
  - 每一項最佳化都量測套用前後的啟動時間。
- 執行時機：每次 render 之後、手動執行 `rsmt doctor`，以及設定精靈的「掃描與診斷」步驟。

## Capabilities

### New Capabilities

- `providers`: provider 的定義、選項、互斥、載入規則與接管辨識。
- `doctor`: 診斷規則、修正、最佳化與量測。

### Modified Capabilities

- `adopt`: 加入 provider 辨識器。
- `shell-render`: render provider module。

## Impact

- 新增 provider 資料檔目錄與 `rsmt doctor`（`--json`、`--fix <id>`）。
- 部分 provider 需要套件，依賴 `add-packages` 安裝；在它完成前只提示缺少的套件。
