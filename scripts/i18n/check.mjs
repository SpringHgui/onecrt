// 静态核对：zh/en 词典键一一对应；面板页签名取自词典、且随当前语言现取。
// CI 的 frontend job 跑它（切语言漏文案这类回归在这里最便宜）。
// 跑法：node scripts/i18n/check.mjs
import { readFileSync } from "node:fs";
import { registerHooks } from "node:module";

// 项目里模块互相 import 不写扩展名（Vite 允许），node 直跑要自己补上 .ts
registerHooks({
  resolve(specifier, context, next) {
    if (specifier.startsWith(".") && !/\.[a-z]+$/.test(specifier)) return next(specifier + ".ts", context);
    return next(specifier, context);
  },
});

const store = {};
global.localStorage = { getItem: (k) => (k in store ? store[k] : null), setItem: (k, v) => { store[k] = String(v); } };
Object.defineProperty(global, "navigator", { value: { language: "zh-CN" }, configurable: true });
global.document = { documentElement: { lang: "" } };

const src = readFileSync(new URL("../../src/i18n.ts", import.meta.url), "utf8");
const block = (name) => {
  const start = src.indexOf(`const ${name}: Dict = {`);
  const end = src.indexOf("\n};", start);
  return [...src.slice(start, end).matchAll(/^\s*"((?:[^"\\]|\\.)+)":\s*"/gm)].map((m) => m[1]);
};
const zh = block("zh");
const en = block("en");

const { setLocale, t, LOCALES, tLocale } = await import("../../src/i18n.ts");
const lay = await import("../../src/layout.ts");

const R = [];
const check = (n, c, e = "") => R.push([c ? "PASS" : "FAIL", n, e]);

check("zh 无重复键", new Set(zh).size === zh.length, zh.filter((k, i) => zh.indexOf(k) !== i).join(","));
check("en 无重复键", new Set(en).size === en.length, en.filter((k, i) => en.indexOf(k) !== i).join(","));
check("zh 有 en 没有的键为空", zh.filter((k) => !en.includes(k)).length === 0, zh.filter((k) => !en.includes(k)).join(","));
check("en 有 zh 没有的键为空", en.filter((k) => !zh.includes(k)).length === 0, en.filter((k) => !zh.includes(k)).join(","));

const panelKeys = Object.entries(lay.PANEL_TAB_KEY);
check("PANEL_TAB_KEY 覆盖 5 个面板", panelKeys.length === 5, `got=${panelKeys.length}`);
for (const [, key] of panelKeys) for (const l of LOCALES) check(`${key} 在 ${l} 有文案`, tLocale(l, key) !== undefined);

// 出厂布局的 tab 名 = 当前语言文案；panelTabJson（视图菜单重开面板走它）必须现取而不是用模块求值时的旧名
const tabNames = (m) =>
  Object.fromEntries(
    (m.layout?.children ?? []).flatMap((ts) => (ts.children ?? []).map((tab) => [tab.component, tab.name]))
  );
setLocale("zh");
const zhDefault = tabNames(lay.defaultLayout());
setLocale("en");
const enDefault = tabNames(lay.defaultLayout());
for (const [id, key] of panelKeys) {
  if (zhDefault[id] !== undefined) {
    setLocale("zh");
    check(`出厂布局 ${id} 用 zh 文案`, zhDefault[id] === t(key), `got=${zhDefault[id]}`);
    setLocale("en");
    check(`出厂布局 ${id} 用 en 文案`, enDefault[id] === t(key), `got=${enDefault[id]}`);
  }
  setLocale("zh");
  check(`panelTabJson(${id}) 跟随 zh`, lay.panelTabJson(id).name === t(key), `got=${lay.panelTabJson(id).name}`);
  setLocale("en");
  check(`panelTabJson(${id}) 跟随 en`, lay.panelTabJson(id).name === t(key), `got=${lay.panelTabJson(id).name}`);
}

let f = 0;
for (const [s, n, e] of R) { if (s === "FAIL") f++; console.log(`${s}  ${n}${e ? "  (" + e + ")" : ""}`); }
console.log(`\n${R.length - f}/${R.length} passed`);
process.exit(f ? 1 : 0);
