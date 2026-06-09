// 共享演示主题目录 —— 与 polaris-deck-studio skill 的 assets/themes.css 一一对应。
// DeckStudio.vue（PPT / 网页幻灯片）与 VideoCourseStudio.vue 共用，保证 UI 预览与
// 实际生成的 deck 视觉一致。id == themes.css 里的 [data-theme] 取值。

export type DeckThemeGroup = "浅色" | "深色" | "特色" | "智能";

export interface DeckTheme {
  id: string;
  name: string;
  group: DeckThemeGroup;
  /** 背景色（或渐变），用于预览底板 */
  bg: string;
  /** 强调色，用于预览圆点 / 副色块 */
  accent: string;
  /** 文字色，用于预览上的示意字 */
  text: string;
  /** 深色主题：预览角标与边框做相应处理 */
  dark?: boolean;
  /** 衬线 / 等宽等特殊字体族，预览示意字用 */
  font?: "serif" | "mono";
}

// 「AI 自由发挥」：不对应具体 CSS 主题，让模型按内容自挑。
export const THEME_AUTO: DeckTheme = {
  id: "auto",
  name: "AI 自由发挥",
  group: "智能",
  bg: "linear-gradient(135deg,#6366f1,#ec4899)",
  accent: "#ffffff",
  text: "#ffffff",
};

