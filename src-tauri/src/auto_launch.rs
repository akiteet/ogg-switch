use crate::error::AppError;
use auto_launch::{AutoLaunch, AutoLaunchBuilder};

/// 获取 macOS 上的 .app bundle 路径
/// 将 `/path/to/OGG Switch.app/Contents/MacOS/OGG Switch` 转换为 `/path/to/OGG Switch.app`
#[cfg(target_os = "macos")]
fn get_macos_app_bundle_path(exe_path: &std::path::Path) -> Option<std::path::PathBuf> {
    let path_str = exe_path.to_string_lossy();
    // 查找 .app/Contents/MacOS/ 模式
    if let Some(app_pos) = path_str.find(".app/Contents/MacOS/") {
        let app_bundle_end = app_pos + 4; // ".app" 的结束位置
        Some(std::path::PathBuf::from(&path_str[..app_bundle_end]))
    } else {
        None
    }
}

/// 初始化 AutoLaunch 实例
fn get_auto_launch() -> Result<AutoLaunch, AppError> {
    let app_name = "OGG Switch";
    let exe_path =
        std::env::current_exe().map_err(|e| AppError::Message(format!("无法获取应用路径: {e}")))?;

    // macOS 需要使用 .app bundle 路径，否则 AppleScript login item 会打开终端
    #[cfg(target_os = "macos")]
    let app_path = get_macos_app_bundle_path(&exe_path).unwrap_or(exe_path);

    #[cfg(not(target_os = "macos"))]
    let app_path = exe_path;

    // 使用 AutoLaunchBuilder 消除平台差异
    // macOS: 使用 AppleScript 方式（默认），需要 .app bundle 路径
    // Windows/Linux: 使用注册表/XDG autostart
    let auto_launch = AutoLaunchBuilder::new()
        .set_app_name(app_name)
        .set_app_path(&app_path.to_string_lossy())
        .build()
        .map_err(|e| AppError::Message(format!("创建 AutoLaunch 失败: {e}")))?;

    Ok(auto_launch)
}

/// 启用开机自启
pub fn enable_auto_launch() -> Result<(), AppError> {
    let auto_launch = get_auto_launch()?;
    auto_launch
        .enable()
        .map_err(|e| AppError::Message(format!("启用开机自启失败: {e}")))?;
    log::info!("已启用开机自启");
    Ok(())
}

/// 禁用开机自启
pub fn disable_auto_launch() -> Result<(), AppError> {
    let auto_launch = get_auto_launch()?;
    auto_launch
        .disable()
        .map_err(|e| AppError::Message(format!("禁用开机自启失败: {e}")))?;
    log::info!("已禁用开机自启");
    Ok(())
}

/// 检查是否已启用开机自启
pub fn is_auto_launch_enabled() -> Result<bool, AppError> {
    let auto_launch = get_auto_launch()?;
    auto_launch
        .is_enabled()
        .map_err(|e| AppError::Message(format!("检查开机自启状态失败: {e}")))
}

/// 自愈清理：删除历史版本残留在注册表里的死自启链。
///
/// 旧产品（Grok Switch 时代）用 `wscript.exe <配置目录>\autostart.vbs` 做静默
/// 自启；该 .vbs 被删除后，每次用户登录 Windows Script Host 都会弹
/// 「无法找到脚本文件」错误框（用户实测）。当前 OGG Switch 用 auto-launch
/// crate 直接把 exe 路径写进注册表，从不创建 .vbs，因此凡是引用
/// `\.grok-switch\autostart.vbs` 且文件已不存在的 Run 条目都是死链，可安全删除。
/// 只匹配该组合条件，避免误删用户自建的其他启动项。
#[cfg(target_os = "windows")]
pub fn cleanup_stale_autostart_entries() {
    use winreg::enums::{RegType, HKEY_CURRENT_USER, KEY_READ, KEY_SET_VALUE};
    use winreg::RegKey;

    const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
    const STALE_MARKER: &str = r"\.grok-switch\autostart.vbs";

    let hkcu = match RegKey::predef(HKEY_CURRENT_USER)
        .open_subkey_with_flags(RUN_KEY, KEY_READ | KEY_SET_VALUE)
    {
        Ok(k) => k,
        Err(e) => {
            log::debug!("打开 Run 注册表键失败，跳过自启死链清理: {e}");
            return;
        }
    };

    for (name, value) in hkcu.enum_values().flatten() {
        // 只看字符串型值；winreg 的 RegValue 是 { bytes, vtype } 结构体，
        // SZ / EXPAND_SZ 经 to_string() 还原为字符串
        let data = match value.vtype {
            RegType::REG_SZ | RegType::REG_EXPAND_SZ => value.to_string(),
            _ => continue,
        };
        // 大小写不敏感匹配路径片段；文件不存在才删（文件还在则说明可能仍被使用）
        if !data.to_lowercase().contains(&STALE_MARKER.to_lowercase()) {
            continue;
        }
        let script_exists = data
            .split_whitespace()
            .find_map(|token| {
                let token = token.trim_matches('"');
                token
                    .to_lowercase()
                    .ends_with("autostart.vbs")
                    .then(|| std::path::PathBuf::from(token))
            })
            .is_some_and(|p| p.exists());
        if script_exists {
            log::info!("启动项 {name} 引用的 autostart.vbs 仍存在，保留不清理");
            continue;
        }
        match hkcu.delete_value(&name) {
            Ok(()) => log::info!("已清理历史残留的死自启链（{name} → {data}）"),
            Err(e) => log::warn!("清理死自启链 {name} 失败: {e}"),
        }
    }
}

