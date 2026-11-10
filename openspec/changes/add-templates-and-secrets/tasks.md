# Tasks

## 1. Template

- [ ] 1.1 加入 `minijinja`，建立沙箱環境（strict 未定義、不提供 loader），實作 `host.*` 與 `vars.*` 的 context，以及 `rsmt validate` 的 template 語法檢查
- [ ] 1.2 以靜態分析找出引用但本機沒有的 `vars.*`；終端機中詢問並以 toml_edit 寫進 `host.toml` 的 `[vars]`
- [ ] 1.3 在 `host.toml` 加入 `[vars]` 與 `[secrets] bitwarden`，更新 JSON Schema 與讀取時的檢查

## 2. 秘密來源

- [ ] 2.1 實作 `bitwarden()`：rbw、bw、bws 的偵測、登入與鎖住判斷、取值，順序覆寫，以及同一次套用內的快取
- [ ] 2.2 實作 `secrets.toml` 的型別、`rsmt schema secrets`、`rsmt validate` 檢查與 `secret()`；只在互動的 `rsmt apply` 執行
- [ ] 2.3 實作 `rsmt secrets bws login|logout`，token 存進 keychain（測試用檔案），執行 bws 時以環境變數傳入
- [ ] 2.4 實作 `bitwarden_ssh_socket()` 與獨立的安裝方式偵測

## 3. 部署

- [ ] 3.1 把 template 接進 dotfile 部署：輸入雜湊、render、加密 template 的解密、新檔案 0600、snapshot，與 `--dry-run`、`--json` 輸出
- [ ] 3.2 互動的 `rsmt apply` 每次 render 全部 template、鎖住時執行解鎖指令；非互動與背景服務只 render 輸入改變的 template
- [ ] 3.3 render 結果被修改時進入待處理清單；`accept` 存成本機專用的 copy 宣告、`discard` 重新 render
- [ ] 3.4 `rsmt add --template`

## 4. 等待項目與背景服務

- [ ] 4.1 待處理清單的等待項目（`locked`、`command`、`unset`）：列出、`--json`、拒絕 accept/ignore/discard、成功後移除
- [ ] 4.2 背景服務在 `host.toml` 改變時重新部署 dotfile，新的等待項目通知一次（中英文字串）

## 5. 驗證

- [ ] 5.1 以假的 rbw、bw、bws 與秘密指令（PATH 中的腳本）做整合測試，涵蓋鎖住、非互動與背景服務的情況
- [ ] 5.2 跑完 fmt、clippy、全部測試、8 個 Linux distro 與 `cargo deny`，更新 THIRD_PARTY_NOTICES 與 README，列出所有使用者看得到的文字供審閱
