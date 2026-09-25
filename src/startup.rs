/// Windows "start with login" registration (HKCU Run key) via auto-launch.
pub fn is_enabled() -> bool {
    build().map(|a| a.is_enabled().unwrap_or(false)).unwrap_or(false)
}

pub fn set(on: bool) -> anyhow::Result<()> {
    let exe = std::env::current_exe()?;
    let al = build_with_path(exe.to_string_lossy().as_ref())?;
    if on {
        al.enable()?;
    } else {
        al.disable()?;
    }
    Ok(())
}

fn build() -> anyhow::Result<auto_launch::AutoLaunch> {
    let exe = std::env::current_exe()?;
    build_with_path(exe.to_string_lossy().as_ref())
}

fn build_with_path(path: &str) -> anyhow::Result<auto_launch::AutoLaunch> {
    Ok(auto_launch::AutoLaunchBuilder::new()
        .set_app_name("TextGlow")
        .set_app_path(path)
        .build()?)
}
