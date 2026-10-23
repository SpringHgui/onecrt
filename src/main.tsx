import { createRoot } from "react-dom/client";
import App from "./App";
import "flexlayout-react/style/dark.css";
import "./flexlayout.css";
import "./styles.css";

// WebView2 的原生右键菜单里带"刷新(Reload)"，一刷新整个前端重载、后端还活着的 SSH 连接
// 就全丢了（布局里的终端标签本来就会被 pruneTerminals 清掉）。全局拦掉这个菜单；
// 可编辑输入框例外，它的原生菜单只有剪切/复制/粘贴，没有刷新，留着方便粘主机名/口令。
window.addEventListener("contextmenu", (e) => {
  const t = e.target as HTMLElement | null;
  if (t && (t.isContentEditable || t.tagName === "INPUT" || t.tagName === "TEXTAREA")) return;
  e.preventDefault();
});

// 滚动条三态靠 html 上这个类切换（见 styles.css）：指针在窗口内才画滑块。
// 判据只能是事件类型——进出窗口时 relatedTarget 都是 null，区分不了。
// 这里不能用 capture：mouseenter/mouseleave 不冒泡，但捕获阶段会收到子元素的同名事件。
document.addEventListener("mouseenter", () => document.documentElement.classList.add("app-mouse-in"));
document.addEventListener("mouseleave", () => document.documentElement.classList.remove("app-mouse-in"));

// 不用 StrictMode：其双重挂载会重建 xterm 实例，导致终端内容丢失
createRoot(document.getElementById("root")!).render(<App />);