export const DECK_THEMES: DeckTheme[] = [
  // ── 浅色 ──
  { id: "minimal-white", name: "极简白", group: "浅色", bg: "#ffffff", accent: "#3b6cff", text: "#111216" },
  { id: "editorial-serif", name: "杂志衬线", group: "浅色", bg: "#faf7f2", accent: "#8a2a1c", text: "#1b1410", font: "serif" },
  { id: "swiss-grid", name: "瑞士网格", group: "浅色", bg: "#ffffff", accent: "#e0254b", text: "#0a0a0a" },
  { id: "magazine-bold", name: "大刊浓墨", group: "浅色", bg: "#f5f1e6", accent: "#e63946", text: "#161210", font: "serif" },
  { id: "japanese-minimal", name: "和风留白", group: "浅色", bg: "#f7f5f0", accent: "#b04a3a", text: "#1c1a16", font: "serif" },
  { id: "xiaohongshu-white", name: "小红书白", group: "浅色", bg: "#fffdfd", accent: "#ff2e4d", text: "#1f1418" },
  { id: "academic-paper", name: "学术白", group: "浅色", bg: "#fbfbf9", accent: "#1a3c8a", text: "#141826", font: "serif" },
  { id: "corporate-clean", name: "商务简净", group: "浅色", bg: "#ffffff", accent: "#2563eb", text: "#0e1726" },
  { id: "soft-pastel", name: "柔彩梦", group: "浅色", bg: "#fdf6fb", accent: "#c084fc", text: "#2a2030" },
  { id: "arctic-cool", name: "极地冷调", group: "浅色", bg: "#f3f8fc", accent: "#2b7fff", text: "#0f2233" },
  { id: "bauhaus", name: "包豪斯", group: "浅色", bg: "#f4f0e6", accent: "#e63329", text: "#1a1a1a" },
  { id: "catppuccin-latte", name: "拿铁拿铁", group: "浅色", bg: "#eff1f5", accent: "#1e66f5", text: "#4c4f69" },
  { id: "engineering-whiteprint", name: "白底工程图", group: "浅色", bg: "#f7f9fb", accent: "#0067c0", text: "#14202b", font: "mono" },
  { id: "midcentury", name: "中世纪现代", group: "浅色", bg: "#f3ece0", accent: "#d9603b", text: "#2a2622" },
  { id: "news-broadcast", name: "新闻播报", group: "浅色", bg: "#ffffff", accent: "#c0152f", text: "#0a1c2e" },
  { id: "sharp-mono", name: "锐利黑白", group: "浅色", bg: "#ffffff", accent: "#111111", text: "#111111", font: "mono" },
  { id: "solarized-light", name: "晒版浅", group: "浅色", bg: "#fdf6e3", accent: "#268bd2", text: "#586e75" },
  { id: "sunset-warm", name: "暖阳落日", group: "浅色", bg: "#fff5ec", accent: "#ff6b4a", text: "#2b1d18" },
  // ── 深色 ──
  { id: "tokyo-night", name: "东京夜", group: "深色", bg: "#1a1b26", accent: "#7aa2f7", text: "#c0caf5", dark: true },
  { id: "dracula", name: "德古拉", group: "深色", bg: "#282a36", accent: "#bd93f9", text: "#f8f8f2", dark: true },
  { id: "nord", name: "极地夜", group: "深色", bg: "#2e3440", accent: "#88c0d0", text: "#eceff4", dark: true },
  { id: "cyberpunk-neon", name: "赛博霓虹", group: "深色", bg: "#0a0a14", accent: "#ff00a0", text: "#eafcff", dark: true, font: "mono" },
  { id: "terminal-green", name: "终端绿", group: "深色", bg: "#0b0f0b", accent: "#33ff88", text: "#caffd9", dark: true, font: "mono" },
  { id: "blueprint", name: "工程蓝图", group: "深色", bg: "#0d1b2a", accent: "#00b4d8", text: "#e6f4fb", dark: true, font: "mono" },
  { id: "aurora", name: "极光", group: "深色", bg: "#0b1020", accent: "#6ee7b7", text: "#e6f0ff", dark: true },
  { id: "catppuccin-mocha", name: "摩卡摩卡", group: "深色", bg: "#1e1e2e", accent: "#cba6f7", text: "#cdd6f4", dark: true },
  { id: "gruvbox-dark", name: "Gruvbox 暗", group: "深色", bg: "#282828", accent: "#fabd2f", text: "#ebdbb2", dark: true, font: "mono" },
  { id: "pitch-deck-vc", name: "融资路演", group: "深色", bg: "#0e1116", accent: "#5b8cff", text: "#e9edf5", dark: true },
  { id: "retro-tv", name: "复古显像管", group: "深色", bg: "#14131a", accent: "#ff5e5e", text: "#f0e9d2", dark: true, font: "mono" },
  { id: "rose-pine", name: "玫瑰松", group: "深色", bg: "#191724", accent: "#ebbcba", text: "#e0def4", dark: true },
  // ── 特色 ──
  { id: "glassmorphism", name: "毛玻璃", group: "特色", bg: "linear-gradient(135deg,#3b2d6b,#6b2d52 60%,#0f1226)", accent: "#818cf8", text: "#f5f7ff", dark: true },
  { id: "neo-brutalism", name: "新粗野", group: "特色", bg: "#fffef0", accent: "#ff5c00", text: "#111111" },
  { id: "memphis-pop", name: "孟菲斯波普", group: "特色", bg: "#fffbe6", accent: "#ff3d7f", text: "#1a1a1a" },
  { id: "rainbow-gradient", name: "彩虹渐变", group: "特色", bg: "linear-gradient(135deg,#ff5c8a,#ffb15c 45%,#5cd0ff)", accent: "#7a2cff", text: "#1a1226" },
  { id: "vaporwave", name: "蒸汽波", group: "特色", bg: "linear-gradient(135deg,#2a0b4e,#1a0b2e)", accent: "#ff71ce", text: "#f5e9ff", dark: true },
  { id: "y2k-chrome", name: "千禧铬金", group: "特色", bg: "linear-gradient(135deg,#e9eef5,#cfd8e6)", accent: "#8a5cf6", text: "#1a1a2a" },
];

/** 含「AI 自由发挥」的完整可选列表（供选择器渲染） */
export const DECK_THEMES_WITH_AUTO: DeckTheme[] = [THEME_AUTO, ...DECK_THEMES];

export function findTheme(id: string): DeckTheme {
  return DECK_THEMES_WITH_AUTO.find((t) => t.id === id) ?? DECK_THEMES[0];
}

export function themeName(id: string): string {
  return findTheme(id).name;
}

/** 按分组归并（保持声明顺序），供分组渲染主题网格 */
export function groupedThemes(includeAuto = true): { group: DeckThemeGroup; items: DeckTheme[] }[] {
  const list = includeAuto ? DECK_THEMES_WITH_AUTO : DECK_THEMES;
  const order: DeckThemeGroup[] = ["智能", "浅色", "深色", "特色"];
  return order
    .map((group) => ({ group, items: list.filter((t) => t.group === group) }))
    .filter((g) => g.items.length > 0);
}
