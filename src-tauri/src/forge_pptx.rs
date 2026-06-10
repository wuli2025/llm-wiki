//! Polaris Forge · 纯 Rust .pptx 打包器(架构文档「自写 OOXML 零新依赖」的首个落地件)。
//!
//! 把一组幻灯图(deck 各页截图 PNG/JPG)打成**合法可打开的 .pptx**——每页一张全幅图。
//! 替掉旧管线的 pptxgenjs(Node)。**三平台同一份**:纯 Rust + zip,字节级一致,win/mac/docker
//! 产出完全相同。配合 `forge_screenshot`(chromium headless 截图)即可端到端 deck→pptx。
//!
//! 设计取舍:首版做「全幅图版式」(像素精确、稳)。隐形文本层 / 真可编辑文本框是架构 v2 的
//! 后续增强(ADR-012),接口预留在 build_pptx 的 per-slide 扩展点。

use serde_json::{json, Value};
use std::io::Write;
use std::path::Path;
use zip::write::SimpleFileOptions;

const NS_CT: &str = "http://schemas.openxmlformats.org/package/2006/content-types";
const NS_REL: &str = "http://schemas.openxmlformats.org/package/2006/relationships";
const NS_A: &str = "http://schemas.openxmlformats.org/drawingml/2006/main";
const NS_R: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";
const NS_P: &str = "http://schemas.openxmlformats.org/presentationml/2006/main";

/// 从 PNG 头(IHDR)读宽高(px)。非 PNG / 损坏 → None。纯 std,不引 image crate。
fn png_size(bytes: &[u8]) -> Option<(u32, u32)> {
    // 8 字节签名 + 4 长度 + 4 "IHDR" → width 在 16..20, height 在 20..24(大端)。
    if bytes.len() < 24 || &bytes[1..4] != b"PNG" || &bytes[12..16] != b"IHDR" {
        return None;
    }
    let w = u32::from_be_bytes([bytes[16], bytes[17], bytes[18], bytes[19]]);
    let h = u32::from_be_bytes([bytes[20], bytes[21], bytes[22], bytes[23]]);
    if w == 0 || h == 0 {
        None
    } else {
        Some((w, h))
    }
}

fn xml_decl() -> &'static str {
    "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\r\n"
}

