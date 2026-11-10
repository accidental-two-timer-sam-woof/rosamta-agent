# shell-render Specification

## Purpose
把 repo 中的 module 轉成每個支援的 shell 能載入的腳本，讓同一份宣告在 sh、bash、zsh、fish 上產生相同的效果。

## Requirements

### Requirement: Supported shells and version floors
rosamta SHALL 為 POSIX sh、bash 3.2 以上、zsh 5.8 以上、fish 3.6 以上產生 generated script。已安裝但低於下限的 shell SHALL 不產生腳本，`rsmt apply` SHALL 說明原因與升級方式。

#### Scenario: Old fish
- **WHEN** host 上的 fish 版本是 3.3.1
- **THEN** rosamta 不產生 fish 的腳本，`rsmt apply` 輸出警告並說明需要 3.6 以上

#### Scenario: bash on macOS
- **WHEN** host 是 macOS，bash 為內建的 3.2.57
- **THEN** rosamta 產生 bash 的腳本，且腳本不使用 bash 4 以後的語法

### Requirement: Version detection from the shell itself
rosamta SHALL 讀取 shell 自己的版本變數判斷版本（bash 的 `BASH_VERSINFO`、zsh 的 `ZSH_VERSION`、fish 的 `version`），不解析 `--version` 的輸出。POSIX sh 不偵測版本。

#### Scenario: Broken --version
- **WHEN** 某個 fish 套件的 `fish --version` 執行失敗，但 shell 本身可以正常啟動
- **THEN** rosamta 仍能取得 fish 的版本

### Requirement: Phase order
generated script SHALL 依載入階段 env、early、framework、tools、prompt、aliases、late 的順序載入。同一階段內 SHALL 依 module 的 `order`（預設 50）由小到大，再依名稱排序。

#### Scenario: Same phase
- **WHEN** tools 階段有 `zoxide`（未設 order）與 `fzf`（`order = 10`）
- **THEN** `fzf` 先於 `zoxide` 載入

### Requirement: Host conditions applied
rosamta SHALL 只 render 套用範圍符合目前 host 的 module。

#### Scenario: Module for another host
- **WHEN** module 宣告 `hosts = ["lab-server"]`，目前 host 是 `mbp`
- **THEN** 本機的 generated script 不包含這個 module

### Requirement: Structured environment variables
結構化 module 的環境變數 SHALL 以各 shell 的語法輸出。值中的 `$NAME`、`${NAME}` 與開頭的 `~` SHALL 在 shell 中展開，其餘字元 SHALL 原樣保留。

#### Scenario: Quote in value
- **WHEN** module 宣告 `GREETING = "it's $USER"`
- **THEN** 在每個 shell 中 `GREETING` 的值是 `it's ` 加上目前使用者名稱

#### Scenario: fish syntax
- **WHEN** module 宣告 `EDITOR = "nvim"`
- **THEN** fish 的 generated script 以全域匯出的方式設定 `EDITOR`，且不建立 universal 變數

### Requirement: PATH entries
結構化 module 可宣告 PATH 的 prepend 與 append 項目。已經在 PATH 中的目錄 SHALL 不再重複加入。

#### Scenario: Re-sourcing
- **WHEN** 使用者在同一個 shell 中重新載入設定
- **THEN** PATH 中不會出現重複的項目

### Requirement: Aliases only in interactive shells
結構化 module 的 alias SHALL 只在互動 shell 中定義。

#### Scenario: Script execution
- **WHEN** 使用者以 `bash script.sh` 執行腳本
- **THEN** module 宣告的 alias 不會在該腳本中定義

### Requirement: Raw modules target compatible shells
片段 module SHALL 宣告適用的 shell 與各 shell 的內容；rosamta SHALL 只把它放進宣告的 shell 的 generated script。

#### Scenario: zsh-only snippet
- **WHEN** 片段 module 只宣告 zsh 的內容
- **THEN** bash、sh、fish 的 generated script 都不包含它

### Requirement: Environment in every shell mode
env 階段的設定 SHALL 在 login shell、非 login 的互動 shell，以及非互動 shell 中都生效。login zsh 中，系統的 zprofile（例如 macOS 的 `path_helper`、Alpine 的 `/etc/profile`）SHALL 不會打亂或移除 module 宣告的 PATH 項目。

#### Scenario: Non-interactive zsh
- **WHEN** 使用者執行 `zsh -c 'echo $EDITOR'`
- **THEN** 輸出 module 宣告的值

#### Scenario: macOS login zsh
- **WHEN** macOS 上開啟 login zsh，path module 宣告 prepend `$HOME/.local/bin`
- **THEN** `$HOME/.local/bin` 排在 `/usr/bin` 之前
