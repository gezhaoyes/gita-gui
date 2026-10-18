# gita-gui

把 Windows 和 WSL 两边的 git 仓库放在一张表里：一眼看出哪些还没备份到远程，一键推送；自动维护一份仓库清单，换电脑或重装系统后可以从清单一键 clone 回来。

仓库登记直接用 [gita](https://github.com/nosarthur/gita) 的数据，和 gita 命令行互通：在这里添加的仓库 `gita ll` 能看到，用 `gita add` 加的这里刷新后也能看到。

## 功能

- **状态总表**：Windows 和 WSL 的仓库合在一张表，按颜色区分
  - 🟢 已备份：没有未推送的提交，也没有未提交的改动
  - 🟡 有改动：有改了但没提交的文件（提交请在编辑器里做，本工具不提交）
  - 🔴 未备份：有未推送的提交、分支没有上游、上游分支已不存在，或者根本没有远程仓库
  - ⚪ 路径不存在：登记的文件夹被移走或删除了
- **添加**：选一个文件夹或把它拖进窗口。本身是仓库就直接加入；不是就往下找，列出找到的仓库勾选加入。`\\wsl.localhost\...` 下的文件夹自动归到 WSL
- **推送**：单个、选中或全部；一个接一个推，失败的逐条显示原因（认证 / 无上游 / 被拒绝 / 网络）并可重试
- **清单**：自动维护 `manifest-windows.json` 和 `manifest-wsl.json`，建议放在 OneDrive 等同步盘里
  - 有变化才写入；别的电脑或手工改过清单时不会覆盖，而是让你选"从清单恢复"或"用本机覆盖"
  - 新电脑上登记为空时，不会把清单清空
- **从清单恢复**：勾选仓库，clone 到原路径（可改），并恢复 gita 分组和多推送地址
- **右键菜单**：在资源管理器 / VS Code 中打开（WSL 仓库用 Remote-WSL 打开）、重新定位、移除登记

工具不做 fetch，"未推送"是和本机记录的远程状态比较。只检查当前所在的分支。

## 需要准备

1. **Windows 10/11**，装好 [Git for Windows](https://git-scm.com/download/win)（自带 Git Credential Manager）
2. **gita**（Windows 侧）：`pipx install gita`
3. **WSL 侧**（不用 WSL 可以跳过）：
   - 装 gita：`pipx install gita`，或者 `python3 -m venv ~/.local/share/gita-venv && ~/.local/share/gita-venv/bin/pip install gita && ln -s ~/.local/share/gita-venv/bin/gita ~/.local/bin/gita`
   - 让 WSL 的 git 借用 Windows 的登录（推送失败时工具里也有"一键配置"）：
     ```bash
     git config --global credential.helper "/mnt/c/Program\ Files/Git/mingw64/bin/git-credential-manager.exe"
     ```
     路径按你的 Git for Windows 安装位置调整。

## 安装

从 [Releases](../../releases) 下载安装包，或者自己构建（见下文）。

第一次打开会让你确认清单目录，默认是 `%OneDrive%\gita-gui`。

## 常见问题

### WSL 里推送报 `Exec format error`

WSL 开启 systemd（`/etc/wsl.conf` 里 `systemd=true`）后，有时会丢掉运行 Windows 程序的注册项 `WSLInterop`，于是调不起 Windows 的凭据管理器。验证：在 WSL 里执行 `cmd.exe /c ver`，同样报 `Exec format error` 就是这个问题。

立即修复（重启 WSL 后会失效）：

```bash
sudo sh -c 'echo :WSLInterop:M::MZ::/init:PF > /proc/sys/fs/binfmt_misc/register'
```

永久修复：加一个开机时自动补注册的服务。

```bash
sudo tee /etc/systemd/system/wsl-interop.service >/dev/null <<'EOF'
[Unit]
Description=Re-register WSL interop after systemd mounts binfmt_misc
After=proc-sys-fs-binfmt_misc.mount

[Service]
Type=oneshot
ExecStart=/bin/sh -c '[ -e /proc/sys/fs/binfmt_misc/WSLInterop ] || echo ":WSLInterop:M::MZ::/init:PF" > /proc/sys/fs/binfmt_misc/register'

[Install]
WantedBy=multi-user.target
EOF
sudo systemctl daemon-reload
sudo systemctl enable --now wsl-interop.service
```

注意：Ubuntu 在 WSL 里不会运行 `systemd-binfmt`，所以往 `/usr/lib/binfmt.d/` 放配置文件不起作用。

### 数据存在哪里

| 内容 | 位置 |
|---|---|
| 仓库登记（gita） | Windows：`%USERPROFILE%\.config\gita\`；WSL：`~/.config/gita/` |
| 清单 | 设置里的清单目录 |
| 工具设置 | `%APPDATA%\com.gezhao.gitagui\config.json` |

## 开发

技术栈：Tauri 2（Rust）+ Vue 3 + TypeScript + Naive UI。需要 Rust（MSVC）、Node.js。

```bash
npm install
npm run tauri dev      # 开发运行
npm run tauri build    # 打包，产物在 src-tauri/target/release/bundle/

cd src-tauri
cargo test                       # 单元测试
cargo test -- --include-ignored  # 含真实 WSL 和 gita 的端到端测试（需要 WSL 发行版 Ubuntu 和两边的 gita）
```

设计说明见 [docs/计划书.md](docs/计划书.md)。

## 许可证

[MIT](LICENSE)
