export interface SshSession {
  id: string;
  name: string;
  host: string;
  port: number;
  username: string;
  password: string;
  keyPath: string;
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
