# Proposal

## Why

只靠結構化與片段 module，rosamta 不知道 zoxide、starship、oh-my-zsh 這些工具的 init 寫法、載入順序限制、彼此的互斥關係與可調整的選項。provider 讓 rosamta 理解常見工具：載入順序由 rosamta 保證，init 的輸出在 render 時預先產生，開 shell 時不再啟動這些程式。

## What Changes

- provider：rosamta 內建、以資料檔描述的第三方工具知識，包含偵測方式、選項（含中英文名稱）、各 shell 的 init、載入階段與先後、互斥關係，以及接管時要辨識的寫法；複雜的部分以 Rust 實作。repo 不能自訂 provider。
- 第一批 provider（依 GitHub 公開 rc 檔的使用量挑選，只收需要 init、載入順序或互斥的工具；只設環境變數與 PATH 的工具仍用結構化 module）：
  - 提示字元：starship、Powerlevel10k、oh-my-posh
  - 框架與插件：oh-my-zsh、compinit、zsh-completions、fzf-tab、zsh-autosuggestions、zsh-history-substring-search、zsh-syntax-highlighting、fast-syntax-highlighting
  - 環境：Homebrew、mise、pyenv、nvm、conda
  - 工具：zoxide、fzf、direnv、atuin
- provider module：`modules/` 下宣告 `provider = "<id>"` 與 `[options]` 的 module；同一個 provider 在一台 host 上最多一個 module 生效。
- render 時執行 init 指令並把輸出寫進 generated script；執行檔改變時重新 render，找不到執行檔時不產生內容並標示未安裝。
- 互斥：同一互斥群組的 provider（或 provider 的單一選項）不能在同一台 host 同時生效；啟用時先確認再停用另一個。
- compinit：zsh 的補全初始化成為內建 provider，與會自行呼叫 compinit 的框架互斥，需要補全的 provider 排在它前後的固定位置。
- 接管：辨識 provider 的寫法並轉成 provider module；本機已有同一 provider 生效時視為重新寫回的內容。
- 主題設定檔（`starship.toml`、`~/.p10k.zsh`、oh-my-posh 主題）以一般 dotfile 管理。
- CLI：`rsmt providers`、`rsmt providers enable`、`rsmt providers disable`。
- GUI：新增「整合」頁、provider 詳細頁、互斥確認，以及模組頁的「整合」徽章。

## Capabilities

### New Capabilities

- `providers`: provider 的定義、provider module、選項、init 輸出、互斥、載入順序與 CLI。

### Modified Capabilities

- `adopt`: 加入 provider 辨識。
- `shell-render`: render provider module 與預先產生的 init 輸出。
- `daemon`: 執行檔改變時重新 render。
- `gui`: 整合頁與詳細頁。

## Impact

- 新增 provider 資料檔目錄（編進執行檔）與 `rsmt providers`。
- 依賴 `refine-adopt-scope` 的套用範圍與合併規則。
- 診斷與最佳化移到 `add-doctor`；安裝缺少的工具留給 `add-packages`，在它完成前只標示未安裝。
