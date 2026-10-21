use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct SshSession {
    pub id: String,
    pub name: String,
    pub host: String,
    pub port: u16,
    pub username: String,
    #[serde(default)]
    pub password: String,
    /// OpenSSH 私钥文件路径；为空则不启用公钥认证
    #[serde(default)]
    pub key_path: String,
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

#[derive(Serialize, Deserialize, Default, Debug)]
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

fn node(id: &str, name: &str, host: &str) -> Node {
    Node::Session(SshSession {
        id: id.into(),
        name: name.into(),
        host: host.into(),
        port: 22,
        username: "root".into(),
        password: String::new(),
        key_path: String::new(),
    })
}

fn folder(id: &str, name: &str, children: Vec<Node>) -> Node {
    Node::Folder {
        id: id.into(),
        name: name.into(),
        expanded: true,
        children,
    }
}

fn seed() -> SessionTree {
    SessionTree {
        sessions: vec![
            folder("f-micro", "0. 微服务", vec![]),
            folder(
                "f-yibu",
                "1.一部",
                vec![
                    node("s-p1-1", "p1-1 116.62.54.209 master", "116.62.54.209"),
                    node("s-p1-2", "p1-2 121.43.50.248", "121.43.50.248"),
                    node("s-p1-3", "p1-3 118.178.254.139", "118.178.254.139"),
                    node("s-p1-4", "p1-4 116.62.33.129", "116.62.33.129"),
                ],
            ),
            folder(
                "f-erbu",
                "1.二部",
                vec![
                    node("s-p2-1", "p2-1 118.178.253.30 master", "118.178.253.30"),
                    node("s-p2-2", "p2-2 118.31.45.207", "118.31.45.207"),
                    node("s-p2-3", "p2-3 116.62.53.203", "116.62.53.203"),
                    node("s-p2-4", "p2-4 120.26.217.17", "120.26.217.17"),
                ],
            ),
            folder(
                "f-benbu",
                "1.本部",
                vec![
                    node("s-p0-1", "p0-1 121.43.37.183 master", "121.43.37.183"),
                    node("s-p0-2", "p0-2 120.27.137.197", "120.27.137.197"),
                    node("s-p0-3", "p0-3 118.31.112.157", "118.31.112.157"),
                    node("s-p0-4", "p0-4 121.43.36.71", "121.43.36.71"),
                ],
            ),
            folder("f-ali", "1.阿里云", vec![]),
            folder(
                "f-vps",
                "Vps",
                vec![
                    node("s-vps-1", "10.70.48.213", "10.70.48.213"),
                    node("s-vps-2", "10.70.49.165", "10.70.49.165"),
                    node("s-vps-3", "10.70.50.201", "10.70.50.201"),
                    node("s-ali-1", "101.37.166.120 阿里云", "101.37.166.120"),
                    node("s-hk-1", "119.28.25.90 香港", "119.28.25.90"),
                    node("s-vps-4", "49.235.101.10", "49.235.101.10"),
                    node("s-vps-5", "8.211.183.139", "8.211.183.139"),
                    node("s-bxz", "dbxz1-1.bx.com.cn", "dbxz1-1.bx.com.cn"),
                ],
            ),
            folder("f-xinyang", "信阳钢铁", vec![]),
            folder(
                "f-lan",
                "内网",
                vec![
                    node("s-lan-1", "10.70.34.73", "10.70.34.73"),
                    node("s-lan-2", "10.70.43.73", "10.70.43.73"),
                    node("s-lan-3", "10.70.50.177", "10.70.50.177"),
                    node("s-lan-4", "192.168.0.18", "192.168.0.18"),
                    node("s-lan-5", "192.168.0.198", "192.168.0.198"),
                    node("s-gitlab", "192.168.0.25 gitlab", "192.168.0.25"),
                    node("s-lan-6", "192.168.0.253", "192.168.0.253"),
                    node("s-lan-7", "192.168.0.93", "192.168.0.93"),
                ],
            ),
            folder("f-pre", "准生产", vec![]),
            folder("f-private", "私有化阿里云", vec![]),
            folder(
                "f-cang",
                "车场",
                vec![
                    node("s-c-1", "10.64.1.55", "10.64.1.55"),
                    node("s-c-2", "10.70.39.121", "10.70.39.121"),
                ],
            ),
        ],
    }
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
        save_tree(&seed())?;
    }
    let text = std::fs::read_to_string(&path).map_err(|e| e.to_string())?;
    serde_json::from_str(&text).map_err(|e| format!("会话文件损坏: {e}"))
}

pub fn save_tree(tree: &SessionTree) -> Result<(), String> {
    let path = store_path()?;
    let text = serde_json::to_string_pretty(tree).map_err(|e| e.to_string())?;
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
}
