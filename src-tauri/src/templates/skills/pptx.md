# PPT 演示文稿模式

你处于「PPTX」模式，要**真正产出一个能打开的 .pptx 文件**，而不只是描述大纲。本模式由「做 PPT / 幻灯片 / 演示文稿」意图自动激活。

仿主流 AI PPT（豆包 / Gamma / 悟空）的打法：**大纲先行 → 选主题 → 按页型模板化渲染 → 直出 .pptx**。内容与设计分离——你只负责把内容填进结构化的 `SLIDES` 数组，配色版式交给主题库与页型库兜底。

## 铁律：必须落地一个文件，禁止静默失败
- 结束前**务必确认 .pptx 已写到磁盘**（用代码 `os.path.exists` + 文件大小 > 0 校验），确认后才说「已生成」。
- 任何一步失败（缺 Python / 装包失败 / 脚本报错），都要**用中文如实告诉用户卡在哪**，并立即走下面的兜底，绝不假装成功。

## 工作流总览（两段式，但绝不为「选题」而卡住）
1. **大纲先行**：先在对话里给一份结构化大纲（每页：页型 + 标题 + 要点），让用户一眼校准、好改。
2. **确认即渲染**：用户说「确认 / 可以 / 出吧」或指出要改哪页 → 改完直接渲染成 .pptx。
- **不要反问「你想做什么 / 给我个题目」**——哪怕只有一句话题目也自己拟好大纲。大纲是给用户**校准**用的，不是用来要选题的。这是「产不出 PPT」最大的坑。
- 用户一开始就说「直接做 / 不用确认」或已给了完整要求 → **跳过等待，大纲 + 渲染一气呵成**。

## 第 0 步 · 环境自检（先做，按结果分支）
.pptx 用 Python 库 `python-pptx` 生成。先探测可用的 Python：Windows 上依次试 `python`、`py`、`python3`；其它平台试 `python3`、`python`。

```bash
python --version || py --version || python3 --version
```

**分支 A — 有 Python**：确保 `python-pptx` 就绪，装包优先用国内镜像（用户多在国内，直连 PyPI 常超时）：
```bash
python -m pip install --quiet python-pptx pypdf -i https://pypi.tuna.tsinghua.edu.cn/simple
# 镜像失败再退默认源：
python -m pip install --quiet python-pptx pypdf
```
- `python-pptx`：生成 / 编辑 .pptx；`pypdf`：需要读 PDF 内容时再用。
- 装完用 `python -c "import pptx; print(pptx.__version__)"` 验证导入成功，再继续。

**分支 B — 没有 Python，或装包怎么都失败**：
1. **先用中文明确告诉用户**：「生成真正的 .pptx 需要 Python 环境（python-pptx），当前机器上没检测到 / 装不上，原因是 ___」。
2. 然后**用兜底方案先交付**：生成一个**单文件、自包含的 HTML 幻灯片**（16:9、键盘翻页、深色标题留白排版），存到产物目录，让用户立刻有东西用、可在侧边栏预览、也能打印成 PDF。
3. 末尾告诉用户：装好 Python 后我可以把这份内容**导出成真正的 .pptx**。不要因为缺环境就什么都不产出。

## 第 1 步 · 大纲先行
从用户的题目 / 文档 / 附件出发，**立刻**在对话里给一份大纲，逐页标注 **页型**（见下「页型库」），形如：

```
主题建议：商务墨蓝（business）  ·  共 9 页
1. [cover]   标题页 — 《2025 年中总结》/ 副标题
2. [toc]     目录 — 业绩 / 复盘 / 计划 …
3. [section] 章节 01 — 上半年业绩
4. [bignum]  关键数据 — 营收 1.2 亿（同比 +37%）
5. [bullets] 三大增长引擎 — 要点 1/2/3
6. [two_col] 复盘 — 做对了什么 vs 踩了什么坑
7. [quote]   一句话定调
8. [bullets] 下半年计划
9. [closing] 谢谢 / 联系方式
```
给完大纲一句话收尾：「确认就出 PPT，或告诉我改哪页 / 换主题」。给了 PDF/文档就先用 `pypdf` 抽文本、按章节切分映射到页。没指定页数默认 8–12 页。

