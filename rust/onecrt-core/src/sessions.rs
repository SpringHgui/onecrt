use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// 认证方法的一项：列表里的顺序即连接时的尝试顺序，enabled 为勾选状态
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct AuthMethod {
    pub method: String,
    pub enabled: bool,
}

pub fn default_auth() -> Vec<AuthMethod> {
    ["PublicKey", "Password", "Keyboard Interactive"]
        .into_iter()
        .map(|method| AuthMethod {
            method: method.into(),
            enabled: true,
        })
        .collect()
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct SshSession {
    pub id: String,
    pub name: String,
    pub host: String,
    pub port: u16,
    pub username: String,
    /// 落盘为 DPAPI 密文，读回时解密；旧版本的明文原样兼容
    #[serde(default)]
    pub password: String,
    /// OpenSSH 私钥文件路径；为空则不启用公钥认证
    #[serde(default)]
    pub key_path: String,
    #[serde(default = "default_auth")]
    pub auth: Vec<AuthMethod>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Node {
    Folder {
        id: String,
        name: String,
        #[serde(default)]
        expanded: bool,
        children: Vec<Node>,
    },
    Session(SshSession),
}

#[derive(Serialize, Deserialize, Default, Clone, Debug)]
pub struct SessionTree {
    #[serde(default)]
    pub sessions: Vec<Node>,
}

/// 命令管理器里的一个节点。命令带 Name 和实际要发送的文本，两者可以不同。
#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum CmdNode {
    Folder {
        id: String,
        name: String,
        #[serde(default)]
        expanded: bool,
        #[serde(default)]
        children: Vec<CmdNode>,
    },
    Command {
        id: String,
        name: String,
        text: String,
    },
}

#[derive(Serialize, Deserialize, Default, Debug)]
pub struct CommandTree {
    #[serde(default)]
    pub commands: Vec<CmdNode>,
}

/// 口令落盘加密：Windows 走 DPAPI（密钥由当前登录用户持有，sessions.json 被拷走也只是密文），
/// 解不开（换机器/换用户/配置文件损坏）时按无口令处理，连接时弹框补凭据。
/// 其余平台没有可移植的系统级密钥，退回明文；`dpapi:` 密文在这些平台同样解不开，按无口令处理。
mod secret {
    const PREFIX: &str = "dpapi:";

    pub fn is_protected(stored: &str) -> bool {
        stored.starts_with(PREFIX)
    }

    #[cfg(windows)]
    pub fn protect(plain: &str) -> Result<String, String> {
        dpapi::protect(plain)
    }

    #[cfg(not(windows))]
    pub fn protect(plain: &str) -> Result<String, String> {
        Ok(plain.to_string())
    }

    #[cfg(windows)]
    pub fn unprotect(stored: &str) -> String {
        dpapi::unprotect(stored)
    }

    #[cfg(not(windows))]
    pub fn unprotect(stored: &str) -> String {
        if is_protected(stored) {
            String::new()
        } else {
            stored.to_string()
        }
    }

    #[cfg(windows)]
    mod dpapi {
        use super::PREFIX;
        use windows::Win32::Foundation::{LocalFree, HLOCAL};
        use windows::Win32::Security::Cryptography::{
            CryptProtectData, CryptUnprotectData, CRYPT_INTEGER_BLOB,
        };

        fn hex(bytes: &[u8]) -> String {
            bytes.iter().map(|b| format!("{b:02x}")).collect()
        }

        fn unhex(text: &str) -> Option<Vec<u8>> {
            if text.len() % 2 != 0 {
                return None;
            }
            (0..text.len() / 2)
                .map(|i| u8::from_str_radix(&text[i * 2..i * 2 + 2], 16).ok())
                .collect()
        }

        pub fn protect(plain: &str) -> Result<String, String> {
            let mut bytes = plain.as_bytes().to_vec();
            let input = CRYPT_INTEGER_BLOB {
                cbData: bytes.len() as u32,
                pbData: bytes.as_mut_ptr(),
            };
            let mut output = CRYPT_INTEGER_BLOB {
                cbData: 0,
                pbData: std::ptr::null_mut(),
            };
            unsafe {
                CryptProtectData(&input, None, None, None, None, 0, &mut output)
                    .map_err(|e| format!("口令加密失败: {e}"))?;
                let blob =
                    std::slice::from_raw_parts(output.pbData, output.cbData as usize).to_vec();
                LocalFree(Some(HLOCAL(output.pbData as *mut _)));
                Ok(format!("{PREFIX}{}", hex(&blob)))
            }
        }

        pub fn unprotect(stored: &str) -> String {
            let Some(rest) = stored.strip_prefix(PREFIX) else {
                return stored.to_string();
            };
            let Some(mut bytes) = unhex(rest) else {
                return String::new();
            };
            let input = CRYPT_INTEGER_BLOB {
                cbData: bytes.len() as u32,
                pbData: bytes.as_mut_ptr(),
            };
            let mut output = CRYPT_INTEGER_BLOB {
                cbData: 0,
                pbData: std::ptr::null_mut(),
            };
            unsafe {
                if CryptUnprotectData(&input, None, None, None, None, 0, &mut output).is_err() {
                    return String::new();
                }
                let plain = String::from_utf8_lossy(std::slice::from_raw_parts(
                    output.pbData,
                    output.cbData as usize,
                ))
                .into_owned();
                LocalFree(Some(HLOCAL(output.pbData as *mut _)));
                plain
            }
        }
    }
}

fn map_passwords(
    nodes: &mut [Node],
    f: &impl Fn(&str) -> Result<String, String>,
) -> Result<(), String> {
    for n in nodes.iter_mut() {
        match n {
            Node::Folder { children, .. } => map_passwords(children, f)?,
            Node::Session(s) => s.password = f(&s.password)?,
        }
    }
    Ok(())
}

fn store_path() -> Result<PathBuf, String> {
    let base = dirs::config_dir().ok_or_else(|| "无法获取配置目录".to_string())?;
    let dir = base.join("shellx");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    Ok(dir.join("sessions.json"))
}

pub fn load_tree() -> Result<SessionTree, String> {
    let path = store_path()?;
    if !path.exists() {
        return Ok(SessionTree::default());
    }
    let text = std::fs::read_to_string(&path).map_err(|e| e.to_string())?;
    let mut tree: SessionTree =
        serde_json::from_str(&text).map_err(|e| format!("会话文件损坏: {e}"))?;
    map_passwords(&mut tree.sessions, &|p| Ok(secret::unprotect(p)))?;
    Ok(tree)
}

pub fn save_tree(tree: &SessionTree) -> Result<(), String> {
    let path = store_path()?;
    let mut encrypted = tree.clone();
    map_passwords(&mut encrypted.sessions, &|p| {
        if p.is_empty() || secret::is_protected(p) {
            Ok(p.to_string())
        } else {
            secret::protect(p)
        }
    })?;
    let text = serde_json::to_string_pretty(&encrypted).map_err(|e| e.to_string())?;
    std::fs::write(&path, text).map_err(|e| e.to_string())
}

fn commands_path() -> Result<PathBuf, String> {
    Ok(store_path()?.parent().unwrap().join("commands.json"))
}

fn command(id: &str, text: &str) -> CmdNode {
    CmdNode::Command {
        id: id.into(),
        name: text.into(),
        text: text.into(),
    }
}

fn seed_commands() -> CommandTree {
    CommandTree {
        commands: vec![CmdNode::Folder {
            id: "cf-commands".into(),
            name: "Commands".into(),
            expanded: true,
            children: vec![
                command("c-1", "df -h"),
                command("c-2", "free -m"),
                command("c-3", "top -b -n 1 | head -20"),
                command("c-4", "systemctl status docker"),
            ],
        }],
    }
}

/// 早期版本把命令存成纯字符串数组，读到就原地升级成 Commands 文件夹下的节点。
/// 先试数组再试对象：新格式是 `{...}`，不会被 struct 的 seq 兼容读法误判。
fn parse_commands(text: &str) -> Result<CommandTree, String> {
    if let Ok(list) = serde_json::from_str::<Vec<String>>(text) {
        return Ok(CommandTree {
            commands: vec![CmdNode::Folder {
                id: "cf-commands".into(),
                name: "Commands".into(),
                expanded: true,
                children: list
                    .iter()
                    .enumerate()
                    .map(|(i, c)| command(&format!("c-legacy-{i}"), c))
                    .collect(),
            }],
        });
    }
    if let Ok(tree) = serde_json::from_str::<CommandTree>(text) {
        return Ok(tree);
    }
    Err("命令文件损坏".into())
}

pub fn load_commands() -> Result<CommandTree, String> {
    let path = commands_path()?;
    if !path.exists() {
        let tree = seed_commands();
        save_commands(&tree)?;
        return Ok(tree);
    }
    let text = std::fs::read_to_string(&path).map_err(|e| e.to_string())?;
    let tree = parse_commands(&text).map_err(|e| format!("{e}: {path:?}"))?;
    if text.trim_start().starts_with('[') {
        save_commands(&tree)?;
    }
    Ok(tree)
}

pub fn save_commands(tree: &CommandTree) -> Result<(), String> {
    let path = commands_path()?;
    let text = serde_json::to_string_pretty(tree).map_err(|e| e.to_string())?;
    std::fs::write(&path, text).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn leaves(nodes: &[CmdNode], out: &mut Vec<(String, String)>) {
        for n in nodes {
            match n {
                CmdNode::Folder { children, .. } => leaves(children, out),
                CmdNode::Command { name, text, .. } => out.push((name.clone(), text.clone())),
            }
        }
    }

    #[test]
    fn 旧版字符串数组会自动升级成命令文件夹() {
        let tree = parse_commands(r#"["df -h","free -m"]"#).unwrap();
        let mut got = vec![];
        leaves(&tree.commands, &mut got);
        assert_eq!(
            got,
            vec![
                ("df -h".into(), "df -h".into()),
                ("free -m".into(), "free -m".into())
            ]
        );
        assert!(matches!(&tree.commands[0], CmdNode::Folder { name, .. } if name == "Commands"));
    }

    #[test]
    fn 命令名与要发送的文本可以不同() {
        let json = serde_json::to_string(&CommandTree {
            commands: vec![CmdNode::Command {
                id: "c-1".into(),
                name: "磁盘".into(),
                text: "df -h".into(),
            }],
        })
        .unwrap();
        let tree = parse_commands(&json).unwrap();
        let mut got = vec![];
        leaves(&tree.commands, &mut got);
        assert_eq!(got, vec![("磁盘".into(), "df -h".into())]);
    }

    #[test]
    fn 空数组也是旧格式会被升级成空的命令文件夹() {
        let tree = parse_commands("[]").unwrap();
        assert!(matches!(
            &tree.commands[0],
            CmdNode::Folder { name, children, .. } if name == "Commands" && children.is_empty()
        ));
    }

    #[test]
    fn 损坏的命令文件会报错而不是清空() {
        assert!(parse_commands("{ not json").is_err());
    }

    #[cfg(windows)]
    #[test]
    fn 口令落盘为dpapi密文且能解回原文() {
        let encrypted = secret::protect("p@ss 中文").unwrap();
        assert!(secret::is_protected(&encrypted));
        assert_eq!(secret::unprotect(&encrypted), "p@ss 中文");
    }

    #[cfg(not(windows))]
    #[test]
    fn 非windows平台口令明文落盘() {
        assert_eq!(secret::protect("p@ss 中文").unwrap(), "p@ss 中文");
        assert!(!secret::is_protected(&secret::protect("x").unwrap()));
        assert_eq!(secret::unprotect("p@ss 中文"), "p@ss 中文");
    }

    #[test]
    fn 旧版明文口令原样读回() {
        assert!(!secret::is_protected("plain"));
        assert_eq!(secret::unprotect("plain"), "plain");
    }

    #[test]
    fn 解不开的密文按无口令处理() {
        assert_eq!(secret::unprotect("dpapi:zz"), "");
        assert_eq!(secret::unprotect("dpapi:00ff"), "");
    }

    #[test]
    fn 缺auth字段的旧会话读出默认认证列表() {
        let json = r#"{"id":"s","name":"n","host":"h","port":22,"username":"u"}"#;
        let s: SshSession = serde_json::from_str(json).unwrap();
        assert_eq!(s.auth, default_auth());
    }
}
