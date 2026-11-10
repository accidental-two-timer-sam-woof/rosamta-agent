# Spec Delta

## Purpose

避免使用者寫在 rc 檔中的 API key、token 等秘密在接管時被寫進 repo，進而被推送到遠端。

## ADDED Requirements

### Requirement: Detect likely secrets
接管時 rosamta SHALL 以已知 token 格式與高熵字串規則檢查每個段落；名稱含 `KEY`、`TOKEN`、`SECRET`、`PASSWORD` 且值為字面字串的環境變數 SHALL 也視為疑似秘密。

#### Scenario: Known token format
- **WHEN** 一段內容含有 `export OPENAI_API_KEY=sk-` 開頭的字串
- **THEN** 該段被標記為疑似秘密

#### Scenario: Reference is not a secret
- **WHEN** 一段內容是 `export GITHUB_TOKEN="$(cat ~/.config/gh/token)"`
- **THEN** 該段不因變數名稱被標記，因為值不是字面字串

### Requirement: Secrets stay local
被標記為疑似秘密的段落 SHALL 存成只在本機生效的 module，位於 rosamta 設定目錄下的 `local/`，不在 repo 內；它 SHALL 照常被 render 並生效，且 SHALL 不出現在任何 commit 中。

#### Scenario: Adopt a secret
- **WHEN** 使用者接管含有 API key 的段落
- **THEN** 新開的 shell 中該變數有值，repo 的歷史中找不到這個值

### Requirement: Secret notice
段落被標記為疑似秘密時，rosamta SHALL 在待處理項目、`rsmt adopt` 的確認畫面與通知中標示它將只存在本機。

#### Scenario: Review shows the marker
- **WHEN** 使用者執行 `rsmt review`
- **THEN** 疑似秘密的項目旁標示它只會存在本機
