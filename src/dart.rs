mod project;

use zed::lsp::CompletionKind;
use zed::settings::LspSettings;
use zed::{CodeLabel, CodeLabelSpan};
use zed_extension_api::serde_json::json;
use zed_extension_api::{
    self as zed, current_platform, serde_json, DebugAdapterBinary, DebugScenario,
    DebugTaskDefinition, Os, Result, StartDebuggingRequestArguments,
    StartDebuggingRequestArgumentsRequest, TaskTemplate, Worktree,
};

/// Marks the tasks from `languages/dart/tasks.json`. They run
/// `sh -c <script> dart-task <dir> <worktree root> <tool> <args>...`, where the
/// script runs the tool from the nearest package root, using the project's
/// FVM SDK if it has one. Zed joins a task's command and arguments into a
/// shell command line without quoting them, so the arguments there are
/// already quoted.
const TASK_MARKER: &str = "dart-task";

fn tool_binary(debug_mode: &str) -> &'static str {
    let (os, _) = current_platform();
    match (debug_mode, os) {
        ("flutter", Os::Windows) => "flutter.bat",
        ("flutter", _) => "flutter",
        (_, Os::Windows) => "dart.bat",
        (_, _) => "dart",
    }
}

/// The tool and its arguments of a task from `languages/dart/tasks.json`.
fn dart_task_command(task: &TaskTemplate) -> Option<Vec<&str>> {
    if task.command != "sh" {
        return None;
    }
    match task.args.as_slice() {
        [flag, _script, marker, _dir, _root, command @ ..]
            if flag == "-c" && marker == TASK_MARKER =>
        {
            Some(command.iter().map(String::as_str).collect())
        }
        _ => None,
    }
}

fn unquote(arg: &str) -> &str {
    arg.strip_prefix('"')
        .and_then(|arg| arg.strip_suffix('"'))
        .unwrap_or(arg)
}

fn file_exists(worktree: &Worktree, path: &str) -> bool {
    worktree.read_text_file(path).is_ok()
}

struct DartBinary {
    pub path: String,
    pub args: Option<Vec<String>>,
}

struct DartExtension;

impl DartExtension {
    fn language_server_binary(
        &mut self,
        _language_server_id: &zed::LanguageServerId,
        worktree: &zed::Worktree,
    ) -> Result<DartBinary> {
        let binary_settings = LspSettings::for_worktree("dart", worktree)
            .ok()
            .and_then(|lsp_settings| lsp_settings.binary);
        let binary_args = binary_settings
            .as_ref()
            .and_then(|binary_settings| binary_settings.arguments.clone());

        if let Some(path) = binary_settings.and_then(|binary_settings| binary_settings.path) {
            return Ok(DartBinary {
                path,
                args: binary_args,
            });
        }

        if let Some(fvm_project) = project::find_fvm_project("", |path| file_exists(worktree, path))
        {
            return Ok(DartBinary {
                path: project::fvm_tool_path(
                    &worktree.root_path(),
                    &fvm_project,
                    tool_binary("dart"),
                ),
                args: binary_args,
            });
        }

        if let Some(path) = worktree.which("dart") {
            return Ok(DartBinary {
                path,
                args: binary_args,
            });
        }

        Err(
            "dart must be installed from dart.dev/get-dart or pointed to by the LSP binary settings"
                .to_string(),
        )
    }
}

impl zed::Extension for DartExtension {
    fn new() -> Self {
        Self
    }

