#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""
Polaris「壹伴」排版引擎 v5 · 微信公众号 —— 两段解耦版
================================================

v4 的根症：把「正文 + 全套样式」一步 `root.innerHTML = …` 硬塞进编辑器。新版公众号编辑器是
ProseMirror（自有文档事务模型），直改 DOM 绕过它的内部状态——内容常被它当脏数据清掉，或视觉上
有了却**没真正进草稿数据**，表现就是「看着传了却存不进 / 保存没反应」。富样式越重越容易死。

v5 两个根治：

  ① **粘贴通道**。真实世界的排版插件（壹伴 / 135editor）全是把 HTML 通过剪贴板**粘贴**进编辑
    器的——粘贴事件会走 ProseMirror 自己的解析器和事务模型，内容才真正入档。这里用合成
    `ClipboardEvent + DataTransfer` 复刻同一条路（不碰系统剪贴板），并配三级降级：
    粘贴 → execCommand('insertHTML') → innerHTML（老路，仅兜底），每级注入后**按字数校验**。

  ② **两段解耦**。publish 拆成：
       第一段：只把**纯语义正文**粘进编辑器（最不容易被拒）→ 填标题 → 保存草稿 → 确认。
       第二段：离屏套主题样式 → 全选粘贴回编辑器（仍走 ProseMirror）→ 再保存 → 确认。
    第二段失败不影响第一段——**文字已经安全躺在草稿箱**，随时可单独重跑样式。

模式：
  render    预览/兜底：无头浏览器离屏套样式，产出成品 HTML 文件。不碰后台，纯确定性。
  publish   两段直送草稿：先稳传文字，再套样式。--text-only 只跑第一段。绝不自动发布。
  restyle   对**已打开的草稿编辑器**原地换主题：读正文 → 剥旧样式 → 套新主题 → 粘贴回 → 保存。
            想换风格/改背景就反复跑它，每次只动样式不动文字。
  panel     可视化排版面板（壹伴插件形态）：CloakBrowser 打开后台，编辑器页面右侧注入面板——
            主题模板墙一点换肤、「AI 改风格」用大白话定制（喊 claude 生成主题参数）、清除样式、
            保存草稿。进程常驻当 AI 桥，关窗口即结束。
  snapshot  长图链路上半场：成品 HTML → 全页截长图（@2x 高清）→ 在**段落空隙**切片（≤2800css px）
            → 切片 png × N + manifest.json。纯本地确定性，不碰后台。
  publish-image 长图链路下半场：开编辑器 →（可选）真文字导语 → 切片按序**粘贴**进正文
            （图片粘贴是编辑器原生欢迎的操作，零清洗零字数问题）→ 等每张真落位 → 填标题 → 存草稿。

用法：
  python wechat_yiban.py --mode render  --body-file body.html --theme 墨韵 --out 预览.html
  python wechat_yiban.py --mode publish --body-file body.html --theme 墨韵 --title "标题"
  python wechat_yiban.py --mode publish --body-file body.html --title "标题" --text-only
  python wechat_yiban.py --mode restyle --theme 科技蓝     # 先在窗口里打开要改的草稿
  python wechat_yiban.py --mode panel                      # 可视化面板,改格式像用壹伴一样点