/// 把图片列表打成 .pptx。返回 {ok, out, slides, slide_size_emu}。
pub fn build_pptx(image_paths: &[String], out_path: &str) -> Result<Value, String> {
    if image_paths.is_empty() {
        return Err("没有图片可打包".into());
    }
    // 读所有图片字节 + 推断版面尺寸(用首图宽高比, 高固定 7.5" = 6858000 EMU, 不失真)。
    let mut images: Vec<(Vec<u8>, String)> = Vec::new(); // (bytes, ext)
    let mut first_ratio: Option<f64> = None;
    for p in image_paths {
        let bytes = std::fs::read(p).map_err(|e| format!("读图失败 {p}: {e}"))?;
        let ext = Path::new(p)
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("png")
            .to_lowercase();
        if first_ratio.is_none() {
            if let Some((w, h)) = png_size(&bytes) {
                first_ratio = Some(w as f64 / h as f64);
            }
        }
        images.push((bytes, ext));
    }
    let cy: u64 = 6_858_000; // 7.5 inch
    let ratio = first_ratio.unwrap_or(16.0 / 9.0);
    let cx: u64 = (cy as f64 * ratio).round() as u64;
    let n = images.len();

    let file = std::fs::File::create(out_path).map_err(|e| format!("创建 {out_path} 失败: {e}"))?;
    let mut zip = zip::ZipWriter::new(file);
    let opt = SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);

    let put = |zip: &mut zip::ZipWriter<std::fs::File>, name: &str, data: &[u8]| -> Result<(), String> {
        zip.start_file(name, opt)
            .map_err(|e| format!("zip 写 {name} 失败: {e}"))?;
        zip.write_all(data).map_err(|e| format!("zip 写入 {name} 失败: {e}"))?;
        Ok(())
    };

    // ── [Content_Types].xml ──
    let mut ct = String::from(xml_decl());
    ct.push_str(&format!("<Types xmlns=\"{NS_CT}\">"));
    ct.push_str("<Default Extension=\"rels\" ContentType=\"application/vnd.openxmlformats-package.relationships+xml\"/>");
    ct.push_str("<Default Extension=\"xml\" ContentType=\"application/xml\"/>");
    ct.push_str("<Default Extension=\"png\" ContentType=\"image/png\"/>");
    ct.push_str("<Default Extension=\"jpeg\" ContentType=\"image/jpeg\"/>");
    ct.push_str("<Default Extension=\"jpg\" ContentType=\"image/jpeg\"/>");
    ct.push_str("<Override PartName=\"/ppt/presentation.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.presentationml.presentation.main+xml\"/>");
    ct.push_str("<Override PartName=\"/ppt/slideMasters/slideMaster1.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.presentationml.slideMaster+xml\"/>");
    ct.push_str("<Override PartName=\"/ppt/slideLayouts/slideLayout1.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.presentationml.slideLayout+xml\"/>");
    ct.push_str("<Override PartName=\"/ppt/theme/theme1.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.theme+xml\"/>");
    for i in 1..=n {
        ct.push_str(&format!("<Override PartName=\"/ppt/slides/slide{i}.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.presentationml.slide+xml\"/>"));
    }
    ct.push_str("</Types>");
    put(&mut zip, "[Content_Types].xml", ct.as_bytes())?;

    // ── _rels/.rels ──
    let rels = format!(
        "{}<Relationships xmlns=\"{NS_REL}\"><Relationship Id=\"rId1\" Type=\"{NS_R}/officeDocument\" Target=\"ppt/presentation.xml\"/></Relationships>",
        xml_decl()
    );
    put(&mut zip, "_rels/.rels", rels.as_bytes())?;

    // ── ppt/presentation.xml ── rId1=master, rId2..=slides
    let mut pres = String::from(xml_decl());
    pres.push_str(&format!("<p:presentation xmlns:a=\"{NS_A}\" xmlns:r=\"{NS_R}\" xmlns:p=\"{NS_P}\">"));
    pres.push_str("<p:sldMasterIdLst><p:sldMasterId id=\"2147483648\" r:id=\"rId1\"/></p:sldMasterIdLst>");
    pres.push_str("<p:sldIdLst>");
    for i in 1..=n {
        pres.push_str(&format!("<p:sldId id=\"{}\" r:id=\"rId{}\"/>", 255 + i, i + 1));
    }
    pres.push_str("</p:sldIdLst>");
    pres.push_str(&format!("<p:sldSz cx=\"{cx}\" cy=\"{cy}\"/><p:notesSz cx=\"6858000\" cy=\"9144000\"/></p:presentation>"));
    put(&mut zip, "ppt/presentation.xml", pres.as_bytes())?;

    // ── ppt/_rels/presentation.xml.rels ──
    let mut prels = String::from(xml_decl());
    prels.push_str(&format!("<Relationships xmlns=\"{NS_REL}\">"));
    prels.push_str(&format!("<Relationship Id=\"rId1\" Type=\"{NS_R}/slideMaster\" Target=\"slideMasters/slideMaster1.xml\"/>"));
    for i in 1..=n {
        prels.push_str(&format!("<Relationship Id=\"rId{}\" Type=\"{NS_R}/slide\" Target=\"slides/slide{i}.xml\"/>", i + 1));
    }
    prels.push_str(&format!("<Relationship Id=\"rId{}\" Type=\"{NS_R}/theme\" Target=\"theme/theme1.xml\"/>", n + 2));
    prels.push_str("</Relationships>");
    put(&mut zip, "ppt/_rels/presentation.xml.rels", prels.as_bytes())?;

    // ── theme / master / layout(最小可用)──
    put(&mut zip, "ppt/theme/theme1.xml", theme_xml().as_bytes())?;
    put(&mut zip, "ppt/slideMasters/slideMaster1.xml", slide_master_xml(cx, cy).as_bytes())?;
    put(
        &mut zip,
        "ppt/slideMasters/_rels/slideMaster1.xml.rels",
        format!(
            "{}<Relationships xmlns=\"{NS_REL}\"><Relationship Id=\"rId1\" Type=\"{NS_R}/slideLayout\" Target=\"../slideLayouts/slideLayout1.xml\"/><Relationship Id=\"rId2\" Type=\"{NS_R}/theme\" Target=\"../theme/theme1.xml\"/></Relationships>",
            xml_decl()
        )
        .as_bytes(),
    )?;
    put(&mut zip, "ppt/slideLayouts/slideLayout1.xml", slide_layout_xml(cx, cy).as_bytes())?;
    put(
        &mut zip,
        "ppt/slideLayouts/_rels/slideLayout1.xml.rels",
        format!(
            "{}<Relationships xmlns=\"{NS_REL}\"><Relationship Id=\"rId1\" Type=\"{NS_R}/slideMaster\" Target=\"../slideMasters/slideMaster1.xml\"/></Relationships>",
            xml_decl()
        )
        .as_bytes(),
    )?;

    // ── 每页:slideN.xml(全幅图)+ rels + 媒体 ──
    for (idx, (bytes, ext)) in images.iter().enumerate() {
        let i = idx + 1;
        let media_ext = if ext == "jpg" || ext == "jpeg" { "jpeg" } else { "png" };
        put(&mut zip, &format!("ppt/media/image{i}.{media_ext}"), bytes)?;
        put(&mut zip, &format!("ppt/slides/slide{i}.xml"), slide_xml(cx, cy).as_bytes())?;
        put(
            &mut zip,
            &format!("ppt/slides/_rels/slide{i}.xml.rels"),
            format!(
                "{}<Relationships xmlns=\"{NS_REL}\"><Relationship Id=\"rId1\" Type=\"{NS_R}/slideLayout\" Target=\"../slideLayouts/slideLayout1.xml\"/><Relationship Id=\"rId2\" Type=\"{NS_R}/image\" Target=\"../media/image{i}.{media_ext}\"/></Relationships>",
                xml_decl()
            )
            .as_bytes(),
        )?;
    }

    zip.finish().map_err(|e| format!("zip 收尾失败: {e}"))?;
    Ok(json!({
        "ok": true,
        "out": out_path,
        "slides": n,
        "slide_size_emu": { "cx": cx, "cy": cy }
    }))
}

