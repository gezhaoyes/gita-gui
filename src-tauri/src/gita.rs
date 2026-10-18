//! 读 gita 的登记文件（repos.csv / groups.csv），写入则调用 gita 命令

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::exec::{self, sh_quote, Env, Out};

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct GitaRepo {
    pub name: String,
    pub path: String,
    pub groups: Vec<String>,
}

const GROUPS_MARK: &str = "@@GG-GROUPS@@";

/// 解析两份登记文件。repos.csv 每行：path,name,type,flags；groups.csv 每行：group:repo1 repo2:group-path
pub fn parse(repos_csv: &str, groups_csv: &str) -> Vec<GitaRepo> {
    let mut repos: Vec<GitaRepo> = csv::ReaderBuilder::new()
        .has_headers(false)
        .flexible(true)
        .from_reader(repos_csv.as_bytes())
        .records()
        .filter_map(Result::ok)
        .filter_map(|r| {
            let path = r.get(0)?.trim();
            let name = r.get(1)?.trim();
            (!path.is_empty() && !name.is_empty()).then(|| GitaRepo {
                name: name.to_string(),
                path: path.to_string(),
                groups: Vec::new(),
            })
        })
        .collect();

    let mut groups_of: HashMap<String, Vec<String>> = HashMap::new();
    for r in csv::ReaderBuilder::new()
        .has_headers(false)
        .flexible(true)
        .delimiter(b':')
        .from_reader(groups_csv.as_bytes())
        .records()
        .filter_map(Result::ok)
    {
        let (Some(group), Some(members)) = (r.get(0), r.get(1)) else {
            continue;
        };
        for repo in members.split_whitespace() {
            groups_of
                .entry(repo.to_string())
                .or_default()
                .push(group.trim().to_string());
        }
    }
    for repo in &mut repos {
        if let Some(g) = groups_of.remove(&repo.name) {
            repo.groups = g;
        }
    }
    repos
}

/// Windows 侧 gita 配置目录，规则和 gita 的 get_config_dir 一致
pub fn windows_config_dir() -> PathBuf {
    let root = std::env::var_os("GITA_PROJECT_HOME")
        .or_else(|| std::env::var_os("XDG_CONFIG_HOME"))
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            let home = std::env::var_os("USERPROFILE").unwrap_or_default();
            PathBuf::from(home).join(".config")
        });
    root.join("gita")
}

const WSL_DIR: &str = "d=\"${GITA_PROJECT_HOME:-${XDG_CONFIG_HOME:-$HOME/.config}}/gita\"\n";

/// 两份登记文件的原文。gbk 表示 Windows 侧文件是按 GBK 写的，写回时保持
/// 也用作修改前的快照：出错时原样写回
pub struct RawFiles {
    repos: String,
    groups: String,
    gbk: bool,
}

/// 只有文件不存在才当作空登记；其他读取错误（被锁、没权限）必须报错，
/// 否则空登记会被当成"所有仓库都取消了登记"，进而把清单清空
fn read_raw_windows(dir: &Path) -> Result<RawFiles, String> {
    let mut gbk = false;
    let mut read = |f: &str| -> Result<String, String> {
        let path = dir.join(f);
        let bytes = match std::fs::read(&path) {
            Ok(b) => b,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Vec::new(),
            Err(e) => return Err(format!("读取 gita 登记文件失败（{}）：{e}", path.display())),
        };
        gbk |= std::str::from_utf8(&bytes).is_err();
        Ok(exec::decode_file(&bytes))
    };
    Ok(RawFiles { repos: read("repos.csv")?, groups: read("groups.csv")?, gbk })
}

pub fn read_raw(env: Env, distro: &str) -> Result<RawFiles, String> {
    match env {
        Env::Windows => read_raw_windows(&windows_config_dir()),
        Env::Wsl => {
            let script = format!(
                "{WSL_DIR}if [ -e \"$d/repos.csv\" ]; then cat \"$d/repos.csv\" || exit 3; fi\n\
                 echo '{GROUPS_MARK}'\n\
                 if [ -e \"$d/groups.csv\" ]; then cat \"$d/groups.csv\" || exit 3; fi"
            );
            let out = exec::run_wsl(distro, &script)?;
            if !out.ok() {
                return Err(format!("读取 WSL 的 gita 登记失败：{}", out.message()));
            }
            let (repos, groups) = out
                .stdout
                .split_once(GROUPS_MARK)
                .ok_or_else(|| format!("WSL 返回内容异常：{}", out.message()))?;
            // echo 前如果文件末尾没有换行，标记会接在最后一行后面；切掉它不影响内容
            Ok(RawFiles { repos: repos.to_string(), groups: groups.trim_start_matches('\n').to_string(), gbk: false })
        }
    }
}

