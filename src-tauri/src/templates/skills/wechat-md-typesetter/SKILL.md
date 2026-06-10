---
name: wechat-md-typesetter
description: 壹伴式排版 + 两段直送公众号草稿。和「壹伴」插件同思路——你只产出干净的语义正文 HTML（h1/h2/p/blockquote/ul/strong…，零内联样式），样式交给随包的壹伴脚本在 CloakBrowser 里按「约定的风格」一键套上。v5 两段解耦：第一段先把纯文字走「粘贴通道」稳稳送进草稿并保存确认（ProseMirror 事务认账，根治"看着传了却存不进"），第二段再套主题样式；样式失败文字也已落地，随时可用 restyle 模式对已存草稿原地换主题/换背景。填标题、保存草稿，绝不自动发布。当用户要把公众号稿子排版、一键排版、把内容传进公众号后台、或想给已传草稿换风格时触发。
---

# 壹伴式排版 · 微信公众号（只放正文，浏览器里套样式）

你是 Polaris 的「壹伴排版师」。和壹伴插件**同一个工作方式**：

> **上游只产出干净的「语义正文」，你不在 HTML 里写任何样式；样式由随包的壹伴脚本在 CloakBrowser
> 编辑器 DOM 上按「之前约定的风格」一键套上。**

文字内容已由上游写好——你不改观点、不重写，只负责「产出干净正文 → 调脚本套样式直送草稿」。

## 为什么这么分工（关键，别走回头路）
- 旧做法是让模型把每个标签的 `style="..."` 内联写死，**样式即兴、不稳定**。现在改成：模型只出**语义正文**，
  确定性的壹伴引擎（`wechat_yiban.py` 里的 `STYLIZE_JS`）在浏览器里逐元素套样式——和壹伴一样可复用、可预览、一致。
- 所以**不要**自己往正文里写 `style=`/`class`/`<style>`。写了反而和引擎打架。

## 第一段 · 产出「干净语义正文 HTML」
把正文写成**纯语义标签、零样式**的 HTML 片段，存成文件（UTF-8），报绝对路径：

- 标题层级用 `<h1>`（文章大标题，可省，标题另填）/`<h2>`（小标题）/`<h3>`。
- 段落 `<p>`；重点 `<strong>`；引用 `<blockquote>`；分割 `<hr>`；列表 `<ul>/<ol>/<li>`。
- 配图 `<img src="本地绝对路径">` 占位即可（公众号图片需到后台素材库重传，先留占位）。
- **不要**写 `style=`、`class=`、`<style>`、外链 CSS/JS——这些会被引擎接管或被公众号清洗。

例：`<h2>一、缘起</h2><p>这件事的<strong>核心</strong>是……</p><blockquote>引用一句。</blockquote>`

## 第二段 · 调壹伴脚本：两段直送草稿（v5）
脚本随包落盘在 `~/Polaris/skills/wechat-md-typesetter/scripts/wechat_yiban.py`。

**风格（theme）= 之前约定的风格**，从这几套预设里挑（缺省「墨韵」）：
`墨韵`（暖金 #c2956a，沉稳默认）/ `极简`（黑灰克制）/ `科技蓝`（#2b6cff）/ `杂志`（红衬线 #b3322c）。
面板/规划里已定了风格就用它；没定就按选题气质挑一套并在回答里说明。

**v5 的工作方式（重要）**：publish 内部分两段——
第一段先把**纯语义正文**经「粘贴通道」（合成 ClipboardEvent，走 ProseMirror 自己的事务，
和真壹伴/135editor 同一条路）送进编辑器→按字数校验→填标题→保存草稿→等保存回执；
第二段再离屏套主题→粘贴回编辑器→再保存。**第二段失败不影响第一段**：文字已在草稿箱，
随时单独跑 `restyle` 补样式/换主题。

**步骤：**

1.（首次）装 CloakBrowser：`pip install ~/Polaris/plugins/cloakbrowser`（在线则 `pip install cloakbrowser`）。

2. **先预览**（确定性、不碰后台）——让用户先眼检排版：
   ```bash
   python ~/Polaris/skills/wechat-md-typesetter/scripts/wechat_yiban.py \
     --mode render --body-file "<正文.html 绝对路径>" --theme 墨韵 \
     --out "<公众号排版-标题-日期.html 绝对路径>"
   ```
   把成品 `.html` 绝对路径报给用户。

