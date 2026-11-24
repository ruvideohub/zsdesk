//! Classify targets before passing them to the Windows Shell.

/// AppsFolder identifiers need a namespace prefix. Absolute Windows paths, Shell
/// namespace paths and URI schemes must instead be passed to the Shell unchanged.
pub fn parsing_name(id: &str) -> String {
    let bytes = id.as_bytes();
    let drive_path = bytes.len() > 2 && bytes[1] == b':' && (bytes[2] == b'\\' || bytes[2] == b'/');
    let network_path = id.starts_with("\\\\") || id.starts_with("//");
    let scheme = id
        .split_once(':')
        .map(|(head, _)| {
            let mut b = head.bytes();
            // A single letter before ':' is a drive designator, not a URI scheme.
            head.len() > 1
                && b.next().is_some_and(|c| c.is_ascii_alphabetic())
                && b.all(|c| c.is_ascii_alphanumeric() || matches!(c, b'+' | b'-' | b'.'))
        })
        .unwrap_or(false);
    if drive_path || network_path || id.starts_with("::") || scheme {
        id.to_string()
    } else {
        format!("shell:AppsFolder\\{id}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn passes_files_and_windows_uri_schemes_directly() {
        for target in [
            r"C:\Program Files\App\app.exe",
            r"\\server\share\file.txt",
            r"\\?\C:\long\file.txt",
            "//server/share/file.txt",
            "shell:RecycleBinFolder",
            "::shell-item",
            "https://example.com/a",
            "mailto:hello@example.com",
            "whatsapp:",
            "ms-settings:display",
            "windowsdefender://threat",
            "Custom.App+v1:open",
        ] {
            assert_eq!(parsing_name(target), target, "{target}");
        }
    }

    #[test]
    fn keeps_apps_folder_ids_separate_from_targets() {
        for id in [
            "Chrome",
            "Microsoft.Windows.Explorer",
            "Microsoft.WindowsCalculator_8wekyb3d8bbwe!App",
            r"{GUID}\Example\app.exe",
            "C:relative",
            "bad_scheme:value",
        ] {
            assert_eq!(parsing_name(id), format!("shell:AppsFolder\\{id}"), "{id}");
        }
    }
}
