//! 执行外部命令：Windows 侧直接运行，WSL 侧把一批命令拼成脚本，通过一次 `wsl.exe -- sh -s` 执行。

use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Env {
    Windows,
    Wsl,
}

impl Env {
    pub fn as_str(self) -> &'static str {
        match self {
            Env::Windows => "windows",
            Env::Wsl => "wsl",
        }
    }
}

#[derive(Debug, Clone)]
pub struct Out {
    pub code: i32,
    pub stdout: String,
    pub stderr: String,
}

impl Out {
    pub fn ok(&self) -> bool {
        self.code == 0
    }

    /// stderr 和 stdout 合在一起，用于展示错误
    pub fn message(&self) -> String {
        let s = format!("{}\n{}", self.stderr.trim(), self.stdout.trim());
        s.trim().to_string()
    }
}

/// 所有 git 调用共用的环境变量：缺凭据直接报错不卡住；输出英文，便于错误分类
const GIT_ENV: &[(&str, &str)] = &[
    ("GIT_TERMINAL_PROMPT", "0"),
    ("GIT_HTTP_LOW_SPEED_LIMIT", "1000"),
    ("GIT_HTTP_LOW_SPEED_TIME", "60"),
    ("LC_ALL", "C"),
    ("LANGUAGE", "en"),
];

/// WSL 脚本开头：同样的环境变量，并把 pipx/venv 装的 gita 所在目录加进 PATH
/// （用户的登录脚本未必会被 sh 读取）
pub const WSL_PREAMBLE: &str = "export GIT_TERMINAL_PROMPT=0 GIT_HTTP_LOW_SPEED_LIMIT=1000 \
GIT_HTTP_LOW_SPEED_TIME=60 LC_ALL=C.UTF-8 LANGUAGE=en PYTHONIOENCODING=utf-8\n\
export PATH=\"$HOME/.local/bin:$PATH\"\n";

#[cfg(windows)]
fn no_window(cmd: &mut Command) {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    cmd.creation_flags(CREATE_NO_WINDOW);
}

#[cfg(not(windows))]
fn no_window(_cmd: &mut Command) {}

/// 在 Windows 侧运行一个程序
pub fn run_local(program: &str, args: &[&str], cwd: Option<&Path>) -> Result<Out, String> {
    let mut cmd = Command::new(program);
    cmd.args(args)
        .envs(GIT_ENV.iter().copied())
        .env("PYTHONIOENCODING", "utf-8")
        .stdin(Stdio::null());
    if let Some(dir) = cwd {
        cmd.current_dir(dir);
    }
    no_window(&mut cmd);
    let output = cmd
        .output()
        .map_err(|e| format!("无法运行 {program}：{e}"))?;
    Ok(Out {
        code: output.status.code().unwrap_or(-1),
        stdout: decode(&output.stdout),
        stderr: decode(&output.stderr),
    })
}

/// 在 Windows 侧运行 git
pub fn git_local(repo: &Path, args: &[&str]) -> Result<Out, String> {
    let mut full = vec!["-c", "core.quotepath=false"];
    full.extend_from_slice(args);
    run_local("git", &full, Some(repo))
}

/// 在 WSL 里执行一段 sh 脚本。脚本通过 stdin 传入，避开 wsl.exe 的参数转义问题
pub fn run_wsl(distro: &str, script: &str) -> Result<Out, String> {
    let mut cmd = Command::new("wsl.exe");
    cmd.args(["-d", distro, "--cd", "~", "--", "sh", "-s"])
        .env("WSL_UTF8", "1")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    no_window(&mut cmd);
    let mut child = cmd.spawn().map_err(|e| format!("无法运行 wsl.exe：{e}"))?;
    {
        let mut stdin = child.stdin.take().expect("stdin is piped");
        let full = format!("{WSL_PREAMBLE}{script}\n");
        stdin
            .write_all(full.as_bytes())
            .map_err(|e| format!("向 WSL 写入脚本失败：{e}"))?;
    }
    let output = child
        .wait_with_output()
        .map_err(|e| format!("等待 WSL 结束失败：{e}"))?;
    Ok(Out {
        code: output.status.code().unwrap_or(-1),
        stdout: decode(&output.stdout),
        stderr: decode(&output.stderr),
    })
}

/// 解码命令输出。wsl.exe 自己的提示是 UTF-16LE，其余（git、sh）是 UTF-8
pub fn decode(bytes: &[u8]) -> String {
    let bytes = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(bytes);
    if looks_utf16le(bytes) {
        let bytes = bytes.strip_prefix(&[0xFF, 0xFE]).unwrap_or(bytes);
        let units: Vec<u16> = bytes
            .chunks_exact(2)
            .map(|c| u16::from_le_bytes([c[0], c[1]]))
            .collect();
        return String::from_utf16_lossy(&units);
    }
    String::from_utf8_lossy(bytes).into_owned()
}

fn looks_utf16le(bytes: &[u8]) -> bool {
    if bytes.starts_with(&[0xFF, 0xFE]) {
        return true;
    }
    if bytes.len() < 4 {
        return false;
    }
    let odd_zeros = bytes.iter().skip(1).step_by(2).filter(|&&b| b == 0).count();
    odd_zeros * 2 > bytes.len() / 2
}

/// 读本地文本文件：先按 UTF-8，失败再按 GBK（Windows 版 gita 用系统默认编码写文件）
pub fn decode_file(bytes: &[u8]) -> String {
    let bytes = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(bytes);
    match std::str::from_utf8(bytes) {
        Ok(s) => s.to_string(),
        Err(_) => encoding_rs::GBK.decode(bytes).0.into_owned(),
    }
}

/// 单引号 shell 转义，路径里有空格、中文、引号都安全
pub fn sh_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', r"'\''"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quote_handles_spaces_and_quotes() {
        assert_eq!(sh_quote("a b"), "'a b'");
        assert_eq!(sh_quote("it's"), r"'it'\''s'");
        assert_eq!(sh_quote("中文 目录"), "'中文 目录'");
    }

    #[test]
    fn decode_utf16_and_utf8() {
        let utf16: Vec<u8> = "WSL 错误".encode_utf16().flat_map(|u| u.to_le_bytes()).collect();
        assert_eq!(decode(&utf16), "WSL 错误");
        assert_eq!(decode("git 输出".as_bytes()), "git 输出");
    }

    #[test]
    fn decode_file_falls_back_to_gbk() {
        let (gbk, _, _) = encoding_rs::GBK.encode("D:\\代码\\仓库");
        assert_eq!(decode_file(&gbk), "D:\\代码\\仓库");
        assert_eq!(decode_file("D:\\代码".as_bytes()), "D:\\代码");
    }
}
