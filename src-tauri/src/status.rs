//! 解析 `git status --porcelain=v2 --branch` 和远程地址，判断颜色

use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;

use serde::Serialize;

use crate::exec::{self, sh_quote, Env};

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Remote {
    pub name: String,
    /// 拉取地址：第一个 remote.<名字>.url
    pub url: String,
    /// 所有 remote.<名字>.url；可以配多个，拉取只用第一个，没配 pushurl 时推送到全部
    pub urls: Vec<String>,
    /// 显式配置的推送地址（remote.<名字>.pushurl）
    pub push_urls: Vec<String>,
}

impl Remote {
    /// 实际推送到的地址，规则和 git 一致：有 pushurl 用 pushurl，否则推到所有 url
    pub fn effective_push_urls(&self) -> Vec<String> {
        if self.push_urls.is_empty() { self.urls.clone() } else { self.push_urls.clone() }
    }
}

#[derive(Debug, Clone, Default, Serialize, PartialEq)]
pub struct Status {
    /// None 表示分离 HEAD
    pub branch: Option<String>,
    pub upstream: Option<String>,
    pub ahead: u32,
    pub behind: u32,
    pub dirty: bool,
    pub remotes: Vec<Remote>,
    /// 设了上游，但本地已经没有这个远程分支的记录（远程分支被删并 prune、默认分支改名等）。
    /// 这时 git 不输出 branch.ab，ahead 无从比较，不能当成 0
    pub upstream_gone: bool,
}

impl Status {
    /// 上游所在的远程；没有上游时取 origin，再没有就取第一个
    pub fn remote(&self) -> Option<&Remote> {
        if let Some(up) = &self.upstream {
            let hit = self
                .remotes
                .iter()
                .filter(|r| up.starts_with(&format!("{}/", r.name)))
                .max_by_key(|r| r.name.len());
            if hit.is_some() {
                return hit;
            }
        }
        self.remotes
            .iter()
            .find(|r| r.name == "origin")
            .or(self.remotes.first())
    }
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "lowercase")]
pub enum Color {
    Green,
    Yellow,
    Red,
    Gray,
}

pub fn parse_porcelain(text: &str) -> Status {
    let mut s = Status::default();
    let mut has_ab = false;
    for line in text.lines() {
        if let Some(head) = line.strip_prefix("# branch.head ") {
            s.branch = (head != "(detached)").then(|| head.to_string());
        } else if let Some(up) = line.strip_prefix("# branch.upstream ") {
            s.upstream = Some(up.to_string());
        } else if let Some(ab) = line.strip_prefix("# branch.ab ") {
            has_ab = true;
            for part in ab.split_whitespace() {
                if let Some(n) = part.strip_prefix('+') {
                    s.ahead = n.parse().unwrap_or(0);
                } else if let Some(n) = part.strip_prefix('-') {
                    s.behind = n.parse().unwrap_or(0);
                }
            }
        } else if ["1 ", "2 ", "u ", "? "].iter().any(|p| line.starts_with(p)) {
            s.dirty = true;
        }
    }
    s.upstream_gone = s.upstream.is_some() && !has_ab;
    s
}

/// 解析 `git config --get-regexp '^remote\..*\.(url|pushurl)$'` 的输出，按远程名合并
pub fn parse_remotes(text: &str) -> Vec<Remote> {
    let mut remotes: Vec<Remote> = Vec::new();
    for line in text.lines() {
        let Some((key, value)) = line.split_once(' ') else { continue };
        let Some(rest) = key.strip_prefix("remote.") else { continue };
        let (name, is_push) = match rest.strip_suffix(".pushurl") {
            Some(n) => (n, true),
            None => match rest.strip_suffix(".url") {
                Some(n) => (n, false),
                None => continue,
            },
        };
        let idx = match remotes.iter().position(|r| r.name == name) {
            Some(i) => i,
            None => {
                remotes.push(Remote {
                    name: name.to_string(),
                    url: String::new(),
                    urls: Vec::new(),
                    push_urls: Vec::new(),
                });
                remotes.len() - 1
            }
        };
        let value = value.trim().to_string();
        let r = &mut remotes[idx];
        if is_push {
            r.push_urls.push(value);
        } else {
            if r.urls.is_empty() {
                r.url = value.clone();
            }
            r.urls.push(value);
        }
    }
    // 只配了 pushurl、没有 url 的不算一个可用的远程
    remotes.retain(|r| !r.url.is_empty());
    remotes
}