    /// ref:
    /// https://github.com/zed-industries/zed/blob/main/crates/dap_adapters/src/gdb.rs
    fn get_dap_binary(
        &mut self,
        _adapter_name: String,
        config: DebugTaskDefinition,
        _user_provided_debug_adapter_path: Option<String>,
        worktree: &Worktree,
    ) -> Result<DebugAdapterBinary, String> {
        let user_config: serde_json::Value = serde_json::from_str(&config.config)
            .map_err(|e| format!("Failed to parse debug config: {e}"))?;

        let root = worktree.root_path();

        let mut program = user_config
            .get("program")
            .and_then(|v| v.as_str())
            .unwrap_or("lib/main.dart")
            .to_string();

        let args = user_config
            .get("args")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str())
                    .map(|s| s.to_string())
                    .collect::<Vec<String>>()
            })
            .unwrap_or_default();

        let use_fvm = user_config.get("useFvm").and_then(|v| v.as_bool());

        // Get debug_mode from user config (flutter or dart)
        let debug_mode = user_config
            .get("type")
            .and_then(|v| v.as_str())
            .filter(|s| !s.trim().is_empty())
            .ok_or_else(|| "type is required and cannot be empty or null".to_string())?;

        let tool = tool_binary(debug_mode);

        // Without an explicit `cwd`, run from the package that contains the
        // program, so apps inside a monorepo can be launched by path alone.
        let cwd = match user_config.get("cwd").and_then(|v| v.as_str()) {
            Some(cwd) => cwd.to_string(),
            None => {
                let package_root =
                    project::relative_to_root(&root, project::without_query(&program)).and_then(
                        |path| {
                            project::find_package_root(project::parent(&path), |path| {
                                file_exists(worktree, path)
                            })
                        },
                    );
                match package_root {
                    Some(package_root) => {
                        if !program.starts_with('/') {
                            program = project::absolute(&root, &program);
                        }
                        project::absolute(&root, &package_root)
                    }
                    None => root.clone(),
                }
            }
        };

        let test_mode = project::is_test_file(&program);

        let mut arguments = vec!["debug_adapter".to_string()];
        if test_mode {
            arguments.push("--test".to_string());
        }

        let fvm_project = match use_fvm {
            Some(_) => None,
            None => project::relative_to_root(&root, &cwd).and_then(|dir| {
                project::find_fvm_project(&dir, |path| file_exists(worktree, path))
            }),
        };

        let command = if use_fvm == Some(true) {
            let fvm_path = worktree.which("fvm").ok_or_else(|| {
                "fvm not found in PATH. Install fvm or set useFvm to false.".to_string()
            })?;
            arguments.insert(0, tool.to_string());
            fvm_path
        } else if let Some(fvm_project) = fvm_project {
            project::fvm_tool_path(&root, &fvm_project, tool)
        } else {
            worktree.which(tool).ok_or_else(|| {
                format!("{tool} not found in PATH. Install from dart.dev or flutter.dev.")
            })?
        };

        // Unit tests run on the host's `flutter_tester`; only integration
        // tests need the device that Zed may have filled in.
        let device_id = user_config
            .get("deviceId")
            .and_then(|v| v.as_str())
            .filter(|_| !test_mode || project::is_integration_test(&program));

        let platform = user_config
            .get("platform")
            .and_then(|v| v.as_str())
            .unwrap_or("web");

        let request = user_config
            .get("request")
            .and_then(|v| v.as_str())
            .unwrap_or("launch");

        let vm_service_uri = user_config.get("vmServiceUri").and_then(|v| v.as_str());

        let stop_on_entry = user_config
            .get("stopOnEntry")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        let flutter_mode = user_config
            .get("flutterMode")
            .and_then(|v| v.as_str())
            .unwrap_or("debug");

        let mut tool_args = user_config
            .get("toolArgs")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str())
                    .map(|s| s.to_string())
                    .collect::<Vec<String>>()
            })
            .unwrap_or_default();

        // Flutter's DAP does not read the top-level `deviceId` field, so the
        // device id must be forwarded via `toolArgs` (`flutter run -d <id>`),
        // which is the only path the DAP actually honors. Only inject when the
        // user set `deviceId` explicitly and did not already pass a device via
        // `toolArgs` — otherwise leave selection to Flutter's auto-pick.
        if debug_mode == "flutter" {
            if let Some(id) = device_id {
                let has_device_arg = tool_args
                    .iter()
                    .any(|arg| arg == "-d" || arg.starts_with("--device-id"));
                if !has_device_arg {
                    tool_args.push("-d".to_string());
                    tool_args.push(id.to_string());
                }
            }
        }

        let env = user_config
            .get("env")
            .and_then(|v| v.as_object())
            .map(|obj| {
                obj.iter()
                    .map(|(k, v)| (k.clone(), v.as_str().unwrap_or_default().to_string()))
                    .collect::<Vec<(String, String)>>()
            })
            .unwrap_or_default();

        // Use debug_mode ("flutter"/"dart") rather than tool binary name so that
        // the config is correct on all platforms (e.g. not "flutter.bat" on Windows).
        let mut config_json = json!({
            "type": debug_mode,
            "request": request,
            "program": program,
            "cwd": cwd.clone(),
            "args": args,
            "flutterMode": flutter_mode,
            "platform": platform,
            "stopOnEntry": stop_on_entry,
            "sendLogsToClient": true
        });

        if let Some(uri) = vm_service_uri {
            config_json["vmServiceUri"] = json!(uri);
        }

        // Kept for forward-compat in case Flutter's DAP ever reads it; only
        // emitted when the user set a device explicitly.
        if let Some(id) = device_id {
            config_json["deviceId"] = json!(id);
        }

        if !tool_args.is_empty() {
            config_json["toolArgs"] = json!(tool_args);
        }

        let debug_adapter_binary = DebugAdapterBinary {
            command: Some(command),
            arguments,
            envs: env,
            cwd: Some(cwd),
            connection: None,
            request_args: StartDebuggingRequestArguments {
                configuration: config_json.to_string(),
                request: match request {
                    "attach" => StartDebuggingRequestArgumentsRequest::Attach,
                    _ => StartDebuggingRequestArgumentsRequest::Launch,
                },
            },
        };
        Result::Ok(debug_adapter_binary)
    }

    fn dap_request_kind(
        &mut self,
        _adapter_name: String,
        config: serde_json::Value,
    ) -> Result<StartDebuggingRequestArgumentsRequest, String> {
        match config.get("request") {
            Some(v) if v == "launch" => Ok(StartDebuggingRequestArgumentsRequest::Launch),
            Some(v) if v == "attach" => Ok(StartDebuggingRequestArgumentsRequest::Attach),
            Some(value) => Err(format!(
                "Unexpected value for `request` key in Dart debug adapter configuration: {value:?}"
            )),
            None => {
                Err("Missing required `request` field in Dart debug adapter configuration".into())
            }
        }
    }

    fn dap_locator_create_scenario(
        &mut self,
        _locator_name: String,
        build_task: TaskTemplate,
        resolved_label: String,
        debug_adapter_name: String,
    ) -> Option<DebugScenario> {
        if debug_adapter_name != "Dart" {
            return None;
        }
        let command = dart_task_command(&build_task)?;
        let program = match command.as_slice() {
            ["flutter", "run", "-t", program] | ["flutter", "test", program] => unquote(program),
            _ => return None,
        };
        let config = json!({
            "type": "flutter",
            "request": "launch",
            "program": program,
        });
        Some(DebugScenario {
            label: resolved_label,
            adapter: debug_adapter_name,
            build: None,
            config: config.to_string(),
            tcp_connection: None,
        })
    }

    fn language_server_command(
        &mut self,
        language_server_id: &zed::LanguageServerId,
        worktree: &zed::Worktree,
    ) -> Result<zed::Command> {
        let dart_binary = self.language_server_binary(language_server_id, worktree)?;

        Ok(zed::Command {
            command: dart_binary.path,
            args: dart_binary.args.unwrap_or_else(|| {
                vec!["language-server".to_string(), "--protocol=lsp".to_string()]
            }),
            env: Default::default(),
        })
    }

    fn language_server_initialization_options(
        &mut self,
        _language_server_id: &zed::LanguageServerId,
        _worktree: &zed::Worktree,
    ) -> Result<Option<serde_json::Value>> {
        // Zed merges `lsp.dart.initialization_options` from the settings on
        // top of these. `outline` and `flutterOutline` stay off until Zed
        // handles their notifications: unhandled ones are logged in full.
        Ok(Some(json!({
            "closingLabels": true,
        })))
    }

    fn language_server_workspace_configuration(
        &mut self,
        _language_server_id: &zed::LanguageServerId,
        worktree: &zed::Worktree,
    ) -> Result<Option<serde_json::Value>> {
        let settings = LspSettings::for_worktree("dart", worktree)
            .ok()
            .and_then(|lsp_settings| lsp_settings.settings.clone())
            .unwrap_or_default();

        Ok(Some(serde_json::json!({
            "dart": settings
        })))
    }

    fn label_for_completion(
        &self,
        _language_server_id: &zed::LanguageServerId,
        completion: zed::lsp::Completion,
    ) -> Option<CodeLabel> {
        let arrow = " → ";

        match completion.kind? {
            CompletionKind::Class => Some(CodeLabel {
                filter_range: (0..completion.label.len()).into(),
                spans: vec![CodeLabelSpan::literal(
                    completion.label,
                    Some("type".into()),
                )],
                code: String::new(),
            }),
            CompletionKind::Function | CompletionKind::Constructor | CompletionKind::Method => {
                let mut parts = completion.detail.as_ref()?.split(arrow);
                let (name, _) = completion.label.split_once('(')?;
                let parameter_list = parts.next()?;
                let return_type = parts.next()?;
                let fn_name = " a";
                let fat_arrow = " => ";
                let call_expr = "();";

                let code =
                    format!("{return_type}{fn_name}{parameter_list}{fat_arrow}{name}{call_expr}");

                let parameter_list_start = return_type.len() + fn_name.len();

                Some(CodeLabel {
                    spans: vec![
                        CodeLabelSpan::code_range(
                            code.len() - call_expr.len() - name.len()..code.len() - call_expr.len(),
                        ),
                        CodeLabelSpan::code_range(
                            parameter_list_start..parameter_list_start + parameter_list.len(),
                        ),
                        CodeLabelSpan::literal(arrow, None),
                        CodeLabelSpan::code_range(0..return_type.len()),
                    ],
                    filter_range: (0..name.len()).into(),
                    code,
                })
            }
            CompletionKind::Property => {
                let class_start = "class A {";
                let get = " get ";
                let property_end = " => a; }";
                let ty = completion.detail?;
                let name = completion.label;

                let code = format!("{class_start}{ty}{get}{name}{property_end}");
                let name_start = class_start.len() + ty.len() + get.len();

                Some(CodeLabel {
                    spans: vec![
                        CodeLabelSpan::code_range(name_start..name_start + name.len()),
                        CodeLabelSpan::literal(arrow, None),
                        CodeLabelSpan::code_range(class_start.len()..class_start.len() + ty.len()),
                    ],
                    filter_range: (0..name.len()).into(),
                    code,
                })
            }
            CompletionKind::Variable => {
                let name = completion.label;

                Some(CodeLabel {
                    filter_range: (0..name.len()).into(),
                    spans: vec![CodeLabelSpan::literal(name, Some("variable".into()))],
                    code: String::new(),
                })
            }
            _ => None,
        }
    }
}