fn slide_xml(cx: u64, cy: u64) -> String {
    format!(
        "{decl}<p:sld xmlns:a=\"{a}\" xmlns:r=\"{r}\" xmlns:p=\"{p}\"><p:cSld><p:spTree>\
<p:nvGrpSpPr><p:cNvPr id=\"1\" name=\"\"/><p:cNvGrpSpPr/><p:nvPr/></p:nvGrpSpPr>\
<p:grpSpPr><a:xfrm><a:off x=\"0\" y=\"0\"/><a:ext cx=\"0\" cy=\"0\"/><a:chOff x=\"0\" y=\"0\"/><a:chExt cx=\"0\" cy=\"0\"/></a:xfrm></p:grpSpPr>\
<p:pic><p:nvPicPr><p:cNvPr id=\"2\" name=\"Slide Image\"/><p:cNvPicPr><a:picLocks noChangeAspect=\"1\"/></p:cNvPicPr><p:nvPr/></p:nvPicPr>\
<p:blipFill><a:blip r:embed=\"rId2\"/><a:stretch><a:fillRect/></a:stretch></p:blipFill>\
<p:spPr><a:xfrm><a:off x=\"0\" y=\"0\"/><a:ext cx=\"{cx}\" cy=\"{cy}\"/></a:xfrm><a:prstGeom prst=\"rect\"><a:avLst/></a:prstGeom></p:spPr></p:pic>\
</p:spTree></p:cSld><p:clrMapOvr><a:masterClrMapping/></p:clrMapOvr></p:sld>",
        decl = xml_decl(), a = NS_A, r = NS_R, p = NS_P
    )
}

fn slide_layout_xml(_cx: u64, _cy: u64) -> String {
    format!(
        "{decl}<p:sldLayout xmlns:a=\"{a}\" xmlns:r=\"{r}\" xmlns:p=\"{p}\" type=\"blank\" preserve=\"1\"><p:cSld name=\"Blank\"><p:spTree>\
<p:nvGrpSpPr><p:cNvPr id=\"1\" name=\"\"/><p:cNvGrpSpPr/><p:nvPr/></p:nvGrpSpPr>\
<p:grpSpPr><a:xfrm><a:off x=\"0\" y=\"0\"/><a:ext cx=\"0\" cy=\"0\"/><a:chOff x=\"0\" y=\"0\"/><a:chExt cx=\"0\" cy=\"0\"/></a:xfrm></p:grpSpPr>\
</p:spTree></p:cSld><p:clrMapOvr><a:masterClrMapping/></p:clrMapOvr></p:sldLayout>",
        decl = xml_decl(), a = NS_A, r = NS_R, p = NS_P
    )
}