/// 读取某一边的登记
pub fn read_registry(env: Env, distro: &str) -> Result<Vec<GitaRepo>, String> {
    let raw = read_raw(env, distro)?;
    Ok(parse(&raw.repos, &raw.groups))
}

fn records(text: &str, delimiter: u8) -> Vec<csv::StringRecord> {
    csv::ReaderBuilder::new()
        .has_headers(false)
        .flexible(true)
        .delimiter(delimiter)
        .from_reader(text.as_bytes())
        .records()
        .filter_map(Result::ok)
        .collect()
}

/// 按 Python csv 模块的默认格式写回（QUOTE_MINIMAL，\r\n 换行），和 gita 自己写的一致
fn write_csv(records: &[csv::StringRecord], delimiter: u8) -> String {
    let mut w = csv::WriterBuilder::new()
        .has_headers(false)
        .flexible(true)
        .delimiter(delimiter)
        .terminator(csv::Terminator::CRLF)
        .quote_style(csv::QuoteStyle::Necessary)
        .from_writer(Vec::new());
    for r in records {
        w.write_record(r).expect("write to memory");
    }
    String::from_utf8(w.into_inner().expect("flush to memory")).expect("utf-8 input")
}

/// 从两份文件的内容里去掉一个仓库；分组里也去掉它，空分组删除（和 gita rm 的行为一致）
fn remove_from(repos: &str, groups: &str, name: &str) -> Option<(String, String)> {
    let all = records(repos, b',');
    let kept: Vec<_> = all.iter().filter(|r| r.get(1).map(str::trim) != Some(name)).cloned().collect();
    if kept.len() == all.len() {
        return None;
    }
    let groups: Vec<csv::StringRecord> = records(groups, b':')
        .into_iter()
        .filter_map(|r| {
            let members: Vec<&str> = r.get(1).unwrap_or("").split_whitespace().filter(|m| *m != name).collect();
            if members.is_empty() {
                return None;
            }
            let mut fields: Vec<String> = r.iter().map(str::to_string).collect();
            fields[1] = members.join(" ");
            Some(csv::StringRecord::from(fields))
        })
        .collect();
    Some((write_csv(&kept, b','), write_csv(&groups, b':')))
}

/// 取消登记一个仓库，不删文件。
/// 不用 `gita rm`：它会把所有路径暂时不存在的登记一并删掉，而且删不了路径已经不存在的登记
pub fn remove(env: Env, distro: &str, name: &str) -> Result<(), String> {
    let raw = read_raw(env, distro)?;
    let (repos, groups) =
        remove_from(&raw.repos, &raw.groups, name).ok_or_else(|| format!("找不到登记：{name}"))?;
    write_raw(env, distro, &RawFiles { repos, groups, gbk: raw.gbk })
}

/// 把两份登记文件整体写回（先写临时文件再改名），编码保持读取时的样子
pub fn write_raw(env: Env, distro: &str, raw: &RawFiles) -> Result<(), String> {
    match env {
        Env::Windows => {
            let dir = windows_config_dir();
            std::fs::create_dir_all(&dir).map_err(|e| format!("创建 gita 配置目录失败：{e}"))?;
            let encode = |s: &str| -> Vec<u8> {
                if raw.gbk { encoding_rs::GBK.encode(s).0.into_owned() } else { s.as_bytes().to_vec() }
            };
            for (file, content) in [("repos.csv", &raw.repos), ("groups.csv", &raw.groups)] {
                let path = dir.join(file);
                let tmp = dir.join(format!("{file}.tmp"));
                std::fs::write(&tmp, encode(content)).map_err(|e| format!("写入 {file} 失败：{e}"))?;
                std::fs::rename(&tmp, &path).map_err(|e| format!("替换 {file} 失败：{e}"))?;
            }
            Ok(())
        }
        Env::Wsl => {
            let script = format!(
                "{WSL_DIR}set -e\nmkdir -p \"$d\"\n\
                 printf '%s' {} > \"$d/repos.csv.tmp\" && mv \"$d/repos.csv.tmp\" \"$d/repos.csv\"\n\
                 printf '%s' {} > \"$d/groups.csv.tmp\" && mv \"$d/groups.csv.tmp\" \"$d/groups.csv\"",
                sh_quote(&raw.repos),
                sh_quote(&raw.groups)
            );
            let out = exec::run_wsl(distro, &script)?;
            if out.ok() { Ok(()) } else { Err(format!("写入 WSL 登记失败：{}", out.message())) }
        }
    }
}

