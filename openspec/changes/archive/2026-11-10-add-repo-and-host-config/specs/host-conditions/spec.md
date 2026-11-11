# Spec Delta

## Purpose

判斷每個 module 與 dotfile 是否套用到目前的 host，讓同一個 repo 能涵蓋不同作業系統、架構與用途的主機。

## ADDED Requirements

### Requirement: Condition fields
module 與 dotfile SHALL 能以 `when` 宣告套用範圍，欄位為 `os`、`arch`、`hosts`、`tags`，每個欄位都是字串清單。沒有 `when` 時 SHALL 套用到所有 host。

#### Scenario: No condition
- **WHEN** 一個 module 沒有 `when`
- **THEN** 它套用到每一台 host

### Requirement: Matching rules
同一個欄位中的多個值 SHALL 以「任一符合」判斷，不同欄位之間 SHALL 以「全部符合」判斷。

#### Scenario: Multiple OS values
- **WHEN** module 宣告 `os = ["macos", "linux"]`，目前 host 是 Linux
- **THEN** 該 module 套用

#### Scenario: OS and tag combined
- **WHEN** module 宣告 `os = ["macos"]` 與 `tags = ["work"]`，目前 host 是 macOS 但沒有 `work` 標籤
- **THEN** 該 module 不套用

### Requirement: Hosts match host IDs
`hosts` 中的值 SHALL 與 host 的識別名稱比對，不與系統 hostname 比對。

#### Scenario: Hostname differs from host ID
- **WHEN** module 宣告 `hosts = ["mbp"]`，目前 host 的識別名稱是 `mbp`、系統 hostname 是 `mbp-2`
- **THEN** 該 module 套用

### Requirement: Known values
`os` 的值 SHALL 是 `macos` 或 `linux`，`arch` 的值 SHALL 是 `x86_64` 或 `aarch64`；其他值 SHALL 讓 `rsmt validate` 失敗，並列出可用的值。

#### Scenario: Typo in os
- **WHEN** module 宣告 `os = ["darwin"]`
- **THEN** `rsmt validate` 失敗，並列出 `macos` 與 `linux`

### Requirement: Unknown host IDs
`hosts` 引用了不在 host 清單中的識別名稱時，`rsmt validate` SHALL 回報警告，但不視為失敗。

#### Scenario: Host not registered yet
- **WHEN** module 宣告 `hosts = ["new-laptop"]`，而 host 清單中沒有 `new-laptop`
- **THEN** `rsmt validate` 以 0 結束，並在 stderr 輸出警告