fn slide_master_xml(cx: u64, cy: u64) -> String {
    format!(
        "{decl}<p:sldMaster xmlns:a=\"{a}\" xmlns:r=\"{r}\" xmlns:p=\"{p}\"><p:cSld><p:bg><p:bgPr><a:solidFill><a:srgbClr val=\"FFFFFF\"/></a:solidFill><a:effectLst/></p:bgPr></p:bg><p:spTree>\
<p:nvGrpSpPr><p:cNvPr id=\"1\" name=\"\"/><p:cNvGrpSpPr/><p:nvPr/></p:nvGrpSpPr>\
<p:grpSpPr><a:xfrm><a:off x=\"0\" y=\"0\"/><a:ext cx=\"{cx}\" cy=\"{cy}\"/><a:chOff x=\"0\" y=\"0\"/><a:chExt cx=\"{cx}\" cy=\"{cy}\"/></a:xfrm></p:grpSpPr>\
</p:spTree></p:cSld>\
<p:clrMap bg1=\"lt1\" tx1=\"dk1\" bg2=\"lt2\" tx2=\"dk2\" accent1=\"accent1\" accent2=\"accent2\" accent3=\"accent3\" accent4=\"accent4\" accent5=\"accent5\" accent6=\"accent6\" hlink=\"hlink\" folHlink=\"folHlink\"/>\
<p:sldLayoutIdLst><p:sldLayoutId id=\"2147483649\" r:id=\"rId1\"/></p:sldLayoutIdLst>\
</p:sldMaster>",
        decl = xml_decl(), a = NS_A, r = NS_R, p = NS_P
    )
}

/// 最小但合法的 Office 主题(clrScheme/fontScheme/fmtScheme 三件齐全, PowerPoint 才认)。
fn theme_xml() -> String {
    format!("{decl}<a:theme xmlns:a=\"{a}\" name=\"Polaris\"><a:themeElements>\
<a:clrScheme name=\"Polaris\"><a:dk1><a:sysClr val=\"windowText\" lastClr=\"000000\"/></a:dk1><a:lt1><a:sysClr val=\"window\" lastClr=\"FFFFFF\"/></a:lt1>\
<a:dk2><a:srgbClr val=\"1F2230\"/></a:dk2><a:lt2><a:srgbClr val=\"EEF1F8\"/></a:lt2>\
<a:accent1><a:srgbClr val=\"7AA2F7\"/></a:accent1><a:accent2><a:srgbClr val=\"B794F6\"/></a:accent2><a:accent3><a:srgbClr val=\"5BE3B0\"/></a:accent3>\
<a:accent4><a:srgbClr val=\"FFD166\"/></a:accent4><a:accent5><a:srgbClr val=\"FF7B8A\"/></a:accent5><a:accent6><a:srgbClr val=\"3B6FE0\"/></a:accent6>\
<a:hlink><a:srgbClr val=\"0563C1\"/></a:hlink><a:folHlink><a:srgbClr val=\"954F72\"/></a:folHlink></a:clrScheme>\
<a:fontScheme name=\"Polaris\"><a:majorFont><a:latin typeface=\"Calibri Light\"/><a:ea typeface=\"\"/><a:cs typeface=\"\"/></a:majorFont><a:minorFont><a:latin typeface=\"Calibri\"/><a:ea typeface=\"\"/><a:cs typeface=\"\"/></a:minorFont></a:fontScheme>\
<a:fmtScheme name=\"Polaris\">\
<a:fillStyleLst><a:solidFill><a:schemeClr val=\"phClr\"/></a:solidFill><a:solidFill><a:schemeClr val=\"phClr\"/></a:solidFill><a:solidFill><a:schemeClr val=\"phClr\"/></a:solidFill></a:fillStyleLst>\
<a:lnStyleLst><a:ln w=\"6350\"><a:solidFill><a:schemeClr val=\"phClr\"/></a:solidFill></a:ln><a:ln w=\"12700\"><a:solidFill><a:schemeClr val=\"phClr\"/></a:solidFill></a:ln><a:ln w=\"19050\"><a:solidFill><a:schemeClr val=\"phClr\"/></a:solidFill></a:ln></a:lnStyleLst>\
<a:effectStyleLst><a:effectStyle><a:effectLst/></a:effectStyle><a:effectStyle><a:effectLst/></a:effectStyle><a:effectStyle><a:effectLst/></a:effectStyle></a:effectStyleLst>\
<a:bgFillStyleLst><a:solidFill><a:schemeClr val=\"phClr\"/></a:solidFill><a:solidFill><a:schemeClr val=\"phClr\"/></a:solidFill><a:solidFill><a:schemeClr val=\"phClr\"/></a:solidFill></a:bgFillStyleLst>\
</a:fmtScheme></a:themeElements></a:theme>",
        decl = xml_decl(), a = NS_A)
}