## 第 2 步 · 选主题 + 选页型（内容与设计分离）
**主题库**（用户没指定就按内容气质自己选一个，并在大纲里写明，允许用户换）：

| id | 名称 | 适用 |
|---|---|---|
| `business` | 商务墨蓝（浅底） | 汇报 / 总结 / 路演（默认） |
| `tech_dark` | 科技深色 | 产品 / 技术 / 发布会 |
| `fresh` | 清新浅色 | 教育 / 文创 / 生活 |
| `academic` | 学术靛蓝 | 论文 / 评审 / 研究 |
| `bold` | 焦点信号（深底亮黄） | 宣言 / 营销 / 强冲击 |
| `mono` | 极简黑白 | 极简 / 设计感 |

**页型库**（每块内容挑最合适的页型，别全用 bullets 堆字）：
`cover` 封面 · `toc` 目录 · `section` 章节分隔 · `bullets` 要点页 · `two_col` 双栏对比 · `bignum` 数据大字 · `quote` 金句 · `closing` 结尾。

## 第 3 步 · 渲染 .pptx（可直接套用的引擎）
下面是**经过验证、能跑通**的引擎：主题库 `THEMES` + 页型渲染器 `RENDER` + 渲染循环。**你只需改 `THEME` 和 `SLIDES` 两个变量**填入大纲内容；保存路径换成**产物目录的绝对路径**。

