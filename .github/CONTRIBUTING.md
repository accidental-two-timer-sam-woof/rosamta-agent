# 開發指南

## 環境

- Rust：安裝 rustup，版本由 `rust-toolchain.toml` 決定，第一次執行 `cargo` 時自動下載。
- macOS：GPUI 需要 Xcode 的 Metal 編譯器。裝好 Xcode 後執行：

  ```sh
  xcodebuild -downloadComponent MetalToolchain
  sudo xcode-select -s /Applications/Xcode.app
  ```

  不想切換 `xcode-select` 時，可以在建置前設定 `DEVELOPER_DIR=/Applications/Xcode.app/Contents/Developer`。
- Linux（Ubuntu 24.04）：

  ```sh
  sudo apt-get install -y --no-install-recommends gcc g++ pkg-config libfontconfig-dev libwayland-dev libxkbcommon-x11-dev libx11-xcb-dev libzstd-dev
  ```

- Docker 測試需要 Docker 或 OrbStack。

## 檢查

送 PR 前在本機跑一次，CI 會跑同一套：

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
cargo deny check
```

依賴有變動時重新產生第三方授權聲明：

```sh
cargo about generate --workspace -c packaging/about.toml packaging/about.md.hbs -o THIRD_PARTY_NOTICES.md
```

## 發行版與 shell 測試

`tests/docker/` 每個 `*.Dockerfile` 是一個發行版映像，裝好該發行版支援的 shell。在每個映像裡印出 shell 版本：

```sh
tests/docker/run.sh                 # 全部
tests/docker/run.sh debian-12 arch  # 指定映像
```

映像一律以 `linux/amd64` 執行，與 CI 相同。把 Linux 執行檔所在的目錄設成 `ROSAMTA_BIN_DIR`，會掛載到容器的 `/opt/rosamta/bin` 並逐一執行 `--version`。

## 安裝程式 fixture

`tests/fixtures/installers/<工具>/` 保存安裝程式寫入 shell 啟動檔前後的內容，供接管功能測試。`before/` 是安裝前的檔案（安裝程式新建的檔案沒有 `before/`），`after/` 是安裝後的檔案，`meta` 記錄擷取日期、版本與指令。家目錄一律寫成 `@HOME@`。

在容器中重新擷取：

```sh
tests/fixtures/installers/capture.sh           # 全部
tests/fixtures/installers/capture.sh uv nvm    # 指定工具
```

只在 macOS 上提供的安裝程式（`orbstack`、`jetbrains-toolbox`）要手動擷取：

1. 備份會被寫入的檔案，例如 `cp ~/.zprofile /tmp/zprofile.before`。
2. 安裝或更新該 app，等它寫入。
3. 用 `diff /tmp/zprofile.before ~/.zprofile` 找出新增的內容，存成 `<工具>/after/<檔名>`，把家目錄換成 `@HOME@`。
4. 更新 `meta` 的日期與版本。
5. 確認沒有留下個人路徑：

   ```sh
   grep -rnE '/(Users|home)/[^/@]' tests/fixtures/installers/*/ | grep -v /home/linuxbrew
   ```

   沒有輸出才能提交。