/// 调用某一边的 gita 命令
pub fn run(env: Env, distro: &str, gita_windows: &str, args: &[&str]) -> Result<Out, String> {
    match env {
        Env::Windows => exec::run_local(gita_windows, args, None),
        Env::Wsl => {
            let quoted: Vec<String> = args.iter().map(|a| sh_quote(a)).collect();
            exec::run_wsl(distro, &format!("gita {}", quoted.join(" ")))
        }
    }
}

/// 调用 gita，失败时把输出变成错误信息
pub fn run_ok(env: Env, distro: &str, gita_windows: &str, args: &[&str]) -> Result<Out, String> {
    let out = run(env, distro, gita_windows, args)?;
    if out.ok() {
        Ok(out)
    } else {
        Err(format!("gita {} 失败：{}", args.join(" "), out.message()))
    }
}

/// 比较路径用的规范形式：Windows 不区分大小写、统一斜杠；两边都去掉末尾斜杠
pub fn norm_path(env: Env, p: &str) -> String {
    match env {
        Env::Windows => p.replace('/', "\\").trim_end_matches('\\').to_lowercase(),
        Env::Wsl => {
            let t = p.trim_end_matches('/');
            if t.is_empty() { "/".into() } else { t.to_string() }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_repos_and_groups() {
        let repos = "D:\\code\\a,a,,\r\n\"D:\\my code\\b,c\",b-c,,\r\n/home/u/x,x,,\r\n";
        let groups = "work:a b-c:\"D:\\code\"\r\nlinux:x:\r\n";
        let r = parse(repos, groups);
        assert_eq!(r.len(), 3);
        assert_eq!(r[0].path, "D:\\code\\a");
        assert_eq!(r[0].groups, vec!["work"]);
        assert_eq!(r[1].path, "D:\\my code\\b,c");
        assert_eq!(r[1].name, "b-c");
        assert_eq!(r[1].groups, vec!["work"]);
        assert_eq!(r[2].groups, vec!["linux"]);
    }

    #[test]
    fn parse_empty_and_multi_group() {
        assert!(parse("", "").is_empty());
        let r = parse("/a,a,,\n", "g1:a:\ng2:a:\n");
        assert_eq!(r[0].groups, vec!["g1", "g2"]);
    }

    #[test]
    fn remove_keeps_other_lines() {
        let repos = "D:\\code\\a,a,,\r\n\"D:\\my code\\b,c\",b-c,,\r\nD:\\gone,gone,,\r\n";
        let groups = "work:a gone:\"D:\\code\"\r\nold:gone:\r\n";
        let (r, g) = remove_from(repos, groups, "a").unwrap();
        assert_eq!(r, "\"D:\\my code\\b,c\",b-c,,\r\nD:\\gone,gone,,\r\n");
        assert_eq!(g, "work:gone:\"D:\\code\"\r\nold:gone:\r\n");
        let (_, g) = remove_from(repos, groups, "gone").unwrap();
        assert_eq!(g, "work:a:\"D:\\code\"\r\n"); // 空分组 old 被删
        assert!(remove_from(repos, groups, "nope").is_none());
    }

    #[test]
    fn read_errors_are_not_empty_registry() {
        let tmp = tempfile::tempdir().unwrap();
        // 目录不存在、文件不存在：就是空登记
        assert!(read_raw_windows(&tmp.path().join("none")).unwrap().repos.is_empty());
        // 读不了（这里用同名目录模拟被锁、没权限）：必须报错
        std::fs::create_dir(tmp.path().join("repos.csv")).unwrap();
        assert!(read_raw_windows(tmp.path()).is_err());
    }

    #[test]
    fn norm_paths() {
        assert_eq!(norm_path(Env::Windows, "D:/Code/A/"), "d:\\code\\a");
        assert_eq!(norm_path(Env::Wsl, "/home/u/x/"), "/home/u/x");
        assert_eq!(norm_path(Env::Wsl, "/"), "/");
    }
}