/// 在用户桌面创建应用快捷方式。
/// - Windows：用 PowerShell 的 WScript.Shell 生成 .lnk，图标指向 exe 自身。
/// - macOS：在 ~/Desktop 建一个指向 .app bundle 的符号链接。
/// - Linux：写入 ~/Desktop/ogg-switch.desktop。
/// 返回快捷方式的完整路径；已存在则覆盖。
pub fn create_desktop_shortcut() -> Result<String, AppError> {
    let exe_path =
        std::env::current_exe().map_err(|e| AppError::Message(format!("无法获取应用路径: {e}")))?;

    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x08000000;

        let desktop =
            dirs::desktop_dir().ok_or_else(|| AppError::Message("无法定位桌面目录".to_string()))?;
        let lnk = desktop.join("OGG Switch.lnk");
        // WScript.Shell 生成 .lnk；PowerShell 单引号字符串里转义单引号。
        let target = exe_path.to_string_lossy().replace('\'', "''");
        let workdir = exe_path
            .parent()
            .map(|p| p.to_string_lossy().replace('\'', "''"))
            .unwrap_or_default();
        let lnk_str = lnk.to_string_lossy().replace('\'', "''");
        let script = format!(
            "$s=(New-Object -ComObject WScript.Shell).CreateShortcut('{lnk_str}');\
$s.TargetPath='{target}';\
$s.WorkingDirectory='{workdir}';\
$s.IconLocation='{target},0';\
$s.Description='OGG Switch';\
$s.Save()"
        );
        let output = std::process::Command::new("powershell")
            .args([
                "-NoProfile",
                "-NonInteractive",
                "-ExecutionPolicy",
                "Bypass",
                "-Command",
                &script,
            ])
            .creation_flags(CREATE_NO_WINDOW)
            .output()
            .map_err(|e| AppError::Message(format!("创建桌面快捷方式失败: {e}")))?;
        if !output.status.success() {
            return Err(AppError::Message(format!(
                "创建桌面快捷方式失败: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            )));
        }
        return Ok(lnk.to_string_lossy().to_string());
    }

    #[cfg(target_os = "macos")]
    {
        let desktop =
            dirs::desktop_dir().ok_or_else(|| AppError::Message("无法定位桌面目录".to_string()))?;
        let bundle = get_macos_app_bundle_path(&exe_path).unwrap_or(exe_path);
        let link = desktop.join("OGG Switch.app");
        let _ = std::fs::remove_file(&link);
        std::os::unix::fs::symlink(&bundle, &link)
            .map_err(|e| AppError::Message(format!("创建桌面快捷方式失败: {e}")))?;
        return Ok(link.to_string_lossy().to_string());
    }

    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    {
        let desktop =
            dirs::desktop_dir().ok_or_else(|| AppError::Message("无法定位桌面目录".to_string()))?;
        let file = desktop.join("ogg-switch.desktop");
        let body = format!(
            "[Desktop Entry]\nType=Application\nName=OGG Switch\nExec={}\nIcon={}\nTerminal=false\n",
            exe_path.display(),
            exe_path.display()
        );
        std::fs::write(&file, body)
            .map_err(|e| AppError::Message(format!("创建桌面快捷方式失败: {e}")))?;
        return Ok(file.to_string_lossy().to_string());
    }
}

#[cfg(test)]
mod tests {
    #[allow(unused_imports)]
    use super::*;

    #[cfg(target_os = "macos")]
    #[test]
    fn test_get_macos_app_bundle_path_valid() {
        let exe_path =
            std::path::Path::new("/Applications/OGG Switch.app/Contents/MacOS/OGG Switch");
        let result = get_macos_app_bundle_path(exe_path);
        assert_eq!(
            result,
            Some(std::path::PathBuf::from("/Applications/OGG Switch.app"))
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn test_get_macos_app_bundle_path_with_spaces() {
        let exe_path =
            std::path::Path::new("/Users/test/My Apps/OGG Switch.app/Contents/MacOS/OGG Switch");
        let result = get_macos_app_bundle_path(exe_path);
        assert_eq!(
            result,
            Some(std::path::PathBuf::from(
                "/Users/test/My Apps/OGG Switch.app"
            ))
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn test_get_macos_app_bundle_path_not_in_bundle() {
        let exe_path = std::path::Path::new("/usr/local/bin/cc-switch");
        let result = get_macos_app_bundle_path(exe_path);
        assert_eq!(result, None);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn test_get_macos_app_bundle_path_dev_build() {
        // 开发环境下的路径通常不在 .app bundle 内
        let exe_path = std::path::Path::new("/Users/dev/project/target/debug/cc-switch");
        let result = get_macos_app_bundle_path(exe_path);
        assert_eq!(result, None);
    }
}
