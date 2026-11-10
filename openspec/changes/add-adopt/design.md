# Design

## Context

接管會修改使用者的 rc 檔（ADR 0002），所以每一步都要能還原，而且寧可保守地保存成片段 module，也不能誤判而改變 shell 的行為。hook、snapshot 與 renderer 由 `add-shell-render` 提供；背景觸發掃描由 `add-daemon` 負責，本 change 只提供掃描與處理的核心以及 CLI。

## Goals / Non-Goals

**Goals:**
- 接管前後，新開的 shell 行為完全相同。
- 辨識不出來的內容一律原樣保存，不嘗試改寫。
- 同一段內容不會被重複接管，也不會因安裝程式重寫而反覆出現在待處理清單。

**Non-Goals:**
- 辨識特定工具的 init 寫法（例如 zoxide、starship）：由 `add-providers-and-doctor` 透過本 change 的擴充點加入。
- 完整解析 shell 語法。

## Decisions

### 以逐行規則辨識，不做完整的 shell 解析

每種 shell 各有一組行規則：POSIX 的 `export NAME=...`、`NAME=...; export NAME`、`alias`；zsh 的 `path=(... $path)`、`path+=(...)`；fish 的 `set -gx`、`fish_add_path`；以及常見的 `case ":$PATH:"` 去重寫法。只要段落中有一行不符合任何規則，整段就當成片段 module。

替代方案是用 tree-sitter 解析 shell，但 bash、zsh、fish 的語法差異大，誤判的代價（改變 shell 行為）遠高於少辨識幾種寫法（只是多一個片段 module）。

### 辨識的擴充點

辨識流程依序詢問一串辨識器：provider 辨識器（本 change 只定義介面）→ 結構化辨識器 → 片段。`add-providers-and-doctor` 只需要註冊新的辨識器。

### 接管紀錄

`$XDG_STATE_HOME/rosamta/adoptions.json` 為每次接管記錄：編號、來源檔、原始行號、前後各兩行的上下文、原文、正規化後的雜湊、產生的 module、commit。正規化是指去掉行尾空白、把家目錄的絕對路徑換成 `$HOME`。用途：

- 重複寫入偵測：新段落的雜湊等於某次接管的雜湊，或它的每個結構化項目都已存在於 repo 的某個 module 中，就直接移除。
- 單次還原：依上下文找回原本的位置插入；找不到時插在該檔 hook 區塊之前。

### 只有註解的段落

前後沒有程式碼、只有註解的段落（例如分隔線 `# =====`）列為待處理項目，接受時只從 rc 檔刪除，不產生 module，原文保存在 snapshot 中。

### 註解模式的標記

註解模式寫入 `# rosamta: adopted → modules/adopted/<name>.toml`，下面接原本每行加上 `# ` 的內容。掃描器看到這個標記行，就略過它與緊接其後的註解行。

### 秘密規則

沿用 gitleaks 的 TOML 規則集（MIT）中與 shell 環境變數相關的子集，在 build 時轉成 Rust `regex` 並確認每條都能編譯，無法編譯的規則直接捨棄並在 build log 中列出。另外對長度 20 以上、Shannon 熵大於 4.0 的字面值標記為疑似秘密。規則來源記錄在 `THIRD_PARTY_NOTICES.md`。

疑似秘密的段落寫到 `$XDG_CONFIG_HOME/rosamta/local/modules/`；renderer 把它當成額外的 module 來源，但 repo 從來看不到這個目錄。

### 待處理清單與提醒

清單存在 `$XDG_STATE_HOME/rosamta/queue.json`，忽略清單存在 `ignored.json`（以正規化雜湊為鍵）。清單內容改變時，同步寫出一個只有一行文字的 `notice` 檔；清單清空或關閉提醒時刪除它。loader 在互動 shell 中用內建指令讀取這一行並印出（`IFS= read -r line < file`），不啟動外部程式，以維持 5 ms 的預算。

### 並行控制

掃描與接管期間對 repo 與狀態目錄加檔案鎖，避免 CLI 與背景服務同時修改。

### 切段以 shell 的語法檢查為準

空白行只在「目前累積的內容能單獨通過 `<shell> -n`」時才切開，否則併入同一段。Debian 預設的 `.bashrc` 在 `if ... fi` 裡有空白行，單純依空白行切段會把它切成兩個語法錯誤的片段。rosamta 自己為 bash 建立 `~/.bash_profile` 時寫入的第一行（source `~/.profile`）不列為可接管的內容。

### 不改變載入順序

結構化 module 的環境變數與 PATH 在 env 階段載入，比來源檔的位置早。late 檔案（`.bashrc`、`.zshrc` 等）中有兩種情況會改保存為片段，避免順序改變造成不同的結果：

- 同一個檔案中，前面已經有片段 module（例如 `eval "$(brew shellenv)"` 可能在看不見的地方改動 PATH），之後含 PATH 的段落。
- 值引用了 `HOME`、`USER`、`LOGNAME`、`PATH` 與同一段定義的變數以外的變數，那個變數可能由前面的片段設定。

每個接管的 module 的 `order` 是「來源檔的啟動順序 × 1000 ＋ 序號」，讓同一階段內維持原本跨檔案、檔案內的先後順序。

`.zprofile` 的內容改在 env 階段載入後，比系統的 zprofile 更早執行，而系統 zprofile 可能在後面加入其他項目（例如 `~/bin`）。因此 `profile.zsh` 對 append 的項目改為「移到最後」，與 prepend 的「移到最前面」對稱；`add-shell-render` 原本是「不存在時才加到最後」。

### 套用失敗時全部復原

接管與單次還原都先量測現在 shell 啟動的 stderr，再寫入 rc 檔與 module 並套用；smoke test 失敗時把 rc 檔與 module 檔全部寫回原狀，也不建立 commit。比對 stderr 時忽略開頭的「檔案:行號」：原本就存在的錯誤只是換到 generated script 回報，不算新的錯誤。

### 單次還原的位置

接管紀錄同時保存原檔中前後各兩行，以及接管後留在檔案中的前後兩行。還原時先用原本的鄰行找位置（其他段落可能已經還原），找不到再用留下來的鄰行，最後才放在 hook 區塊之前。還原後該段的雜湊加入忽略清單，之後不再被接管或列入待處理。

### 待處理清單的掃描時機

在 `add-daemon` 之前沒有背景掃描，所以 `rsmt review` 先掃描並依接管偏好處理，再列出清單。只有註解的段落與重複寫入的段落不建立接管紀錄，原文保存在 snapshot。提醒由 rc loader 印到 stdout，smoke test 只比對 stderr，因此不受影響。

### 秘密規則的細節

gitleaks 的 `generic-api-key` 規則誤判多，由「名稱含 KEY、TOKEN、SECRET、PASSWORD 且值為字面字串」的規則取代。`pypi-upload-token` 與 `vault-batch-token` 編譯後超過 regex 的大小上限而略過。每條規則只在文字含有它的關鍵字時才編譯與比對；全部預先編譯在 debug build 下需要數秒。

## Risks / Trade-offs

- [行規則漏判，導致大量片段 module] → 片段 module 仍然正確運作，只是在 GUI 中較不好編輯；以安裝程式 fixture 持續補規則。
- [刪除原行後使用者找不到設定] → 刪除前存 snapshot、每次接管都有 commit，並可用 `rsmt restore <id>` 單次還原。
- [秘密規則誤判或漏判] → 誤判只會讓內容留在本機，代價小；漏判由 `add-git-sync` 在推送前再掃描一次整個 diff 作為第二道防線。
