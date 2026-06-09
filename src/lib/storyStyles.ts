// 故事视频「美术风格」库
// ──────────────────────────────────────────────────────────────
// 与 deckThemes（PPT 的 CSS 主题）不同：这里每套风格是一段**拼进生图 prompt 的风格后缀**，
// 写进 storyboard.style，使 MiniMax image-01 全片画风统一。
// bg/accent 仅用于前端预览色卡，不参与生图。

export type StoryStyleGroup = "写实" | "动画" | "插画" | "国风" | "特色" | "智能";

export interface StoryStyle {
  id: string;
  name: string;
  group: StoryStyleGroup;
  bg: string; // 预览背景（纯色或渐变）
  accent: string; // 预览强调色
  dark?: boolean;
  /** 拼进每条生图 prompt 的风格后缀（中文，越具体越稳定） */
  prompt: string;
}

// AI 自由发挥（让模型按故事内容自选最合适画风）
export const STORY_STYLE_AUTO: StoryStyle = {
  id: "auto",
  name: "AI 自由发挥",
  group: "智能",
  bg: "linear-gradient(135deg,#6366f1,#ec4899)",
  accent: "#ffffff",
  dark: true,
  prompt: "",
};

export const STORY_STYLES: StoryStyle[] = [
  // ── 写实 ──
  {
    id: "cinematic-real",
    name: "电影写实",
    group: "写实",
    bg: "linear-gradient(135deg,#1c2230,#3a2f44)",
    accent: "#e6b980",
    dark: true,
    prompt:
      "电影感写实风格，戏剧性光影，浅景深，胶片质感，富有氛围的环境光，高细节，35mm 镜头，电影级调色",
  },
  {
    id: "documentary-photo",
    name: "纪实摄影",
    group: "写实",
    bg: "#2b2b2b",
    accent: "#cfcfcf",
    dark: true,
    prompt: "纪实摄影风格，自然真实光线，真实肌理与材质，临场感，高分辨率照片质感",
  },
  {
    id: "dark-quote",
    name: "黑金语录",
    group: "写实",
    bg: "#0b0b0d",
    accent: "#d4af37",
    dark: true,
    prompt:
      "极简暗调氛围画面，纯黑或深色背景，单一主体，强烈明暗对比与轮廓光，留出大面积负空间用于放大字幕，高级金句短视频质感",
  },
  // ── 动画 ──
  {
    id: "pixar-3d",
    name: "皮克斯 3D",
    group: "动画",
    bg: "linear-gradient(135deg,#4cc9f0,#fca311)",
    accent: "#ffffff",
    prompt: "皮克斯/迪士尼式 3D 渲染动画，柔和体积光，圆润可爱造型，明亮鲜艳，电影级 3D 质感",
  },
  {
    id: "anime",
    name: "日系动画",
    group: "动画",
    bg: "linear-gradient(135deg,#89c4f4,#f7c5cc)",
    accent: "#ff6b9d",
    prompt: "日式动画赛璐璐风格，新海诚式细腻光影与天空，清新色彩，干净线条，动画电影截图质感",
  },
  {
    id: "claymation",
    name: "黏土定格",
    group: "动画",
    bg: "linear-gradient(135deg,#e8b08a,#9a6b4f)",
    accent: "#ff7f50",
    prompt: "黏土定格动画风格，手作黏土材质与指纹肌理，柔和影棚布光，温暖手工质感",
  },
  // ── 插画 ──
  {
    id: "storybook",
    name: "治愈绘本",
    group: "插画",
    bg: "linear-gradient(135deg,#fde2c4,#f7b7a3)",
    accent: "#ef8354",
    prompt: "温暖治愈系儿童绘本插画，柔和水彩与色铅笔质感，圆润可爱，明亮柔光，温馨氛围",
  },
  {
    id: "watercolor",
    name: "水彩",
    group: "插画",
    bg: "linear-gradient(135deg,#cdeccd,#a8d8ea)",
    accent: "#5e8b7e",
    prompt: "手绘水彩插画，湿润晕染，透明叠色，纸张纹理，清新淡雅，留白",
  },
  {
    id: "ink-watercolor-comic",
    name: "美式漫画",
    group: "插画",
    bg: "linear-gradient(135deg,#1d3557,#e63946)",
    accent: "#ffd166",
    dark: true,
    prompt: "美式漫画/分镜风格，粗黑描边，网点与平涂高饱和色块，强对比，动感构图",
  },
  {
    id: "crayon",
    name: "蜡笔童趣",
    group: "插画",
    bg: "linear-gradient(135deg,#ffe066,#ff9aa2)",
    accent: "#ff595e",
    prompt: "儿童蜡笔涂鸦风格，稚拙线条，明快撞色，粗糙蜡质笔触，天真可爱",
  },
  // ── 国风 ──
  {
    id: "guofeng-ink",
    name: "国风水墨",
    group: "国风",
    bg: "#f4f1ea",
    accent: "#8a2a1c",
    prompt: "中国水墨动画风格，写意留白，淡彩晕染，毛笔笔触，山水意境，东方美学",
  },
  {
    id: "guochao",
    name: "国潮插画",
    group: "国风",
    bg: "linear-gradient(135deg,#c1121f,#fdf0d5)",
    accent: "#c1121f",
    prompt: "现代国潮插画，传统纹样与扁平撞色，朱红与金，复古海报构成，东方时尚感",
  },
  {
    id: "gongbi",
    name: "工笔重彩",
    group: "国风",
    bg: "linear-gradient(135deg,#1b3a4b,#c08552)",
    accent: "#e0a458",
    dark: true,
    prompt: "中国工笔重彩风格，细腻勾线，矿物颜料厚重设色，金线点缀，古典华美",
  },
  // ── 特色 ──
  {
    id: "cyberpunk",
    name: "赛博朋克",
    group: "特色",
    bg: "#0a0a14",
    accent: "#ff00a0",
    dark: true,
    prompt: "赛博朋克风格，霓虹光污染，雨夜湿润街道反光，高对比冷暖撞色，未来都市，体积光",
  },
  {
    id: "dark-fantasy",
    name: "暗黑奇幻",
    group: "特色",
    bg: "linear-gradient(135deg,#2b2d42,#5a189a)",
    accent: "#9d4edd",
    dark: true,
    prompt: "暗黑奇幻插画，浓郁厚涂，戏剧体积光与雾气，史诗氛围，高细节概念设定画质感",
  },
  {
    id: "vintage-film",
    name: "复古胶片",
    group: "特色",
    bg: "linear-gradient(135deg,#7f5539,#ddb892)",
    accent: "#bc6c25",
    prompt: "复古胶片风格，柯达暖色调，颗粒感，轻微漏光与暗角，80–90 年代怀旧氛围",
  },
];

export const STORY_STYLES_WITH_AUTO: StoryStyle[] = [STORY_STYLE_AUTO, ...STORY_STYLES];

export function findStoryStyle(id: string): StoryStyle {
  return STORY_STYLES_WITH_AUTO.find((s) => s.id === id) ?? STORY_STYLES[0];
}

export function storyStyleName(id: string): string {
  return findStoryStyle(id).name;
}
