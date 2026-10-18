//! 暴露给前端的命令。耗时操作都放到后台线程，界面不卡

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Mutex;

use serde::Serialize;
use tauri::{AppHandle, Manager};

use crate::config::{self, AppConfig};
use crate::discover;
use crate::exec::{self, sh_quote, Env};
use crate::gita::{self, norm_path};
use crate::manifest::{self, CloneResult, Entry, Manifest, SyncState};
use crate::push::{self, PushResult};
use crate::status::{self, RepoRow};

pub struct AppState {
    pub config: Mutex<AppConfig>,
    pub config_path: PathBuf,
    /// 同一时间只让一个清单同步在跑，避免哈希记录错乱
    pub manifest_lock: Mutex<()>,
}

impl AppState {
    fn cfg(&self) -> AppConfig {
        self.config.lock().unwrap().clone()
    }

    fn update_cfg(&self, f: impl FnOnce(&mut AppConfig)) -> Result<(), String> {
        let mut cfg = self.config.lock().unwrap();
        f(&mut cfg);
        cfg.save(&self.config_path)
    }
}

async fn blocking<T: Send + 'static>(
    app: AppHandle,
    f: impl FnOnce(&AppState) -> Result<T, String> + Send + 'static,
) -> Result<T, String> {
    tauri::async_runtime::spawn_blocking(move || f(&app.state::<AppState>()))
        .await
        .map_err(|e| e.to_string())?
}

// ---------- 配置 ----------

#[derive(Serialize)]
pub struct ConfigView {
    #[serde(flatten)]
    config: AppConfig,
    default_manifest_dir: Option<String>,
}

#[tauri::command]
pub fn get_config(state: tauri::State<'_, AppState>) -> ConfigView {
    ConfigView {
        config: state.cfg(),
        default_manifest_dir: config::default_manifest_dir().map(|p| p.to_string_lossy().into_owned()),
    }
}

#[tauri::command]
pub fn save_config(state: tauri::State<'_, AppState>, config: AppConfig) -> Result<(), String> {
    state.update_cfg(|c| {
        // 清单目录换了，旧的哈希就不再适用
        if c.manifest_dir != config.manifest_dir {
            c.manifest_hashes.clear();
        }
        // 哈希和推送失败记录由后端维护，不用前端传回来的旧值
        let hashes = std::mem::take(&mut c.manifest_hashes);
        let failures = std::mem::take(&mut c.push_failures);
        *c = AppConfig { manifest_hashes: hashes, push_failures: failures, ..config };
    })
}

// ---------- 总表 ----------

#[derive(Serialize)]
pub struct LoadResult {
    rows: Vec<RepoRow>,
    manifest: SyncState,
}

/// 读取一边的登记和状态；读取成功后顺带同步这一边的清单
#[tauri::command]
pub async fn load_repos(app: AppHandle, env: Env) -> Result<LoadResult, String> {
    blocking(app, move |st| {
        let cfg = st.cfg();
        let registry = gita::read_registry(env, &cfg.wsl_distro)?;
        let paths: Vec<String> = registry.iter().map(|r| r.path.clone()).collect();
        let raws = status::collect(env, &cfg.wsl_distro, &paths)?;
        let mut rows: Vec<RepoRow> = registry
            .into_iter()
            .zip(raws)
            .map(|(repo, raw)| RepoRow::new(env, repo, raw))
            .collect();
        for row in &mut rows {
            if let Some(msg) = cfg.push_failures.get(&push_key(env, &row.path)) {
                row.mark_push_failed(msg);
            }
        }
        let manifest = sync_manifest_rows(st, env, &rows, false);
        Ok(LoadResult { rows, manifest })
    })
    .await
}

fn sync_manifest_rows(st: &AppState, env: Env, rows: &[RepoRow], force: bool) -> SyncState {
    let _guard = st.manifest_lock.lock().unwrap();
    let cfg = st.cfg();
    let Some(dir) = cfg.manifest_dir.as_deref().map(Path::new) else {
        return SyncState::NotConfigured;
    };
    let previous = manifest::read(dir, env).ok().flatten();
    let content = manifest::render(&manifest::build(env, rows, previous.as_ref()));
    let stored = cfg.manifest_hashes.get(env.as_str()).map(String::as_str);
    let (state, new_hash) = manifest::sync(dir, env, &content, stored, force);
    if let Some(h) = new_hash.filter(|h| Some(h.as_str()) != stored) {
        if let Err(message) = st.update_cfg(|c| {
            c.manifest_hashes.insert(env.as_str().into(), h);
        }) {
            return SyncState::Error { message };
        }
    }
    state
}

