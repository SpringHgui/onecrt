export interface SshSession {
  id: string;
  name: string;
  host: string;
  port: number;
  username: string;
  password: string;
  keyPath: string;
  auth?: AuthMethod[];
}

/** 认证列表的一项：数组顺序即连接时的尝试顺序 */
export interface AuthMethod {
  method: string;
  enabled: boolean;
}

export const AUTH_METHODS = ["PublicKey", "Password", "Keyboard Interactive"];

export function defaultAuth(): AuthMethod[] {
  return AUTH_METHODS.map((method) => ({ method, enabled: true }));
}

export function enabledMethods(auth: AuthMethod[] | undefined): string[] {
  return (auth?.length ? auth : defaultAuth()).filter((a) => a.enabled).map((a) => a.method);
}

export interface Folder {
  kind: "folder";
  id: string;
  name: string;
  expanded: boolean;
  children: Node[];
}

export type Node = Folder | ({ kind: "session" } & SshSession);

export function isFolder(n: Node): n is Folder {
  return n.kind === "folder";
}

export interface SavedCommand {
  kind: "command";
  id: string;
  name: string;
  /** 实际发往终端的文本，不含结尾换行 */
  text: string;
}

export interface CommandFolder {
  kind: "folder";
  id: string;
  name: string;
  expanded: boolean;
  children: CommandNode[];
}

export type CommandNode = CommandFolder | SavedCommand;

export function isCommandFolder(n: CommandNode): n is CommandFolder {
  return n.kind === "folder";
}

export function newId(prefix: string): string {
  return `${prefix}-${Date.now().toString(36)}-${Math.random().toString(36).slice(2, 8)}`;
}
