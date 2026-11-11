# Spec Delta

## Purpose

讓私密的 dotfile 以加密的形式存在 repo 中，並讓新 host 只需輸入一次密碼就能解開。

## ADDED Requirements

### Requirement: Encrypted in the repo
`encrypted = true` 的 dotfile SHALL 以 age armor 格式加密後存在 repo 中，檔名與目標相同；部署時解密後寫到目標。

#### Scenario: Add with encryption
- **WHEN** 使用者執行 `rsmt add --encrypt ~/.ssh/config`
- **THEN** repo 的 `home/.ssh/config` 以 `-----BEGIN AGE ENCRYPTED FILE-----` 開頭，`~/.ssh/config` 的內容不變

### Requirement: Passphrase-protected key
rosamta SHALL 在第一次加密時產生一把 age 私鑰，以使用者設定的密碼加密後存成 repo 的 `key.age`，並把私鑰存成本機的 `$XDG_CONFIG_HOME/rosamta/age.key`，權限 0600。設定密碼時 SHALL 要求輸入兩次。

#### Scenario: First encryption
- **WHEN** repo 還沒有 `key.age`，使用者在終端機中執行 `rsmt add --encrypt ~/.ssh/config`
- **THEN** 指令要求設定密碼並輸入兩次，repo 新增 `key.age`，本機新增權限 0600 的 `age.key`

### Requirement: Unlock on a new host
本機沒有私鑰、而要部署加密的 dotfile 時，在互動終端機中執行的 `rsmt apply` SHALL 要求輸入密碼，解開 `key.age` 後存成本機私鑰，之後不再詢問。非互動執行與背景服務 SHALL 略過加密的 dotfile，並回報需要輸入密碼。

#### Scenario: New host
- **WHEN** 新 host clone repo 後在終端機中執行 `rsmt apply`
- **THEN** 指令要求輸入一次密碼，加密的 dotfile 解密後部署，之後的套用不再要求密碼

#### Scenario: Wrong passphrase
- **WHEN** 使用者連續三次輸入錯誤的密碼
- **THEN** 加密的 dotfile 不部署，其他 dotfile 照常部署，指令以 1 結束
