import { createRoot } from "react-dom/client";
import App from "./App";
import "flexlayout-react/style/dark.css";
import "./flexlayout.css";
import "./styles.css";

// 不用 StrictMode：其双重挂载会重建 xterm 实例，导致终端内容丢失
createRoot(document.getElementById("root")!).render(<App />);
