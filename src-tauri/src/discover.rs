//! 从选中的目录往下找仓库。找到 `.git` 就停，跳过配置里的目录，有深度上限

use std::path::Path;

use crate::exec::{self, sh_quote, Env};

/// 把用户选的路径分到某一边。`\\wsl.localhost\<发行版>\...` 和 `\\wsl$\<发行版>\...` 属于 WSL
/// 返回 (环境, 该环境里的路径, WSL 发行版名)
pub fn classify(input: &str) -> (Env, String, Option<String>) {
    let unified = input.replace('/', "\\");
    let lower = unified.to_lowercase();
    for prefix in ["\\\\wsl.localhost\\", "\\\\wsl$\\"] {
        if lower.starts_with(prefix) {
            let rest = &unified[prefix.len()..];
            let (distro, path) = rest.split_once('\\').unwrap_or((rest, ""));
            let linux = format!("/{}", path.replace('\\', "/"));
            let linux = if linux.len() > 1 { linux.trim_end_matches('/').to_string() } else { linux };
            return (Env::Wsl, linux, Some(distro.to_string()));
        }
    }
    (Env::Windows, input.to_string(), None)
}

pub fn is_repo_windows(dir: &Path) -> bool {
    dir.join(".git").exists()
}

/// 这个目录本身是不是 git 仓库（有 .git 目录或文件）
pub fn is_repo(env: Env, distro: &str, path: &str) -> Result<bool, String> {
    match env {
        Env::Windows => Ok(is_repo_windows(Path::new(path))),
        Env::Wsl => {
            let out = exec::run_wsl(distro, &format!("[ -e {}/.git ] && echo YES || echo NO", sh_quote(path)))?;
            match out.stdout.trim() {
                "YES" => Ok(true),
                "NO" => Ok(false),
                _ => Err(format!("WSL 调用失败：{}", out.message())),
            }
        }
    }
}

pub fn discover(
    env: Env,
    distro: &str,
    root: &str,
    depth: usize,
    skip: &[String],
) -> Result<Vec<String>, String> {
    match env {
        Env::Windows => {
            let root = Path::new(root);
            if !root.is_dir() {
                return Err(format!("目录不存在：{}", root.display()));
            }
            let mut found = Vec::new();
            walk(root, depth, skip, &mut found);
            Ok(found)
        }
        Env::Wsl => discover_wsl(distro, root, depth, skip),
    }
}

fn walk(dir: &Path, depth: usize, skip: &[String], out: &mut Vec<String>) {
    if is_repo_windows(dir) {
        out.push(dir.to_string_lossy().into_owned());
        return;
    }
    if depth == 0 {
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    let mut subdirs: Vec<_> = entries
        .filter_map(Result::ok)
        .filter(|e| {
            // 不跟随符号链接和目录联接，避免死循环
            e.file_type().map(|t| t.is_dir() && !t.is_symlink()).unwrap_or(false)
        })
        .filter(|e| {
            let name = e.file_name().to_string_lossy().to_lowercase();
            name != ".git" && !skip.iter().any(|s| s.to_lowercase() == name)
        })
        .map(|e| e.path())
        .collect();
    subdirs.sort();
    for sub in subdirs {
        walk(&sub, depth - 1, skip, out);
    }
}

pub fn wsl_find_script(root: &str, depth: usize, skip: &[String]) -> String {
    let prune: Vec<String> = skip.iter().map(|s| format!("-name {}", sh_quote(s))).collect();
    let prune = if prune.is_empty() {
        String::new()
    } else {
        format!("\\( -type d \\( {} \\) -prune \\) -o ", prune.join(" -o "))
    };
    let q = sh_quote(root);
    format!(
        "[ -d {q} ] || {{ echo \"目录不存在：\"{q} >&2; exit 2; }}\n\
         find {q} -maxdepth {} {prune}\\( -name .git -print -prune \\) 2>/dev/null; true",
        depth + 1
    )
}

/// find 会列出嵌套在仓库里的 .git（子模块等），只保留最外层
pub fn outermost(mut git_dirs: Vec<String>) -> Vec<String> {
    let mut repos: Vec<String> = git_dirs
        .drain(..)
        .filter_map(|g| g.strip_suffix("/.git").map(str::to_string))
        .collect();
    repos.sort();
    let mut kept: Vec<String> = Vec::new();
    for r in repos {
        let nested = kept.iter().any(|k| r.starts_with(&format!("{}/", k.trim_end_matches('/'))));
        if !nested {
            kept.push(r);
        }
    }
    kept
}

fn discover_wsl(distro: &str, root: &str, depth: usize, skip: &[String]) -> Result<Vec<String>, String> {
    let out = exec::run_wsl(distro, &wsl_find_script(root, depth, skip))?;
    if !out.ok() {
        return Err(out.message());
    }
    Ok(outermost(out.stdout.lines().map(str::to_string).collect()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classify_paths() {
        assert_eq!(
            classify(r"\\wsl.localhost\Ubuntu\home\u\code"),
            (Env::Wsl, "/home/u/code".into(), Some("Ubuntu".into()))
        );
        assert_eq!(classify(r"\\wsl$\Ubuntu\").1, "/");
        assert_eq!(classify(r"D:\code").0, Env::Windows);
    }

    #[test]
    fn outermost_drops_nested() {
        let r = outermost(vec![
            "/h/a/.git".into(),
            "/h/a/sub/.git".into(),
            "/h/ab/.git".into(),
        ]);
        assert_eq!(r, vec!["/h/a", "/h/ab"]);
    }

    #[test]
    fn walk_finds_and_skips() {
        let tmp = tempfile::tempdir().unwrap();
        let mk = |p: &str| std::fs::create_dir_all(tmp.path().join(p)).unwrap();
        mk("a/.git");
        mk("a/inner/.git"); // 仓库里面的不再往下找
        mk("x/y/b/.git");
        mk("node_modules/c/.git");
        mk("deep/1/2/3/d/.git");
        let skip = vec!["node_modules".to_string()];
        let found = discover(Env::Windows, "", tmp.path().to_str().unwrap(), 3, &skip).unwrap();
        let names: Vec<_> = found
            .iter()
            .map(|p| Path::new(p).file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names, vec!["a", "b"]);
    }
}