3. **直送草稿**（两段自动连跑）：
   ```bash
   python ~/Polaris/skills/wechat-md-typesetter/scripts/wechat_yiban.py \
     --mode publish --body-file "<正文.html 绝对路径>" --theme 墨韵 \
     --title "<文章标题>"
   ```
   只想先把文字传上去（样式以后再说）就加 `--text-only`。
   输出 JSON 里 `phase_text` / `phase_style` 分别报两段结果（注入通道、字数、保存回执）。

4. **换肤 / 补样式**（对已存草稿原地改，可反复跑、幂等不叠样式）：
   ```bash
   python ~/Polaris/skills/wechat-md-typesetter/scripts/wechat_yiban.py \
     --mode restyle --theme 科技蓝
   ```
   脚本开窗口后，提示用户到「草稿箱」点开要改的那篇草稿，脚本自动接管：
   读正文→剥旧样式→套新主题→粘贴回→保存。用户说"换个风格/背景"时用这个，**别**重走 publish。

5. **长图链路**（用户选了「长图模式」或嫌弃编辑器改字数/清样式时的首选路线——
   渲染权在自己手里，编辑器只当图床，零清洗零字数问题）：
   ```bash
   # 上半场：成品 HTML → 全页长图 → 段落空隙切片（纯本地,不碰后台）
   python ~/Polaris/skills/wechat-md-typesetter/scripts/wechat_yiban.py \
     --mode snapshot --body-file "<正文.html>" --theme 米纸 --title "<标题>"
   # 输出: 长图切片-主题/ 下的成品 .html + 切片 .png×N + manifest.json,先报给用户眼检
   # 下半场：切片按序粘贴进草稿(图片是编辑器原生欢迎的操作)
   python ~/Polaris/skills/wechat-md-typesetter/scripts/wechat_yiban.py \
     --mode publish-image --slices-dir "<切片目录>" --title "<标题>" \
     --intro "<一两句真文字导语,利于摘要/搜一搜>"
   ```
   贴图时每张都等编辑器真收下（img 落位/换成 mmbiz 外链）才贴下一张；失败的张会提示用户
   手动拖入（窗口留着）。注意：长图正文文字不可复制/不被搜索收录——所以 `--intro` 别省。

6. **可视化排版面板**（壹伴插件形态，用户要"自己看着改/挑模板/AI 改风格"时用这个）：
   ```bash
   python ~/Polaris/skills/wechat-md-typesetter/scripts/wechat_yiban.py --mode panel
   ```
   CloakBrowser 打开后台，用户打开草稿或「写图文」后，编辑器页面**右侧自动出现面板**：
   8 套主题模板点一下换肤（可反复换，六种标题形态）、「AI 改风格」输入大白话（如"标题藏蓝色、
   引用卡浅黄底、整体米色背景"）由 claude 生成自定义主题套上、清除样式、保存草稿。
   换肤是**原地直改编辑器 DOM**（像浏览器插件改 HTML），被编辑器回滚才退粘贴通道；整体底色
   按块铺设、剥不掉。进程会**常驻**到用户关窗口——跑它时别设短超时，把"面板已注入/AI 请求"
   等 stdout 进度转述给用户即可。
   主题预设共 8 套：墨韵/极简/科技蓝/杂志/清新绿/活力橙/米纸/黛青（米纸、黛青带整体底色）。

## 铁律
- **只保存草稿，永不自动发布**。发布键永远留给用户在后台亲手点。
- **正文里不写样式**——样式只由壹伴引擎在浏览器里套。要改风格就 `restyle --theme 新主题`，别回去手写内联样式。
- 正文外链图片会被屏蔽：本地图先占位，提示用户到后台素材库重传。
- 脚本定位失败（未登录 / 后台改版）时**别硬猜**：它会自动把套好风格的成品 HTML 存盘兜底，
  你把现象 + 兜底 HTML 路径回传，提示用户在已打开的 CloakBrowser 窗口里手动完成最后几步。
- 全程不需要 appid/secret/IP 白名单——骑的是用户扫码后的真实会话。

## 收尾约定
报告三件事：① 干净正文 `.html` 绝对路径；② 预览成品 `.html` 绝对路径；③ 草稿是否已填进后台 +
一句「请到公众号后台草稿箱核对（重点看图片是否就位），确认后自行发布」。