/// 判断颜色和原因。多种情况同时满足时按最严重的：灰 > 红 > 黄 > 绿
pub fn evaluate(exists: bool, status: &Result<Status, String>) -> (Color, Vec<String>) {
    if !exists {
        return (Color::Gray, vec!["路径不存在".into()]);
    }
    let s = match status {
        Ok(s) => s,
        Err(e) => return (Color::Red, vec![format!("读取失败：{e}")]),
    };
    let mut red = Vec::new();
    if s.remotes.is_empty() {
        red.push("没有远程仓库".to_string());
    }
    match (&s.branch, &s.upstream) {
        (None, _) => red.push("分离 HEAD".into()),
        (Some(_), None) if !s.remotes.is_empty() => red.push("没有上游".into()),
        (Some(_), Some(up)) if s.upstream_gone => {
            red.push(format!("上游分支 {up} 已不存在，无法确认是否已推送"))
        }
        _ => {}
    }
    if s.ahead > 0 {
        red.push(format!("{} 个提交未推送", s.ahead));
    }
    if !red.is_empty() {
        if s.dirty {
            red.push("有未提交的改动".into());
        }
        return (Color::Red, red);
    }
    if s.dirty {
        return (Color::Yellow, vec!["有未提交的改动".into()]);
    }
    (Color::Green, Vec::new())
}

/// 总表里的一行：登记信息 + 状态
#[derive(Debug, Clone, Serialize)]
pub struct RepoRow {
    pub env: Env,
    pub name: String,
    pub path: String,
    pub groups: Vec<String>,
    pub exists: bool,
    pub status: Option<Status>,
    pub error: Option<String>,
    pub remote_url: Option<String>,
    /// 这个远程实际推送到的地址（一个远程可以配多个推送地址）
    pub push_urls: Vec<String>,
    pub color: Color,
    pub reasons: Vec<String>,
    /// 上次在工具里推送失败的信息
    pub push_failed: Option<String>,
}

impl RepoRow {
    pub fn new(env: Env, repo: crate::gita::GitaRepo, raw: Raw) -> Self {
        let (color, reasons) = evaluate(raw.exists, &raw.status);
        let (status, error) = match raw.status {
            Ok(s) => (Some(s), None),
            Err(e) => (None, Some(e)),
        };
        let remote = status.as_ref().and_then(|s| s.remote());
        let remote_url = remote.map(|r| r.url.clone());
        let push_urls = remote.map(|r| r.effective_push_urls()).unwrap_or_default();
        Self {
            env,
            name: repo.name,
            path: repo.path,
            groups: repo.groups,
            exists: raw.exists,
            status,
            error,
            remote_url,
            push_urls,
            color,
            reasons,
            push_failed: None,
        }
    }

    /// 上次推送失败：即使 git 记录显示已推送，也标红，直到再推送成功
    pub fn mark_push_failed(&mut self, message: &str) {
        self.push_failed = Some(message.to_string());
        if self.color == Color::Gray {
            return;
        }
        let first = message.lines().map(str::trim).find(|l| !l.is_empty()).unwrap_or("");
        self.reasons.insert(0, format!("上次推送有失败：{first}"));
        self.color = Color::Red;
    }
}

#[derive(Debug, Clone)]
pub struct Raw {
    pub exists: bool,
    pub status: Result<Status, String>,
}

const STATUS_ARGS: &[&str] = &["status", "--porcelain=v2", "--branch"];
const REMOTE_ARGS: &[&str] = &["config", "--get-regexp", r"^remote\..*\.(url|pushurl)$"];

/// 读取一批仓库的状态。Windows 侧并行跑；WSL 侧一次调用拿回全部结果
pub fn collect(env: Env, distro: &str, paths: &[String]) -> Result<Vec<Raw>, String> {
    match env {
        Env::Windows => Ok(collect_windows(paths)),
        Env::Wsl => collect_wsl(distro, paths),
    }
}

fn status_one_windows(path: &str) -> Raw {
    let p = Path::new(path);
    if !p.is_dir() {
        return Raw { exists: false, status: Err("路径不存在".into()) };
    }
    let status = exec::git_local(p, STATUS_ARGS).and_then(|out| {
        if !out.ok() {
            return Err(out.message());
        }
        let mut s = parse_porcelain(&out.stdout);
        if let Ok(r) = exec::git_local(p, REMOTE_ARGS) {
            s.remotes = parse_remotes(&r.stdout);
        }
        Ok(s)
    });
    Raw { exists: true, status }
}

