# AGENTS.md

給在這個 repo 工作的 agent 的規則。rosamta 是 Rust 寫的 dotfile 管理工具，有 CLI（`rosamta`、`rsmt`）、GPUI 桌面 App 與背景服務。

## 先讀的文件

- 建置環境、送 PR 前的檢查、Docker 測試與 fixture：[`.github/CONTRIBUTING.md`](.github/CONTRIBUTING.md)。
- 命名、寫 spec 或介面文字時：[`GLOSSARY.md`](GLOSSARY.md)。用它的用語，`_Avoid_` 列出的不用。
- 修改行為之前：`openspec/specs/<capability>/spec.md`。行為以 spec 為準。
- 做難以反悔的決定之前：[`docs/adr/`](docs/adr/)。做法與既有的 ADR 不同時，提出新的 ADR。
- 寫 OpenSpec 的產物時：[`openspec/config.yaml`](openspec/config.yaml) 規定語言與格式。

## 由維護者決定的事

做到下列幾處就停下來，得到明確的同意再往下。

- **實作**：change 的 proposal 經維護者核准之後才開始。
- **介面**：先給預覽，維護者說可以才實作。
- **使用者看得到的文字**（介面、通知、CLI 的訊息）：push 之前把改動列給維護者看。
- **push、開 PR、合併、打 tag、發布**：每一次都要同意，一次同意只涵蓋那一次。維護者事先給了條件（例如「CI 過了就合併」）時照條件做。本機的 commit 是檢查點，可以直接做。
- **難以反悔而且有取捨的決定**：提出選項與建議，維護者回答之後寫成 ADR。
- **不確定的事**：寫下問題、程式碼的位置與考慮過的選項，問維護者。

## 維護者的主機與機密

agent 在維護者自己的電腦上執行，而 rosamta 會改寫 shell 啟動檔。

- `rosamta`、`rsmt` 只在暫存的 `HOME` 裡執行。測試用 `crates/cli/tests/common` 的 `Home`：`Home::cmd` 與 `Home::spawn` 把 `HOME` 指到暫存目錄，移除會指向外面的變數（`XDG_*`、`ZDOTDIR`、`ENV`、`BASH_ENV`、`ROSAMTA_*`、`GIT_*`），隔離 git 的設定，並設定 `ROSAMTA_NO_KEYCHAIN=1` 與 `ROSAMTA_NO_NOTIFY=1`；`Home::shell` 以清空的最小環境啟動 shell，沒有設這兩個變數。
- 手動試跑時從空的環境開始，用這個 worktree 建出來的執行檔：`env -i HOME=<暫存目錄> PATH="$PATH" ROSAMTA_NO_KEYCHAIN=1 ROSAMTA_NO_NOTIFY=1 GIT_CONFIG_GLOBAL=/dev/null GIT_CONFIG_NOSYSTEM=1 target/debug/rsmt …`。只換 `HOME` 不夠：rosamta 會讀 `XDG_*` 與 `ZDOTDIR`。
- 維護者真正的 `~/.zshrc`、`~/.config/rosamta` 等檔案不讀也不改。
- 不印出環境變數（不帶參數的 `env`、`printenv`、`set`、`export -p`），測試的失敗訊息也不帶出環境。曾有一個失敗的測試把維護者的環境變數（包含密碼管理工具的 session）印進 agent 的對話。
- 測試需要像密鑰的字串時，用一看就知道是假的值。

## 規格

- 新功能與設計的改變從 `openspec/changes/<name>/` 開始：proposal 經維護者核准，再寫 design、delta spec 與 tasks，實作，最後 archive。`.claude/` 有對應的 skill 與 `/opsx:*` 指令；`/opsx:propose` 會一次產生全部產物，要先核准時只寫 proposal。
- 修正改變了 spec 寫的行為時，在同一個 PR 修改 `openspec/specs/`。
- 改過 spec 之後執行 `openspec validate --specs --strict`；改過 change 之後執行 `openspec validate <name> --type change --strict`。CI 不跑這兩個檢查。
- `add-doctor`、`add-packages`、`add-providers` 還沒有 delta spec，所以 `openspec validate --all --strict` 與對這三個 change 的檢查都會失敗。這是已知的狀態，等 proposal 核准後補上 delta spec 才會通過。

## 程式與測試

- 程式碼、註解與 commit message 用英文；spec、ADR 與文件用繁體中文。新加的註解只寫程式看不出來的原因，盡量一行。
- 邏輯放在 `crates/core`，CLI 與 GUI 都透過它讀寫。GUI 與通知的文字放在 `crates/core/locales/`；GUI 文字的規則在 `openspec/specs/gui/spec.md` 的「Language and theme」。
- 修正之前先寫測試，確認它在修改前失敗，再改到通過。
- 既有的測試與任務的要求衝突時，停下來問維護者；測試維持原樣。
- 支援的範圍是全部的使用者：sh、bash、zsh、fish，macOS 與 Linux，包含沒有桌面的主機。維護者自己的主機用來發現問題，不是支援範圍的邊界。

## Git 與 PR

- Conventional Commits，用英文：標題加條列的內文，PR 標題也用這個格式。每完成一個段落 commit 一次。
- commit 與 tag 都要 GPG 簽章。簽章失敗時停下來告訴維護者，先不 commit。
- main 只經由 PR 改變。合併用 merge commit，保留全部 commit；合併之後刪除分支。
- 合併的條件：最後一個 head 的 CI 全部通過；AI reviewer 的每個 thread 都回覆並 resolve；Greptile 評分 5/5。Greptile 沒有回應時，改由另一個 agent 在本機審查到沒有 HIGH。Greptile 評 3 分以下時，另外檢查其他地方有沒有同類的問題。
- PR 描述依序寫 Summary（最小的圖或 diff 草圖）、Evidence（修改前後的輸出；修正要有修改前失敗的測試）、Merge Danger（能不能回頭、影響範圍）。

## 已知的狀況

- 多個 worktree 共用 `CARGO_TARGET_DIR` 時，測試可能執行到另一個分支建出來的執行檔。每個 worktree 用自己的 target 目錄；共用時一次只跑一個 cargo 指令，看到「修改前就通過」的測試時先重新建置。
- `tests/docker` 預設以 `linux/amd64` 執行。在 Apple Silicon 上模擬時，多執行緒的測試可能整個停住，每個執行緒都停在 `rt_mutex_schedule`；這是模擬造成的，不是 rosamta 的死結。逾時寫成 `ROSAMTA_TEST_CMD='timeout 600 sh /opt/rosamta/tests/check.sh'`（這個變數取代 `check.sh`）；確認時 `tests/docker/build-tests.sh` 與 `run.sh` 都以 `DOCKER_DEFAULT_PLATFORM=linux/arm64` 執行，測試執行檔才是 arm64 的。
