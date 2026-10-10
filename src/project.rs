//! Path lookups inside a worktree.
//!
//! Extensions can only see files through `Worktree::read_text_file`, which
//! takes paths relative to the worktree root and fails for files Zed doesn't
//! index (for example anything under a gitignored `.fvm/` directory). The
//! helpers here therefore work on root-relative paths and take an `exists`
//! callback, so they can be tested without a worktree.

/// Files that mark a directory as pinned to a Flutter SDK by FVM: `.fvmrc`
/// (FVM 3+) and the legacy `.fvm/fvm_config.json`.
const FVM_CONFIG_FILES: &[&str] = &[".fvmrc", ".fvm/fvm_config.json"];

/// Returns `dir` and its ancestors up to the worktree root, nearest first.
/// `dir` is relative to the worktree root, and the root itself is `""`.
fn ancestors(dir: &str) -> impl Iterator<Item = &str> {
    let mut next = Some(dir.trim_matches('/'));
    std::iter::from_fn(move || {
        let current = next?;
        next = match current.rfind('/') {
            Some(index) => Some(&current[..index]),
            None if current.is_empty() => None,
            None => Some(""),
        };
        Some(current)
    })
}

fn join(dir: &str, file: &str) -> String {
    if dir.is_empty() {
        file.to_string()
    } else {
        format!("{dir}/{file}")
    }
}

/// Finds the nearest directory at or above `dir` that contains `pubspec.yaml`.
pub fn find_package_root(dir: &str, exists: impl Fn(&str) -> bool) -> Option<String> {
    ancestors(dir)
        .find(|candidate| exists(&join(candidate, "pubspec.yaml")))
        .map(str::to_string)
}

/// Finds the nearest directory at or above `dir` that is pinned to a Flutter
/// SDK by FVM.
pub fn find_fvm_project(dir: &str, exists: impl Fn(&str) -> bool) -> Option<String> {
    ancestors(dir)
        .find(|candidate| {
            FVM_CONFIG_FILES
                .iter()
                .any(|file| exists(&join(candidate, file)))
        })
        .map(str::to_string)
}

/// The path of an SDK tool (`dart`, `flutter`) in the SDK that FVM links into
/// `<fvm_project>/.fvm/flutter_sdk`.
pub fn fvm_tool_path(root: &str, fvm_project: &str, tool: &str) -> String {
    absolute(
        root,
        &join(fvm_project, &format!(".fvm/flutter_sdk/bin/{tool}")),
    )
}

/// Turns a root-relative path into an absolute one.
pub fn absolute(root: &str, relative: &str) -> String {
    let root = root.trim_end_matches('/');
    if relative.is_empty() {
        root.to_string()
    } else {
        format!("{root}/{relative}")
    }
}

/// Makes `path` relative to the worktree root. Relative paths are taken to
/// be relative to the root already; absolute paths outside the root give
/// `None`.
pub fn relative_to_root(root: &str, path: &str) -> Option<String> {
    if !path.starts_with('/') {
        return Some(
            path.trim_start_matches("./")
                .trim_end_matches('/')
                .to_string(),
        );
    }
    let root = root.trim_end_matches('/');
    let rest = path.strip_prefix(root)?;
    if rest.is_empty() {
        Some(String::new())
    } else {
        rest.strip_prefix('/')
            .map(|rest| rest.trim_end_matches('/').to_string())
    }
}

/// The directory part of a root-relative file path.
pub fn parent(path: &str) -> &str {
    path.rfind('/').map_or("", |index| &path[..index])
}

/// Strips a `?line=..` style selector that `dart test` and `flutter test`
/// accept after a test file path.
pub fn without_query(program: &str) -> &str {
    program.split_once('?').map_or(program, |(path, _)| path)
}

/// Whether `program` is a test, which the Dart and Flutter debug adapters
/// run in their `--test` mode. Uses the same file name rule as `dart test`.
pub fn is_test_file(program: &str) -> bool {
    without_query(program).ends_with("_test.dart")
}

/// Whether `program` is an integration test, which runs on a device unlike
/// other tests.
pub fn is_integration_test(program: &str) -> bool {
    let path = without_query(program);
    path.starts_with("integration_test/") || path.contains("/integration_test/")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn files(paths: &'static [&'static str]) -> impl Fn(&str) -> bool {
        move |path| paths.contains(&path)
    }

    #[test]
    fn test_ancestors() {
        assert_eq!(
            ancestors("app/bakai/lib").collect::<Vec<_>>(),
            ["app/bakai/lib", "app/bakai", "app", ""]
        );
        assert_eq!(ancestors("").collect::<Vec<_>>(), [""]);
    }

    #[test]
    fn test_find_package_root() {
        let exists = files(&["pubspec.yaml", "app/bakai/pubspec.yaml"]);
        assert_eq!(
            find_package_root("app/bakai/lib/src", &exists).as_deref(),
            Some("app/bakai")
        );
        assert_eq!(find_package_root("tool", &exists).as_deref(), Some(""));
        assert_eq!(find_package_root("tool", files(&[])), None);
    }

    #[test]
    fn test_find_fvm_project() {
        assert_eq!(
            find_fvm_project("app/bakai", files(&[".fvmrc"])).as_deref(),
            Some("")
        );
        assert_eq!(
            find_fvm_project("app/bakai", files(&["app/bakai/.fvm/fvm_config.json"])).as_deref(),
            Some("app/bakai")
        );
        assert_eq!(find_fvm_project("app/bakai", files(&[])), None);
    }

    #[test]
    fn test_paths() {
        assert_eq!(
            fvm_tool_path("/mib/", "", "dart"),
            "/mib/.fvm/flutter_sdk/bin/dart"
        );
        assert_eq!(
            fvm_tool_path("/mib", "app/bakai", "flutter"),
            "/mib/app/bakai/.fvm/flutter_sdk/bin/flutter"
        );
        assert_eq!(
            relative_to_root("/mib", "/mib/app/bakai/lib/main.dart").as_deref(),
            Some("app/bakai/lib/main.dart")
        );
        assert_eq!(relative_to_root("/mib", "/mib").as_deref(), Some(""));
        assert_eq!(relative_to_root("/mib", "/mibx/lib/main.dart"), None);
        assert_eq!(relative_to_root("/mib", "/other/main.dart"), None);
        assert_eq!(
            relative_to_root("/mib", "./lib/main.dart").as_deref(),
            Some("lib/main.dart")
        );
        assert_eq!(parent("app/bakai/lib/main.dart"), "app/bakai/lib");
        assert_eq!(parent("main.dart"), "");
    }

    #[test]
    fn test_test_files() {
        assert!(is_test_file("/mib/app/bakai/test/foo_test.dart?line=12"));
        assert!(!is_test_file("lib/main.dart"));
        assert!(is_integration_test("integration_test/app_test.dart"));
        assert!(is_integration_test(
            "/mib/app/integration_test/app_test.dart"
        ));
        assert!(!is_integration_test("test/app_test.dart"));
    }
}