/// 冲突时用户选"用本机覆盖"
#[tauri::command]
pub async fn overwrite_manifest(app: AppHandle, env: Env) -> Result<SyncState, String> {
    blocking(app, move |st| {
        let cfg = st.cfg();
        let registry = gita::read_registry(env, &cfg.wsl_distro)?;
        let paths: Vec<String> = registry.iter().map(|r| r.path.clone()).collect();
        let raws = status::collect(env, &cfg.wsl_distro, &paths)?;
        let rows: Vec<RepoRow> = registry
            .into_iter()
            .zip(raws)
            .map(|(repo, raw)| RepoRow::new(env, repo, raw))
            .collect();
        Ok(sync_manifest_rows(st, env, &rows, true))
    })
    .await
}

// ---------- 添加 ----------

#[derive(Serialize)]
pub struct Found {
    path: String,
    registered: bool,
}

#[derive(Serialize)]
pub struct DiscoverResult {
    env: Env,
    root: String,
    root_is_repo: bool,
    found: Vec<Found>,
}

#[tauri::command]
pub async fn discover(app: AppHandle, path: String) -> Result<DiscoverResult, String> {
    blocking(app, move |st| {
        let cfg = st.cfg();
        let (env, root, distro) = discover::classify(&path);
        if let Some(d) = distro.filter(|d| !d.eq_ignore_ascii_case(&cfg.wsl_distro)) {
            return Err(format!("这个目录属于 WSL 发行版 {d}，配置里的发行版是 {}", cfg.wsl_distro));
        }
        let paths = discover::discover(env, &cfg.wsl_distro, &root, cfg.discover_depth, &cfg.skip_dirs)?;
        let registered: Vec<String> = gita::read_registry(env, &cfg.wsl_distro)?
            .iter()
            .map(|r| norm_path(env, &r.path))
            .collect();
        let root_is_repo = paths.len() == 1 && norm_path(env, &paths[0]) == norm_path(env, &root);
        let found = paths
            .into_iter()
            .map(|p| Found { registered: registered.contains(&norm_path(env, &p)), path: p })
            .collect();
        Ok(DiscoverResult { env, root, root_is_repo, found })
    })
    .await
}

#[tauri::command]
pub async fn add_repos(app: AppHandle, env: Env, paths: Vec<String>, group: Option<String>) -> Result<String, String> {
    blocking(app, move |st| {
        let cfg = st.cfg();
        let mut args: Vec<&str> = vec!["add"];
        let group = group.filter(|g| !g.trim().is_empty());
        if let Some(g) = &group {
            args.extend(["-g", g.as_str()]);
        }
        args.extend(paths.iter().map(String::as_str));
        Ok(gita::run_ok(env, &cfg.wsl_distro, &cfg.gita_windows, &args)?.stdout)
    })
    .await
}

#[tauri::command]
pub async fn remove_repo(app: AppHandle, env: Env, name: String) -> Result<(), String> {
    blocking(app, move |st| {
        let cfg = st.cfg();
        let path = gita::read_registry(env, &cfg.wsl_distro)?
            .into_iter()
            .find(|r| r.name == name)
            .map(|r| r.path);
        gita::remove(env, &cfg.wsl_distro, &name)?;
        if let Some(p) = path {
            st.update_cfg(|c| {
                c.push_failures.remove(&push_key(env, &p));
            })?;
        }
        Ok(())
    })
    .await
}

/// 重新定位：取消旧登记，登记新路径，并恢复原来的分组
#[tauri::command]
pub async fn relocate_repo(app: AppHandle, env: Env, name: String, new_path: String) -> Result<(), String> {
    blocking(app, move |st| {
        let cfg = st.cfg();
        let (picked_env, new_path, _) = discover::classify(&new_path);
        if picked_env != env {
            return Err("新位置必须和原仓库在同一边（Windows / WSL）".into());
        }
        manifest::relocate(env, &cfg.wsl_distro, &cfg.gita_windows, &name, &new_path)
    })
    .await
}

// ---------- 推送 ----------

pub fn push_key(env: Env, path: &str) -> String {
    format!("{}:{}", env.as_str(), norm_path(env, path))
}

/// 推送并记下结果：失败记入 push_failures，成功则清除
fn push_and_record(st: &AppState, env: Env, path: &str, set_upstream: bool) -> Result<PushResult, String> {
    let result = push::push(env, &st.cfg().wsl_distro, path, set_upstream);
    let key = push_key(env, path);
    st.update_cfg(|c| {
        if result.ok {
            c.push_failures.remove(&key);
        } else {
            c.push_failures.insert(key, result.message.clone());
        }
    })?;
    Ok(result)
}

#[tauri::command]
pub async fn push(app: AppHandle, env: Env, path: String) -> Result<PushResult, String> {
    blocking(app, move |st| push_and_record(st, env, &path, false)).await
}