主题：墨韵 / 极简 / 科技蓝 / 杂志 / 清新绿 / 活力橙 / 米纸 / 黛青（缺省墨韵；六种标题形态 h2Mode，
米纸/黛青带整体底色——底色按块铺设不靠包裹层，编辑器剥不掉；AI 可生成自定义主题对象）。
DOM 选择器全部抽到顶部 SELECTORS，公众号后台改版时只改这里。
"""

import argparse
import json
import os
import re
import shutil
import subprocess
import sys
import time

# ───────────────────────── CloakBrowser（默认浏览器，drop-in 替换 Playwright）─────────────────────────
# 离线源码副本已随 Polaris 附带：pip install ~/Polaris/plugins/cloakbrowser
try:
    from cloakbrowser import launch, launch_persistent_context  # type: ignore
except Exception:  # pragma: no cover —— 没装就退回原生 playwright，便于本地调试
    try:
        from playwright.sync_api import sync_playwright  # type: ignore
    except Exception:
        sync_playwright = None

    def launch(headless=True, humanize=False, **_):
        pw = sync_playwright().start()
        b = pw.chromium.launch(headless=headless)
        b._pw = pw  # 关闭时一并 stop
        return b

    def launch_persistent_context(user_data_dir=".", headless=True, humanize=False, **_):
        pw = sync_playwright().start()
        ctx = pw.chromium.launch_persistent_context(user_data_dir, headless=headless)
        ctx._pw = pw
        return ctx


# ───────────────────────── 后台 DOM 选择器（改版只动这里）─────────────────────────
SELECTORS = {
    # 公众号图文正文编辑器：新版是 ProseMirror（div.ProseMirror[contenteditable]），
    # 老版是 UEditor 的 iframe body(#ueditor_0)。编辑器多半开在**新标签**，_find_editor 已跨标签扫。
    "editor_body": [
        "div.ProseMirror[contenteditable=true]",   # 新版富文本编辑器
        "#ueditor_0",                              # 老版 UEditor iframe 内 body
        "[contenteditable=true].rich_media_content",
        "#js_editor [contenteditable=true]",
        "[contenteditable=true]",                  # 兜底：页面里任一可编辑容器
    ],
    # 文章标题：编辑器里是 textarea（占位「请在这里输入标题」/#js_title）。
    # 注意别命中草稿箱列表页的搜索框（占位「输入标题/关键词」, 是 input.weui-desktop-form__input）。
    "title_input": [
        "#js_title",
        "textarea[placeholder*='请在这里输入标题']",
        "textarea[placeholder*='输入标题']",
    ],
    "save_draft": [
        "#js_submit",                              # 历史「保存」按钮
        "button:has-text('保存为草稿')",
        "a:has-text('保存为草稿')",
        "div.weui-desktop-btn:has-text('保存为草稿')",
        "*:has-text('保存为草稿')",
    ],
    # 保存成功的回执：toast「保存成功」或编辑器状态条「已保存」（自动保存也算——内容入档即可）
    "save_ok_hint": [
        "*:has-text('保存成功')",
        "*:has-text('已保存')",
        "*:has-text('保存为草稿成功')",
    ],
    # 判断是否已登录：登录后后台/草稿箱页会出现这些（weui-desktop 侧栏菜单文案，实测自 dump）
    "logged_in_hint": [
        "*:has-text('草稿箱')",
        "*:has-text('素材库')",
        "*:has-text('新的创作')",
    ],
    # 进入「写图文」编辑器的入口：草稿箱页「新的创作」面板里的「写图文」。
    # text-is 精确匹配，优先点 a/li 这类可点击元素，避免点到大容器。
    "new_article_entry": [
        "a:has-text('写图文')",
        "li:has-text('写图文')",
        "div.weui-desktop-card__type:has-text('图文')",
        "*:text-is('写图文')",
        "a:has-text('新的创作')",
        "*:text-is('新的创作')",
    ],
    # 编辑器工具栏「图片」按钮（publish-image 的 file-chooser 通道用）
    "img_button": [
        "#js_editor_insertimage",
        "[title='图片']",
        "[title*='图片']",
        ".edui-for-insertimage",
        "[aria-label*='图片']",
        "li:has-text('图片')",
    ],
}

MP_HOME = "https://mp.weixin.qq.com/"
SESSION_DIR = os.path.expanduser("~/Polaris/sessions/wechat")


# ───────────────────────── 壹伴样式引擎（浏览器内执行；预览 / 直传 / 换肤共用同一份）─────────────────────────
# 注意：这是注入浏览器的 JS 源码（Python 普通三引号串，${} / 反引号都按字面保留）。
# 入参 (root, themeName)：先 normalize（剥掉旧内联样式→回到语义态，使**换肤幂等**），再在 root 子树上
# 逐元素套主题内联样式，返回处理后的 innerHTML。生成物全部打 data-polaris-* 标记，重跑能认出自己。
# 公众号会剥离 <style>/class，只有内联稳。
STYLIZE_JS = r"""
(function (root, themeName, inPlace) {
  // —— 主题预设：palette + h2Mode(标题形态：bar竖条/underline下划线/pill胶囊/center居中/block色块/tag标签)
  //    + 可选 bg(整体底色)。底色**按块铺设**(每个块自带 background 内联)——不靠外包 section,
  //    编辑器剥不掉,这是 135editor 背景模板的真实做法。——
  var THEMES = {
    "墨韵":   { accent:"#b08550", text:"#3a3a3a", quoteBg:"#f7f5f1", quoteBd:"#d9c7a8", quoteTx:"#8a8378", size:15.5, lh:1.85, hFont:"inherit", h2Mode:"bar" },
    "极简":   { accent:"#191919", text:"#262626", quoteBg:"#f6f6f6", quoteBd:"#191919", quoteTx:"#737373", size:16,  lh:1.9,  hFont:"inherit", h2Mode:"underline" },
    "科技蓝": { accent:"#2b6cff", text:"#2c3338", quoteBg:"#eef3ff", quoteBd:"#2b6cff", quoteTx:"#5a6b8c", size:15.5, lh:1.8,  hFont:"inherit", h2Mode:"pill" },
    "杂志":   { accent:"#b3322c", text:"#2b2b2b", quoteBg:"#faf6f0", quoteBd:"#b3322c", quoteTx:"#8a7a6a", size:16,  lh:1.85, hFont:"Georgia,'Songti SC',serif", h2Mode:"center" },
    "清新绿": { accent:"#1f7a4d", text:"#2f3a33", quoteBg:"#e9f5ee", quoteBd:"#1f7a4d", quoteTx:"#587a66", size:15.5, lh:1.8,  hFont:"inherit", h2Mode:"block" },
    "活力橙": { accent:"#e8622c", text:"#33302c", quoteBg:"#fff3ec", quoteBd:"#ffb38a", quoteTx:"#9a7a66", size:15.5, lh:1.8,  hFont:"inherit", h2Mode:"tag" },
    "米纸":   { accent:"#8a6d3b", text:"#4a4337", quoteBg:"#f1e8d2", quoteBd:"#c8b48a", quoteTx:"#8a7c63", size:15.5, lh:1.9,  hFont:"Georgia,'Songti SC',serif", h2Mode:"center", bg:"#faf5e9" },
    "黛青":   { accent:"#34566b", text:"#2f3b42", quoteBg:"#e7eef2", quoteBd:"#34566b", quoteTx:"#5d7382", size:15.5, lh:1.85, hFont:"inherit", h2Mode:"underline", bg:"#f3f7f9" }
  };
  // themeName 可以是预设名，也可以是 AI 生成的主题对象（缺省字段从墨韵补齐）。
  var base = THEMES["墨韵"];
  var t = (themeName && typeof themeName === "object") ? themeName : (THEMES[themeName] || base);
  Object.keys(base).forEach(function (k) { if (t[k] === undefined) t[k] = base[k]; });

  function set(el, css) { el.setAttribute("style", css); }
  // 块间距：无底色用 margin；有底色改用 padding（每块自己把底色铺满，块间 margin:0 → 不露白条）
  function sp(top, bottom) {
    return t.bg ? ("margin:0;padding:" + top + "px 16px " + bottom + "px;background:" + t.bg + ";")
                : ("margin:" + top + "px 0 " + bottom + "px;");
  }

  // —— normalize：先拆历史版本的背景包裹层，再剥旧内联样式，回到语义态（换肤幂等、不叠加）——
  // inPlace=true 是「直接改活编辑器 DOM」模式：class 是编辑器自己的，绝不能动。
  Array.prototype.slice.call(root.querySelectorAll("[data-polaris-bg]")).forEach(function (w) {
    while (w.firstChild) w.parentNode.insertBefore(w.firstChild, w);
    w.remove();
  });
  Array.prototype.slice.call(root.querySelectorAll("*")).forEach(function (el) {
    el.removeAttribute("style");
    if (!inPlace) el.removeAttribute("class");
  });

  // —— plain：素颜模式——只 normalize 不套样式（面板「清除样式」用）——
  if (t.plain) { root.removeAttribute("data-polaris-theme"); return root.innerHTML; }

  // —— 容器基线（活 DOM 上丢了也不要紧——每个块都带完整内联样式）——
  root.style.cssText += ";font-size:" + t.size + "px;line-height:" + t.lh +
    ";color:" + t.text + ";letter-spacing:.3px;word-break:break-word;" +
    (t.bg ? "background:" + t.bg + ";" : "");

  // —— 一级标题：文章大标题，居中加粗 ——
  root.querySelectorAll("h1").forEach(function (el) {
    set(el, "font-size:21px;font-weight:700;text-align:center;color:" + t.text + ";" +
      sp(10, 24) + "line-height:1.4;font-family:" + t.hFont + ";");
  });

  // —— 二级标题：六种形态（主题的「长相」主要靠它）——
  var m2 = t.h2Mode || "bar";
  root.querySelectorAll("h2").forEach(function (el) {
    var css = "font-size:17px;font-weight:700;line-height:1.5;font-family:" + t.hFont + ";" + sp(26, 12);
    if (m2 === "underline")  css += "color:" + t.text + ";border-bottom:2px solid " + t.accent + ";padding-bottom:6px;";
    else if (m2 === "pill")  css += "color:#ffffff;background:" + t.accent + ";display:inline-block;padding:5px 16px;border-radius:6px;";
    else if (m2 === "center") css += "color:" + t.accent + ";text-align:center;letter-spacing:2px;";
    else if (m2 === "block") css += "color:" + t.accent + ";background:" + t.quoteBg + ";padding:8px 14px;border-radius:5px;";
    else if (m2 === "tag")   css += "color:" + t.accent + ";display:inline-block;background:" + t.quoteBg + ";border:1px solid " + t.accent + ";padding:4px 16px;border-radius:999px;";
    else                     css += "color:" + t.accent + ";border-left:4px solid " + t.accent + ";padding-left:12px;";
    set(el, css);
  });

  // —— 三级标题：主色加粗，略小 ——
  root.querySelectorAll("h3").forEach(function (el) {
    set(el, "font-size:15.5px;font-weight:700;color:" + t.accent + ";" + sp(20, 8) +
      "line-height:1.5;font-family:" + t.hFont + ";");
  });

  // —— 正文段落 ——
  root.querySelectorAll("p").forEach(function (el) {
    if (el.getAttribute("data-polaris-li")) return; // 列表段落在下面统一处理
    set(el, "font-size:" + t.size + "px;line-height:" + t.lh + ";color:" + t.text +
      ";letter-spacing:.3px;" + sp(14, 14));
  });

  // —— 引用块：浅底左竖条卡片 ——
  root.querySelectorAll("blockquote").forEach(function (el) {
    set(el, "background:" + t.quoteBg + ";border-left:3px solid " + t.quoteBd +
      ";padding:12px 16px;color:" + t.quoteTx +
      ";border-radius:0 8px 8px 0;font-size:" + (t.size - 0.5) + "px;line-height:1.75;" +
      (t.bg ? "margin:0;" : "margin:18px 0;"));
  });

  // —— 重点 / 强调：主色加粗 ——
  root.querySelectorAll("strong,b").forEach(function (el) {
    set(el, "color:" + t.accent + ";font-weight:700;");
  });
  root.querySelectorAll("em,i").forEach(function (el) {
    set(el, "font-style:normal;color:" + t.accent + ";");
  });

  // —— 分割线：主色渐变细线（生成物打标记，重跑时按标记重新上色而不是叠一层）——
  // 有整体底色时退化成同色空白带（单元素铺不了"底色+居中细线"双层，别硬画）。
  var hrCss = t.bg
    ? ("height:14px;background:" + t.bg + ";margin:0;")
    : ("height:1px;background:linear-gradient(90deg,rgba(0,0,0,0)," + t.accent + ",rgba(0,0,0,0));margin:26px 0;");
  root.querySelectorAll("hr").forEach(function (el) {
    var d = document.createElement("section");
    d.setAttribute("data-polaris-hr", "1");
    set(d, hrCss);
    el.replaceWith(d);
  });
  root.querySelectorAll("[data-polaris-hr]").forEach(function (el) { set(el, hrCss); });

  // —— 列表：转成带行距的段落（公众号会吃掉原生 <ul>/<ol> 样式，所以自己画序号/圆点）——
  function flattenList(list, ordered) {
    var frag = document.createDocumentFragment();
    var i = 0;
    list.querySelectorAll(":scope > li").forEach(function (li) {
      i += 1;
      var p = document.createElement("p");
      var mark = ordered
        ? ('<span data-polaris-mark="1" style="color:' + t.accent + ';font-weight:700;">' + i + '. </span>')
        : ('<span data-polaris-mark="1" style="color:' + t.accent + ';font-weight:700;">· </span>');
      p.innerHTML = mark + li.innerHTML;
      p.setAttribute("data-polaris-li", "1");
      set(p, "font-size:" + t.size + "px;line-height:" + t.lh + ";color:" + t.text + ";" + sp(8, 8));
      frag.appendChild(p);
    });
    list.replaceWith(frag);
  }
  // 先取快照再替换，避免边遍历边改 DOM 漏项
  Array.prototype.slice.call(root.querySelectorAll("ol")).forEach(function (l) { flattenList(l, true); });
  Array.prototype.slice.call(root.querySelectorAll("ul")).forEach(function (l) { flattenList(l, false); });

  // —— 此前 flatten 出的序号/圆点（换肤重跑路径）：重新上主题色 ——
  root.querySelectorAll("[data-polaris-mark]").forEach(function (el) {
    set(el, "color:" + t.accent + ";font-weight:700;");
  });
  root.querySelectorAll("[data-polaris-li]").forEach(function (el) {
    set(el, "font-size:" + t.size + "px;line-height:" + t.lh + ";color:" + t.text + ";" + sp(8, 8));
  });

  // —— 链接：未认证号正文外链会被屏蔽，降级为主色文字（保留文案，去掉跳转误导）——
  root.querySelectorAll("a").forEach(function (el) {
    set(el, "color:" + t.accent + ";text-decoration:none;font-weight:600;");
  });

  // —— 配图：限制最大宽度、圆角；有底色时连图片块一起铺底 ——
  root.querySelectorAll("img").forEach(function (el) {
    set(el, "max-width:100%;height:auto;border-radius:8px;display:block;" +
      (t.bg ? "margin:0;padding:14px 16px;background:" + t.bg + ";box-sizing:border-box;"
            : "margin:18px auto;"));
  });

  // —— 自定义微调（AI 主题用）：{ "css选择器": "追加的内联样式" }，基线之上叠加 ——
  if (t.overrides) {
    Object.keys(t.overrides).forEach(function (k) {
      try {
        root.querySelectorAll(k).forEach(function (el) {
          el.setAttribute("style", (el.getAttribute("style") || "") + ";" + t.overrides[k]);
        });
      } catch (e) {}
    });
  }

  root.setAttribute("data-polaris-theme", (typeof themeName === "string") ? themeName : "custom");
  return root.innerHTML;
})
"""

# ───────────────────────── 编辑器注入 JS（粘贴通道 + 降级，全部小步骤分次 evaluate）─────────────────────────
# 选区与粘贴拆成两次调用，中间留时隙让 ProseMirror 的 selectionchange 同步——同步选区后粘贴才落对位置。
JS_FOCUS_SELECT = r"""
(root) => {
  root.focus();
  var sel = window.getSelection();
  var range = document.createRange();
  range.selectNodeContents(root);
  sel.removeAllRanges();
  sel.addRange(range);
  return true;
}
"""

# 合成粘贴：ClipboardEvent + DataTransfer（Chromium 支持构造），不碰系统剪贴板。
# ProseMirror 的 paste handler 会接住它→走自己的 schema 解析和事务→内容真正入档。
JS_PASTE = r"""
(root, args) => {
  try {
    var dt = new DataTransfer();
    dt.setData("text/html", args.html);
    dt.setData("text/plain", args.text || "");
    var ev = new ClipboardEvent("paste", { clipboardData: dt, bubbles: true, cancelable: true });
    root.dispatchEvent(ev);
    return true;
  } catch (e) { return false; }
}
"""

# 降级 2：execCommand('insertHTML')——走 beforeinput/input，多数富文本编辑器也认账。
JS_EXEC_INSERT = r"""
(root, args) => {
  root.focus();
  try { document.execCommand("selectAll", false, null); } catch (e) {}
  try { return document.execCommand("insertHTML", false, args.html); } catch (e) { return false; }
}
"""

# 降级 3（老路，仅兜底）：innerHTML 直写 + 补发 input 事件推编辑器同步一次。
JS_RAW_SET = r"""
(root, args) => {
  root.innerHTML = args.html;
  try { root.dispatchEvent(new InputEvent("input", { bubbles: true })); } catch (e) {}
  return true;
}
"""

JS_TEXT_LEN = r"""
(root) => ((root.innerText || root.textContent || "").replace(/\s+/g, "").length)
"""

JS_GET_HTML = r"""(root) => root.innerHTML"""

# 离屏套样式：在 frame 里建分离 div 跑壹伴引擎（不动编辑器），拿到成品 HTML 再走粘贴通道送回。
JS_OFFSCREEN_STYLE = (
    "(args) => { var d = document.createElement('div'); d.innerHTML = args.body; "
    "return (" + STYLIZE_JS + ")(d, args.theme); }"
)

# ───────────────────────── panel 模式：注入编辑器页面的可视化侧栏（壹伴插件形态）─────────────────────────
# 右侧固定面板：主题模板卡一点即换肤 + 「AI 改风格」自然语言定制（走 window.polarisAIBridge → python →
# claude CLI）+ 清除样式 + 保存草稿。全部逻辑在页面内完成（找编辑器/离屏套样式/粘贴通道/点保存），
# python 只负责注入和 AI 桥。页面导航后由 run_panel 的常驻循环重新注入。
PANEL_BODY_JS = r"""
function (STYLIZE) {
  if (document.getElementById("polaris-yiban-panel")) return "exists";

  var EDITOR_SEL = [
    "div.ProseMirror[contenteditable=true]",
    "#ueditor_0",
    "[contenteditable=true].rich_media_content",
    "#js_editor [contenteditable=true]",
    "[contenteditable=true]"
  ];
  var PRESETS = [
    ["墨韵", "#b08550", "暖金竖条标题 · 沉稳大气"],
    ["极简", "#191919", "黑白下划线标题 · 克制"],
    ["科技蓝", "#2b6cff", "胶囊白字标题 · 科技感"],
    ["杂志", "#b3322c", "衬线居中标题 · 杂志腔"],
    ["清新绿", "#1f7a4d", "浅绿色块标题 · 清新"],
    ["活力橙", "#e8622c", "圆角标签标题 · 明快"],
    ["米纸", "#8a6d3b", "米色纸底+衬线 · 书卷气"],
    ["黛青", "#34566b", "雾蓝纸底 · 青灰文艺"]
  ];

  var current = null;        // 最近一次套的主题（对象或预设名），AI 定制的基线
  var currentLabel = "墨韵";

  function findEditor() {
    var docs = [document];
    var ifr = document.querySelectorAll("iframe");
    for (var i = 0; i < ifr.length; i++) {
      try { if (ifr[i].contentDocument) docs.push(ifr[i].contentDocument); } catch (e) {}
    }
    for (var d = 0; d < docs.length; d++) {
      for (var s = 0; s < EDITOR_SEL.length; s++) {
        var el = null;
        try { el = docs[d].querySelector(EDITOR_SEL[s]); } catch (e) {}
        if (el && !el.closest("#polaris-yiban-panel")) return el;
      }
    }
    return null;
  }

  function stripText(html) {
    var d = document.createElement("div");
    d.innerHTML = html;
    return (d.textContent || "").replace(/\s+/g, "");
  }
  function plainLen(el) { return ((el.innerText || el.textContent || "").replace(/\s+/g, "")).length; }

  // 粘贴通道（与脚本 publish 同一条路）：选区→合成 paste→校验，逐级降级
  function pasteInto(ed, html) {
    var doc = ed.ownerDocument, win = doc.defaultView || window;
    var expect = stripText(html).length;
    ed.focus();
    try {
      var sel = win.getSelection(); var range = doc.createRange();
      range.selectNodeContents(ed); sel.removeAllRanges(); sel.addRange(range);
    } catch (e) {}
    return new Promise(function (resolve) {
      setTimeout(function () {
        try {
          var dt = new DataTransfer();
          dt.setData("text/html", html); dt.setData("text/plain", stripText(html));
          ed.dispatchEvent(new ClipboardEvent("paste", { clipboardData: dt, bubbles: true, cancelable: true }));
        } catch (e) {}
        setTimeout(function () {
          if (plainLen(ed) >= expect * 0.6) return resolve("paste");
          try {
            ed.focus(); doc.execCommand("selectAll", false, null);
            if (doc.execCommand("insertHTML", false, html) && plainLen(ed) >= expect * 0.6)
              return resolve("execCommand");
          } catch (e) {}
          ed.innerHTML = html;
          try { ed.dispatchEvent(new InputEvent("input", { bubbles: true })); } catch (e) {}
          resolve("innerHTML");
        }, 800);
      }, 350);
    });
  }

  function clickSave() {
    var byId = document.getElementById("js_submit");
    if (byId) { byId.click(); return true; }
    var cands = document.querySelectorAll("button, a, div.weui-desktop-btn");
    for (var i = 0; i < cands.length; i++) {
      var t = (cands[i].textContent || "").trim();
      if (t === "保存为草稿") { cands[i].click(); return true; }
    }
    for (var j = 0; j < cands.length; j++) {
      if ((cands[j].textContent || "").trim() === "保存") { cands[j].click(); return true; }
    }
    return false;
  }

  var statusEl = null;
  function toast(msg, isErr) {
    if (!statusEl) return;
    statusEl.textContent = msg;
    statusEl.style.color = isErr ? "#d23c3c" : "#2e7d4f";
  }

  // 像改 HTML 文件一样：优先**直接在活编辑器 DOM 上**逐元素写内联样式（浏览器插件做法）。
  // 若编辑器把改动回滚（验证不到样式），再退回粘贴通道整体替换。
  function verifyInPlace(ed) {
    var el = ed.querySelector("h1,h2,h3,p,blockquote");
    return !!(el && (el.getAttribute("style") || "").length > 10);
  }

  function pasteFallback(ed, theme, label) {
    var box = document.createElement("div"); box.innerHTML = ed.innerHTML;
    var styled;
    try { styled = STYLIZE(box, theme, false); } catch (e) { toast("套样式出错：" + e, true); return; }
    pasteInto(ed, styled).then(function (method) {
      current = theme; currentLabel = label; window.__polarisCurrent = theme;
      var saved = clickSave();
      toast("已套「" + label + "」（粘贴通道 " + method + (saved ? "，已点保存草稿）" : "，请手动保存草稿）"));
    });
  }

  function applyTheme(theme, label) {
    var ed = findEditor();
    if (!ed) { toast("没找到正文编辑器——请先打开一篇草稿或「写图文」", true); return; }
    if (stripText(ed.innerHTML).length < 2) { toast("正文是空的——先把文字传进来再换肤", true); return; }
    toast("正在套「" + label + "」……");
    var ok = false;
    try { STYLIZE(ed, theme, true); ok = true; } catch (e) { ok = false; }
    if (!ok) { pasteFallback(ed, theme, label); return; }
    try { ed.dispatchEvent(new InputEvent("input", { bubbles: true })); } catch (e) {}
    setTimeout(function () {
      if (verifyInPlace(ed)) {
        current = theme; currentLabel = label; window.__polarisCurrent = theme;
        var saved = clickSave();
        toast("已套「" + label + "」（原地修改" + (saved ? "，已点保存草稿）" : "，请手动保存草稿）"));
      } else {
        pasteFallback(ed, theme, label); // 编辑器回滚了原地改动 → 整体替换兜底
      }
    }, 900);
  }

  // AI 改风格：零桥接轮询握手——指令放进 window.__polarisAI.pending，
  // python 常驻循环捡走→喊 claude 生成主题 JSON→调 window.__polarisAIResult 回填。
  window.__polarisAI = window.__polarisAI || { pending: null };
  window.__polarisCurrent = currentLabel;
  window.__polarisAIResult = function (raw) {
    try {
      var res = JSON.parse(raw);
      if (res.error) { toast(res.error, true); return; }
      applyTheme(res.theme, "AI 定制");
    } catch (e) { toast("AI 结果解析失败：" + e, true); }
  };
  function aiRestyle() {
    var ta = document.getElementById("polaris-ai-input");
    var instr = (ta && ta.value || "").trim();
    if (!instr) { toast("先在上面描述你想要的风格", true); return; }
    window.__polarisAI.pending = instr;
    toast("AI 思考中……（北极星后台代跑，约 1～2 分钟，别关窗口）");
  }

  // —— 面板 DOM ——（全内联样式，避免被页面 CSS 干扰）
  var panel = document.createElement("div");
  panel.id = "polaris-yiban-panel";
  panel.setAttribute("style",
    "position:fixed;top:0;right:0;width:300px;height:100vh;z-index:2147483646;" +
    "background:#fff;border-left:1px solid #e3e3e3;box-shadow:-6px 0 24px rgba(0,0,0,.08);" +
    "font:13px/1.6 -apple-system,'PingFang SC','Microsoft YaHei',sans-serif;color:#333;" +
    "display:flex;flex-direction:column;transition:transform .2s;");

  var head = document.createElement("div");
  head.setAttribute("style",
    "padding:12px 14px;border-bottom:1px solid #eee;display:flex;align-items:center;gap:8px;flex:none;");
  head.innerHTML =
    '<span style="font-size:15px;">🌟</span>' +
    '<b style="font-size:14px;">北极星 · 排版面板</b>' +
    '<span id="polaris-yp-fold" style="margin-left:auto;cursor:pointer;color:#999;padding:2px 8px;">收起 ›</span>';
  panel.appendChild(head);

  var body = document.createElement("div");
  body.setAttribute("style", "flex:1;overflow-y:auto;padding:12px 14px;");

  var sec1 = document.createElement("div");
  sec1.innerHTML = '<div style="font-weight:700;margin-bottom:8px;color:#666;">主题模板 <span style="font-weight:400;color:#aaa;">点一下换肤，可反复换</span></div>';
  PRESETS.forEach(function (p) {
    var card = document.createElement("div");
    card.setAttribute("style",
      "display:flex;align-items:center;gap:10px;padding:9px 10px;margin:6px 0;border:1px solid #eee;" +
      "border-radius:8px;cursor:pointer;background:#fafafa;");
    card.innerHTML =
      '<span style="width:18px;height:18px;border-radius:50%;background:' + p[1] + ';flex:none;"></span>' +
      '<span><b>' + p[0] + '</b><br><span style="color:#999;font-size:12px;">' + p[2] + '</span></span>';
    card.onclick = function () { applyTheme(p[0], p[0]); };
    card.onmouseenter = function () { card.style.borderColor = p[1]; };
    card.onmouseleave = function () { card.style.borderColor = "#eee"; };
    sec1.appendChild(card);
  });
  body.appendChild(sec1);

  var sec2 = document.createElement("div");
  sec2.setAttribute("style", "margin-top:16px;");
  sec2.innerHTML =
    '<div style="font-weight:700;margin-bottom:8px;color:#666;">AI 改风格 <span style="font-weight:400;color:#aaa;">用大白话描述</span></div>' +
    '<textarea id="polaris-ai-input" placeholder="例：标题改成藏蓝色、正文行距加大一点、引用卡换浅黄底、整体加米色背景" ' +
    'style="width:100%;height:72px;border:1px solid #ddd;border-radius:8px;padding:8px;font:12.5px/1.5 inherit;' +
    'resize:vertical;box-sizing:border-box;outline:none;"></textarea>' +
    '<button id="polaris-ai-btn" style="width:100%;margin-top:8px;padding:9px 0;border:0;border-radius:8px;' +
    'background:#2b6cff;color:#fff;font-weight:700;cursor:pointer;font-size:13px;">✨ AI 改风格</button>';
  body.appendChild(sec2);

  var sec3 = document.createElement("div");
  sec3.setAttribute("style", "margin-top:16px;display:flex;gap:8px;");
  sec3.innerHTML =
    '<button id="polaris-yp-plain" style="flex:1;padding:8px 0;border:1px solid #ddd;border-radius:8px;' +
    'background:#fff;cursor:pointer;">清除样式</button>' +
    '<button id="polaris-yp-save" style="flex:1;padding:8px 0;border:1px solid #ddd;border-radius:8px;' +
    'background:#fff;cursor:pointer;">保存草稿</button>';
  body.appendChild(sec3);

  statusEl = document.createElement("div");
  statusEl.setAttribute("style", "margin-top:12px;min-height:36px;font-size:12.5px;color:#2e7d4f;word-break:break-all;");
  statusEl.textContent = "面板就绪。先确认正文已在编辑器里，然后点模板或用 AI。";
  body.appendChild(statusEl);

  var foot = document.createElement("div");
  foot.setAttribute("style", "margin-top:10px;padding-top:10px;border-top:1px dashed #eee;color:#bbb;font-size:11.5px;");
  foot.textContent = "只动样式不动文字 · 只存草稿，发布永远由你亲手点。";
  body.appendChild(foot);

  panel.appendChild(body);
  document.body.appendChild(panel);

  // 收起/展开：折成右侧小把手
  var folded = false;
  var handle = document.createElement("div");
  handle.id = "polaris-yiban-handle";
  handle.setAttribute("style",
    "position:fixed;top:40%;right:0;z-index:2147483646;background:#2b6cff;color:#fff;padding:10px 6px;" +
    "border-radius:8px 0 0 8px;cursor:pointer;writing-mode:vertical-rl;font-size:12px;display:none;" +
    "box-shadow:-2px 2px 10px rgba(0,0,0,.2);");
  handle.textContent = "🌟 排版面板";
  handle.onclick = function () {
    folded = false;
    panel.style.transform = "translateX(0)";
    handle.style.display = "none";
  };
  document.body.appendChild(handle);
  document.getElementById("polaris-yp-fold").onclick = function () {
    folded = true;
    panel.style.transform = "translateX(100%)";
    handle.style.display = "block";
  };

  document.getElementById("polaris-ai-btn").onclick = aiRestyle;
  document.getElementById("polaris-yp-plain").onclick = function () { applyTheme({ plain: true }, "素颜"); };
  document.getElementById("polaris-yp-save").onclick = function () {
    toast(clickSave() ? "已点「保存为草稿」。" : "没找到保存键——请手动保存。", false);
  };

  return "injected";
}
"""


def _panel_boot_js():
    """组装注入脚本：把壹伴引擎作为参数喂给面板。"""
    return "(" + PANEL_BODY_JS + ")(" + STYLIZE_JS + ")"


def _read(path):
    with open(path, "r", encoding="utf-8") as f:
        return f.read()


def _plain_text(html):
    """语义 HTML → 纯文本（粘贴的 text/plain 兜底 + 字数校验基准）。"""
    txt = re.sub(r"<[^>]+>", " ", html)
    txt = (txt.replace("&nbsp;", " ").replace("&lt;", "<").replace("&gt;", ">")
              .replace("&amp;", "&").replace("&quot;", '"').replace("&#39;", "'"))
    return re.sub(r"\s+", "", txt)


def _first(frame_or_page, selectors):
    """在给定 frame/page 上按候选选择器依次找，返回第一个命中的元素句柄（找不到返回 None）。"""
    for sel in selectors:
        try:
            el = frame_or_page.query_selector(sel)
            if el:
                return el, sel
        except Exception:
            continue
    return None, None


def _wait_any(page, selectors, seconds):
    """轮询等待任一选择器出现（toast 这类闪现元素用轮询比 wait_for_selector 稳）。"""
    deadline = time.time() + seconds
    while time.time() < deadline:
        el, _ = _first(page, selectors)
        if el:
            return True
        time.sleep(0.5)
    return False


# ───────────────────────── 壹伴引擎：在某个 page 上把正文套成成品 HTML（render 与兜底共用）─────────────────────────
def _styled_html(page, body_html, theme):
    """about:blank + evaluate 自建容器→注入正文→跑壹伴引擎→返回包好的完整成品 HTML。
    （避开 set_content 在隐身 Chromium 下等 "load" 事件超时的坑。）"""
    page.goto("about:blank")
    styled = page.evaluate(
        "(args) => { document.body.innerHTML = \"<div id='polaris-root'></div>\"; "
        "var root = document.getElementById('polaris-root'); root.innerHTML = args.body; "
        "return (" + STYLIZE_JS + ")(root, args.theme); }",
        {"body": body_html, "theme": theme},
    )
    return (
        "<!doctype html><meta charset='utf-8'>"
        "<div style='max-width:677px;margin:0 auto;padding:20px 16px;"
        "background:#fff;-webkit-font-smoothing:antialiased;'>" + styled + "</div>"
    )


# 新版公众号编辑器里，**标题框与正文框都是 div.ProseMirror[contenteditable]**——标题框矮(~30px)、
# 正文框高(~585px)。v5 的测量 bug = 用「第一个 ProseMirror」当正文，恰好抓到标题框，于是「数正文里
# 几张图」永远 0、导语被写进标题框。根治：选**最高**的 ProseMirror 打唯一标记，下游所有操作(粘贴/
# 注入/数图/传图)统一按 [data-polaris-body] 定位。幂等(先清旧标记)、单 PM 时结果不变、失败则回退原选择器。
JS_MARK_TALLEST_BODY = r"""
() => {
  var pms = Array.prototype.slice.call(
    document.querySelectorAll("div.ProseMirror[contenteditable=true]"));
  if (!pms.length) return null;
  var best = null, bestH = -1;
  pms.forEach(function (el) {
    var h = el.offsetHeight || el.clientHeight || 0;
    if (h > bestH) { bestH = h; best = el; }
  });
  if (!best) return null;
  pms.forEach(function (el) { el.removeAttribute("data-polaris-body"); });
  best.setAttribute("data-polaris-body", "1");
  return pms.length;   // 返回 PM 总数：>1 说明确实有标题/正文之分，标记有意义
}
"""


def _mark_body(frame):
    """在 frame 内把最高的 ProseMirror 标成正文，返回标记选择器（标记成功）或 None。"""
    try:
        n = frame.evaluate(JS_MARK_TALLEST_BODY)
        if n:
            return '[data-polaris-body="1"]'
    except Exception:
        pass
    return None


# 新版编辑器标题框也是 ProseMirror（最矮的那个）。仅当存在 ≥2 个 PM（标题+正文）才标记——
# 老版编辑器标题是 textarea、只有 1 个正文 PM，这里返回 None，交给 textarea 路处理。
JS_MARK_SHORTEST_TITLE = r"""
() => {
  var pms = Array.prototype.slice.call(
    document.querySelectorAll("div.ProseMirror[contenteditable=true]"));
  if (pms.length < 2) return null;
  var best = null, bestH = Infinity;
  pms.forEach(function (el) {
    var h = el.offsetHeight || el.clientHeight || 0;
    if (h > 0 && h < bestH) { bestH = h; best = el; }
  });
  if (!best) return null;
  pms.forEach(function (el) { el.removeAttribute("data-polaris-title"); });
  best.setAttribute("data-polaris-title", "1");
  return true;
}
"""


def _find_editor(ctx_or_page):
    """跨「所有标签页 × 所有 frame」找正文可编辑容器（公众号「写图文」常开在新标签，正文又在 UEditor
    的 iframe 里）。返回 (page, frame, selector)；找不到返回 (None, None, None)。"""
    pages = list(ctx_or_page.pages) if hasattr(ctx_or_page, "pages") else [ctx_or_page]
    for pg in pages:
        try:
            frames = pg.frames
        except Exception:
            continue
        for frame in frames:
            el, sel = _first(frame, SELECTORS["editor_body"])
            if el:
                # 新版 ProseMirror：标题与正文同类，选最高的当正文（修 v5 抓错标题框的测量 bug）。
                if "ProseMirror" in sel or "contenteditable" in sel:
                    marked = _mark_body(frame)
                    if marked:
                        sel = marked
                return pg, frame, sel
    return None, None, None


def _is_logged_in(ctx_or_page):
    pages = list(ctx_or_page.pages) if hasattr(ctx_or_page, "pages") else [ctx_or_page]
    for pg in pages:
        el, _ = _first(pg, SELECTORS["logged_in_hint"])
        if el:
            return True
    return False


# ───────────────────────── 注入：粘贴通道 + 三级降级 + 字数校验 ─────────────────────────
def _inject_html(frame, body_sel, html, expect_len):
    """把 html 灌进编辑器，按「剥标签后的字数 ≥ 60% 预期」校验是否真落进去了。
    返回 (ok, method, landed_len)。三级：paste → execCommand → innerHTML。"""
    plain = _plain_text(html)
    threshold = max(1, int(expect_len * 0.6))

    def landed():
        try:
            return frame.eval_on_selector(body_sel, JS_TEXT_LEN)
        except Exception:
            return -1

    # ① 粘贴通道（ProseMirror 正道）：选区和粘贴分两步，给编辑器留同步选区的时隙
    try:
        frame.eval_on_selector(body_sel, JS_FOCUS_SELECT)
        time.sleep(0.4)
        frame.eval_on_selector(body_sel, JS_PASTE, {"html": html, "text": plain})
        time.sleep(0.8)
        n = landed()
        if n >= threshold:
            return True, "paste", n
    except Exception:
        pass

    # ② execCommand('insertHTML')
    try:
        frame.eval_on_selector(body_sel, JS_EXEC_INSERT, {"html": html})
        time.sleep(0.6)
        n = landed()
        if n >= threshold:
            return True, "execCommand", n
    except Exception:
        pass

    # ③ innerHTML（老路，最后兜底）
    try:
        frame.eval_on_selector(body_sel, JS_RAW_SET, {"html": html})
        time.sleep(0.6)
        n = landed()
        if n >= threshold:
            return True, "innerHTML", n
        return False, "innerHTML", n
    except Exception:
        return False, "none", -1


def _fill_title(frame, editor_page, title):
    """填标题（best-effort，新旧编辑器都兼容）：
    ① 老版：标题是 textarea（#js_title）→ fill/type。
    ② 新版：标题框也是 ProseMirror（最矮的那个）→ 标记最矮 PM、点击聚焦、键入。
    标题框可能在编辑器 frame 或其所在标签页主文档。"""
    if not title:
        return False
    # ① 老版 textarea 标题
    tin, _ = _first(frame, SELECTORS["title_input"])
    if not tin:
        tin, _ = _first(editor_page, SELECTORS["title_input"])
    if tin:
        try:
            tin.fill(title)
            return True
        except Exception:
            try:
                tin.click()
                tin.type(title)
                return True
            except Exception:
                pass
    # ② 新版 ProseMirror 标题（修复文档①：新版编辑器标题也是 PM，旧 textarea 选择器抓不到）
    for fr in (frame, editor_page):
        try:
            if fr.evaluate(JS_MARK_SHORTEST_TITLE):
                el = fr.query_selector('[data-polaris-title="1"]')
                if el:
                    el.click()  # 聚焦标题 PM
                    editor_page.keyboard.type(title)  # 真键入，走 ProseMirror 事务
                    return True
        except Exception:
            continue
    return False


def _save_draft(editor_page):
    """点「保存为草稿」并等回执。找不到按钮才退 Ctrl+S（公众号编辑器拦截了它）。
    返回 (clicked, confirmed)——confirmed=出现「保存成功/已保存」类提示（自动保存也算入档）。"""
    clicked = False
    btn, _ = _first(editor_page, SELECTORS["save_draft"])
    if btn:
        try:
            btn.click()
            clicked = True
        except Exception:
            pass
    if not clicked:
        try:
            editor_page.keyboard.press("Control+s")
            clicked = True
        except Exception:
            pass
    confirmed = _wait_any(editor_page, SELECTORS["save_ok_hint"], 10) if clicked else False
    return clicked, confirmed


# ───────────────────────── 等编辑器：登录 + 进「写图文」（publish）/ 用户自己打开草稿（restyle）─────────────────────────
def _open_editor_direct(ctx):
    """从已登录页面 URL 抠 token，直接打开「写图文」编辑器 URL——比在首页找按钮点稳得多
    （首页改版按钮就找不到，URL 协议多年没变）。"""
    token = None
    for p in list(ctx.pages):
        try:
            m = re.search(r"[?&]token=(\d+)", p.url or "")
        except Exception:
            m = None
        if m:
            token = m.group(1)
            break
    if not token:
        return False
    url = ("https://mp.weixin.qq.com/cgi-bin/appmsg?t=media/appmsg_edit_v2&action=edit"
           "&isNew=1&type=77&createType=0&token=%s&lang=zh_CN" % token)
    try:
        pg = ctx.new_page()
        pg.goto(url, wait_until="domcontentloaded")
        print("[壹伴] 已直开「写图文」编辑器（token 直达，绕过首页按钮）。", flush=True)
        return True
    except Exception:
        return False


def _wait_editor(ctx, timeout, auto_click_entry, hint):
    """循环等「登录 + 编辑器出现」。auto_click_entry=True 时登录后先 token 直开编辑器，
    失败再替用户点「写图文」（restyle 必须 False——会新建文章而不是改既有草稿！）。每 20s 报进度。"""
    frame = body_sel = pg = None
    clicked_entry = 0
    tried_direct = False
    announced_login = False
    last_tick = time.time()
    deadline = time.time() + timeout
    while time.time() < deadline:
        pg, frame, body_sel = _find_editor(ctx)
        if frame:
            return pg, frame, body_sel
        logged_in = _is_logged_in(ctx)
        if logged_in and not announced_login:
            print("[壹伴] 已登录。" + hint, flush=True)
            announced_login = True
        # 首选：token 直开编辑器 URL（只试一次；URL 比首页 DOM 稳定得多）
        if auto_click_entry and logged_in and not tried_direct:
            tried_direct = True
            if _open_editor_direct(ctx):
                time.sleep(4)
                continue
        # 次选：登录后最多替用户点 2 次「写图文」（隔 45s 才重试，避免开出一堆标签）
        if auto_click_entry and logged_in and clicked_entry < 2:
            for p in list(ctx.pages):
                entry, _ = _first(p, SELECTORS["new_article_entry"])
                if entry:
                    try:
                        entry.click()
                        clicked_entry += 1
                        p.wait_for_timeout(2500)
                    except Exception:
                        pass
                    break
            if clicked_entry == 1:
                # 第一次点完等 45s 再考虑第二次
                next_retry = time.time() + 45
                while time.time() < min(next_retry, deadline):
                    pg, frame, body_sel = _find_editor(ctx)
                    if frame:
                        return pg, frame, body_sel
                    time.sleep(2)
        if time.time() - last_tick > 20:
            remain = int(deadline - time.time())
            state = "已登录，等编辑器打开" if logged_in else "等扫码登录"
            print("[壹伴] %s……（剩 %ds）" % (state, remain), flush=True)
            last_tick = time.time()
        time.sleep(2)
    return None, None, None


# ───────────────────────── 模式一：render（预览 / 兜底，确定性）─────────────────────────
def run_render(body_html, theme, out_path):
    browser = launch(headless=True, humanize=False)
    try:
        page = browser.new_page()
        full = _styled_html(page, body_html, theme)
        with open(out_path, "w", encoding="utf-8") as f:
            f.write(full)
        print(json.dumps({"ok": True, "mode": "render", "out": os.path.abspath(out_path),
                          "theme": theme}, ensure_ascii=False))
    finally:
        try:
            browser.close()
        except Exception:
            pass
        pw = getattr(browser, "_pw", None)
        if pw:
            pw.stop()


# ───────────────────────── 模式二：publish（两段：先稳传文字，再套样式）─────────────────────────
def run_publish(body_html, theme, title, save_fallback, text_only, timeout):
    os.makedirs(SESSION_DIR, exist_ok=True)
    ctx = launch_persistent_context(user_data_dir=SESSION_DIR, headless=False, humanize=True)
    try:
        page = ctx.new_page() if hasattr(ctx, "new_page") else ctx.pages[0]
        page.goto(MP_HOME, wait_until="domcontentloaded")

        if not _is_logged_in(ctx):
            print("[壹伴] 未检测到登录态——请在已打开的 CloakBrowser 窗口里扫码登录公众号后台。"
                  "登录后脚本会自动点「写图文」打开编辑器（最多等 %ds）。" % timeout, flush=True)

        pg, frame, body_sel = _wait_editor(
            ctx, timeout, auto_click_entry=True,
            hint="正在尝试打开图文编辑器……（若没自动打开，请手动点「写图文」）")
        if not frame:
            return _publish_failed(ctx, body_html, theme, save_fallback,
                                   reason="超时未找到正文编辑器（可能未登录、未点进「写图文」，或后台改版）")
        editor_page = pg  # 编辑器所在的标签页（标题框/保存键在它上面）

        expect_len = len(_plain_text(body_html))

        # ━━ 第一段：纯语义正文走粘贴通道（最不容易被编辑器拒），先把文字稳稳送进草稿 ━━
        print("[壹伴] 第一段：注入纯文字正文……", flush=True)
        ok_a, method_a, landed_a = _inject_html(frame, body_sel, body_html, expect_len)
        if not ok_a:
            return _publish_failed(ctx, body_html, theme, save_fallback,
                                   reason="正文注入三级通道全部失败（落入编辑器字数 %d / 预期 %d）"
                                          % (landed_a, expect_len))
        print("[壹伴] 正文已入编辑器（通道=%s，字数 %d/%d）。" % (method_a, landed_a, expect_len), flush=True)

        title_filled = _fill_title(frame, editor_page, title)
        saved_a, confirmed_a = _save_draft(editor_page)
        print("[壹伴] 第一段保存：%s%s" % ("已点保存" if saved_a else "没找到保存键",
              "，已见保存回执。" if confirmed_a else "，未见明确回执（编辑器多半已自动保存）。"), flush=True)

        phase_text = {"injected": True, "method": method_a, "chars": landed_a,
                      "title_filled": title_filled, "save_clicked": saved_a,
                      "save_confirmed": confirmed_a}

        if text_only:
            print(json.dumps({
                "ok": True, "mode": "publish", "theme": None, "text_only": True,
                "phase_text": phase_text,
                "note": "纯文字已入草稿（--text-only）。要套样式随时跑："
                        "wechat_yiban.py --mode restyle --theme " + theme +
                        "（先在窗口里打开这篇草稿）。窗口已留好，绝不自动发布。",
            }, ensure_ascii=False))
            print("[壹伴] 文字已落草稿。窗口保持打开，套样式可随后用 restyle 模式。", flush=True)
            return

        # ━━ 第二段：离屏套主题 → 全选粘贴回编辑器（仍走 ProseMirror）→ 再保存 ━━
        # 这段失败不影响第一段——文字已经安全在草稿里。
        print("[壹伴] 第二段：套「%s」主题样式……" % theme, flush=True)
        phase_style = {"applied": False, "method": None, "save_confirmed": False}
        try:
            styled = frame.evaluate(JS_OFFSCREEN_STYLE, {"body": body_html, "theme": theme})
            ok_b, method_b, landed_b = _inject_html(frame, body_sel, styled, expect_len)
            if ok_b:
                saved_b, confirmed_b = _save_draft(editor_page)
                phase_style = {"applied": True, "method": method_b, "chars": landed_b,
                               "save_clicked": saved_b, "save_confirmed": confirmed_b}
                print("[壹伴] 样式已套上（通道=%s）%s" % (method_b,
                      "，已见保存回执。" if confirmed_b else "。"), flush=True)
            else:
                print("[壹伴] 套样式注入未通过校验——文字版草稿不受影响。可稍后用 restyle 重试。", flush=True)
        except Exception as e:
            print("[壹伴] 套样式出错（%s）——文字版草稿不受影响。可稍后用 restyle 重试。" % e, flush=True)

        # 成品 HTML 永远落盘一份（用户想手动贴/外部预览都用得上）
        fallback_path = None
        try:
            fb = ctx.new_page()
            full = _styled_html(fb, body_html, theme)
            with open(save_fallback, "w", encoding="utf-8") as f:
                f.write(full)
            fallback_path = os.path.abspath(save_fallback)
            try:
                fb.close()
            except Exception:
                pass
        except Exception:
            pass

        print(json.dumps({
            "ok": True, "mode": "publish", "theme": theme,
            "phase_text": phase_text, "phase_style": phase_style,
            "styled_preview": fallback_path,
            "note": "第一段：文字已入草稿" +
                    ("（已见保存回执）" if confirmed_a else "") + "；第二段：" +
                    ("样式已套上。" if phase_style["applied"]
                     else "样式未成功——文字不受影响，可用 restyle 模式重试/换主题。") +
                    " 图片请在后台核对/重传；窗口已留待你确认后自行发布。绝不自动发布。",
        }, ensure_ascii=False))
        print("[壹伴] 完成。窗口保持打开，核对无误后请自行发布。", flush=True)
    finally:
        # publish 模式刻意不 close()，把窗口留给用户；进程结束由上层管。
        pass


# ───────────────────────── 模式三：restyle（对已打开的草稿原地换主题——「上传完再改格式」）─────────────────────────
def run_restyle(theme, timeout):
    os.makedirs(SESSION_DIR, exist_ok=True)
    ctx = launch_persistent_context(user_data_dir=SESSION_DIR, headless=False, humanize=True)
    page = ctx.new_page() if hasattr(ctx, "new_page") else ctx.pages[0]
    page.goto(MP_HOME, wait_until="domcontentloaded")

    print("[壹伴] 换肤模式：请在窗口里打开「草稿箱」→ 点要改的那篇草稿进入编辑器，"
          "脚本会自动接管换成「%s」主题（最多等 %ds）。" % (theme, timeout), flush=True)
    # 千万不能 auto_click_entry——那会新建一篇空文章，而不是改既有草稿
    pg, frame, body_sel = _wait_editor(
        ctx, timeout, auto_click_entry=False,
        hint="请打开草稿箱里要换肤的那篇草稿，脚本会自动接管。")
    if not frame:
        print(json.dumps({
            "ok": False, "mode": "restyle",
            "reason": "超时未等到草稿编辑器——请先在窗口里打开要改的草稿，再重跑本命令。",
        }, ensure_ascii=False))
        return

    try:
        current = frame.eval_on_selector(body_sel, JS_GET_HTML)
        expect_len = len(_plain_text(current))
        if expect_len < 5:
            print(json.dumps({"ok": False, "mode": "restyle",
                              "reason": "编辑器正文是空的——确认打开的是有内容的草稿。"},
                             ensure_ascii=False))
            return
        # 离屏：剥旧样式（normalize 在 STYLIZE_JS 里）→ 套新主题 → 粘贴回编辑器
        styled = frame.evaluate(JS_OFFSCREEN_STYLE, {"body": current, "theme": theme})
        ok, method, landed = _inject_html(frame, body_sel, styled, expect_len)
        if not ok:
            print(json.dumps({
                "ok": False, "mode": "restyle", "theme": theme,
                "reason": "换肤注入未通过校验（落入 %d / 预期 %d）——原草稿内容未动，可直接重试。"
                          % (landed, expect_len),
            }, ensure_ascii=False))
            return
        saved, confirmed = _save_draft(pg)
        print(json.dumps({
            "ok": True, "mode": "restyle", "theme": theme,
            "method": method, "chars": landed,
            "save_clicked": saved, "save_confirmed": confirmed,
            "note": "已换成「" + theme + "」主题" +
                    ("，已见保存回执。" if confirmed else "，未见明确回执——请在窗口里确认一下保存状态。") +
                    " 不满意再跑一次换别的主题即可（幂等，不会叠样式）。绝不自动发布。",
        }, ensure_ascii=False))
        print("[壹伴] 换肤完成。窗口保持打开，请核对后自行保存/发布。", flush=True)
    finally:
        # 同 publish：窗口留给用户
        pass


# ───────────────────────── 模式五/六：长图链路（snapshot 截切片 → publish-image 贴图入草稿）─────────────────────────
# 思路：把渲染权拿回自己手里。成品 HTML 在我们的 CloakBrowser 里渲染到像素级完美 → 全页截长图
# （在**段落空隙处**切片，不切断文字行）→ 公众号编辑器只当图床（粘贴图片是它原生欢迎的操作，
# 零清洗、零字数改写、零样式剥离）。
SNAP_WIDTH = 720       # 视口 CSS 宽：对齐公众号 677px 显示宽，左右留白
SNAP_SCALE = 2         # 2x 导出，手机端不糊
SLICE_MAX_CSS = 2800   # 单张切片最大 CSS 像素高（@2x≈5600 设备像素）；超长单图客户端会压糊

# 量内容块边界（页面坐标），切点选在块与块之间 = 段落空隙
JS_CUT_POINTS = r"""
() => {
  var box = document.getElementById("polaris-snap-root") || document.body;
  // 下钻"单子链":快照容器里通常还包着内容 wrapper(甚至主题背景层),
  // 一路钻到第一个有多个块级孩子的容器,才是真正的段落层。
  var hops = 0;
  while (box.children.length === 1 && hops < 5) { box = box.children[0]; hops += 1; }
  var sy = window.scrollY || 0;
  var r = (document.getElementById("polaris-snap-root") || document.body).getBoundingClientRect();
  var bottoms = [];
  Array.prototype.forEach.call(box.children, function (el) {
    bottoms.push(el.getBoundingClientRect().bottom + sy);
  });
  return { top: r.top + sy, bottom: r.bottom + sy, bottoms: bottoms };
}
"""


def _plan_cuts(top, bottom, bottoms, max_h):
    """贪心切片：每段尽量贴近 max_h，但切点必须落在某个块的底边（段落空隙）；
    单块超过 max_h 才不得不硬切。返回 [(y0, y1), ...] 页面坐标。"""
    cuts = []
    bl = sorted(b for b in bottoms if top < b < bottom)
    start = top
    while bottom - start > max_h:
        target = start + max_h
        cand = [b for b in bl if start + 200 < b <= target]
        cut = max(cand) if cand else target
        cuts.append((start, cut))
        start = cut
    cuts.append((start, bottom))
    return [(round(a), round(b)) for a, b in cuts if b - a > 4]


def run_snapshot(body_html, theme, out_dir, base_name, raw_file=None, no_slice=False):
    """成品 HTML → 全页长图切片。输出：成品 .html + 切片 .png × N + manifest.json。
    不碰公众号后台，纯本地确定性。
    raw_file：直截模式——给定的就是带完整样式的 HTML 文档（PRD/落地页/任何网页），
    不套主题不改一字，原样渲染原样截。"""
    os.makedirs(out_dir, exist_ok=True)
    browser = launch(headless=True, humanize=False)
    try:
        if raw_file:
            html_path = os.path.abspath(raw_file)
        else:
            # ① 离屏套样式拿成品（同 render 管线）
            p0 = browser.new_page()
            full = _styled_html(p0, body_html, theme)
            try:
                p0.close()
            except Exception:
                pass
            # 包一层可定位的快照容器（body 去 margin，背景白，避免截出杂边）
            inner = full.split(">", 2)[-1]  # 去掉 doctype+meta 前缀，保留内容 div
            snap_html = ("<!doctype html><meta charset='utf-8'>"
                         "<body style='margin:0;background:#ffffff;'>"
                         "<div id='polaris-snap-root'>" + inner + "</div></body>")
            html_path = os.path.join(out_dir, base_name + ".html")
            with open(html_path, "w", encoding="utf-8") as f:
                f.write(snap_html)

        # ② 打开成品页（file:// 才能加载本地配图），量块边界算切点
        page = browser.new_page(viewport={"width": SNAP_WIDTH, "height": 900},
                                device_scale_factor=SNAP_SCALE)
        page.goto("file:///" + os.path.abspath(html_path).replace(os.sep, "/"))
        page.wait_for_timeout(900)  # 等字体/图片
        info = page.evaluate(JS_CUT_POINTS)
        if no_slice:
            # --no-slice：一张到底（Q3）。技术可行(整页约 1.7MB 远低于单图 10MB 上限)，
            # 代价=手机端对超长单图整体缩放可能压糊，故非默认；想要时显式开。
            cuts = [(info["top"], info["bottom"])]
        else:
            cuts = _plan_cuts(info["top"], info["bottom"], info["bottoms"], SLICE_MAX_CSS)

        # ③ 逐段 clip 截图（clip 用页面 CSS 坐标，可超出视口）
        slices = []
        for i, (y0, y1) in enumerate(cuts, 1):
            sp_ = os.path.join(out_dir, "%s-%02d.png" % (base_name, i))
            # full_page=True + clip：先全页成像再按页面坐标裁切——只有这样才能裁到视口外
            page.screenshot(path=sp_, full_page=True,
                            clip={"x": 0, "y": y0, "width": SNAP_WIDTH, "height": y1 - y0})
            slices.append(os.path.abspath(sp_))
            print("[壹伴] 切片 %d/%d：%d~%dpx" % (i, len(cuts), y0, y1), flush=True)

        manifest = {"slices": slices, "html": os.path.abspath(html_path),
                    "theme": theme, "width_css": SNAP_WIDTH, "scale": SNAP_SCALE}
        with open(os.path.join(out_dir, "manifest.json"), "w", encoding="utf-8") as f:
            json.dump(manifest, f, ensure_ascii=False, indent=2)
        print(json.dumps({"ok": True, "mode": "snapshot", "count": len(slices), **manifest},
                         ensure_ascii=False))
    finally:
        try:
            browser.close()
        except Exception:
            pass
        pw = getattr(browser, "_pw", None)
        if pw:
            pw.stop()


# 粘贴图片进编辑器：File→DataTransfer→合成 paste。和用户截图后 Ctrl+V 完全同路，
# 编辑器自己负责上传素材库+插入正文——这是它原生欢迎的操作，不会被清洗。
JS_PASTE_IMAGE = r"""
async (root, args) => {
  var res = await fetch(args.dataUrl);
  var blob = await res.blob();
  var file = new File([blob], args.name, { type: blob.type || "image/png" });
  var dt = new DataTransfer();
  dt.items.add(file);
  root.focus();
  var doc = root.ownerDocument, win = doc.defaultView || window;
  try {
    var sel = win.getSelection(); var range = doc.createRange();
    range.selectNodeContents(root); range.collapse(false);
    sel.removeAllRanges(); sel.addRange(range);
  } catch (e) {}
  root.dispatchEvent(new ClipboardEvent("paste", { clipboardData: dt, bubbles: true, cancelable: true }));
  return true;
}
"""

JS_IMG_STATS = r"""
(root) => {
  var imgs = root.querySelectorAll("img");
  var up = 0;
  imgs.forEach(function (im) {
    if ((im.getAttribute("src") || "").indexOf("http") === 0) up += 1;
  });
  return { total: imgs.length, uploaded: up };
}
"""


def _confirm_upload_dialog(page):
    """传图后可能弹素材/确认框：best-effort 点可见的「确定/插入/完成」。点不到就算了。"""
    for txt in ("确定", "插入", "完成"):
        for sel in ("button:has-text('%s')" % txt, "a:has-text('%s')" % txt):
            try:
                el = page.query_selector(sel)
                if el and el.is_visible():
                    el.click()
                    page.wait_for_timeout(1000)
                    return True
            except Exception:
                continue
    return False


def _wait_img(frame, body_sel, want_n, seconds, page=None):
    """等编辑器收图到第 want_n 张（顺手点掉弹出的确认框）。优先等到换成 http 外链。"""
    deadline = time.time() + seconds
    landed = False
    while time.time() < deadline:
        if page is not None:
            _confirm_upload_dialog(page)
        try:
            st = frame.eval_on_selector(body_sel, JS_IMG_STATS)
        except Exception:
            st = None
        if st and st["total"] >= want_n:
            landed = True
            if st["uploaded"] >= want_n:
                break
        time.sleep(1.5)
    return landed


def _upload_via_input(editor_page, frame, body_sel, fp, want_n):
    """图片走「文件输入」通道：①页面常驻的 input[type=file] 直喂；
    ②工具栏「图片」按钮 + 文件选择器。合成粘贴对文件无效时的真通道。"""
    # ① 现成 file input（编辑器页常驻隐藏 input,set_input_files 直接触发其 change 上传逻辑）
    for fr in list(editor_page.frames):
        try:
            inputs = fr.query_selector_all("input[type=file]")
        except Exception:
            continue
        for inp in inputs:
            try:
                inp.set_input_files(fp)
            except Exception:
                continue
            if _wait_img(frame, body_sel, want_n, 35, page=editor_page):
                return "file-input"
    # ② 工具栏「图片」按钮 → 系统文件选择器（Playwright 拦截直喂）
    btn, _ = _first(editor_page, SELECTORS["img_button"])
    if btn:
        try:
            with editor_page.expect_file_chooser(timeout=5000) as fc:
                btn.click()
            fc.value.set_files(fp)
            if _wait_img(frame, body_sel, want_n, 60, page=editor_page):
                return "file-chooser"
        except Exception:
            pass
    return None


def run_publish_image(slices_dir, title, intro, timeout):
    """长图入草稿：开编辑器 →（可选）先放一段真文字导语 → 切片按序粘贴 → 填标题 → 存草稿。
    每张贴完都等编辑器真把图收下（img 出现/换成 mmbiz 外链）再贴下一张。绝不自动发布。"""
    import base64

    if not slices_dir or not os.path.isdir(slices_dir):
        print(json.dumps({"ok": False, "reason": "--slices-dir 不存在：%s" % slices_dir},
                         ensure_ascii=False))
        return
    # manifest 优先（保证顺序与 snapshot 一致），否则按文件名排序
    files = []
    mf = os.path.join(slices_dir, "manifest.json")
    if os.path.exists(mf):
        try:
            files = [p for p in json.load(open(mf, encoding="utf-8"))["slices"]
                     if os.path.exists(p)]
        except Exception:
            files = []
    if not files:
        files = sorted(os.path.join(slices_dir, n) for n in os.listdir(slices_dir)
                       if n.lower().endswith((".png", ".jpg", ".jpeg")))
    if not files:
        print(json.dumps({"ok": False, "reason": "目录里没有切片图：%s" % slices_dir},
                         ensure_ascii=False))
        return

    os.makedirs(SESSION_DIR, exist_ok=True)
    ctx = launch_persistent_context(user_data_dir=SESSION_DIR, headless=False, humanize=True)
    try:
        page = ctx.new_page() if hasattr(ctx, "new_page") else ctx.pages[0]
        page.goto(MP_HOME, wait_until="domcontentloaded")
        if not _is_logged_in(ctx):
            print("[壹伴] 未检测到登录态——请在窗口里扫码登录，脚本会自动进编辑器（最多等 %ds）。"
                  % timeout, flush=True)
        pg, frame, body_sel = _wait_editor(
            ctx, timeout, auto_click_entry=True,
            hint="正在打开图文编辑器……（若没自动打开，请手动点「写图文」）")
        if not frame:
            print(json.dumps({
                "ok": False, "mode": "publish-image",
                "reason": "超时未找到编辑器。切片仍在磁盘：%s ——窗口已留好，可把图直接拖进编辑器手动完成。"
                          % slices_dir}, ensure_ascii=False))
            return
        editor_page = pg

        # 真文字导语放最前（摘要/搜一搜/转发预览全靠它）——趁编辑器还空整体写入
        if intro:
            esc = (intro.replace("&", "&amp;").replace("<", "&lt;").replace(">", "&gt;"))
            intro_html = "<p>" + esc + "</p>"
            ok_i, m_i, _n = _inject_html(frame, body_sel, intro_html,
                                         len(_plain_text(intro_html)))
            print("[壹伴] 导语%s（通道 %s）。" % ("已写入" if ok_i else "写入失败,跳过", m_i),
                  flush=True)

        # 通道策略：先用首图实测「合成粘贴」灵不灵（文字粘贴灵≠文件粘贴灵——编辑器对
        # 文件 paste 校验更严）；不灵就全程切「文件输入」通道（input 直喂/文件选择器）。
        ok_count = 0
        use_paste = True
        for i, fp in enumerate(files, 1):
            print("[壹伴] 贴第 %d/%d 张：%s" % (i, len(files), os.path.basename(fp)), flush=True)
            landed_via = None
            if use_paste:
                try:
                    with open(fp, "rb") as fh:
                        b64 = base64.b64encode(fh.read()).decode("ascii")
                    mime = "image/jpeg" if fp.lower().endswith((".jpg", ".jpeg")) else "image/png"
                    frame.eval_on_selector(body_sel, JS_PASTE_IMAGE,
                                           {"dataUrl": "data:%s;base64,%s" % (mime, b64),
                                            "name": os.path.basename(fp)})
                    if _wait_img(frame, body_sel, i, 25, page=editor_page):
                        landed_via = "paste"
                except Exception:
                    pass
                if not landed_via:
                    use_paste = False
                    print("[壹伴] 粘贴通道对图片无效，切换「文件输入」通道。", flush=True)
            if not landed_via:
                landed_via = _upload_via_input(editor_page, frame, body_sel, fp, i)
            if landed_via:
                ok_count += 1
                print("[壹伴] 第 %d 张已落位（通道 %s）。" % (i, landed_via), flush=True)
            else:
                print("[壹伴] 第 %d 张三条通道都没成——请把 %s 手动拖进编辑器补上。" % (i, fp),
                      flush=True)

        title_filled = _fill_title(frame, editor_page, title)
        saved, confirmed = _save_draft(editor_page)
        print(json.dumps({
            "ok": ok_count == len(files), "mode": "publish-image",
            "images_total": len(files), "images_ok": ok_count,
            "title_filled": title_filled, "save_clicked": saved, "save_confirmed": confirmed,
            "note": "长图已按序贴入（%d/%d 张确认落位）%s。窗口留着请核对图序与清晰度，确认后自行发布。绝不自动发布。"
                    % (ok_count, len(files),
                       "，已点保存草稿" if saved else "，请手动保存草稿"),
        }, ensure_ascii=False))
    finally:
        # 同 publish：窗口留给用户核对
        pass


# ───────────────────────── 模式四：panel（可视化排版面板——右侧模板墙 + AI 改风格）─────────────────────────
def _resolve_claude():
    """找本机 claude CLI（AI 改风格用）。Polaris 环境医生装的是原生 exe；npm 装的是 .cmd。"""
    for name in ("claude.exe", "claude.cmd", "claude.bat", "claude"):
        p = shutil.which(name)
        if p:
            return p
    for c in (os.path.expanduser("~/.local/bin/claude.exe"),
              os.path.join(os.environ.get("APPDATA", ""), "npm", "claude.cmd")):
        if c and os.path.exists(c):
            return c
    return None


def _ai_theme(instruction, current_json):
    """用大白话指令喊 claude 生成主题 JSON。prompt 走 stdin（UTF-8，避开 Windows argv 乱码/32k 上限）。"""
    exe = _resolve_claude()
    if not exe:
        return {"error": "没找到 claude CLI——AI 改风格要用 Polaris 装好的 claude。模板换肤不受影响。"}
    prompt = (
        "你是公众号排版主题设计师。基于「当前主题」和「用户要求」，生成一个新的主题参数 JSON。\n\n"
        "当前主题（可能是预设名或参数对象）：" + current_json + "\n"
        "用户要求：" + instruction + "\n\n"
        "可用字段（都可选，缺省继承当前值）：\n"
        "- accent 主色 / text 正文色 / quoteBg 引用底色 / quoteBd 引用边色 / quoteTx 引用文字色（CSS 颜色）\n"
        "- size 正文字号(px 数字, 14~17) / lh 行高(数字, 1.6~2.0) / hFont 标题字体族\n"
        "- h2Mode 小标题形态：bar(左竖条)/underline(下划线)/pill(实色胶囊白字)/center(居中字距)/block(浅色块)/tag(描边圆标签)\n"
        "- bg 整体背景色（要纸纹/米色/浅色底就设它，会按块铺设；不要深色——公众号正文区是白底生态）\n"
        "- overrides 微调表：{\"css选择器\": \"追加的内联样式\"}，如 {\"h2\": \"background:#eef;border-radius:4px;padding:4px 10px;border-left:0\"}\n\n"
        "硬约束：只能用内联样式语义（公众号会剥 class/<style>）；不要 position/script/动画；颜色对比度够读。\n"
        "只输出一个 JSON 对象，不要任何解释、不要代码围栏。"
    )
    cmd = [exe, "-p", "--output-format", "text"]
    if exe.lower().endswith((".cmd", ".bat")):
        cmd = ["cmd", "/c"] + cmd
    try:
        r = subprocess.run(cmd, input=prompt.encode("utf-8"), capture_output=True, timeout=180)
    except Exception as e:
        return {"error": "claude 调用失败：%s" % e}
    out = (r.stdout or b"").decode("utf-8", "ignore")
    m = re.search(r"\{[\s\S]*\}", out)
    if not m:
        return {"error": "AI 没回出主题 JSON（输出开头：%s）" % out[:120].strip()}
    try:
        theme = json.loads(m.group(0))
    except Exception:
        return {"error": "AI 回的 JSON 解析失败——再说一次试试。"}
    return {"theme": theme}


def run_panel(timeout):
    """可视化排版面板：开 CloakBrowser → 用户打开草稿/写图文 → 往编辑器页面注入右侧面板。
    面板内：主题模板一点换肤（直接改活 DOM，像浏览器插件改 HTML）、AI 大白话改风格、清除样式、
    保存草稿。本进程常驻两件事：（重）注入面板 + 轮询 AI 请求（页面变量握手，不依赖 expose_function
    ——CloakBrowser 某些版本不支持它）。窗口全关才退出。"""
    os.makedirs(SESSION_DIR, exist_ok=True)
    ctx = launch_persistent_context(user_data_dir=SESSION_DIR, headless=False, humanize=True)
    page = ctx.new_page() if hasattr(ctx, "new_page") else ctx.pages[0]
    page.goto(MP_HOME, wait_until="domcontentloaded")

    print("[壹伴] 排版面板模式已启动。请在窗口里登录（如需）→ 打开草稿箱里的文章或「写图文」，"
          "右侧会自动出现「北极星·排版面板」：点模板换肤、或用大白话让 AI 改风格。", flush=True)
    print("[壹伴] 关掉浏览器窗口即结束本面板会话。", flush=True)

    boot = _panel_boot_js()
    announced = False
    idle_since = time.time()
    while True:
        try:
            pages = list(ctx.pages)
        except Exception:
            break
        if not pages:
            break
        pg, frame, _sel = _find_editor(ctx)
        if pg is not None:
            try:
                has = pg.evaluate("() => !!document.getElementById('polaris-yiban-panel')")
            except Exception:
                has = True  # 页面正在导航等瞬态，下轮再说
            if not has:
                try:
                    pg.evaluate(boot)
                    print("[壹伴] 面板已注入编辑器页面（右侧）。", flush=True)
                    announced = True
                except Exception:
                    pass
            # —— AI 请求轮询：页面把指令放进 window.__polarisAI.pending，这里取走→喊 claude→回填 ——
            req = None
            try:
                req = pg.evaluate(
                    "() => { var q = window.__polarisAI;"
                    " if (q && q.pending) { var p = q.pending; q.pending = null;"
                    "   return JSON.stringify({ instr: p, cur: window.__polarisCurrent || '墨韵' }); }"
                    " return null; }")
            except Exception:
                pass
            if req:
                try:
                    data = json.loads(req)
                except Exception:
                    data = None
                if data and data.get("instr"):
                    print("[壹伴] AI 改风格请求：%s" % data["instr"], flush=True)
                    res = _ai_theme(data["instr"], json.dumps(data.get("cur"), ensure_ascii=False))
                    print("[壹伴] %s" % ("AI 失败：" + res["error"] if "error" in res
                                         else "AI 主题已生成，面板正在套用。"), flush=True)
                    try:
                        pg.evaluate("(raw) => window.__polarisAIResult && window.__polarisAIResult(raw)",
                                    json.dumps(res, ensure_ascii=False))
                    except Exception:
                        pass
            idle_since = time.time()
        elif not announced and time.time() - idle_since > 60:
            print("[壹伴] 还没等到编辑器——请打开一篇草稿或点「写图文」，面板会自动出现。", flush=True)
            idle_since = time.time()
        time.sleep(2)

    print(json.dumps({"ok": True, "mode": "panel",
                      "note": "浏览器窗口已关闭，排版面板会话结束。草稿以后台保存的为准。"},
                     ensure_ascii=False))


def _publish_failed(ctx, body_html, theme, save_fallback, reason):
    """编辑器定位/注入失败的兜底：复用已打开的 ctx 开一个新标签跑壹伴引擎产出成品 HTML 存盘，
    提示用户手动全选复制兜底。复用 ctx（而非另起 launch）才不会在 publish 的 asyncio loop 里
    再开一个同步 Playwright 触发 'Sync API inside the asyncio loop'。"""
    try:
        fb = ctx.new_page()
        full = _styled_html(fb, body_html, theme)
        with open(save_fallback, "w", encoding="utf-8") as f:
            f.write(full)
        try:
            fb.close()
        except Exception:
            pass
    except Exception as e:
        print(json.dumps({"ok": False, "reason": reason, "render_fallback_error": str(e)},
                         ensure_ascii=False))
        return
    print(json.dumps({
        "ok": False, "mode": "publish", "reason": reason,
        "fallback_html": os.path.abspath(save_fallback),
        "note": "已把套好「" + theme + "」风格的成品 HTML 存到 fallback_html；"
                "请在已打开的 CloakBrowser 窗口里手动进入图文编辑器，"
                "用该 HTML（浏览器打开→全选复制）兜底贴入，保存草稿。绝不自动发布。",
    }, ensure_ascii=False))


def main():
    ap = argparse.ArgumentParser(description="Polaris 壹伴排版引擎 v8（公众号·两段解耦 + 面板 + 长图链路）")
    ap.add_argument("--mode", choices=["render", "publish", "restyle", "panel",
                                       "snapshot", "publish-image"], default="publish")
    ap.add_argument("--body-file", default="", help="干净语义正文 HTML（render/publish/snapshot 必填）")
    ap.add_argument("--theme", default="墨韵", help="风格预设：墨韵/极简/科技蓝/杂志/清新绿/活力橙/米纸/黛青")
    ap.add_argument("--title", default="", help="文章标题（publish/publish-image 填进后台；snapshot 用作切片文件名）")
    ap.add_argument("--out", default="", help="render 模式输出文件路径 / publish 成品落盘路径")
    ap.add_argument("--out-dir", default="", help="snapshot 切片输出目录（缺省=正文旁的 长图切片-主题/）")
    ap.add_argument("--slices-dir", default="", help="publish-image 的切片目录（即 snapshot 的 out-dir）")
    ap.add_argument("--intro", default="", help="publish-image 开头插的真文字导语（利于摘要/搜一搜）")
    ap.add_argument("--raw", action="store_true",
                    help="snapshot 直截模式：--body-file 给的是带完整样式的 HTML 文档（PRD/网页），不套主题原样截")
    ap.add_argument("--text-only", action="store_true",
                    help="publish 只跑第一段（纯文字入草稿），样式稍后用 restyle/panel 套")
    ap.add_argument("--no-slice", action="store_true",
                    help="snapshot 一张到底：不切片，整页截成单张长图（默认切片以保手机端清晰）")
    ap.add_argument("--timeout", type=int, default=300, help="等登录+编辑器的秒数（默认 300）")
    args = ap.parse_args()

    if args.mode == "panel":
        run_panel(args.timeout)
        return
    if args.mode == "restyle":
        run_restyle(args.theme, args.timeout)
        return
    if args.mode == "publish-image":
        run_publish_image(args.slices_dir, args.title, args.intro, args.timeout)
        return

    if not args.body_file:
        print(json.dumps({"ok": False, "reason": "--body-file 必填（render/publish/snapshot 模式）"},
                         ensure_ascii=False))
        sys.exit(2)
    body = _read(args.body_file)

    if args.mode == "render":
        out = args.out or os.path.join(os.path.dirname(os.path.abspath(args.body_file)),
                                       "公众号排版-预览.html")
        run_render(body, args.theme, out)
    elif args.mode == "snapshot":
        out_dir = args.out_dir or os.path.join(
            os.path.dirname(os.path.abspath(args.body_file)),
            "长图切片-" + ("原样" if args.raw else args.theme))
        run_snapshot(body, args.theme, out_dir, args.title or "公众号长图",
                     raw_file=args.body_file if args.raw else None,
                     no_slice=args.no_slice)
    else:
        fallback = args.out or os.path.join(os.path.dirname(os.path.abspath(args.body_file)),
                                            "公众号排版-成品兜底.html")
        run_publish(body, args.theme, args.title, fallback, args.text_only, args.timeout)


if __name__ == "__main__":
    main()
