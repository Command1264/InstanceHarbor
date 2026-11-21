#[link(name = "dwmapi")]
unsafe extern "system" {
    fn DwmGetColorizationColor(color: *mut u32, opaque_blend: *mut i32) -> i32;
}

/// Read the current Windows accent RGB. The DWM call does not change theme or registry state.
pub fn current() -> Result<String, String> {
    let mut argb = 0_u32;
    let mut opaque_blend = 0_i32;
    let status = unsafe { DwmGetColorizationColor(&mut argb, &mut opaque_blend) };
    if status != 0 {
        return Err(format!("WindowsAccentUnavailable: HRESULT {status:#010x}"));
    }
    Ok(format_color(argb))
}

fn format_color(argb: u32) -> String {
    format!("#{:06x}", argb & 0x00ff_ffff)
}

#[cfg(test)]
mod tests {
    #[test]
    fn colorization_alpha_is_not_part_of_css_color() {
        assert_eq!(super::format_color(0xc4881798), "#881798");
    }
}
