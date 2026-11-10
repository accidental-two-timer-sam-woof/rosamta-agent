# Design

## Context

repo 目前只有規劃文件。相關的既定決策：GPUI 透過 `gpui-kit`（內含 `gpui-component`）引入並鎖定版本（ADR 0005）、不提供 Flatpak 與 Snap（ADR 0009）、macOS 最低版本 13、授權為 Apache-2.0。

## Goals / Non-Goals

**Goals:**
- 本機與 CI 用同一套指令建置與測試。
- 在真實的 shell 與發行版上跑整合測試，而不是只靠模擬。
- 從第一個版本開始就能產出已簽章的 macOS app 與各 Linux 套件。

**Non-Goals:**
- Windows 的建置與測試。
- 自動更新機制（更新交給套件管理器）。

## Decisions

### Workspace 結構

`crates/rosamta-core`、`crates/rosamta`、`crates/rosamta-gui` 三個 crate。CLI 不依賴 GPUI，所以伺服器上只裝 CLI 不會拉進 GUI 的依賴。`rust-toolchain.toml` 鎖定開工當時的 stable 版本，升級時另開 PR。

### 兩個命令名稱

`crates/rosamta` 有 `rosamta` 與 `rsmt` 兩個 bin，入口放在 `src/lib.rs`，`src/main.rs` 與 `src/bin/rsmt.rs` 都只呼叫它（兩個 bin 指向同一個檔案時 cargo 每次建置都會警告），讓 `cargo install` 的使用者也能直接用兩個名稱。發布的套件只放 `rosamta`，`rsmt` 以 symlink 建立，避免重複佔用空間。

替代方案是只有一個執行檔、由使用者自己設定 alias，但這樣文件裡的 `rsmt` 指令在 `cargo install` 的環境下會失效。

### 授權檢查

`deny.toml` 允許 MIT、Apache-2.0、BSD-2-Clause、BSD-3-Clause、ISC、Zlib、Unicode-3.0、MPL-2.0，以及經人工判斷後加入的 0BSD 與 CC0-1.0（`notify` 與 GPUI 的依賴使用）；其他授權讓 CI 失敗，交由人工判斷。已知漏洞一律失敗；unmaintained 只檢查 workspace 直接依賴的 crate，GPUI 深層依賴的部分交給上游。`cargo-about` 依 `about.toml` 產生第三方聲明，release 時一併打包。

### CI

- `ci.yml` 在每個 PR 與 push 上，於 `ubuntu-24.04` 與 `macos-latest` 執行 fmt、`clippy -D warnings`、`cargo test`，在 Ubuntu 執行 `cargo deny check`。repo 為 public，Actions 分鐘數不受限，macOS job 不需要只在 `main` 上執行。
- Docker 矩陣：在 CI 先建出 Linux x86_64 的 musl 靜態執行檔與整合測試執行檔，再掛載到各發行版容器中執行。容器內不安裝 Rust，映像保持精簡，也能直接用 GHA cache。
- 本機開發用 OrbStack 執行同一支 `tests/docker/run.sh`。

### Release

- macOS：在 arm64 runner 上分別建置 `aarch64-apple-darwin` 與 `x86_64-apple-darwin`，用 `lipo` 合成 universal binary，包成一個 `.app`、一個 `.dmg`。以 Developer ID Application 憑證簽章，用 `xcrun notarytool` 送 notarization 並 staple。
- Linux：x86_64 與 aarch64（使用 `ubuntu-24.04-arm` runner）分別建置，以 `cargo-deb`、`cargo-generate-rpm` 打包，另附 tarball。
  - 用 `cargo zigbuild` 指定 glibc 2.28 連結，讓套件能裝在 `tests/docker` 的每個發行版上（最舊的是 Rocky 8）。runner 上的共享函式庫引用了較新的 glibc，所以連結時加上 `--allow-shlib-undefined`；執行時用的是目標系統自己的函式庫。
  - GUI 以 `dlopen` 載入 Vulkan 與 Wayland，自動偵測看不到，deb 與 rpm 都手動列出。cargo-generate-rpm 內建的偵測會把建置機上的間接依賴與 soname 寫進去，所以 rpm 關閉自動偵測，改列 GUI 實際連結的 soname。
- Homebrew：release 對 tap repo 開 PR 更新 cask；AUR：推送更新後的 PKGBUILD。
- `release.yml` 提供 `workflow_dispatch` 的 dry-run 模式：只建置與打包，略過簽章與發布，用來在沒有 secrets 時驗證流程。
- 版本帶 `-` 的 tag（例如 `v0.1.0-rc.1`）發布為 prerelease，不推送 AUR，用來在不佔用正式版本號的情況下測試完整流程。deb 版本由 cargo-deb 轉成 `0.1.0~rc.1`；rpm 版本不能含 `-`，由 workflow 轉成 `~`。
- `.app` 的 bundle ID 為 `tw.xinshou.rosamta`，執行檔以 `MACOSX_DEPLOYMENT_TARGET=13.0` 建置。CLI 放在 `Contents/MacOS/rosamta`，由 cask 的 `binary` 連到 PATH，`rsmt` 也一樣。
- deb 用 cargo-deb 的 symlink asset 建立 `rsmt`；cargo-generate-rpm 沒有 symlink asset，rpm 改在安裝後的腳本建立，完整移除時刪除。
- 需要的 secrets：`MACOS_CERT_P12`、`MACOS_CERT_PASSWORD`、`MACOS_SIGN_IDENTITY`、`MACOS_NOTARY_KEY`、`MACOS_NOTARY_KEY_ID`、`MACOS_NOTARY_ISSUER`、`HOMEBREW_TAP_TOKEN`、`AUR_SSH_PRIVATE_KEY`；variables：`HOMEBREW_TAP`、`AUR_USERNAME`、`AUR_EMAIL`。

### Dependabot

`cargo` 與 `github-actions` 每週一次，minor 與 patch 合成一個 group PR；`gpui-kit`、`gpui-component` 與 `gpui-pre*` 列入 ignore，它們的升級依 ADR 0005 另開 change。

### 安裝程式 fixture

在 Linux 容器內實際執行安裝程式（bun、uv、rustup、nvm、conda、Homebrew on Linux、gcloud 等），保存 rc 檔安裝前後的內容到 `tests/fixtures/installers/<tool>/`。只在 macOS 上才有的安裝程式（OrbStack、JetBrains Toolbox）由開發者在 Mac 上手動擷取，並移除個人路徑後提交。

## Risks / Trade-offs

- [`gpui-pre` 是第三方發布的 snapshot，可能突然無法建置] → 版本鎖定，並在 CI 快取依賴；GUI crate 建置失敗不影響 CLI 的 release。
- [安裝程式的行為會隨版本改變，fixture 過時] → 擷取腳本可重跑，fixture 註明擷取日期與版本。
