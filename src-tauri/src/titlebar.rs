//! 原生标题栏染色（Windows 11 DWM）—— 让系统标题栏与应用框面色调融为一体。
//!
//! 前端在主题应用/切换时调用：浅色传暖米框面色、黑夜传深空框面色，
//! 标题文字色一并联动。Win10 不支持该属性时调用返回失败，静默忽略
//! （保持系统默认标题栏，不影响其它功能）。

/// `#rrggbb` → Win32 COLORREF（0x00BBGGRR）
#[cfg(windows)]
fn colorref(hex: &str) -> Result<u32, String> {
    let h = hex.trim_start_matches('#');
    if h.len() != 6 {
        return Err(format!("非法颜色: {hex}"));
    }
    let r = u32::from_str_radix(&h[0..2], 16).map_err(|e| e.to_string())?;
    let g = u32::from_str_radix(&h[2..4], 16).map_err(|e| e.to_string())?;
    let b = u32::from_str_radix(&h[4..6], 16).map_err(|e| e.to_string())?;
    Ok((b << 16) | (g << 8) | r)
}

#[tauri::command]
pub fn set_titlebar_color(
    window: tauri::WebviewWindow,
    caption: String,
    text: String,
) -> Result<(), String> {
    #[cfg(windows)]
    {
        use windows_sys::Win32::Graphics::Dwm::DwmSetWindowAttribute;
        // windows-sys 的 DWMWINDOWATTRIBUTE 枚举未含这两个较新值，按文档取常量
        const DWMWA_CAPTION_COLOR: u32 = 35;
        const DWMWA_TEXT_COLOR: u32 = 36;
        let cap = colorref(&caption)?;
        let txt = colorref(&text)?;
        let hwnd = window.hwnd().map_err(|e| e.to_string())?.0 as *mut core::ffi::c_void;
        unsafe {
            DwmSetWindowAttribute(
                hwnd,
                DWMWA_CAPTION_COLOR,
                &cap as *const u32 as *const core::ffi::c_void,
                4,
            );
            DwmSetWindowAttribute(
                hwnd,
                DWMWA_TEXT_COLOR,
                &txt as *const u32 as *const core::ffi::c_void,
                4,
            );
        }
    }
    #[cfg(not(windows))]
    {
        let _ = (window, caption, text);
    }
    Ok(())
}
