// 参考界面（VS Code 系）图标墨迹实测约 1.5 逻辑像素线宽。
// Tabler 的 stroke-width 以 24 网格计，视觉线宽 = stroke × size ÷ 24，故 stroke 取 36 ÷ size 可让各尺寸恒为 1.5px。
export const ink = (size: number) => ({ size, stroke: Math.round((36 / size) * 100) / 100 });
