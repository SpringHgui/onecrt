import { IconPin, IconPinFilled } from "@tabler/icons-react";
import { ink } from "./icons";
import { useI18n } from "./i18n";

/// 工具窗口标题栏右上角的图钉：钉住=常驻（边 tabset），取消=自动隐藏（收进边条）。
/// 放在面板自己的标题栏里而不是边缘条上，跟 VS 的工具窗口一致。
export default function PanelPin({ pinned, onToggle }: { pinned: boolean; onToggle(): void }) {
  const { t } = useI18n();
  return (
    <button
      className="panel-pin"
      title={pinned ? t("pin.unpinTitle") : t("pin.pinTitle")}
      aria-label={pinned ? t("pin.unpin") : t("pin.pin")}
      aria-pressed={pinned}
      onClick={onToggle}
      onDragStart={(e) => {
        // 标题栏是拖拽把手，但拽图钉不该拖动整个面板
        e.preventDefault();
        e.stopPropagation();
      }}
    >
      {pinned ? <IconPinFilled {...ink(14)} /> : <IconPin {...ink(14)} />}
    </button>
  );
}