/// 用 chromium headless CLI 给一个 URL/本地 HTML 截图成 PNG。跨平台:容器走镜像 chromium,
/// win/mac 走 preflight 找到的 Chrome/Edge。这是 Forge capture 的确定性原始能力。
pub fn screenshot(url_or_file: &str, out_png: &str, width: u32, height: u32) -> Result<Value, String> {
    let chromium = crate::forge::find_chromium()
        .ok_or_else(|| "未找到 chromium/chrome：Docker 需 full 镜像，桌面需装 Chrome/Edge".to_string())?;
    // 本地文件转 file:// URL。
    let target = if url_or_file.starts_with("http://")
        || url_or_file.starts_with("https://")
        || url_or_file.starts_with("file://")
    {
        url_or_file.to_string()
    } else {
        let abs = std::fs::canonicalize(url_or_file)
            .map_err(|e| format!("找不到文件 {url_or_file}: {e}"))?;
        format!("file://{}", abs.to_string_lossy().replace('\\', "/"))
    };
    let status = std::process::Command::new(&chromium)
        .args([
            "--headless=new",
            "--no-sandbox",
            "--disable-dev-shm-usage",
            "--disable-gpu",
            "--hide-scrollbars",
            &format!("--screenshot={out_png}"),
            &format!("--window-size={width},{height}"),
            &target,
        ])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map_err(|e| format!("启动 chromium 失败: {e}"))?;
    if !status.success() || !Path::new(out_png).is_file() {
        return Err("chromium 截图失败(未生成 PNG)".into());
    }
    Ok(json!({ "ok": true, "out": out_png, "chromium": chromium }))
}

/// 数 deck.html 里的幻灯页数:统计 class 列表含独立 token `slide` 的元素(排除 slide-number 等)。
/// 与 runtime.js 的 `.deck > .slide` 结构一致。数不到则返回 0(调用方退化为整页一张)。
pub fn count_slides(html: &str) -> usize {
    let mut n = 0;
    for seg in html.split("class") {
        let s = seg.trim_start();
        let s = match s.strip_prefix('=') {
            Some(x) => x.trim_start(),
            None => continue,
        };
        let (q, rest) = match s.chars().next() {
            Some(c @ '"') => (c, &s[1..]),
            Some(c @ '\'') => (c, &s[1..]),
            _ => continue,
        };
        if let Some(end) = rest.find(q) {
            if rest[..end].split_whitespace().any(|t| t == "slide") {
                n += 1;
            }
        }
    }
    n
}