#[tauri::command]
pub async fn push_set_upstream(app: AppHandle, env: Env, path: String) -> Result<PushResult, String> {
    blocking(app, move |st| push_and_record(st, env, &path, true)).await
}

const EXEC_FAIL: &str = "@@GG-EXEC-FAIL@@";
const INTEROP_HELP: &str = "已写入配置，但 WSL 运行不了 Windows 程序（WSLInterop 未注册，常见于开启 systemd 后）。\
请在 WSL 终端执行（立即生效，重启 WSL 后会失效；永久修复见 README）：\n\
sudo sh -c 'echo :WSLInterop:M::MZ::/init:PF > /proc/sys/fs/binfmt_misc/register'";

/// 一键配置：让 WSL 的 git 借用 Windows 的 Git Credential Manager，并验证能运行
#[tauri::command]
pub async fn setup_wsl_auth(app: AppHandle) -> Result<String, String> {
    blocking(app, move |st| {
        let exec_path = exec::run_local("git", &["--exec-path"], None)?.stdout;
        // .../Git/mingw64/libexec/git-core → .../Git/mingw64/bin/git-credential-manager.exe
        let gcm = Path::new(exec_path.trim())
            .ancestors()
            .nth(2)
            .map(|p| p.join("bin").join("git-credential-manager.exe"))
            .filter(|p| p.exists())
            .ok_or("没找到 Windows 的 Git Credential Manager，请确认安装了 Git for Windows")?;
        let win = gcm.to_string_lossy().replace('/', "\\");
        let script = format!(
            "p=$(wslpath -u {}) || exit 1\n\
             git config --global credential.helper \"$(printf '%s' \"$p\" | sed 's/ /\\\\ /g')\"\n\
             git config --global credential.https://dev.azure.com.useHttpPath true\n\
             \"$p\" --version >/dev/null 2>&1 || {{ echo '{EXEC_FAIL}' >&2; exit 3; }}\n\
             git config --global --get credential.helper",
            sh_quote(&win)
        );
        let out = exec::run_wsl(&st.cfg().wsl_distro, &script)?;
        if out.ok() {
            Ok(out.stdout.trim().to_string())
        } else if out.stderr.contains(EXEC_FAIL) {
            Err(INTEROP_HELP.into())
        } else {
            Err(out.message())
        }
    })
    .await
}

// ---------- 清单恢复 ----------

#[tauri::command]
pub async fn read_manifest(app: AppHandle, env: Env) -> Result<Option<Manifest>, String> {
    blocking(app, move |st| {
        let dir = st.cfg().manifest_dir.ok_or("还没有设置清单目录")?;
        manifest::read(Path::new(&dir), env)
    })
    .await
}

#[tauri::command]
pub async fn clone_entry(app: AppHandle, env: Env, entry: Entry, target: String) -> Result<CloneResult, String> {
    blocking(app, move |st| {
        let cfg = st.cfg();
        Ok(manifest::clone_one(env, &cfg.wsl_distro, &cfg.gita_windows, &entry, &target))
    })
    .await
}

// ---------- 打开 ----------

fn wsl_unc(distro: &str, path: &str) -> String {
    format!(r"\\wsl.localhost\{}{}", distro, path.replace('/', "\\"))
}

#[tauri::command]
pub fn open_in_explorer(state: tauri::State<'_, AppState>, env: Env, path: String) -> Result<(), String> {
    let target = match env {
        Env::Windows => path,
        Env::Wsl => wsl_unc(&state.cfg().wsl_distro, &path),
    };
    // explorer.exe 成功时也可能返回非 0，所以只看能否启动
    Command::new("explorer.exe")
        .arg(target)
        .spawn()
        .map(|_| ())
        .map_err(|e| format!("无法打开资源管理器：{e}"))
}

#[tauri::command]
pub fn open_in_editor(state: tauri::State<'_, AppState>, env: Env, path: String) -> Result<(), String> {
    let cfg = state.cfg();
    let args: Vec<String> = match env {
        Env::Windows => vec![path],
        Env::Wsl => vec!["--remote".into(), format!("wsl+{}", cfg.wsl_distro), path],
    };
    // VS Code 的 code 命令在 Windows 上是 code.cmd
    let spawn = |prog: &str| {
        let mut cmd = Command::new(prog);
        cmd.args(&args);
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            cmd.creation_flags(0x0800_0000);
        }
        cmd.spawn()
    };
    spawn(&cfg.editor)
        .or_else(|_| spawn(&format!("{}.cmd", cfg.editor)))
        .map(|_| ())
        .map_err(|e| format!("无法启动编辑器 {}：{e}", cfg.editor))
}
