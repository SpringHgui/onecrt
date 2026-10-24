/// 底部页签条（钉住工具窗口的宿主）挤不下文字时退成只显示图标。
/// 平分空间和省略号是纯 CSS 的（见 flexlayout.css），只有"份额够不够放 5 个汉字"这一步要量宽度，
/// CSS 没有把长度和长度相比的能力，所以留这一趟扫描。
/// 判据用 container.clientWidth ÷ 页签数，不看页签自己的实测宽度：换成图标后容器宽度不变，
/// 不会因为"隐藏文字→变宽→又显示文字"来回抖。

/// 文字之外的固定开销：图标 14px（不随字号缩放）+ 间距 .3em + 左右内边距 1em
const TAB_CHROME_EM = 1.3;

const fitStrip = (outer: HTMLElement) => {
  const container = outer.querySelector<HTMLElement>(".flexlayout__tabset_tabbar_inner_tab_container");
  if (!container) return;
  const tabs = [...container.querySelectorAll<HTMLElement>(".flexlayout__tab_button")];
  const size = parseFloat(getComputedStyle(outer).fontSize) || 13;
  // 份额连"5 个汉字 + 图标"都放不下时，截成 "Se…" 之类既读不出词又占地方，不如只留图标
  const narrow = tabs.length > 0 && container.clientWidth / tabs.length < size * (5 + TAB_CHROME_EM) + 14;
  outer.classList.toggle("dock_strip--icon", narrow);
  for (const t of tabs) {
    // 文字被藏起来后，名字只能靠悬停提示；库把页签名写进了 aria-label
    const label = t.getAttribute("aria-label") ?? "";
    if (!label) continue;
    if (narrow) t.title = label;
    else if (t.title === label) t.removeAttribute("title");
  }
};

/// 盯着所有底部页签条：窗口/分隔条改宽度、页签增删、宿主新建或拆除都要重算。
/// 页签条这个 DOM 不归应用渲染（浮窗并回、拖出新栏时机会改写它），所以按 DOM 现状扫，
/// 而不是跟着 model 动作走——Model 是同一个引用，挂 useEffect([model]) 根本不会重跑。
export function startDockStripFit(): () => void {
  const strips = new Set<HTMLElement>();

  const resync = () => {
    for (const el of document.querySelectorAll<HTMLElement>(".flexlayout__tabset_tabbar_outer_bottom")) {
      if (!strips.has(el)) {
        strips.add(el);
        observer.observe(el);
      }
    }
    for (const el of strips) {
      if (el.isConnected) continue;
      strips.delete(el);
      observer.unobserve(el);
    }
  };

  let queued = false;
  const schedule = () => {
    if (queued) return;
    queued = true;
    requestAnimationFrame(() => {
      queued = false;
      resync();
      for (const el of strips) fitStrip(el);
    });
  };

  const observer = new ResizeObserver(schedule);
  // 页签条是库渲染的 DOM，增删没有回调可挂；终端输出会频繁改子树，所以扫描按帧合并
  const mutations = new MutationObserver(schedule);
  schedule();
  mutations.observe(document.body, { childList: true, subtree: true });

  return () => {
    observer.disconnect();
    mutations.disconnect();
    strips.clear();
  };
}
