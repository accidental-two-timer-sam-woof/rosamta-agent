## REMOVED Requirements

### Requirement: Environment loaded once
**Reason**: 結構化內容可以重複載入，片段要依來源檔被讀取的次數執行；跨程序的防重複讓巢狀 shell 失去沒有 export 的設定（#35）。
**Migration**: 不需要動作。PATH 不重複由 shell-render 的 PATH entries 負責。

## ADDED Requirements

### Requirement: Every shell loads what it reads
每個 shell 程序 SHALL 載入它讀取的啟動檔所對應的全部內容，不因父程序已經載入過而略過；同一個程序讀取多個啟動檔時，結構化的內容 SHALL 不產生重複的 PATH 項目。

#### Scenario: Nested zsh
- **WHEN** `~/.zshenv` 的片段設定 `fpath` 與 `setopt extended_glob`，在 zsh 中再執行 `zsh -c`
- **THEN** 內層的 zsh 也有這兩個設定

#### Scenario: Login bash that reads bashrc
- **WHEN** login bash 同時讀取 `~/.bash_profile` 與 `~/.bashrc`
- **THEN** PATH 沒有重複的項目，rosamta 的互動設定只載入一次
