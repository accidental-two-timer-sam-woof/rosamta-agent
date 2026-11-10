# 設定格式用 TOML，不用 YAML

交接文件原本定 YAML。改用 TOML 的原因：daemon 與 GUI 會頻繁自動寫入設定，而 serde 系列的 YAML crate 寫回時都會丟掉註解，`toml_edit` 則能局部修改並保留註解與格式；Rust 的 YAML crate 也還在換代（`serde_yaml` 停止維護、`serde_yml` 已 deprecated）。為了避開 TOML 深層巢狀難讀的問題，一個 module 一個檔案，Capture 只建立新檔，不改使用者手寫的檔案。
