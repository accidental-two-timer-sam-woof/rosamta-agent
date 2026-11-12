# macOS 也用 XDG 路徑

Rust 的 `dirs` crate 在 macOS 上回傳 `~/Library/Application Support`，但 rosamta 在 macOS 也把設定、repo 與狀態放在 `~/.config`、`~/.local/share`、`~/.local/state`；有設定 `XDG_*` 環境變數時以環境變數為準。理由是 starship、mise、gh 等開發者工具在 macOS 上也使用 `~/.config`，使用者會到那裡找；CLI、GUI 與 daemon 共用同一組路徑，兩個平台的文件與測試也能一致。