/// deck.html → 逐页截图到临时目录,返回 (帧目录, PNG 路径列表)。供 pptx / 视频共用。
/// 利用 runtime.js 的 `?export=1#/N` 深链(只有 .is-active 页可见,base.css 已防叠页)。
/// 多页 = 多次 chromium 进程(每页一次);CDP 批量复用单浏览器是后续优化(ADR-002),此版求稳。
pub fn capture_slides(
    deck: &str,
    width: u32,
    height: u32,
    slides_override: Option<usize>,
) -> Result<(std::path::PathBuf, Vec<String>), String> {
    let is_http = deck.starts_with("http://") || deck.starts_with("https://");
    let file_base = if is_http {
        deck.to_string()
    } else {
        let abs = std::fs::canonicalize(deck).map_err(|e| format!("找不到 deck {deck}: {e}"))?;
        format!("file://{}", abs.to_string_lossy().replace('\\', "/"))
    };
    let n = match slides_override {
        Some(n) if n > 0 => n,
        _ => {
            if is_http {
                1
            } else {
                count_slides(&std::fs::read_to_string(deck).unwrap_or_default()).max(1)
            }
        }
    };
    let frames = std::env::temp_dir().join(format!("forge_deck_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&frames);
    std::fs::create_dir_all(&frames).map_err(|e| format!("建临时帧目录失败: {e}"))?;
    let mut pngs: Vec<String> = Vec::new();
    for i in 1..=n {
        let png = frames.join(format!("slide-{i}.png"));
        let url = format!("{file_base}?export=1#/{i}");
        screenshot(&url, &png.to_string_lossy(), width, height)
            .map_err(|e| format!("第 {i} 页截图失败: {e}"))?;
        pngs.push(png.to_string_lossy().to_string());
    }
    Ok((frames, pngs))
}

/// deck.html → 多页 .pptx 一步到位(三平台同一份)。
pub fn render_deck_to_pptx(
    deck: &str,
    out_pptx: &str,
    width: u32,
    height: u32,
    slides_override: Option<usize>,
) -> Result<Value, String> {
    let (frames, pngs) = capture_slides(deck, width, height, slides_override)?;
    let n = pngs.len();
    let r = build_pptx(&pngs, out_pptx);
    let _ = std::fs::remove_dir_all(&frames);
    let r = r?;
    Ok(json!({ "ok": true, "out": out_pptx, "slides": n, "detail": r }))
}

#[cfg(test)]
mod tests {
    use super::*;

    // 原生验证打包器(在 cargo test 所在 OS 上跑——Windows 宿主即验 win 路径):
    // 喂任意字节当「图」(build_pptx 只为取尺寸才解析 PNG，非 PNG 退 16:9)，验产出是合法 zip 结构。
    #[test]
    fn build_pptx_produces_valid_package() {
        let dir = std::env::temp_dir().join("polaris_forge_pptx_test");
        let _ = std::fs::create_dir_all(&dir);
        let img1 = dir.join("a.png");
        let img2 = dir.join("b.png");
        std::fs::write(&img1, b"fake-image-bytes-1").unwrap();
        std::fs::write(&img2, b"fake-image-bytes-2").unwrap();
        let out = dir.join("out.pptx");
        let r = build_pptx(
            &[img1.to_string_lossy().into(), img2.to_string_lossy().into()],
            &out.to_string_lossy(),
        )
        .expect("build_pptx 应成功");
        assert_eq!(r["slides"], 2);
        assert!(out.is_file());
        // 重新打开 zip 验结构。
        let f = std::fs::File::open(&out).unwrap();
        let mut z = zip::ZipArchive::new(f).expect("产出应是合法 zip");
        let names: Vec<String> = (0..z.len())
            .map(|i| z.by_index(i).unwrap().name().to_string())
            .collect();
        for need in [
            "[Content_Types].xml",
            "ppt/presentation.xml",
            "ppt/slides/slide1.xml",
            "ppt/slides/slide2.xml",
            "ppt/theme/theme1.xml",
            "ppt/slideMasters/slideMaster1.xml",
            "ppt/slideLayouts/slideLayout1.xml",
        ] {
            assert!(names.iter().any(|n| n == need), "缺部件 {need}");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn count_slides_counts_slide_token_only() {
        let html = r#"<div class="deck"><section class="slide is-active"><div class="slide-number"></div></section><section class="slide"></section><section class='slide cover'></section></div>"#;
        assert_eq!(count_slides(html), 3); // 三个 .slide，不数 .slide-number / .deck
        assert_eq!(count_slides("<p class=\"slides foo\">x</p>"), 0); // slides ≠ slide
    }

    #[test]
    fn png_size_reads_ihdr() {
        // 1x1 PNG。
        let png: [u8; 24] = [
            0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a, // sig
            0, 0, 0, 13, b'I', b'H', b'D', b'R', // len + type
            0, 0, 0, 1, 0, 0, 0, 1, // w=1 h=1
        ];
        assert_eq!(png_size(&png), Some((1, 1)));
        assert_eq!(png_size(b"not a png at all really"), None);
    }
}
