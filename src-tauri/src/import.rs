//! 从外部客户端的配置里提取 SSH2 会话，转成本项目的会话树节点。
//!
//! SecureCRT 把会话组织成 `<key name="Sessions">` 下的嵌套 `<key>`：
//! 含 `<string name="Hostname">` 的 key 是一条会话，只含子 `<key>` 的是文件夹。
//!
//! WinSCP 把会话存成 INI 段落 `[Sessions\组/会话名]`，段名里的 `/` 是分组层级。
//!
//! 两者导出的配置都没有可用的明文口令（SecureCRT 压根不导出，WinSCP 存的是自家
//! 混淆密文），所以口令一律留空——连接时会弹框补凭据，与手动新建会话的行为一致。

use crate::sessions::{default_auth, Node, SshSession};
use roxmltree::Node as XNode;

/// 取直接子级里 `<string name="...">` 的文本
fn child_string<'a>(node: XNode<'a, '_>, name: &str) -> Option<String> {
    node.children()
        .find(|c| c.has_tag_name("string") && c.attribute("name") == Some(name))
        .map(|c| c.text().unwrap_or("").trim().to_string())
}

/// 取直接子级里 `<dword name="...">` 的整数值
fn child_dword<'a>(node: XNode<'a, '_>, name: &str) -> Option<u32> {
    node.children()
        .find(|c| c.has_tag_name("dword") && c.attribute("name") == Some(name))
        .and_then(|c| c.text())
        .and_then(|t| t.trim().parse().ok())
}

/// 递归把一个 `<key>` 下的子 key 转成节点，追加到 out
fn convert(node: XNode, out: &mut Vec<Node>) {
    for child in node.children().filter(|c| c.has_tag_name("key")) {
        let key_name = child.attribute("name").unwrap_or("").trim().to_string();

        // 会话判定：有非空 Hostname。SecureCRT 导出的会话协议全是 SSH2，
        // 端口取 `[SSH2] Port`，缺省 22。
        let host = child_string(child, "Hostname").unwrap_or_default();
        if !host.is_empty() {
            let port = child_dword(child, "[SSH2] Port").unwrap_or(22).clamp(1, 65535) as u16;
            out.push(Node::Session(SshSession {
                // id 留空，由前端合并进树时统一分配，避免和现有会话撞号
                id: String::new(),
                name: if key_name.is_empty() { host.clone() } else { key_name },
                host,
                port,
                username: child_string(child, "Username").unwrap_or_default(),
                password: String::new(),
                key_path: child_string(child, "Identity Filename V2").unwrap_or_default(),
                auth: default_auth(),
            }));
            continue;
        }

        // 否则当文件夹：先递归收集子节点
        let mut children = Vec::new();
        convert(child, &mut children);
        // SecureCRT 的 "Default"/"Default Settings" 是新会话继承的模板（Hostname 为空、
        // 无子节点），不是真实会话，跳过；用户自建的空文件夹仍保留。
        if children.is_empty() && (key_name == "Default" || key_name == "Default Settings") {
            continue;
        }
        out.push(Node::Folder {
            id: String::new(),
            name: key_name,
            expanded: true,
            children,
        });
    }
}

/// 解析 SecureCRT 导出的 XML，返回会话树顶层节点（文件夹/会话混合）。
/// 找不到 Sessions 节点或 XML 非法时报错。
pub fn parse_securecrt(xml: &str) -> Result<Vec<Node>, String> {
    let doc = roxmltree::Document::parse(xml).map_err(|e| format!("XML 解析失败: {e}"))?;
    let sessions = doc
        .root_element()
        .children()
        .find(|c| c.has_tag_name("key") && c.attribute("name") == Some("Sessions"))
        .ok_or_else(|| "文件里没有 Sessions 节点，可能不是 SecureCRT 配置".to_string())?;
    let mut out = Vec::new();
    convert(sessions, &mut out);
    Ok(out)
}