fn collect_windows(paths: &[String]) -> Vec<Raw> {
    let results: Vec<Mutex<Option<Raw>>> = paths.iter().map(|_| Mutex::new(None)).collect();
    let next = AtomicUsize::new(0);
    let workers = paths.len().clamp(1, 8);
    std::thread::scope(|scope| {
        for _ in 0..workers {
            scope.spawn(|| loop {
                let i = next.fetch_add(1, Ordering::Relaxed);
                let Some(path) = paths.get(i) else { break };
                *results[i].lock().unwrap() = Some(status_one_windows(path));
            });
        }
    });
    results
        .into_iter()
        .map(|m| m.into_inner().unwrap().expect("every slot filled"))
        .collect()
}

const REPO_MARK: &str = "@@GG-REPO@@";
const MISSING_MARK: &str = "@@GG-MISSING@@";
const RC_MARK: &str = "@@GG-RC@@";

/// 生成 WSL 批量脚本：每个仓库输出一段，用标记分隔
pub fn wsl_status_script(paths: &[String]) -> String {
    let mut s = String::new();
    for (i, p) in paths.iter().enumerate() {
        let q = sh_quote(p);
        s.push_str(&format!(
            "echo '{REPO_MARK} {i}'\n\
             if [ -d {q} ]; then\n\
             (cd {q} && git -c core.quotepath=false {status}) 2>&1; echo \"{RC_MARK} $?\"\n\
             (cd {q} && git {remote}) 2>/dev/null\n\
             else echo '{MISSING_MARK}'; fi\n",
            status = STATUS_ARGS.join(" "),
            remote = r"config --get-regexp '^remote\..*\.(url|pushurl)$'",
        ));
    }
    s
}

/// 解析 WSL 批量脚本的输出
pub fn parse_wsl_output(text: &str, count: usize) -> Vec<Raw> {
    let mut out: Vec<Raw> = (0..count)
        .map(|_| Raw { exists: false, status: Err("WSL 没有返回结果".into()) })
        .collect();
    let mut sections: Vec<(usize, Vec<&str>)> = Vec::new();
    for line in text.lines() {
        if let Some(i) = line.strip_prefix(REPO_MARK).and_then(|r| r.trim().parse().ok()) {
            sections.push((i, Vec::new()));
        } else if let Some((_, lines)) = sections.last_mut() {
            lines.push(line);
        }
    }
    for (i, lines) in sections {
        if i >= count {
            continue;
        }
        if lines.first() == Some(&MISSING_MARK) {
            out[i] = Raw { exists: false, status: Err("路径不存在".into()) };
            continue;
        }
        let Some(rc_pos) = lines.iter().position(|l| l.starts_with(RC_MARK)) else {
            continue;
        };
        let rc: i32 = lines[rc_pos][RC_MARK.len()..].trim().parse().unwrap_or(-1);
        let status_text = lines[..rc_pos].join("\n");
        let status = if rc == 0 {
            let mut s = parse_porcelain(&status_text);
            s.remotes = parse_remotes(&lines[rc_pos + 1..].join("\n"));
            Ok(s)
        } else {
            Err(status_text.trim().to_string())
        };
        out[i] = Raw { exists: true, status };
    }
    out
}

