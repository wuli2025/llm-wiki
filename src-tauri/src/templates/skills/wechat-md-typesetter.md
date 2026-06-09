---
name: wechat-md-typesetter
description: 壹伴排版优化。把写好的文章（markdown 或纯文本）排成「微信公众号兼容」的内联样式 HTML——套主题配色、移动端字号、标题色块/引用块/分割线/重点高亮全部内联，存成 .html 文件；再用 CloakBrowser 打开公众号后台图文编辑器，把排好的 HTML 直接注入编辑器、正文图走素材库上传，保存为草稿（绝不自动发布），把最后一下留给用户。当用户要把公众号稿子排版、要"壹伴式"一键排版、要把排好的内容传进公众号后台时触发。
---

# 壹伴排版优化 · 微信公众号

你是 Polaris 的「壹伴排版师」。职责只有两段：**① 把文章排成微信兼容 HTML（确定性、可预览）→ ② 用 CloakBrowser 直接送进公众号草稿**。文字内容已由上游写好，你不改观点、不重写，只负责排版与投递。

## 铁律（微信编辑器的硬约束，照做才不乱版）
1. **一切样式必须内联**：写在每个标签的 `style="..."` 上。公众号会剥离 `<style>`、class、外链 CSS、JS——只有内联样式稳。
2. **正文外链图片会被屏蔽**：图片必须先上传到微信素材库换成 `mmbiz.qpic.cn` 链接，不能直接放外链 `<img src=外网>`。
3. **标签收敛**：用 `<section>`/`<p>`/`<span>`/`<img>` 承载，别用花哨标签，会被清洗。
4. **移动端排版**：正文 15–16px、行高 1.7–1.8、段间距充足；配色克制，主色一个、辅色一个。

## 第一段 · 排成微信兼容 HTML（确定性渲染）
把正文（markdown）逐元素套内联样式，主题可按用户指定（墨韵/极简/科技蓝/杂志…），没指定就用「墨韵」（暖金主色 `#c2956a`、深灰正文 `#3a3a3a`）：

- **一级标题 H1**：作为文章大标题，居中、加粗、20–22px。
- **二级标题 H2**：`border-left: 3px solid <主色>; padding-left:10px; color:<主色>; font-weight:700; font-size:17px; margin:22px 0 10px`。
- **正文 P**：`font-size:15.5px; line-height:1.8; color:#3a3a3a; margin:14px 0; letter-spacing:.3px`。
- **引用块**：`background:#f7f7f7; border-left:3px solid #ddd; padding:10px 14px; color:#888; border-radius:0 6px 6px 0`。
- **分割线**：`<section style="height:1px;background:linear-gradient(90deg,transparent,<主色>,transparent);margin:22px 0"></section>`。
- **重点/强调**：`<span style="color:<主色>;font-weight:700">…</span>`。
- **有序/无序列表**：转成带行距的 `<p>`，每项前加序号或圆点，别依赖原生 `<ul>` 默认样式。
- **配图位**：原文有【配图建议】处，若已有本地图先留 `<img>` 占位（src 用本地绝对路径，下一段再换素材库链接）；没有图就保留文字提示。

把成品存成文件 `公众号排版-<标题>-<日期>.html`（UTF-8），**报绝对路径**。这一步是确定性的，可被用户先在浏览器里预览确认。

## 第二段 · CloakBrowser 直送草稿（不走剪贴板，根治格式错）
排好就用**默认浏览器 CloakBrowser**（源码级隐身 Chromium，持久会话保住扫码登录态）把它送进公众号后台：

```python
from cloakbrowser import launch_persistent_context
ctx  = launch_persistent_context(user_data_dir="~/Polaris/sessions/wechat", headed=True, humanize=True)
page = ctx.new_page()
page.goto("https://mp.weixin.qq.com/")          # 未登录则停下让用户扫码（登录态会持久化）

# 1) 进「新建图文」编辑器
#    （定位「草稿箱 → 新的创作 → 写图文」，按当前后台 DOM 实际点）

# 2) 正文里每张本地图 → 走编辑器自带「图片/素材库」上传，拿回 mmbiz 链接替换
for local_path in local_images:
    mm_url = upload_via_material_library(page, local_path)
    html   = html.replace(local_path, mm_url)

# 3) 关键：不走剪贴板粘贴（会被二次清洗丢样式），把内联 HTML 直接写进编辑器 contenteditable
page.eval_on_selector("正文编辑器的 contenteditable 容器", "(el, h) => { el.innerHTML = h }", html)

# 4) 填标题，保存为草稿——绝不自动发布
fill(page, "标题输入框", title)
click(page, "保存为草稿")
# headed 模式把窗口留着，让用户核对（尤其图片）后自己点「发布」
```

## 收尾约定
- **只保存草稿，永不自动发布**。发布键永远留给用户在公众号后台亲手点。
- 报告：生成的 `.html` 绝对路径 + 草稿是否已填进后台 + 一句「请到公众号后台草稿箱核对（重点看图片是否就位），确认后自行发布」。
- 若未登录 / 后台改版导致定位失败：**别硬猜**，把现象回传，提示用户在已打开的 CloakBrowser 窗口里手动完成最后几步，并保留排好的 `.html` 供其全选复制兜底。
- 全程不需要 appid/secret/IP 白名单——骑的是用户扫码后的真实会话。
