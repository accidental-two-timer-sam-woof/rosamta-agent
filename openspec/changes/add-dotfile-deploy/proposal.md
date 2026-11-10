# Proposal

## Why

完全取代 chezmoi，除了 shell 環境，也要管理 nvim、ghostty、git 等其他 dotfile。這些檔案要能在各 host 間同步，app 自己改了設定也要能回到 repo，而私密檔案必須加密後才能進 repo。

## What Changes

- `dotfiles.toml` 明確宣告每個 dotfile 的部署方式、權限、加密與套用範圍（ADR 0007）；`home/` 中沒有宣告的檔案由診斷回報，不會被部署。
- 三種部署方式：
  - link（預設）：以目錄為單位 symlink 回 repo；app 把 symlink 換成一般檔案時，自動把內容收回 repo 並重建 symlink。
  - copy：複製到目標位置；被外部修改時依接管偏好收回 repo。
  - template：render 成實體檔案；被外部修改時無法反推，只通知並提供三方 diff 與「在編輯器開啟 template」。
- 權限：可宣告檔案權限（例如 `0600`），套用時強制設定。
- 加密：以 age 加密檔案；私鑰以使用者設定的密碼加密後放進 repo，新 host 輸入一次密碼即可（ADR 0012）。
- 目標位置已有內容不同的檔案時不直接覆蓋，逐一讓使用者選擇採用 repo 版本、保留本機版本（收進 repo 並限定本機）或稍後處理。
- CLI：`rsmt add <path>`（自動寫入 `dotfiles.toml`）、`rsmt forget <path>`。

## Capabilities

### New Capabilities

- `dotfile-deploy`: dotfile 的宣告、三種部署方式、權限、衝突處理與外部修改的處理。
- `encryption`: 加密 dotfile、私鑰的保存與在新 host 上解開。

### Modified Capabilities

- `apply`: 套用流程加入 dotfile 部署。
- `daemon`: 監看 copy 與 template 部署出去的檔案，以及 link 是否被取代。

## Impact

- 依賴 `add-repo-and-host-config` 的套用範圍、`add-shell-render` 的套用流程、`add-adopt` 的接管偏好。
- template 部署方式需要 `add-templates-and-secrets`；在它完成前，template 模式先回報為不支援。
- 新依賴：`age` crate（需要 armor 功能以讀取 chezmoi 加密的檔案）。