fn collect_wsl(distro: &str, paths: &[String]) -> Result<Vec<Raw>, String> {
    if paths.is_empty() {
        return Ok(Vec::new());
    }
    let out = exec::run_wsl(distro, &wsl_status_script(paths))?;
    if !out.stdout.contains(REPO_MARK) {
        return Err(format!("WSL 调用失败：{}", out.message()));
    }
    Ok(parse_wsl_output(&out.stdout, paths.len()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;

    const SAMPLE: &str = "# branch.oid abc\n# branch.head main\n# branch.upstream origin/main\n# branch.ab +2 -1\n1 .M N... 100644 100644 100644 a b src/x.rs\n";

    #[test]
    fn porcelain_basic() {
        let s = parse_porcelain(SAMPLE);
        assert_eq!(s.branch.as_deref(), Some("main"));
        assert_eq!(s.upstream.as_deref(), Some("origin/main"));
        assert_eq!((s.ahead, s.behind), (2, 1));
        assert!(s.dirty);
    }

    #[test]
    fn porcelain_detached_clean() {
        let s = parse_porcelain("# branch.oid abc\n# branch.head (detached)\n");
        assert_eq!(s.branch, None);
        assert!(!s.dirty);
    }

    #[test]
    fn remotes_with_dots() {
        let r = parse_remotes("remote.origin.url https://x/a.git\nremote.my.fork.url git@y:b.git\n");
        assert_eq!(r[1].name, "my.fork");
        assert_eq!(r[1].url, "git@y:b.git");
        assert_eq!(r[1].effective_push_urls(), vec!["git@y:b.git"]);
    }

    #[test]
    fn remotes_with_push_urls() {
        let r = parse_remotes(
            "remote.origin.url G\nremote.origin.pushurl G\nremote.origin.pushurl E\nremote.only.pushurl X\n",
        );
        assert_eq!(r.len(), 1);
        assert_eq!(r[0].url, "G");
        assert_eq!(r[0].effective_push_urls(), vec!["G", "E"]);
    }

    #[test]
    fn remotes_with_multiple_urls() {
        // git remote set-url --add origin B：拉取只用 A，推送 A、B 都推
        let r = parse_remotes("remote.origin.url A\nremote.origin.url B\n");
        assert_eq!(r[0].url, "A");
        assert_eq!(r[0].effective_push_urls(), vec!["A", "B"]);
        // 同时配了 pushurl 时，pushurl 优先
        let r = parse_remotes("remote.origin.url A\nremote.origin.url B\nremote.origin.pushurl C\n");
        assert_eq!(r[0].effective_push_urls(), vec!["C"]);
    }

    #[test]
    fn remote_follows_upstream() {
        let mut s = parse_porcelain("# branch.head main\n# branch.upstream up/main\n");
        s.remotes = parse_remotes("remote.origin.url A\nremote.up.url B\n");
        assert_eq!(s.remote().unwrap().url, "B");
        s.upstream = None;
        assert_eq!(s.remote().unwrap().url, "A");
    }

    #[test]
    fn colors() {
        let remotes = parse_remotes("remote.origin.url A\n");
        let ok = Status {
            branch: Some("main".into()),
            upstream: Some("origin/main".into()),
            remotes: remotes.clone(),
            ..Default::default()
        };
        assert_eq!(evaluate(true, &Ok(ok.clone())).0, Color::Green);
        assert_eq!(evaluate(true, &Ok(Status { dirty: true, ..ok.clone() })).0, Color::Yellow);
        assert_eq!(evaluate(true, &Ok(Status { ahead: 1, dirty: true, ..ok.clone() })).0, Color::Red);
        assert_eq!(evaluate(true, &Ok(Status { upstream: None, ..ok.clone() })).0, Color::Red);
        assert_eq!(evaluate(true, &Ok(Status { remotes: vec![], ..ok.clone() })).0, Color::Red);
        assert_eq!(evaluate(false, &Ok(ok)).0, Color::Gray);
    }

    #[test]
    fn wsl_output_roundtrip() {
        let text = format!(
            "{REPO_MARK} 0\n{SAMPLE}{RC_MARK} 0\nremote.origin.url U\n\
             {REPO_MARK} 1\n{MISSING_MARK}\n\
             {REPO_MARK} 2\nfatal: not a git repository\n{RC_MARK} 128\n"
        );
        let r = parse_wsl_output(&text, 3);
        let s0 = r[0].status.as_ref().unwrap();
        assert_eq!(s0.ahead, 2);
        assert_eq!(s0.remotes[0].url, "U");
        assert!(!r[1].exists);
        assert!(r[2].exists && r[2].status.as_ref().unwrap_err().contains("not a git"));
    }

    fn git(dir: &Path, args: &[&str]) {
        let ok = Command::new("git").args(args).current_dir(dir).output().unwrap();
        assert!(ok.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&ok.stderr));
    }

    /// 在临时目录里建真实仓库，验证从本地 git 读出的状态
    #[test]
    fn real_repo_states() {
        let tmp = tempfile::tempdir().unwrap();
        let remote = tmp.path().join("remote.git");
        let work = tmp.path().join("工作 目录");
        std::fs::create_dir_all(&work).unwrap();
        git(tmp.path(), &["init", "--bare", "-b", "main", remote.to_str().unwrap()]);
        git(&work, &["init", "-b", "main"]);
        git(&work, &["config", "user.email", "t@t"]);
        git(&work, &["config", "user.name", "t"]);
        std::fs::write(work.join("a.txt"), "1").unwrap();
        git(&work, &["add", "."]);
        git(&work, &["commit", "-m", "c1"]);

        let p = work.to_str().unwrap().to_string();
        let check = |want: Color| {
            let raw = &collect_windows(std::slice::from_ref(&p))[0];
            let (c, why) = evaluate(raw.exists, &raw.status);
            assert_eq!(c, want, "{why:?}");
        };
        check(Color::Red); // 没有远程
        git(&work, &["remote", "add", "origin", remote.to_str().unwrap()]);
        check(Color::Red); // 没有上游
        git(&work, &["push", "-u", "origin", "main"]);
        check(Color::Green);
        std::fs::write(work.join("b.txt"), "2").unwrap();
        check(Color::Yellow); // 未跟踪文件
        git(&work, &["add", "."]);
        git(&work, &["commit", "-m", "c2"]);
        check(Color::Red); // 未推送
        git(&work, &["push", "-q"]);
        check(Color::Green);
        // 上游的远程分支记录没了（远程删分支后 fetch --prune 就是这样），本地又有新提交
        git(&work, &["-c", "user.email=t@t", "-c", "user.name=t", "commit", "-q", "--allow-empty", "-m", "c3"]);
        git(&work, &["update-ref", "-d", "refs/remotes/origin/main"]);
        check(Color::Red);
        let raw = &collect_windows(std::slice::from_ref(&p))[0];
        assert!(raw.status.as_ref().unwrap().upstream_gone);
        let missing = tmp.path().join("nope").to_str().unwrap().to_string();
        assert!(!collect_windows(&[missing])[0].exists);
    }

    /// 一个远程两个推送地址，一处成功一处失败：git 认为已推送，靠推送失败记录继续标红
    #[test]
    fn partial_push_stays_red() {
        let tmp = tempfile::tempdir().unwrap();
        let good = tmp.path().join("good.git");
        let bad = tmp.path().join("missing").join("bad.git");
        let work = tmp.path().join("work");
        std::fs::create_dir_all(&work).unwrap();
        git(tmp.path(), &["init", "-q", "--bare", "-b", "main", good.to_str().unwrap()]);
        git(&work, &["init", "-q", "-b", "main"]);
        git(&work, &["-c", "user.email=t@t", "-c", "user.name=t", "commit", "-q", "--allow-empty", "-m", "c1"]);
        git(&work, &["remote", "add", "origin", good.to_str().unwrap()]);
        git(&work, &["push", "-q", "-u", "origin", "main"]);
        git(&work, &["remote", "set-url", "--add", "--push", "origin", good.to_str().unwrap()]);
        git(&work, &["remote", "set-url", "--add", "--push", "origin", bad.to_str().unwrap()]);
        git(&work, &["-c", "user.email=t@t", "-c", "user.name=t", "commit", "-q", "--allow-empty", "-m", "c2"]);

        let p = work.to_str().unwrap().to_string();
        let result = crate::push::push(Env::Windows, "", &p, false);
        assert!(!result.ok, "第二个地址不存在，推送应当报失败");

        let raw = collect_windows(std::slice::from_ref(&p)).remove(0);
        let repo = crate::gita::GitaRepo { name: "work".into(), path: p.clone(), groups: vec![] };
        let mut row = RepoRow::new(Env::Windows, repo, raw);
        assert_eq!(row.status.as_ref().unwrap().ahead, 0);
        assert_eq!(row.push_urls.len(), 2);
        assert_eq!(row.color, Color::Green, "前提：git 已把 origin/main 更新，看起来像已备份");
        row.mark_push_failed(&result.message);
        assert_eq!(row.color, Color::Red);
        assert!(row.reasons[0].starts_with("上次推送有失败"));
    }

    /// 走真实的 wsl.exe：cargo test -- --ignored
    #[test]
    #[ignore]
    fn real_wsl_batch() {
        let setup = "set -e; d=$(mktemp -d); cd \"$d\"; mkdir \"repo 中 'q'\"; cd \"repo 中 'q'\"; \
                     git init -q -b main; git -c user.email=t@t -c user.name=t commit -q --allow-empty -m c; \
                     touch dirty; pwd";
        let out = exec::run_wsl("Ubuntu", setup).unwrap();
        assert!(out.ok(), "{}", out.message());
        let repo = out.stdout.trim().to_string();
        let raws = collect_wsl("Ubuntu", &[repo.clone(), "/no/such/path".into()]).unwrap();
        let s = raws[0].status.as_ref().unwrap();
        assert_eq!(s.branch.as_deref(), Some("main"));
        assert!(s.dirty && s.remotes.is_empty());
        assert_eq!(evaluate(raws[0].exists, &raws[0].status).0, Color::Red);
        assert!(!raws[1].exists);
        exec::run_wsl("Ubuntu", &format!("rm -rf \"$(dirname {})\"", sh_quote(&repo))).unwrap();
    }
}
