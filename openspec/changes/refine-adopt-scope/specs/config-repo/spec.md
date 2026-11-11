## ADDED Requirements

### Requirement: Recently added on other hosts
`rsmt recent` SHALL 列出 repo 中已啟用、目前不在本機載入的 module 與 dotfile，依它們加入 repo 的時間由新到舊排列；每一項顯示名稱（dotfile 為路徑）、加入的日期，以及套用範圍中的主機、OS 與 shell。`--json` SHALL 輸出相同的內容。只存在本機的 module 不列出。

#### Scenario: New host
- **WHEN** 使用者在剛加入 repo 的主機 `box` 執行 `rsmt recent`，`mbp` 上週接管了 `bun`、昨天新增了 `~/.config/ghostty`，兩者都只套用到 `mbp`
- **THEN** 依序列出 `~/.config/ghostty` 與 `bun`，各自顯示日期與 `mbp`、`macos` 等套用範圍

### Requirement: Include this host
`rsmt include <名稱>...` 或 `rsmt include --all`（所有 `rsmt recent` 列出的項目）SHALL 放寬每個項目的套用範圍，讓它在本機載入：

- `hosts` 有值時加入本機的識別名稱；
- `os`、`arch` 有值且不含本機的值時加入本機的值；
- 結構化 module 的 `shells` 不含任何本機已掛載的 shell 時，加入那些 shell。

片段 module 只套用到本機沒有掛載的 shell，或 `tags` 不符合本機時，SHALL 不修改該項目並說明原因。修改前 SHALL 列出每個項目要放寬的條件並要求確認，`--yes` 可略過；確認後 SHALL 建立一個 commit `include: <名稱>, … on <識別名稱>`、套用，並依推送偏好推送。名稱不存在或已在本機載入時 SHALL 以 2 結束，不修改任何檔案。

#### Scenario: Include a module from another Mac
- **WHEN** macOS 主機 `air` 執行 `rsmt include bun --yes`，`bun` 宣告 `hosts = ["mbp"]`、`os = ["macos"]`、`shells = ["zsh"]`
- **THEN** `bun` 變成 `hosts = ["mbp", "air"]`，repo 多一個 commit，新開的 zsh 載入 bun 的 PATH

#### Scenario: Different OS needs confirmation
- **WHEN** Linux 主機 `box` 執行 `rsmt include bun`
- **THEN** rosamta 列出會加入 `box` 與 `linux`，使用者確認後才修改

#### Scenario: Snippet for another shell
- **WHEN** 只掛載 bash 的主機執行 `rsmt include orbstack`，`orbstack` 是只有 zsh 的片段 module
- **THEN** `orbstack` 不被修改，輸出說明它只適用於 zsh
