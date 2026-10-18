//! 推送和错误分类。每个仓库单独调用一次

use std::path::Path;

use serde::Serialize;

use crate::exec::{self, sh_quote, Env, Out};

#[derive(Debug, Clone, Copy, Serialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum PushError {
    /// WSL 运行不了 Windows 的凭据管理器（WSLInterop 未注册）
    Interop,
    Auth,
    NoUpstream,
    Rejected,
    Network,
    Other,
}

#[derive(Debug, Clone, Serialize)]
pub struct PushResult {
    pub ok: bool,
    pub kind: Option<PushError>,
    pub message: String,
}

pub fn classify(msg: &str) -> PushError {
    let m = msg.to_lowercase();
    let has = |keys: &[&str]| keys.iter().any(|k| m.contains(k));
    if has(&["exec format error"]) {
        PushError::Interop
    } else if has(&["[rejected]", "non-fast-forward", "fetch first", "updates were rejected"]) {
        PushError::Rejected
    } else if has(&["has no upstream branch", "no configured push destination"]) {
        PushError::NoUpstream
    } else if has(&[
        "authentication failed",
        "could not read username",
        "could not read password",
        "terminal prompts disabled",
        "permission denied",
        "invalid username or password",
        "repository not found",
        "error: 403",
        "returned error: 401",
    ]) {
        PushError::Auth
    } else if has(&[
        "could not resolve host",
        "failed to connect",
        "timed out",
        "connection refused",
        "network is unreachable",
        "operation too slow",
        "connection reset",
        "could not connect",
        "unable to access",
    ]) {
        PushError::Network
    } else {
        PushError::Other
    }
}

fn to_result(out: Result<Out, String>) -> PushResult {
    match out {
        Ok(o) if o.ok() => PushResult { ok: true, kind: None, message: o.message() },
        Ok(o) => {
            let message = o.message();
            PushResult { ok: false, kind: Some(classify(&message)), message }
        }
        Err(e) => PushResult { ok: false, kind: Some(PushError::Other), message: e },
    }
}

/// set_upstream 为 true 时执行 `git push -u <远程> HEAD`：只有一个远程就用它，否则用 origin
pub fn push(env: Env, distro: &str, path: &str, set_upstream: bool) -> PushResult {
    match env {
        Env::Windows => {
            let dir = Path::new(path);
            if !set_upstream {
                return to_result(exec::git_local(dir, &["push"]));
            }
            let remotes = match exec::git_local(dir, &["remote"]) {
                Ok(o) => o.stdout,
                Err(e) => return to_result(Err(e)),
            };
            let names: Vec<&str> = remotes.lines().map(str::trim).filter(|l| !l.is_empty()).collect();
            let remote = if names.contains(&"origin") || names.is_empty() { "origin" } else { names[0] };
            to_result(exec::git_local(dir, &["push", "-u", remote, "HEAD"]))
        }
        Env::Wsl => {
            let q = sh_quote(path);
            let cmd = if set_upstream {
                "r=$(git remote | grep -qx origin && echo origin || git remote | head -n1); \
                 git push -u \"${r:-origin}\" HEAD"
            } else {
                "git push"
            };
            to_result(exec::run_wsl(distro, &format!("cd {q} && {{ {cmd}; }} 2>&1")))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classify_messages() {
        assert_eq!(
            classify(" ! [rejected]        main -> main (fetch first)\nerror: failed to push"),
            PushError::Rejected
        );
        assert_eq!(
            classify("fatal: The current branch dev has no upstream branch."),
            PushError::NoUpstream
        );
        assert_eq!(
            classify("fatal: could not read Username for 'https://github.com': terminal prompts disabled"),
            PushError::Auth
        );
        assert_eq!(
            classify("fatal: unable to access 'https://github.com/x/y.git/': The requested URL returned error: 403"),
            PushError::Auth
        );
        assert_eq!(
            classify("fatal: unable to access 'https://github.com/x/': Could not resolve host: github.com"),
            PushError::Network
        );
        assert_eq!(
            classify("/mnt/d/Program Files/Git/mingw64/bin/git-credential-manager.exe: Exec format error
fatal: could not read Username for 'https://github.com': terminal prompts disabled"),
            PushError::Interop
        );
        assert_eq!(classify("something odd"), PushError::Other);
    }
}
