# Tasks

## 1. Workspace

- [x] 1.1 建立 Cargo workspace、三個 crate 與 `rust-toolchain.toml`，確認 `cargo build --workspace` 成功
- [x] 1.2 在 `crates/rosamta` 宣告 `rosamta` 與 `rsmt` 兩個 bin，確認兩者執行 `--version` 輸出相同
- [x] 1.3 `rosamta-gui` 以鎖定確切版本的 `gpui-kit` 開出一個空視窗，確認在 macOS 上 `cargo run -p rosamta-gui` 能顯示視窗

## 2. 授權與相依套件

- [x] 2.1 新增 `deny.toml`，確認 `cargo deny check` 通過
- [x] 2.2 新增 `about.toml` 並產生 `THIRD_PARTY_NOTICES.md`，確認檔案列出所有相依套件的授權

## 3. CI

- [x] 3.1 新增 `ci.yml`（fmt、clippy、test、cargo-deny；Ubuntu 與 macOS 都在每個 PR 上執行），確認在一個測試 PR 上全部通過
- [x] 3.2 新增各發行版的 Dockerfile 與 `tests/docker/run.sh`，確認每個映像都能印出 sh、bash、zsh、fish 的版本（Rocky 8 只有 sh 與 bash），本機 OrbStack 與 CI 都要通過
- [x] 3.3 把 Docker 矩陣加進 `ci.yml`，確認 musl 靜態執行檔掛載進容器後能執行
- [x] 3.4 新增 `.github/dependabot.yml`（排除 `gpui-kit`、`gpui-component` 與 `gpui-pre*`），推上 GitHub 後確認 repo 的 Dependabot 設定頁沒有錯誤，且第一次排程有開出 PR 或回報已是最新

## 4. Release

- [x] 4.1 新增 `release.yml` 的建置與打包步驟（macOS universal、Linux x86_64／aarch64、deb、rpm、tarball），確認 `workflow_dispatch` dry-run 產出全部檔案
- [x] 4.2 確認 deb 與 rpm 在對應的 Docker 映像中安裝後，`rosamta` 與 `rsmt` 都能執行
- [x] 4.3 加入 macOS 簽章、notarization 與 staple 步驟，在使用者設定好 secrets 後以測試 tag 驗證 `spctl --assess` 通過
- [x] 4.4 加入 Homebrew tap PR 與 AUR 推送步驟，對測試用的 tap repo 驗證會產生正確的 cask PR

## 5. 安裝程式 fixture

- [x] 5.1 撰寫擷取腳本，在容器中執行安裝程式並保存 rc 檔前後內容，確認重跑可得到相同結構的 fixture
- [x] 5.2 手動擷取 OrbStack 與 JetBrains Toolbox 的寫入內容並移除個人路徑，確認 fixture 中沒有 `/Users/<name>` 這類路徑
- [x] 5.3 在 `CONTRIBUTING.md` 說明如何重新擷取 fixture，確認文件中的指令可照抄執行
