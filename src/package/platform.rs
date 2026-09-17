use std::env::consts::{ARCH, OS};

/// Check if the target platform matches the package's `os` field in package.json.
/// npm rules:
/// - If `os` is empty or None, it matches all platforms.
/// - If `os` contains entries prefixed with `!`, it negates (e.g. `!win32`).
/// - Otherwise, at least one entry must match current platform.
pub fn is_os_supported(os_list: &Option<Vec<String>>) -> bool {
    let Some(list) = os_list else {
        return true;
    };
    if list.is_empty() {
        return true;
    }

    let current_os = normalize_os(OS);

    let has_negations = list.iter().any(|s| s.starts_with('!'));
    if has_negations {
        // Must not match any negation
        for item in list {
            if let Some(negated) = item.strip_prefix('!') {
                if negated == current_os {
                    return false;
                }
            }
        }
        true
    } else {
        // Must match at least one positive
        list.iter().any(|s| s == current_os)
    }
}

/// Check if the target CPU architecture matches the package's `cpu` field in package.json.
/// npm cpu names: `x64`, `arm64`, `ia32`, `arm`, etc.
pub fn is_cpu_supported(cpu_list: &Option<Vec<String>>) -> bool {
    let Some(list) = cpu_list else {
        return true;
    };
    if list.is_empty() {
        return true;
    }

    let current_cpu = normalize_cpu(ARCH);

    let has_negations = list.iter().any(|s| s.starts_with('!'));
    if has_negations {
        for item in list {
            if let Some(negated) = item.strip_prefix('!') {
                if negated == current_cpu {
                    return false;
                }
            }
        }
        true
    } else {
        list.iter().any(|s| s == current_cpu)
    }
}

fn normalize_os(os: &str) -> &str {
    match os {
        "windows" => "win32",
        "macos" => "darwin",
        "linux" => "linux",
        "freebsd" => "freebsd",
        "openbsd" => "openbsd",
        other => other,
    }
}

fn normalize_cpu(arch: &str) -> &str {
    match arch {
        "x86_64" => "x64",
        "aarch64" => "arm64",
        "x86" => "ia32",
        "arm" => "arm",
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_os_support() {
        assert!(is_os_supported(&None));
        assert!(is_os_supported(&Some(vec![])));

        #[cfg(windows)]
        {
            let win_os = Some(vec!["win32".to_string(), "linux".to_string()]);
            assert!(is_os_supported(&win_os));

            let not_win_os = Some(vec!["!win32".to_string()]);
            assert!(!is_os_supported(&not_win_os));
        }

        #[cfg(not(windows))]
        {
            let win_only_os = Some(vec!["win32".to_string()]);
            assert!(!is_os_supported(&win_only_os));

            let not_win_os = Some(vec!["!win32".to_string()]);
            assert!(is_os_supported(&not_win_os));
        }
    }

    #[test]
    fn test_cpu_support() {
        assert!(is_cpu_supported(&None));
        #[cfg(target_arch = "x86_64")]
        {
            let x64_cpu = Some(vec!["x64".to_string()]);
            assert!(is_cpu_supported(&x64_cpu));

            let not_x64 = Some(vec!["!x64".to_string()]);
            assert!(!is_cpu_supported(&not_x64));
        }

        #[cfg(not(target_arch = "x86_64"))]
        {
            let x64_only = Some(vec!["x64".to_string()]);
            assert!(!is_cpu_supported(&x64_only));

            let not_x64 = Some(vec!["!x64".to_string()]);
            assert!(is_cpu_supported(&not_x64));
        }
    }
}