zed::register_extension!(DartExtension);

#[cfg(test)]
mod tests {
    use super::*;
    use zed::Extension as _;

    fn tasks() -> Vec<TaskTemplate> {
        let tasks: serde_json::Value =
            serde_json::from_str(include_str!("../languages/dart/tasks.json")).unwrap();
        tasks
            .as_array()
            .unwrap()
            .iter()
            .map(|task| TaskTemplate {
                label: task["label"].as_str().unwrap().to_string(),
                command: task["command"].as_str().unwrap().to_string(),
                args: task["args"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|arg| arg.as_str().unwrap().to_string())
                    .collect(),
                env: Vec::new(),
                cwd: None,
            })
            .collect()
    }

    fn scenario_config(label: &str) -> Option<serde_json::Value> {
        let task = tasks()
            .into_iter()
            .find(|task| task.label == label)
            .unwrap();
        DartExtension
            .dap_locator_create_scenario(
                "dart".to_string(),
                task,
                label.to_string(),
                "Dart".to_string(),
            )
            .map(|scenario| serde_json::from_str(&scenario.config).unwrap())
    }

    #[test]
    fn test_all_tasks_use_the_wrapper() {
        for task in tasks() {
            assert!(dart_task_command(&task).is_some(), "{}", task.label);
        }
    }

