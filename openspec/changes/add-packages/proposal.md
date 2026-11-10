# Proposal

## Why

新機器上要先裝好工具，設定才有意義；provider 與秘密來源也依賴特定的程式（例如 starship、rbw）。目前沒有現成的資料能把一個應用程式名稱對應到各套件管理器的 ID，所以 rosamta 需要自己維護 catalog，並能在 GUI 中一鍵安裝。

## What Changes

- catalog：隨 rosamta 發布的 TOML 資料，每筆記錄應用程式名稱、偵測方式，以及在 brew、apt、dnf、pacman、zypper、apk、nix、winget、scoop、choco、snap、flatpak、npm、cargo、aqua 上的 ID；使用者可在 repo 的 `packages/` 自行新增。
- 安裝來源的優先順序：先用 host 的原生套件管理器，其次 flatpak 或 snap，再來是語言套件管理器，最後是 GitHub release；每台 host 可調整順序。
- `packages.toml` 宣告各 host 應有的應用程式（可加套用範圍）；provider 與秘密來源也可宣告需要的應用程式。`rsmt bootstrap` 補裝缺少的應用程式。
- 安裝一律先顯示完整指令，確認後才執行：
  - macOS：brew 以使用者身分執行，並設定 `HOMEBREW_NO_SUDO=1`；遇到需要 sudo 的 cask 時，改為顯示指令與「複製」按鈕。
  - Linux：需要 root 的套件管理器透過 pkexec 執行；沒有 polkit agent 時，CLI 在終端機中用 sudo，GUI 則顯示指令與「複製」按鈕。
  - rosamta 永遠不經手使用者的密碼；app 無法得知使用者預設的終端機，所以不提供「在終端機開啟」。
- 背景服務不自動安裝應用程式。
- 指令組裝依賴 mise 的 `mise-system-packages` crate 並鎖定版本；不合用時在保留 MIT 聲明的前提下移植。

## Capabilities

### New Capabilities

- `package-catalog`: catalog 的格式、內建資料與使用者擴充。
- `packages`: 應用程式的宣告、偵測、安裝來源的選擇、安裝流程與權限提升。

### Modified Capabilities

（無）
- GUI：在 `add-gui` 建立的 app 中加入本 change 對應的頁面或步驟。

## Impact

- 新依賴：`mise-system-packages`（鎖定版本）。
- 新增 `rsmt install <package>`、`rsmt bootstrap`。
- `add-providers-and-doctor` 與 `add-templates-and-secrets` 會宣告它們需要的應用程式。
