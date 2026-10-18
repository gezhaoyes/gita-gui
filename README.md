# gita-gui

English | [简体中文](README.zh-CN.md)

One table for all your git repositories on **Windows and WSL**: see at a glance which ones are not backed up to a remote, and push them with one click. It also keeps a manifest of your repositories, so after switching computers or reinstalling the system you can clone everything back in one go.

Repository registration uses the data of [gita](https://github.com/nosarthur/gita), so it works together with the gita command line: repositories added here show up in `gita ll`, and repositories added with `gita add` show up here after a refresh.

> The user interface is in Chinese.

## Features

- **Status table**: Windows and WSL repositories in one table, color-coded
  - 🟢 Backed up: no unpushed commits and no uncommitted changes
  - 🟡 Changed: there are uncommitted changes (commit them in your editor — this tool does not commit)
  - 🔴 Not backed up: unpushed commits, no upstream branch, upstream branch gone, or no remote at all
  - ⚪ Path missing: the registered folder was moved or deleted
- **Add**: pick a folder or drag it into the window. If it is a repository it is added directly; otherwise the tool searches below it and lets you choose which repositories to add. Folders under `\\wsl.localhost\...` are recognized as WSL
- **Push**: one, selected, or all; pushed one after another, failures shown per repository with the cause (authentication / no upstream / rejected / network) and a retry button
- **Manifest**: `manifest-windows.json` and `manifest-wsl.json` are maintained automatically; putting them in a synced folder such as OneDrive is recommended
  - Written only when the content changes. If the manifest was changed on another computer or by hand, it is not overwritten — you choose between "restore from manifest" and "overwrite with this computer"
  - A new computer with an empty registry will not wipe the manifest
- **Restore from manifest**: select repositories, clone them to their original paths (editable), and restore gita groups and multiple push URLs
- **Context menu**: open in Explorer / VS Code (WSL repositories open via Remote-WSL), relocate, unregister

The tool does not fetch: "unpushed" is compared against the remote state recorded locally. Only the currently checked-out branch is checked.

## Requirements

1. **Windows 10/11** with [Git for Windows](https://git-scm.com/download/win) (includes Git Credential Manager)
2. **gita** on Windows: `pipx install gita`
3. **WSL** (skip if you do not use WSL):
   - Install gita: `pipx install gita`, or `python3 -m venv ~/.local/share/gita-venv && ~/.local/share/gita-venv/bin/pip install gita && ln -s ~/.local/share/gita-venv/bin/gita ~/.local/bin/gita`
   - Let git in WSL use the Windows login (the tool also offers a one-click setup when a push fails):
     ```bash
     git config --global credential.helper "/mnt/c/Program\ Files/Git/mingw64/bin/git-credential-manager.exe"
     ```
     Adjust the path to where Git for Windows is installed.

## Installation

Download the installer from [Releases](../../releases), or build it yourself (see below).

On first launch you are asked to confirm the manifest folder; the default is `%OneDrive%\gita-gui`.

## FAQ

### Pushing from WSL fails with `Exec format error`

With systemd enabled in WSL (`systemd=true` in `/etc/wsl.conf`), the `WSLInterop` registration that lets WSL run Windows programs is sometimes lost, so the Windows credential manager cannot be started. To check: run `cmd.exe /c ver` in WSL — if it also fails with `Exec format error`, this is the problem.

Quick fix (lost when WSL restarts):

```bash
sudo sh -c 'echo :WSLInterop:M::MZ::/init:PF > /proc/sys/fs/binfmt_misc/register'
```

Permanent fix: add a service that re-registers it at boot.

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

Note: Ubuntu does not run `systemd-binfmt` inside WSL, so putting a file in `/usr/lib/binfmt.d/` has no effect.

### Where is the data stored

| Content | Location |
|---|---|
| Repository registry (gita) | Windows: `%USERPROFILE%\.config\gita\`; WSL: `~/.config/gita/` |
| Manifests | The manifest folder set in Settings |
| Tool settings | `%APPDATA%\com.gezhao.gitagui\config.json` |

## Development

Stack: Tauri 2 (Rust) + Vue 3 + TypeScript + Naive UI. Requires Rust (MSVC toolchain) and Node.js.

```bash
npm install
npm run tauri dev      # run in development
npm run tauri build    # build installers into src-tauri/target/release/bundle/

cd src-tauri
cargo test                       # unit tests
cargo test -- --include-ignored  # plus end-to-end tests against real WSL and gita (needs the Ubuntu distro and gita on both sides)
```

The design document is in Chinese: [docs/计划书.md](docs/计划书.md).

## License

[MIT](LICENSE)