/// WinSCP 的段名去掉 `[` `]` 后拆成层级路径：紧跟 `Sessions` 的那道 `\` 是 INI 的
/// 段落分隔，往下的分组层级用的是 `/`（真实配置里 `Sessions\Tencent/root@host`）。
fn winscp_path(inner: &str) -> Vec<String> {
    inner
        .split(|c| c == '\\' || c == '/')
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .collect()
}

/// 只有 `[Sessions\...]`（至少再带一段名字）才是会话或分组段落
fn is_winscp_session_path(p: &[String]) -> bool {
    p.len() >= 2 && p[0].eq_ignore_ascii_case("Sessions")
}

/// WinSCP 的 INI 值会把 `\` 等字符写成 `%5C`，还原回原文
fn winscp_unescape(value: &str) -> String {
    let bytes = value.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            let hex = &bytes[i + 1..i + 3];
            if let Ok(b) = u8::from_str_radix(std::str::from_utf8(hex).unwrap_or(""), 16) {
                out.push(b);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// 把会话挂到 groups 指定的文件夹层级下，缺的文件夹按文件出现顺序补建
fn insert_session(nodes: &mut Vec<Node>, groups: &[String], session: Node) {
    let Some((head, rest)) = groups.split_first() else {
        nodes.push(session);
        return;
    };
    let idx = match nodes.iter().position(|n| matches!(n, Node::Folder { name, .. } if name == head)) {
        Some(i) => i,
        None => {
            nodes.push(Node::Folder {
                id: String::new(),
                name: head.clone(),
                expanded: true,
                children: Vec::new(),
            });
            nodes.len() - 1
        }
    };
    if let Node::Folder { children, .. } = &mut nodes[idx] {
        insert_session(children, rest, session);
    }
}

/// 解析 WinSCP 的配置存储（WinSCP.ini），返回会话树顶层节点。
/// 段落结构不像 WinSCP 时报错；像 WinSCP 但没有可连会话时返回空列表。
pub fn parse_winscp(text: &str) -> Result<Vec<Node>, String> {
    let mut sections: Vec<(Vec<String>, Vec<(String, String)>)> = Vec::new();
    let mut path: Option<Vec<String>> = None;
    let mut kv: Vec<(String, String)> = Vec::new();
    for raw in text.trim_start_matches('\u{feff}').lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with(';') {
            continue;
        }
        if line.starts_with('[') && line.ends_with(']') {
            if let Some(p) = path.take() {
                sections.push((p, std::mem::take(&mut kv)));
            }
            path = Some(winscp_path(&line[1..line.len() - 1]));
        } else if let Some((k, v)) = line.split_once('=') {
            if path.is_some() {
                kv.push((k.trim().to_string(), v.trim().to_string()));
            }
        }
    }
    if let Some(p) = path {
        sections.push((p, kv));
    }

    if !sections.iter().any(|(p, _)| is_winscp_session_path(p)) {
        return Err("文件里没有 [Sessions\\...] 段落，可能不是 WinSCP 配置".to_string());
    }

    let mut out = Vec::new();
    for (p, kv) in sections {
        if !is_winscp_session_path(&p) {
            continue;
        }
        let get = |key: &str| {
            kv.iter()
                .find(|(k, _)| k.eq_ignore_ascii_case(key))
                .map(|(_, v)| v.clone())
        };
        // 没有 HostName 的是分组段落或 WinSCP 的占位段落，不是可连会话
        let host = get("HostName").map(|h| winscp_unescape(&h)).unwrap_or_default();
        if host.is_empty() {
            continue;
        }
        let name = p.last().cloned().unwrap_or_default();
        let groups = &p[1..p.len() - 1]; // 去掉首尾（Sessions 与会话名），中间是分组层级
        insert_session(
            &mut out,
            groups,
            Node::Session(SshSession {
                id: String::new(),
                name: if name.is_empty() { host.clone() } else { name },
                host,
                // WinSCP 缺省端口 22，端口写在 PortNumber（如自建端口的 VPS）
                port: get("PortNumber")
                    .and_then(|v| v.parse::<u32>().ok())
                    .unwrap_or(22)
                    .clamp(1, 65535) as u16,
                username: get("UserName").map(|u| winscp_unescape(&u)).unwrap_or_default(),
                password: String::new(),
                key_path: String::new(),
                auth: default_auth(),
            }),
        );
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn session_count(nodes: &[Node]) -> usize {
        nodes.iter().map(|n| match n {
            Node::Folder { children, .. } => session_count(children),
            Node::Session(_) => 1,
        }).sum()
    }

    const SAMPLE: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<VanDyke version="3.0">
  <key name="Global"><string name="x">y</string></key>
  <key name="Sessions">
    <key name="生产">
      <key name="web 10.0.0.1">
        <dword name="[SSH2] Port">2222</dword>
        <string name="Hostname">10.0.0.1</string>
        <string name="Username">root</string>
        <string name="Protocol Name">SSH2</string>
        <string name="Identity Filename V2"/>
      </key>
      <key name="db 10.0.0.2">
        <string name="Hostname">10.0.0.2</string>
        <string name="Username">admin</string>
      </key>
    </key>
    <key name="散机 10.0.0.9">
      <string name="Hostname">10.0.0.9</string>
    </key>
  </key>
</VanDyke>"#;

    #[test]
    fn 解析出文件夹与嵌套会话() {
        let nodes = parse_securecrt(SAMPLE).unwrap();
        assert_eq!(session_count(&nodes), 3);
        // 顶层：一个"生产"文件夹 + 一条散机会话
        assert_eq!(nodes.len(), 2);
        match &nodes[0] {
            Node::Folder { name, children, expanded, .. } => {
                assert_eq!(name, "生产");
                assert!(*expanded);
                assert_eq!(children.len(), 2);
            }
            _ => panic!("expected folder"),
        }
    }

    #[test]
    fn 端口与用户名正确映射_缺省端口回落22() {
        let nodes = parse_securecrt(SAMPLE).unwrap();
        let folder = match &nodes[0] { Node::Folder { children, .. } => children, _ => unreachable!() };
        let web = match &folder[0] { Node::Session(s) => s, _ => unreachable!() };
        assert_eq!(web.name, "web 10.0.0.1");
        assert_eq!(web.host, "10.0.0.1");
        assert_eq!(web.port, 2222);
        assert_eq!(web.username, "root");
        // db 没有 [SSH2] Port，回落 22
        let db = match &folder[1] { Node::Session(s) => s, _ => unreachable!() };
        assert_eq!(db.port, 22);
        assert_eq!(db.username, "admin");
    }

    #[test]
    fn 口令一律留空_id留空待前端分配() {
        let nodes = parse_securecrt(SAMPLE).unwrap();
        let mut ids = vec![];
        fn walk(nodes: &[Node], ids: &mut Vec<(String, String)>) {
            for n in nodes {
                match n {
                    Node::Folder { id, children, .. } => { ids.push((id.clone(), String::new())); walk(children, ids); }
                    Node::Session(s) => ids.push((s.id.clone(), s.password.clone())),
                }
            }
        }
        walk(&nodes, &mut ids);
        assert!(ids.iter().all(|(id, pwd)| id.is_empty() && pwd.is_empty()));
    }

    #[test]
    fn 无sessions节点报错() {
        let xml = r#"<VanDyke><key name="Global"></key></VanDyke>"#;
        assert!(parse_securecrt(xml).is_err());
    }

    #[test]
    fn 非法xml报错() {
        assert!(parse_securecrt("<not closed").is_err());
    }

    #[test]
    fn 没有hostname的空文件夹也能保留() {
        let xml = r#"<VanDyke><key name="Sessions"><key name="空组"></key></key></VanDyke>"#;
        let nodes = parse_securecrt(xml).unwrap();
        assert_eq!(nodes.len(), 1);
        assert!(matches!(&nodes[0], Node::Folder { name, children, .. } if name == "空组" && children.is_empty()));
    }

    #[test]
    fn default模板会话被跳过_不产生游离空文件夹() {
        let xml = r#"<VanDyke><key name="Sessions">
          <key name="Default"><string name="Hostname"/><string name="Username">root</string></key>
          <key name="真机"><string name="Hostname">1.2.3.4</string></key>
        </key></VanDyke>"#;
        let nodes = parse_securecrt(xml).unwrap();
        assert_eq!(nodes.len(), 1);
        assert!(matches!(&nodes[0], Node::Session(s) if s.host == "1.2.3.4"));
    }

    // 结构照真实的 WinSCP.ini：段名里 `/` 是分组层级，值里 `\` 转义成 %5C
    const WINSCP: &str = r#"; Stored by WinSCP
[Configuration]
CheckLDM=0

[Sessions]
NumberOfSessions=0

[Sessions\root@203.0.113.10]
HostName=203.0.113.10
UserName=root
LocalDirectory=C:%5CUsers%5Cme%5CDocuments

[Sessions\Tencent/web 203.0.113.20]
HostName=203.0.113.20
UserName=deploy
PortNumber=2222

[Sessions\Tencent\深圳/db 203.0.113.30]
HostName=203.0.113.30
UserName=root

[Sessions\只有分组的段落]
RemoteDirectory=/tmp
"#;

    #[test]
    fn winscp_按段名还原分组层级() {
        let nodes = parse_winscp(WINSCP).unwrap();
        assert_eq!(session_count(&nodes), 3);
        // 顶层：散机会话 + Tencent 文件夹（保持文件里的出现顺序）
        assert_eq!(nodes.len(), 2);
        assert!(matches!(&nodes[0], Node::Session(s) if s.name == "root@203.0.113.10"));
        match &nodes[1] {
            Node::Folder { name, children, expanded, .. } => {
                assert_eq!(name, "Tencent");
                assert!(*expanded);
                assert_eq!(children.len(), 2);
                match &children[1] {
                    Node::Folder { name, children, .. } => {
                        assert_eq!(name, "深圳");
                        assert!(matches!(&children[0], Node::Session(s) if s.host == "203.0.113.30"));
                    }
                    _ => panic!("expected nested folder"),
                }
            }
            _ => panic!("expected folder"),
        }
    }

    #[test]
    fn winscp_端口与用户名映射_缺省端口22_转义值还原() {
        let nodes = parse_winscp(WINSCP).unwrap();
        let flat = match &nodes[0] { Node::Session(s) => s, _ => unreachable!() };
        assert_eq!(flat.port, 22);
        assert_eq!(flat.username, "root");
        let web = match &nodes[1] {
            Node::Folder { children, .. } => match &children[0] { Node::Session(s) => s, _ => unreachable!() },
            _ => unreachable!(),
        };
        assert_eq!(web.port, 2222);
        assert_eq!(web.username, "deploy");

        let ini = "[Sessions\\dc]\nHostName=192.0.2.1\nUserName=CORP%5Cadmin\nPortNumber=0\n";
        let nodes = parse_winscp(ini).unwrap();
        let s = match &nodes[0] { Node::Session(s) => s, _ => unreachable!() };
        assert_eq!(s.username, r"CORP\admin");
        // 非法端口收进 1..=65535
        assert_eq!(s.port, 1);
    }

    #[test]
    fn winscp_口令与id留空_密文口令不落地() {
        let ini = "[Sessions\\a]\nHostName=192.0.2.1\nUserName=root\nPassword=A35C40501EB1E9A6\n";
        let nodes = parse_winscp(ini).unwrap();
        let s = match &nodes[0] { Node::Session(s) => s, _ => unreachable!() };
        assert!(s.id.is_empty() && s.password.is_empty() && s.key_path.is_empty());
    }

    #[test]
    fn winscp_没有sessions段落报错() {
        assert!(parse_winscp("[Configuration]\nCheckLDM=0\n").is_err());
        // 光有 [Sessions] 汇总段落（没有分组/会话层级）也不算 WinSCP 会话存储
        assert!(parse_winscp("[Sessions]\nNumberOfSessions=0\n").is_err());
    }
}
