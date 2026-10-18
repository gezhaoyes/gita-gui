//! 清单：两份（windows / wsl），从 gita 登记派生，自动写到清单目录（默认 OneDrive）。
//! 防误覆盖：记下上次写入内容的哈希，文件被别处改过就不覆盖，交给用户决定。

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::exec::{self, sh_quote, Env};
use crate::gita::{self, norm_path};
use crate::status::RepoRow;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Entry {
    pub name: String,
    pub path: String,
    pub remote: Option<String>,
    pub branch: Option<String>,
    #[serde(default)]
    pub groups: Vec<String>,
    /// 显式配置的推送地址（一个远程推到多处时才有）；只推到 remote 时为空、不写进文件
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub push_urls: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Manifest {
    pub version: u32,
    pub env: Env,
    pub repos: Vec<Entry>,
}

pub fn file_path(dir: &Path, env: Env) -> PathBuf {
    dir.join(format!("manifest-{}.json", env.as_str()))
}

/// 从总表的行生成清单。读不到状态的仓库（路径不存在、读取失败）沿用旧清单里的远程和分支
pub fn build(env: Env, rows: &[RepoRow], previous: Option<&Manifest>) -> Manifest {
    let old: HashMap<String, &Entry> = previous
        .map(|m| m.repos.iter().map(|e| (norm_path(env, &e.path), e)).collect())
        .unwrap_or_default();
    let mut repos: Vec<Entry> = rows
        .iter()
        .map(|row| {
            let prev = old.get(&norm_path(env, &row.path));
            let (remote, branch, push_urls) = match &row.status {
                Some(s) => {
                    let only_remote = row.remote_url.iter().cloned().collect::<Vec<_>>();
                    let push = if row.push_urls == only_remote { Vec::new() } else { row.push_urls.clone() };
                    (row.remote_url.clone(), s.branch.clone(), push)
                }
                None => prev
                    .map(|e| (e.remote.clone(), e.branch.clone(), e.push_urls.clone()))
                    .unwrap_or_default(),
            };
            // gita 改分组时会顺手丢掉路径不存在的仓库，这类仓库的分组沿用旧清单
            let mut groups = match prev {
                Some(e) if !row.exists && row.groups.is_empty() => e.groups.clone(),
                _ => row.groups.clone(),
            };
            groups.sort();
            Entry { name: row.name.clone(), path: row.path.clone(), remote, branch, groups, push_urls }
        })
        .collect();
    repos.sort_by_key(|e| norm_path(env, &e.path));
    Manifest { version: 1, env, repos }
}

pub fn render(m: &Manifest) -> String {
    let mut s = serde_json::to_string_pretty(m).expect("manifest serializes");
    s.push('\n');
    s
}

pub fn hash(content: &str) -> String {
    Sha256::digest(content.as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

pub fn read(dir: &Path, env: Env) -> Result<Option<Manifest>, String> {
    let path = file_path(dir, env);
    match std::fs::read(&path) {
        Ok(b) => serde_json::from_slice(&b)
            .map(Some)
            .map_err(|e| format!("清单格式错误（{}）：{e}", path.display())),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(format!("读取清单失败（{}）：{e}", path.display())),
    }
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum SyncState {
    /// 还没设置清单目录
    NotConfigured,
    Unchanged,
    Written,
    /// 文件被别处改过（别的电脑或手工），没有覆盖
    Conflict { only_in_file: Vec<String>, only_local: Vec<String> },
    Error { message: String },
}

/// 把生成好的清单内容同步到文件。返回状态，以及应当记下的新哈希
pub fn sync(
    dir: &Path,
    env: Env,
    content: &str,
    stored_hash: Option<&str>,
    force: bool,
) -> (SyncState, Option<String>) {
    let path = file_path(dir, env);
    let new_hash = hash(content);
    match std::fs::read(&path) {
        Ok(bytes) => {
            let existing = String::from_utf8_lossy(&bytes);
            if existing == content {
                return (SyncState::Unchanged, Some(new_hash));
            }
            if !force && stored_hash != Some(hash(&existing).as_str()) {
                return (conflict(env, &existing, content), None);
            }
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => {
            return (SyncState::Error { message: format!("读取清单失败：{e}") }, None);
        }
    }
    match write_atomic(&path, content) {
        Ok(()) => (SyncState::Written, Some(new_hash)),
        Err(message) => (SyncState::Error { message }, None),
    }
}

fn conflict(env: Env, file: &str, local: &str) -> SyncState {
    let paths = |s: &str| -> Vec<String> {
        serde_json::from_str::<Manifest>(s)
            .map(|m| m.repos.into_iter().map(|e| e.path).collect())
            .unwrap_or_default()
    };
    let (f, l) = (paths(file), paths(local));
    let has = |list: &[String], p: &str| list.iter().any(|x| norm_path(env, x) == norm_path(env, p));
    SyncState::Conflict {
        only_in_file: f.iter().filter(|p| !has(&l, p)).cloned().collect(),
        only_local: l.iter().filter(|p| !has(&f, p)).cloned().collect(),
    }
}

/// 先写临时文件再改名，避免 OneDrive 同步到写了一半的文件
fn write_atomic(path: &Path, content: &str) -> Result<(), String> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("创建清单目录失败：{e}"))?;
    }
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, content).map_err(|e| format!("写入清单失败：{e}"))?;
    std::fs::rename(&tmp, path).map_err(|e| format!("替换清单失败：{e}"))
}

#[derive(Debug, Clone, Serialize)]
pub struct CloneResult {
    /// cloned / skipped / failed
    pub state: &'static str,
    pub message: String,
}

/// 按清单的一条恢复一个仓库：目标已存在就跳过 clone；然后登记到 gita 并恢复分组
pub fn clone_one(env: Env, distro: &str, gita_windows: &str, entry: &Entry, target: &str) -> CloneResult {
    let fail = |message: String| CloneResult { state: "failed", message };
    let (state, mut message) = match clone_repo(env, distro, entry, target) {
        Ok(v) => v,
        Err(e) => return fail(e),
    };
    if state == "cloned" && !entry.push_urls.is_empty() {
        if let Err(e) = set_push_urls(env, distro, target, &entry.push_urls) {
            return fail(format!("{message}；配置推送地址失败：{e}"));
        }
        message.push_str(&format!("；推送地址 {} 个", entry.push_urls.len()));
    }
    if let Err(e) = register(env, distro, gita_windows, target, &entry.groups) {
        return fail(format!("{message}；登记失败：{e}"));
    }
    if !entry.groups.is_empty() {
        message.push_str(&format!("；分组：{}", entry.groups.join(", ")));
    }
    CloneResult { state, message }
}

fn clone_repo(env: Env, distro: &str, entry: &Entry, target: &str) -> Result<(&'static str, String), String> {
    let remote = entry.remote.as_deref().ok_or("清单里没有远程地址，无法恢复")?;
    match env {
        Env::Windows => {
            let dir = Path::new(target);
            if dir.exists() {
                return Ok(("skipped", "本地已存在，跳过 clone".into()));
            }
            if let Some(parent) = dir.parent() {
                std::fs::create_dir_all(parent).map_err(|e| format!("创建目录失败：{e}"))?;
            }
            let mut out = None;
            if let Some(b) = &entry.branch {
                out = Some(exec::run_local("git", &["clone", "-b", b, remote, target], None)?);
            }
            if !out.as_ref().is_some_and(|o| o.ok()) {
                out = Some(exec::run_local("git", &["clone", remote, target], None)?);
            }
            let out = out.expect("set above");
            if out.ok() { Ok(("cloned", "已 clone".into())) } else { Err(out.message()) }
        }
        Env::Wsl => {
            let (t, r) = (sh_quote(target), sh_quote(remote));
            let with_branch = entry
                .branch
                .as_deref()
                .map(|b| format!("git clone -b {} {r} {t} || ", sh_quote(b)))
                .unwrap_or_default();
            let script = format!(
                "if [ -e {t} ]; then echo '@@GG-EXISTS@@'; exit 0; fi\n\
                 mkdir -p \"$(dirname {t})\" && {{ {with_branch}git clone {r} {t}; }} 2>&1"
            );
            let out = exec::run_wsl(distro, &script)?;
            if out.stdout.contains("@@GG-EXISTS@@") {
                Ok(("skipped", "本地已存在，跳过 clone".into()))
            } else if out.ok() {
                Ok(("cloned", "已 clone".into()))
            } else {
                Err(out.message())
            }
        }
    }
}

/// clone 下来的远程叫 origin，按清单给它配上推送地址
fn set_push_urls(env: Env, distro: &str, target: &str, urls: &[String]) -> Result<(), String> {
    match env {
        Env::Windows => {
            for u in urls {
                let out = exec::git_local(Path::new(target), &["remote", "set-url", "--add", "--push", "origin", u])?;
                if !out.ok() {
                    return Err(out.message());
                }
            }
            Ok(())
        }
        Env::Wsl => {
            let cmds: Vec<String> = urls
                .iter()
                .map(|u| format!("git remote set-url --add --push origin {}", sh_quote(u)))
                .collect();
            let out = exec::run_wsl(distro, &format!("cd {} && {} 2>&1", sh_quote(target), cmds.join(" && ")))?;
            if out.ok() { Ok(()) } else { Err(out.message()) }
        }
    }
}

/// 重新定位：取消旧登记、登记新路径、恢复原来的分组。
/// 先确认新位置是仓库；中途任何一步失败，就把修改前的登记文件原样写回
pub fn relocate(env: Env, distro: &str, gita_windows: &str, name: &str, new_path: &str) -> Result<(), String> {
    if !crate::discover::is_repo(env, distro, new_path)? {
        return Err(format!("{new_path} 不是 git 仓库（里面没有 .git），原登记没有改动"));
    }
    let snapshot = gita::read_raw(env, distro)?;
    let old = gita::read_registry(env, distro)?
        .into_iter()
        .find(|r| r.name == name)
        .ok_or_else(|| format!("找不到登记：{name}"))?;
    if norm_path(env, &old.path) == norm_path(env, new_path) {
        return Ok(());
    }
    let result = gita::remove(env, distro, name)
        .and_then(|_| register(env, distro, gita_windows, new_path, &old.groups));
    if let Err(e) = result {
        return Err(match gita::write_raw(env, distro, &snapshot) {
            Ok(()) => format!("{e}（原登记已恢复）"),
            Err(r) => format!("{e}；恢复原登记也失败了：{r}"),
        });
    }
    Ok(())
}

/// 登记到 gita（已登记则不重复），并补上缺少的分组
pub fn register(env: Env, distro: &str, gita_windows: &str, path: &str, groups: &[String]) -> Result<(), String> {
    let find = || -> Result<Option<gita::GitaRepo>, String> {
        Ok(gita::read_registry(env, distro)?
            .into_iter()
            .find(|r| norm_path(env, &r.path) == norm_path(env, path)))
    };
    let repo = match find()? {
        Some(r) => r,
        None => {
            gita::run_ok(env, distro, gita_windows, &["add", path])?;
            find()?.ok_or("gita add 之后仍然找不到这个仓库")?
        }
    };
    for g in groups.iter().filter(|g| !repo.groups.contains(g)) {
        gita::run_ok(env, distro, gita_windows, &["group", "add", &repo.name, "-n", g])?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gita::GitaRepo;
    use crate::status::{Raw, Status};

    fn row(path: &str, status: Option<Status>) -> RepoRow {
        let raw = Raw {
            exists: status.is_some(),
            status: status.ok_or_else(|| "路径不存在".to_string()),
        };
        RepoRow::new(
            Env::Wsl,
            GitaRepo { name: path.rsplit('/').next().unwrap().into(), path: path.into(), groups: vec!["g".into()] },
            raw,
        )
    }

    fn status(url: &str) -> Status {
        Status {
            branch: Some("main".into()),
            upstream: Some("origin/main".into()),
            remotes: vec![crate::status::Remote {
                name: "origin".into(),
                url: url.into(),
                urls: vec![url.into()],
                push_urls: vec![],
            }],
            ..Default::default()
        }
    }

    #[test]
    fn build_sorts_and_keeps_missing_from_previous() {
        let prev = Manifest {
            version: 1,
            env: Env::Wsl,
            repos: vec![Entry {
                name: "gone".into(),
                path: "/h/gone".into(),
                remote: Some("OLD".into()),
                branch: Some("dev".into()),
                groups: vec![],
                push_urls: vec![],
            }],
        };
        let rows = vec![row("/h/z", Some(status("Z"))), row("/h/gone", None)];
        let m = build(Env::Wsl, &rows, Some(&prev));
        assert_eq!(m.repos[0].path, "/h/gone");
        assert_eq!(m.repos[0].remote.as_deref(), Some("OLD"));
        assert_eq!(m.repos[1].remote.as_deref(), Some("Z"));
        // 同样的输入，输出逐字节相同
        assert_eq!(render(&m), render(&build(Env::Wsl, &rows, Some(&prev))));
    }

    #[test]
    fn sync_protects_foreign_changes() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        let m1 = render(&build(Env::Wsl, &[row("/h/a", Some(status("A")))], None));
        let empty = render(&build(Env::Wsl, &[], None));

        // 第一次写入
        let (s, h1) = sync(dir, Env::Wsl, &m1, None, false);
        assert_eq!(s, SyncState::Written);
        // 内容不变
        assert_eq!(sync(dir, Env::Wsl, &m1, h1.as_deref(), false).0, SyncState::Unchanged);

        // 新电脑：登记为空、没有记过哈希 → 不覆盖
        let (s, h) = sync(dir, Env::Wsl, &empty, None, false);
        assert!(matches!(&s, SyncState::Conflict { only_in_file, .. } if only_in_file == &["/h/a"]));
        assert!(h.is_none());
        assert_eq!(std::fs::read_to_string(file_path(dir, Env::Wsl)).unwrap(), m1);

        // 本机写过的文件可以正常更新
        let (s, h2) = sync(dir, Env::Wsl, &empty, h1.as_deref(), false);
        assert_eq!(s, SyncState::Written);

        // 别处改过 → 冲突；强制覆盖则写入
        std::fs::write(file_path(dir, Env::Wsl), &m1).unwrap();
        assert!(matches!(sync(dir, Env::Wsl, &empty, h2.as_deref(), false).0, SyncState::Conflict { .. }));
        assert_eq!(sync(dir, Env::Wsl, &empty, h2.as_deref(), true).0, SyncState::Written);
        assert!(!dir.join("manifest-wsl.json.tmp").exists());
    }

    fn load_rows(env: Env) -> Vec<RepoRow> {
        let registry = gita::read_registry(env, "Ubuntu").unwrap();
        let paths: Vec<String> = registry.iter().map(|r| r.path.clone()).collect();
        let raws = crate::status::collect(env, "Ubuntu", &paths).unwrap();
        registry.into_iter().zip(raws).map(|(r, raw)| RepoRow::new(env, r, raw)).collect()
    }

    /// 完整流程：登记 → 生成清单 → 仓库丢失、登记清空 → 新电脑不覆盖清单 → 从清单恢复（含分组）
    fn restore_flow(
        env: Env,
        work: &str,
        manifest_dir: &Path,
        push_urls: &[String],
        other_repo: Option<&str>,
        move_away: impl Fn(),
    ) {
        let g = |args: &[&str]| gita::run_ok(env, "Ubuntu", "gita", args).unwrap();
        g(&["add", "-g", "grp-a", work]);
        let name = gita::read_registry(env, "Ubuntu").unwrap()[0].name.clone();
        g(&["group", "add", &name, "-n", "grp-b"]);

        let rows = load_rows(env);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].color, crate::status::Color::Green, "{:?}", rows[0].reasons);
        let content = render(&build(env, &rows, None));
        let (s, _) = sync(manifest_dir, env, &content, None, false);
        assert_eq!(s, SyncState::Written);

        // 重新定位到普通文件夹：报错，原登记和分组都不变
        let before = gita::read_registry(env, "Ubuntu").unwrap();
        let plain_dir = work.rsplit_once(['\\', '/']).unwrap().0;
        let err = relocate(env, "Ubuntu", "gita", &name, plain_dir).unwrap_err();
        assert!(err.contains("不是 git 仓库"), "{err}");
        assert_eq!(gita::read_registry(env, "Ubuntu").unwrap(), before);
        // 新位置是仓库，但登记中途失败（gita 命令不存在）：回滚到原样
        if let Some(other) = other_repo {
            let err = relocate(env, "Ubuntu", "no-such-gita-cmd", &name, other).unwrap_err();
            assert!(err.contains("原登记已恢复"), "{err}");
            assert_eq!(gita::read_registry(env, "Ubuntu").unwrap(), before);
        }

        // 模拟换电脑：仓库没了，登记也是空的
        move_away();
        gita::remove(env, "Ubuntu", &name).unwrap();
        g(&["ls"]); // gita 还能正常读我们写回的文件
        let empty = render(&build(env, &load_rows(env), None));
        assert!(matches!(sync(manifest_dir, env, &empty, None, false).0, SyncState::Conflict { .. }));

        let m = read(manifest_dir, env).unwrap().unwrap();
        let entry = &m.repos[0];
        assert_eq!(entry.groups, vec!["grp-a", "grp-b"]);
        assert_eq!(entry.push_urls, push_urls);
        let r = clone_one(env, "Ubuntu", "gita", entry, &entry.path);
        assert_eq!(r.state, "cloned", "{}", r.message);
        let rows = load_rows(env);
        assert_eq!(rows.len(), 1);
        let mut groups = rows[0].groups.clone();
        groups.sort();
        assert_eq!(groups, vec!["grp-a", "grp-b"]);
        if !push_urls.is_empty() {
            assert_eq!(rows[0].push_urls, push_urls);
        }
        assert_eq!(rows[0].color, crate::status::Color::Green, "{:?}", rows[0].reasons);
        // 再恢复一次：已存在，跳过
        assert_eq!(clone_one(env, "Ubuntu", "gita", entry, &entry.path).state, "skipped");
    }

    /// 用真实的 gita（Windows 和 WSL）跑一遍，登记放在临时目录里：cargo test -- --ignored
    #[test]
    #[ignore]
    fn real_restore_both_sides() {
        use std::process::Command;
        let tmp = tempfile::tempdir().unwrap();
        let manifest_dir = tmp.path().join("manifests");
        let git = |dir: &Path, args: &[&str]| {
            let o = Command::new("git").args(args).current_dir(dir).output().unwrap();
            assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
        };

        // ---- Windows ----
        unsafe {
            std::env::set_var("GITA_PROJECT_HOME", tmp.path().join("gita-home"));
        }
        let remote = tmp.path().join("remote.git");
        let work = tmp.path().join("代码 a").join("repo");
        std::fs::create_dir_all(&work).unwrap();
        git(tmp.path(), &["init", "-q", "--bare", "-b", "main", remote.to_str().unwrap()]);
        git(&work, &["init", "-q", "-b", "main"]);
        git(&work, &["-c", "user.email=t@t", "-c", "user.name=t", "commit", "-q", "--allow-empty", "-m", "c"]);
        git(&work, &["remote", "add", "origin", remote.to_str().unwrap()]);
        git(&work, &["push", "-q", "-u", "origin", "main"]);
        // 一个远程推到两处
        let remote2 = tmp.path().join("remote2.git");
        git(tmp.path(), &["init", "-q", "--bare", "-b", "main", remote2.to_str().unwrap()]);
        let push_urls = vec![remote.to_str().unwrap().to_string(), remote2.to_str().unwrap().to_string()];
        for u in &push_urls {
            git(&work, &["remote", "set-url", "--add", "--push", "origin", u]);
        }
        let other = tmp.path().join("other");
        std::fs::create_dir_all(&other).unwrap();
        git(&other, &["init", "-q"]);
        let gone = tmp.path().join("gone");
        restore_flow(Env::Windows, work.to_str().unwrap(), &manifest_dir, &push_urls, other.to_str(), || {
            std::fs::rename(&work, &gone).unwrap();
        });

        // ---- WSL ----
        let setup = "set -e; d=$(mktemp -d); cd \"$d\"; git init -q --bare -b main remote.git; \
                     mkdir -p \"代码 b/repo\"; cd \"代码 b/repo\"; git init -q -b main; \
                     git -c user.email=t@t -c user.name=t commit -q --allow-empty -m c; \
                     git remote add origin \"$d/remote.git\"; git push -q -u origin main 2>/dev/null; \
                     git init -q --bare -b main \"$d/remote2.git\"; \
                     git remote set-url --add origin \"$d/remote2.git\"; echo \"$d\"";
        let out = exec::run_wsl("Ubuntu", setup).unwrap();
        assert!(out.ok(), "{}", out.message());
        let d = out.stdout.trim().to_string();
        unsafe {
            std::env::set_var("GITA_PROJECT_HOME", format!("{d}/gita-home"));
            std::env::set_var("WSLENV", "GITA_PROJECT_HOME/u");
        }
        let work = format!("{d}/代码 b/repo");
        // 这边用另一种写法推到两处：两个 url（git remote set-url --add，不带 --push）
        let push_urls = vec![format!("{d}/remote.git"), format!("{d}/remote2.git")];
        restore_flow(Env::Wsl, &work, &manifest_dir, &push_urls, None, || {
            let o = exec::run_wsl("Ubuntu", &format!("mv {} {}", sh_quote(&work), sh_quote(&format!("{d}/gone")))).unwrap();
            assert!(o.ok());
        });
        exec::run_wsl("Ubuntu", &format!("rm -rf {}", sh_quote(&d))).unwrap();
        assert!(file_path(&manifest_dir, Env::Windows).exists() && file_path(&manifest_dir, Env::Wsl).exists());
    }
}
