# Design

## Context

相關的既定決策：設定用 TOML、一個 module 一個檔（ADR 0001）；dotfile 的屬性寫在 `dotfiles.toml`（ADR 0007）；macOS 也用 XDG 路徑（ADR 0008）；套用範圍用識別名稱而不是 hostname（ADR 0011）；git 操作一律呼叫系統的 `git`。

## Goals / Non-Goals

**Goals:**
- 一套 Rust 型別同時負責解析、驗證、JSON Schema 與寫回。
- 驗證錯誤指得出檔案與行號。
- 程式寫回時不破壞使用者的註解。

**Non-Goals:**
- module 內容的語意（render、provider 選項）：由 `add-shell-render` 與 `add-providers-and-doctor` 定義。
- `dotfiles.toml` 與 `packages.toml` 的欄位：由 `add-dotfile-deploy` 與 `add-packages` 定義；本 change 只保留檔名。

## Decisions

### 讀與寫分開處理

讀取時用 `toml` crate 搭配 serde 反序列化成型別，所有型別加上 `deny_unknown_fields`；`toml` 的錯誤帶有位置資訊，可換算成行號。寫回時改用 `toml_edit::DocumentMut` 只修改目標欄位，保留註解與排序。

替代方案是把整份檔案序列化後覆寫，但這會丟掉使用者的註解，違反 ADR 0001 選 TOML 的理由。

### JSON Schema

用 `schemars` 從同一套型別產生 schema，型別改了 schema 就跟著改。`rsmt schema <kind>` 輸出；TOML 編輯器（taplo、Tombi）可透過檔案開頭的 `#:schema` 註解使用。

### XDG 路徑自己實作

規則只有三條：讀環境變數、非絕對路徑視為未設定、套用預設值。約 30 行，不值得為此加依賴。`dirs` crate 在 macOS 回傳 `~/Library/Application Support`，不符合 ADR 0008。

### 識別名稱的產生

macOS 讀 `scutil --get LocalHostName`，Linux 用 `gethostname()`；去掉 `.local` 等網域後綴，只保留英數與 `-`。只在建立 `host.toml` 時產生一次。

### host 清單

`rosamta.toml` 以 `hosts = ["mbp", "lab-server"]` 記錄。這份清單讓 GUI 能列出可選的 host，也讓 validate 能警告拼錯的識別名稱。

### 格式遷移

每一版格式各有一個遷移函式（`v1 → v2`、`v2 → v3`），依序套用；遷移完成後呼叫 `git commit`，訊息為 `chore(repo): migrate format N to M`。遇到較新的格式時直接回報錯誤，不嘗試部分讀取。

### `rsmt init`

新建 repo 時執行 `git init` 並建立第一個 commit；加入既有 repo 時呼叫 `git clone`。兩者都不安裝 hook、不接管任何內容；那些步驟在 `add-shell-render` 與 `add-adopt` 中接在 init 之後。

## Risks / Trade-offs

- [`deny_unknown_fields` 讓使用者在舊版 rosamta 上無法讀取新版加入的欄位] → 新增欄位一律提升 `format`，舊版會明確拒絕並提示升級，而不是讀錯。
- [讀取與寫回走兩條路徑，可能出現不一致] → 每個寫回操作的測試都在寫回後重新以讀取路徑解析，並比對結果。
- [`scutil` 只在 macOS 上有] → 取不到時改用 `gethostname()`。
