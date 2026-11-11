# secret-guard Specification

## Purpose
避免使用者寫在 rc 檔中的 API key、token 等秘密在接管時被寫進 repo，進而被推送到遠端。

## Requirements

### Requirement: Detect likely secrets
接管時 rosamta SHALL 以已知 token 格式與高熵字串規則檢查每個段落。值為字面字串的變數，名稱含 `KEY`、`TOKEN`、`SECRET`、`PASSWORD`、`PASSWD`、`PASSPHRASE`、`CREDENTIAL`、`PWD`，或全大寫且最後一段是 `AUTH`、`OAUTH`、`PAT`、`PASS`、`DSN`、`CREDS` 時，SHALL 也視為疑似秘密；值不可能是秘密時 SHALL 不算（見 Plain values）。網址中 `://使用者:密碼@` 的字面密碼 SHALL 視為疑似秘密。

#### Scenario: Known token format
- **WHEN** 一段內容含有 `export OPENAI_API_KEY=sk-` 開頭的字串
- **THEN** 該段被標記為疑似秘密

#### Scenario: Password in a URL
- **WHEN** 一段內容是 `export DATABASE_URL=postgres://admin:S3cretPass@db.internal/prod`
- **THEN** 該段被標記為疑似秘密

#### Scenario: Plain value under a secret-looking name
- **WHEN** 一段內容是 `export KEYTIMEOUT=1` 或 `export PASSWORD_STORE_DIR=~/.password-store`
- **THEN** 該段不被標記

#### Scenario: Reference is not a secret
- **WHEN** 一段內容是 `export GITHUB_TOKEN="$(cat ~/.config/gh/token)"`
- **THEN** 該段不因變數名稱被標記，因為值不是字面字串

### Requirement: Plain values
名稱像秘密的變數，值是開關（`true`、`false`、`yes`、`no`、`on`、`off`）、路徑或短數字時 SHALL 不視為疑似秘密。數字在 3 位以內一律不算；4 到 6 位只有在名稱以 `_` 分開的每一段都不是以秘密的字結尾時才不算。以 `/` 開頭、只由 base64 字元組成且各段不像名稱的值 SHALL 不視為路徑。`$` 只有在後面接著名稱、`{`、`(`、`[` 或特殊參數時才視為展開，引號外接著引號時也是，其餘視為字面值的一部分。

#### Scenario: A number that is a setting
- **WHEN** rc 檔新增 `export KEYTIMEOUT=1000` 或 `export MAX_TOKENS=4096`
- **THEN** 不被標記為疑似秘密

#### Scenario: A number that is the secret
- **WHEN** rc 檔新增 `export DB_PASSWORD=482913`
- **THEN** 被標記為疑似秘密

#### Scenario: Base64 that starts with a slash
- **WHEN** rc 檔新增名稱為 `AWS_SECRET_ACCESS_KEY` 的變數，值以 `/` 開頭且只由 base64 字元組成
- **THEN** 被標記為疑似秘密，不當成路徑

#### Scenario: A dollar that stays
- **WHEN** rc 檔新增 `export DB_PASSWORD=Summer2024$`
- **THEN** 被標記為疑似秘密

#### Scenario: A dollar inside double quotes
- **WHEN** rc 檔新增 `export DB_PASSWORD="Xk9$)vB2"`
- **THEN** 被標記為疑似秘密

### Requirement: Unsure literals wait for a decision
接管偏好為「自動」時，沒有被標記為疑似秘密、但含有 16 個字元以上且不是由單字組成的字面值（例如一般名稱下的 hex 字串）的段落，SHALL 不自動接管，而是放進待處理清單，由使用者決定。

#### Scenario: Hex key under a plain name
- **WHEN** 接管偏好為「自動」，安裝程式在 `~/.zshrc` 寫入 `export SERVICE_ID=` 加上 32 個 hex 字元
- **THEN** 這一行留在 `~/.zshrc` 並出現在待處理清單，repo 的歷史中找不到這個值

#### Scenario: Ordinary setting
- **WHEN** 接管偏好為「自動」，安裝程式寫入 `export EDITOR=nvim`
- **THEN** 這一行被自動接管

### Requirement: Secrets stay local
被標記為疑似秘密的段落 SHALL 存成只在本機生效的 module，位於 rosamta 設定目錄下的 `local/`，不在 repo 內，檔案權限為 0600；它 SHALL 照常被 render 並生效，且 SHALL 不出現在任何 commit 中。

#### Scenario: Adopt a secret
- **WHEN** 使用者接管含有 API key 的段落
- **THEN** 新開的 shell 中該變數有值，repo 的歷史中找不到這個值

### Requirement: Secret notice
段落被標記為疑似秘密時，rosamta SHALL 在待處理項目、`rsmt adopt` 的確認畫面與通知中標示它將只存在本機。

#### Scenario: Review shows the marker
- **WHEN** 使用者執行 `rsmt review`
- **THEN** 疑似秘密的項目旁標示它只會存在本機

### Requirement: Scan before pushing
每次推送前，rosamta SHALL 以相同的秘密規則掃描所有即將推送的 commit 內容；為使用者手動修改建立 commit 之前，SHALL 先掃描這些修改。發現疑似秘密時 SHALL 停止推送、不建立也不修改任何 commit，並回報檔案、行號與命中的規則，推送狀態標記為受阻。

#### Scenario: Secret in a hand-edited module
- **WHEN** 使用者手動在 `modules/env.toml` 寫入一個 API key，並執行 `rsmt publish`
- **THEN** 遠端沒有收到任何 commit，指令以 1 結束並指出 `modules/env.toml` 的行號

#### Scenario: Secret already committed
- **WHEN** 使用者自行以 git commit 了一個含 API key 的 module，並執行 `rsmt publish`
- **THEN** 遠端沒有收到任何 commit，本機的 commit 維持原樣，指令以 1 結束並指出該檔案的行號

### Requirement: Allow list
repo 根目錄的 `allow.toml` SHALL 可列出不檢查的內容：`files` 是 repo 中的檔案或目錄，`lines` 是單行內容的雜湊。列出的檔案與行 SHALL 不被標記為疑似秘密，接管、`rsmt add` 與推送前的檢查都 SHALL 套用。推送受阻時回報的每一行 SHALL 附上該行的雜湊；規則要跨行才符合時 SHALL 不附雜湊，因為只列其中一行會讓同樣的那一行在任何地方都關掉這條規則。`allow.toml` 無法讀取時 SHALL 停止推送並指出該檔，`rsmt validate` SHALL 回報它的格式錯誤。

#### Scenario: A line that is not a secret
- **WHEN** 推送因 `home/.gitconfig` 的某一行受阻，使用者把回報的雜湊加進 `allow.toml` 的 `lines` 後再推送
- **THEN** 推送成功；內容相同的行在其他檔案或其他位置也不再受阻，內容不同的行仍會受阻

#### Scenario: A whole file
- **WHEN** `allow.toml` 的 `files` 列出 `home/.p10k.zsh`
- **THEN** `rsmt add ~/.p10k.zsh` 與之後的推送都不檢查這個檔

#### Scenario: Broken list
- **WHEN** `allow.toml` 有無法解析的內容，使用者執行 `rsmt publish`
- **THEN** 遠端沒有收到任何 commit，指令指出 `allow.toml`

#### Scenario: A private key
- **WHEN** 推送的檔案含有未加密的私鑰
- **THEN** 回報為私鑰且不附雜湊，即使其中的行單獨看像高熵字串