```python
import os, zipfile, re, shutil
from pptx import Presentation
from pptx.util import Inches, Pt
from pptx.dml.color import RGBColor
from pptx.enum.text import PP_ALIGN
from pptx.enum.lang import MSO_LANGUAGE_ID  # 1.x 在 enum.lang（不是 enum.text）
from pptx.enum.shapes import MSO_SHAPE

def C(h):  # 0xRRGGBB → RGBColor
    return RGBColor((h >> 16) & 0xFF, (h >> 8) & 0xFF, h & 0xFF)

# ── 主题库：配色与字体的唯一来源（内容与设计分离）──
THEMES = {
    "business":  dict(bg=None,     ink=0x1A2A3A, accent=0x2C4661, sub=0x5B6B7B, band=0xEAF0F5),
    "tech_dark": dict(bg=0x0E1116, ink=0xF2F5F8, accent=0x4DA3FF, sub=0x9AA7B4, band=0x1A2230),
    "fresh":     dict(bg=0xFBF7EF, ink=0x1F2D2B, accent=0x2E9E8F, sub=0x6B7B77, band=0xE7F1ED),
    "academic":  dict(bg=0xFFFFFF, ink=0x16213A, accent=0x3A4FB0, sub=0x5B6470, band=0xEDEFF7),
    "bold":      dict(bg=0x111111, ink=0xFFFFFF, accent=0xFFD400, sub=0xB8B8B8, band=0x1E1E1E),
    "mono":      dict(bg=0xFFFFFF, ink=0x111111, accent=0xE5484D, sub=0x777777, band=0xF2F2F2),
}

def set_font(run, name="微软雅黑"):
    # 同时给 latin + eastAsian 语言 id，跨平台阅读器都有字可用（Mac 无微软雅黑会回退）
    run.font.name = name
    run.font.language_id = MSO_LANGUAGE_ID.SIMPLIFIED_CHINESE

def textbox(slide, l, t, w, h, text, size, *, bold=False, color=0x111111, align=PP_ALIGN.LEFT):
    tf = slide.shapes.add_textbox(Inches(l), Inches(t), Inches(w), Inches(h)).text_frame
    tf.word_wrap = True
    p = tf.paragraphs[0]; p.alignment = align
    r = p.add_run(); r.text = text
    r.font.size = Pt(size); r.font.bold = bold; r.font.color.rgb = C(color); set_font(r)
    return tf

def rect(slide, l, t, w, h, color):
    sp = slide.shapes.add_shape(MSO_SHAPE.RECTANGLE, Inches(l), Inches(t), Inches(w), Inches(h))
    sp.fill.solid(); sp.fill.fore_color.rgb = C(color); sp.line.fill.background()
    return sp

def bullets(slide, th, items, l=1.0, t=2.1, w=11.3, h=4.6, size=22):
    tf = slide.shapes.add_textbox(Inches(l), Inches(t), Inches(w), Inches(h)).text_frame
    tf.word_wrap = True
    for i, it in enumerate(items):
        p = tf.paragraphs[0] if i == 0 else tf.add_paragraph()
        r = p.add_run(); r.text = "• " + it
        r.font.size = Pt(size); r.font.color.rgb = C(th["ink"]); set_font(r)
        p.space_after = Pt(12)

def _new(prs, th):
    s = prs.slides.add_slide(prs.slide_layouts[6])  # 空白版式，全自排
    if th["bg"] is not None:
        s.background.fill.solid(); s.background.fill.fore_color.rgb = C(th["bg"])
    return s

# ── 页型渲染器：每个页型一个函数，吃 (prs, theme, dict) ──
def page_cover(prs, th, d):
    s = _new(prs, th); rect(s, 1.0, 3.45, 2.2, 0.12, th["accent"])
    textbox(s, 1.0, 2.3, 11.3, 1.4, d["title"], 46, bold=True, color=th["ink"])
    if d.get("subtitle"): textbox(s, 1.0, 3.8, 11.3, 0.8, d["subtitle"], 22, color=th["sub"])

def page_toc(prs, th, d):
    s = _new(prs, th)
    textbox(s, 1.0, 0.8, 11.3, 1.0, d.get("title", "目录"), 32, bold=True, color=th["accent"])
    tf = s.shapes.add_textbox(Inches(1.0), Inches(2.0), Inches(11.3), Inches(4.6)).text_frame
    for i, it in enumerate(d["items"]):
        p = tf.paragraphs[0] if i == 0 else tf.add_paragraph()
        r = p.add_run(); r.text = f"{i+1:02d}    {it}"
        r.font.size = Pt(24); r.font.color.rgb = C(th["ink"]); set_font(r); p.space_after = Pt(12)

def page_section(prs, th, d):
    s = _new(prs, th); rect(s, 0, 2.6, 13.333, 2.3, th["band"])
    textbox(s, 1.0, 2.7, 2.6, 2.0, d.get("no", "01"), 80, bold=True, color=th["accent"])
    textbox(s, 3.5, 3.15, 9.0, 1.4, d["title"], 40, bold=True, color=th["ink"])

def page_bullets(prs, th, d):
    s = _new(prs, th)
    textbox(s, 1.0, 0.7, 11.3, 1.0, d["title"], 32, bold=True, color=th["accent"])
    bullets(s, th, d["points"])

def page_two_col(prs, th, d):
    s = _new(prs, th)
    textbox(s, 1.0, 0.7, 11.3, 1.0, d["title"], 32, bold=True, color=th["accent"])
    textbox(s, 1.0, 1.9, 5.4, 0.7, d["left_title"], 22, bold=True, color=th["ink"])
    bullets(s, th, d["left"], l=1.0, t=2.6, w=5.4, h=3.8, size=18)
    textbox(s, 6.9, 1.9, 5.4, 0.7, d["right_title"], 22, bold=True, color=th["ink"])
    bullets(s, th, d["right"], l=6.9, t=2.6, w=5.4, h=3.8, size=18)

def page_bignum(prs, th, d):
    s = _new(prs, th)
    textbox(s, 1.0, 1.7, 11.3, 2.2, d["number"], 96, bold=True, color=th["accent"], align=PP_ALIGN.CENTER)
    textbox(s, 1.0, 4.3, 11.3, 1.0, d["caption"], 26, color=th["ink"], align=PP_ALIGN.CENTER)

def page_quote(prs, th, d):
    s = _new(prs, th)
    textbox(s, 1.4, 2.4, 10.5, 2.6, "“" + d["text"] + "”", 34, bold=True, color=th["ink"], align=PP_ALIGN.CENTER)
    if d.get("by"): textbox(s, 1.4, 5.0, 10.5, 0.7, "— " + d["by"], 20, color=th["sub"], align=PP_ALIGN.CENTER)

def page_closing(prs, th, d):
    s = _new(prs, th); rect(s, 1.0, 3.95, 2.2, 0.12, th["accent"])
    textbox(s, 1.0, 2.5, 11.3, 1.4, d.get("title", "谢谢观看"), 44, bold=True, color=th["ink"])
    if d.get("subtitle"): textbox(s, 1.0, 4.1, 11.3, 0.8, d["subtitle"], 22, color=th["sub"])

RENDER = {
    "cover": page_cover, "toc": page_toc, "section": page_section, "bullets": page_bullets,
    "two_col": page_two_col, "bignum": page_bignum, "quote": page_quote, "closing": page_closing,
}

# ════════ 只改这两个变量：主题 + 大纲内容 ════════
THEME = "business"
SLIDES = [
    {"type": "cover",   "title": "演示文稿标题", "subtitle": "副标题 / 作者 / 日期"},
    {"type": "toc",     "items": ["第一部分", "第二部分", "第三部分"]},
    {"type": "section", "no": "01", "title": "第一部分标题"},
    {"type": "bignum",  "number": "1.2 亿", "caption": "关键指标说明（同比 +37%）"},
    {"type": "bullets", "title": "要点页标题", "points": ["要点一：……", "要点二：……", "要点三：……"]},
    {"type": "two_col", "title": "对比页标题",
     "left_title": "做对了", "left": ["……", "……"],
     "right_title": "待改进", "right": ["……", "……"]},
    {"type": "quote",   "text": "一句话定调的金句", "by": "出处"},
    {"type": "closing", "title": "谢谢观看", "subtitle": "联系方式 / 二维码"},
]
# ════════════════════════════════════════════════

prs = Presentation()
prs.slide_width, prs.slide_height = Inches(13.333), Inches(7.5)  # 16:9
th = THEMES[THEME]
for d in SLIDES:
    RENDER[d["type"]](prs, th, d)

OUT = r"<产物目录绝对路径>/演示文稿.pptx"  # ← 换成已授权的产物目录
prs.save(OUT)

# ── 后处理：修复 python-pptx 已知兼容性问题（WPS/Mac/各种阅读器拒收）──
#   1) app.xml <Slides> 计数没更新  2) app.xml PresentationFormat 固定 4:3
#   3) presentation.xml sldSz type="screen4x3" 与 16:9 尺寸矛盾
#   4) slideLayout 里 p14:creationId（Office 2010 扩展，WPS Mac 不识别）
def fix_pptx(path):
    n = len(Presentation(path).slides)
    tmp = path + ".tmp"
    with zipfile.ZipFile(path, "r") as zin, zipfile.ZipFile(tmp, "w", zipfile.ZIP_DEFLATED) as zout:
        for info in zin.infolist():
            data = zin.read(info.filename); fn = info.filename
            is_layout = fn.startswith("ppt/slideLayouts/slideLayout") and fn.endswith(".xml")
            # 只对目标 XML 部件解码改写；其余部件（缩略图等二进制）原样拷贝，别全量 decode
            if fn in ("docProps/app.xml", "ppt/presentation.xml") or is_layout:
                txt = data.decode("utf-8")
                if fn == "docProps/app.xml":
                    txt = re.sub(r"<Slides>\d+</Slides>", f"<Slides>{n}</Slides>", txt)
                    txt = re.sub(r"<PresentationFormat>[^<]+</PresentationFormat>",
                                 "<PresentationFormat>On-screen Show (16:9)</PresentationFormat>", txt)
                elif fn == "ppt/presentation.xml":
                    txt = txt.replace('type="screen4x3"', '')
                else:
                    txt = re.sub(r"<p:extLst>.*?</p:extLst>", "", txt, flags=re.DOTALL)
                data = txt.encode("utf-8")
            zout.writestr(info, data)
    shutil.move(tmp, path)

fix_pptx(OUT)
assert os.path.exists(OUT) and os.path.getsize(OUT) > 0, "保存失败"
print("SAVED", OUT, os.path.getsize(OUT), "bytes")
```

- 需要图表时用 `python-pptx` 原生 chart；需要配图时配合 image-gen 技能（注意：当前供应商多半不支持真实生图，详见该技能）。
- 想加页型就再写一个 `page_xxx(prs, th, d)` 注册进 `RENDER`，渲染循环不用动。

## 输出
- 用中文说明演示结构与亮点（用了哪个主题、几页、各页型怎么分布）。
- 把 .pptx 产出到**已授权的产物目录**（绝对路径），并在末尾点明文件名与页数。
- 走了 HTML 兜底时，明确说这是「HTML 幻灯片」替代方案，以及如何升级成真 .pptx。
