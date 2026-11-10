# Condition 以 Host ID 指稱主機，不用系統 hostname

系統的 hostname 會變，例如 macOS 在區網名稱衝突時會自動在名稱後加上 `-2`；直接拿它當條件，module 可能某天突然不再套用。rosamta 在第一次設定時從 hostname 產生 Host ID，之後固定不變，condition 的 `hosts` 一律指 Host ID。介面上只顯示「識別名稱」，不顯示系統 hostname，避免使用者以為修改它會改到系統設定。改名時，repo 裡引用舊名稱的 condition 會一起更新。