    /// Runs a task the way Zed does: substitutes the variables, joins the
    /// command and arguments with spaces and hands the line to a shell.
    fn run_task(label: &str, tool: &[&str], variables: &[(&str, &str)]) -> String {
        let task = tasks()
            .into_iter()
            .find(|task| task.label == label)
            .unwrap();
        let mut args = task.args.clone();
        let tool_index = 5;
        args.splice(
            tool_index..=tool_index,
            tool.iter().map(|arg| arg.to_string()),
        );
        let mut line = std::iter::once(task.command.clone())
            .chain(args)
            .collect::<Vec<_>>()
            .join(" ");
        for (name, value) in variables {
            line = line
                .replace(&format!("${{{name}:}}"), value)
                .replace(&format!("${name}"), value);
        }
        let output = std::process::Command::new("sh")
            .args(["-c", &line])
            .output()
            .unwrap();
        assert!(output.status.success(), "{label}: {output:?}");
        String::from_utf8(output.stdout).unwrap()
    }

    #[test]
    fn test_tasks_run_from_the_package_root() {
        let root = std::env::temp_dir().join(format!("zed dart tasks {}", std::process::id()));
        let package = root.join("app dir/bakai");
        std::fs::create_dir_all(package.join("test")).unwrap();
        std::fs::write(package.join("pubspec.yaml"), "name: bakai\n").unwrap();
        let root_path = root.to_str().unwrap();
        let test_dir = package.join("test");
        let test_file = test_dir.join("a_test.dart");
        let variables = [
            ("ZED_DIRNAME", test_dir.to_str().unwrap()),
            ("ZED_WORKTREE_ROOT", root_path),
            ("ZED_FILE", test_file.to_str().unwrap()),
            ("ZED_ROW", "3"),
        ];

        assert_eq!(
            run_task(
                "flutter test $ZED_CUSTOM_DART_TEST_NAME",
                &["printf", "'%s|'"],
                &variables
            ),
            format!("test|{}?line=3|", test_file.display())
        );
        assert_eq!(
            run_task("flutter pub get", &["sh", "-c", "pwd"], &variables).trim_end(),
            package.to_str().unwrap()
        );
        assert_eq!(
            run_task(
                "flutter pub get",
                &["sh", "-c", "pwd"],
                &[("ZED_DIRNAME", ""), ("ZED_WORKTREE_ROOT", root_path)]
            )
            .trim_end(),
            root_path
        );

        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn test_locator_debugs_flutter_run_and_test_tasks() {
        assert_eq!(
            scenario_config("flutter run $ZED_STEM"),
            Some(json!({ "type": "flutter", "request": "launch", "program": "$ZED_FILE" }))
        );
        assert_eq!(
            scenario_config("flutter test $ZED_STEM"),
            Some(json!({ "type": "flutter", "request": "launch", "program": "$ZED_FILE" }))
        );
        assert_eq!(
            scenario_config("flutter test $ZED_CUSTOM_DART_TEST_NAME"),
            Some(json!({
                "type": "flutter",
                "request": "launch",
                "program": "$ZED_FILE?line=$ZED_ROW",
            }))
        );
        assert_eq!(scenario_config("flutter run $ZED_STEM --profile"), None);
        assert_eq!(scenario_config("flutter pub get"), None);
    }

    #[test]
    fn test_locator_ignores_other_adapters_and_tasks() {
        let task = tasks()
            .into_iter()
            .find(|task| task.label == "flutter run $ZED_STEM")
            .unwrap();
        assert!(DartExtension
            .dap_locator_create_scenario(
                "dart".to_string(),
                task,
                String::new(),
                "CodeLLDB".to_string()
            )
            .is_none());

        let task = TaskTemplate {
            label: "flutter run".to_string(),
            command: "flutter".to_string(),
            args: vec!["run".to_string()],
            env: Vec::new(),
            cwd: None,
        };
        assert!(DartExtension
            .dap_locator_create_scenario(
                "dart".to_string(),
                task,
                String::new(),
                "Dart".to_string()
            )
            .is_none());
    }
}
