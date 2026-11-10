# Proposal

## Why

之後每個 change 都需要同一套 Cargo workspace、測試環境與 CI。先把這些建好，每個 PR 才能自動驗證，跨發行版與跨 shell 的行為也才有地方測。

## What Changes

- 建立 Cargo workspace：`rosamta-core`（library）、`rosamta`（CLI，含 daemon 子指令）、`rosamta-gui`（GPUI，先只有空視窗）。執行檔同時提供 `rosamta` 與 `rsmt` 兩個名稱。
- 授權與相依套件檢查：`cargo-deny` 檢查授權是否與 Apache-2.0 相容、是否有已知漏洞；以 `cargo-about` 產生 `THIRD_PARTY_NOTICES.md`。
- GitHub Actions：
  - `ci.yml`：每個 PR 與 push 執行 fmt、clippy、測試、cargo-deny，以及 Docker 的發行版 × shell 矩陣。
  - `release.yml`：推上 `v*` tag 時建置、簽章、notarize 並發布到 GitHub Releases、Homebrew tap 與 AUR。
- Dependabot：`cargo` 與 `github-actions` 每週檢查，排除 `gpui-kit`、`gpui-component` 與 `gpui-pre*`（ADR 0005）。
- Docker 測試環境：Ubuntu 22.04／24.04、Debian 12／13、Fedora、Arch、Alpine、AlmaLinux 9 各裝齊 sh、bash、zsh、fish；Rocky 8 只測 sh 與 bash。
- 安裝程式 fixture：收集真實安裝程式寫入 rc 檔前後的內容，供接管功能測試。

## Capabilities

### New Capabilities

（無。本 change 只建立工具與流程，不改變系統行為。）

### Modified Capabilities

（無）

## Impact

- 新增 `Cargo.toml`、`rust-toolchain.toml`、`crates/`、`.github/`、`deny.toml`、`about.toml`、`tests/docker/`、`tests/fixtures/installers/`。
- 使用者需要在 GitHub Secrets 加入 Apple Developer ID 憑證與 notarization 金鑰，release workflow 才能完成簽章；另需一個 Homebrew tap repo 與 AUR 帳號。
- 其餘所有 change 都建立在這個 workspace 上。
