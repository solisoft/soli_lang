//! OS protocol-handler helpers for desktop deep links.
//!
//! Generates install scripts next to a built artifact so `myapp://path` launches
//! the executable with that URL (consumed by [`super::deeplink`]).

use std::fs;
use std::path::{Path, PathBuf};

/// Write platform registration scripts beside `exe`.
///
/// - Linux: `<scheme>.desktop` + `register-protocol.sh`
/// - Windows: `register-protocol.ps1` / `.reg` snippet
/// - macOS: notes only (use a native shell / Info.plist CFBundleURLTypes)
pub fn write_registration_helpers(
    exe: &Path,
    scheme: &str,
    app_name: &str,
) -> Result<Vec<PathBuf>, String> {
    let scheme = sanitize_scheme(scheme)?;
    let dir = exe
        .parent()
        .ok_or_else(|| "executable path has no parent directory".to_string())?;
    let exe_abs = exe.canonicalize().unwrap_or_else(|_| exe.to_path_buf());
    let exe_path = exe_abs.display().to_string();
    let mut written = Vec::new();

    // Linux desktop file
    let desktop_path = dir.join(format!("{scheme}.desktop"));
    let desktop = format!(
        "[Desktop Entry]\n\
         Name={app_name}\n\
         Comment={app_name} (Soli desktop)\n\
         Exec=\"{exe_path}\" %u\n\
         Type=Application\n\
         Terminal=false\n\
         Categories=Utility;\n\
         MimeType=x-scheme-handler/{scheme};\n"
    );
    fs::write(&desktop_path, desktop)
        .map_err(|e| format!("cannot write {}: {}", desktop_path.display(), e))?;
    written.push(desktop_path.clone());

    let sh_path = dir.join("register-protocol.sh");
    let sh = format!(
        "#!/bin/bash\n\
         set -euo pipefail\n\
         DIR=\"$(cd \"$(dirname \"$0\")\" && pwd)\"\n\
         DESKTOP=\"$DIR/{scheme}.desktop\"\n\
         APP_DIR=\"${{XDG_DATA_HOME:-$HOME/.local/share}}/applications\"\n\
         mkdir -p \"$APP_DIR\"\n\
         install -m 644 \"$DESKTOP\" \"$APP_DIR/{scheme}.desktop\"\n\
         xdg-mime default {scheme}.desktop x-scheme-handler/{scheme}\n\
         update-desktop-database \"$APP_DIR\" 2>/dev/null || true\n\
         echo \"Registered {scheme}:// → {exe_path}\"\n"
    );
    fs::write(&sh_path, sh).map_err(|e| format!("cannot write {}: {}", sh_path.display(), e))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(&sh_path, fs::Permissions::from_mode(0o755));
    }
    written.push(sh_path);

    // Windows PowerShell
    let ps1_path = dir.join("register-protocol.ps1");
    let ps1 = format!(
        "$exe = \"{exe_path}\"\n\
         $base = \"HKCU:\\Software\\Classes\\{scheme}\"\n\
         New-Item -Path $base -Force | Out-Null\n\
         Set-ItemProperty -Path $base -Name \"(Default)\" -Value \"URL:{app_name}\"\n\
         Set-ItemProperty -Path $base -Name \"URL Protocol\" -Value \"\"\n\
         $cmd = Join-Path $base \"shell\\open\\command\"\n\
         New-Item -Path $cmd -Force | Out-Null\n\
         Set-ItemProperty -Path $cmd -Name \"(Default)\" -Value \"`\"$exe`\" `\"%1`\"\"\n\
         Write-Host \"Registered {scheme}:// -> $exe\"\n"
    );
    fs::write(&ps1_path, ps1).map_err(|e| format!("cannot write {}: {}", ps1_path.display(), e))?;
    written.push(ps1_path);

    let readme = dir.join("DEEP_LINKS.txt");
    let notes = format!(
        "Deep links for {app_name}\n\
         ========================\n\
         \n\
         Launch examples:\n\
           {exe_path} --open /pings/3\n\
           {exe_path} {scheme}://host/pings/3\n\
           SOLI_DESKTOP_OPEN=/dashboard {exe_path}\n\
         \n\
         Register the custom scheme on this machine:\n\
           Linux:   ./register-protocol.sh\n\
           Windows: .\\register-protocol.ps1\n\
           macOS:   prefer a native shell with CFBundleURLTypes, or open via URL handler app\n\
         \n\
         See docs: /docs/development-tools/desktop#deep-links\n"
    );
    fs::write(&readme, notes).map_err(|e| format!("cannot write {}: {}", readme.display(), e))?;
    written.push(readme);

    let _ = desktop_path;
    Ok(written)
}

fn sanitize_scheme(scheme: &str) -> Result<String, String> {
    let s = scheme.trim().to_lowercase();
    if s.is_empty() {
        return Err("scheme must not be empty".into());
    }
    if !s
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '+' || c == '-' || c == '.')
    {
        return Err(format!(
            "invalid scheme '{scheme}' (use letters, digits, +, -, .)"
        ));
    }
    if s.starts_with(|c: char| c.is_ascii_digit()) {
        return Err("scheme must not start with a digit".into());
    }
    Ok(s)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn writes_helpers() {
        let dir = tempfile::tempdir().unwrap();
        let exe = dir.path().join("myapp");
        fs::write(&exe, b"#!/bin/sh\n").unwrap();
        let paths = write_registration_helpers(&exe, "MyApp", "Demo").unwrap();
        assert!(paths.iter().any(|p| p.ends_with("myapp.desktop")));
        assert!(paths.iter().any(|p| p.ends_with("register-protocol.sh")));
        assert!(paths.iter().any(|p| p.ends_with("register-protocol.ps1")));
        let desktop = fs::read_to_string(dir.path().join("myapp.desktop")).unwrap();
        assert!(desktop.contains("x-scheme-handler/myapp"));
    }

    #[test]
    fn rejects_bad_scheme() {
        assert!(sanitize_scheme("").is_err());
        assert!(sanitize_scheme("has space").is_err());
        assert!(sanitize_scheme("9bad").is_err());
    }
}
