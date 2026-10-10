> [!IMPORTANT]
> Remove this line to confirm you've reviewed this PR before submitting.
# Zed Dart

A [Dart](https://dart.dev/) extension for [Zed](https://zed.dev).

## Recommended Configuration

To make the most of the Dart LSP in Zed, you can configure it to automatically organize imports and apply fixes on format.

### Settings (`settings.json`)

Add the following to your `settings.json` to enable features like organizing imports on save:

```json
{
  "languages": {
    "Dart": {
      "format_on_save": "on",
      "code_actions_on_format": {
        "source.organizeImports": true,
        "source.fixAll": true
      }
    }
  }
}
```

## Flutter SDK and FVM

The language server, the debug adapter and the tasks use the SDK that [FVM](https://fvm.app)
links into `.fvm/flutter_sdk` when the project has an `.fvmrc` (or `.fvm/fvm_config.json`),
and `dart` / `flutter` from `PATH` otherwise. Run `fvm install` if the link is missing.
The `lsp.dart.binary.path` setting and the `useFvm` debug option take precedence.

## Run and debug

The run button next to `main()`, `test()`, `testWidgets()` and `group()` offers:

- `flutter run` (also `--profile` and `--release`) and `flutter test` for the file, test or group;
- a debug session for `flutter run` and `flutter test`.

Tasks run from the nearest directory with a `pubspec.yaml`, so they work in monorepos and pub
workspaces. The same tasks (`flutter pub get`, `flutter clean`, `dart run build_runner build` /
`watch`, …) are in `task: spawn`. They need a POSIX `sh`.

In `.zed/debug.json`, `cwd` defaults to the package that contains `program`, and a `program` ending
in `_test.dart` is run by the test debug adapter:

```json
[
  {
    "label": "Bakai (debug)",
    "adapter": "Dart",
    "type": "flutter",
    "request": "launch",
    "program": "app/bakai/lib/main.dart",
    "toolArgs": ["--flavor=dev"]
  }
]
```

## Snippets

`stless`, `stful`, `stanim`, `inh`, `initS`, `dis`, `didChangeD`, `didUpdateW`, `build`, `test`,
`testW` and `group`.

## Analysis server options

The extension enables closing labels (`closingLabels`). Other
[initialization options](https://github.com/dart-lang/sdk/blob/main/pkg/analysis_server/tool/lsp_spec/README.md#initialization-options)
can be set in the settings, for example:

```json
{
  "lsp": {
    "dart": {
      "initialization_options": {
        "outline": true,
        "flutterOutline": true
      }
    }
  }
}
```

## Documentation

See:
- [Zed Dart Language Docs](https://zed.dev/docs/languages/dart)
- [Dart LSP Support Docs](https://github.com/dart-lang/sdk/blob/main/pkg/analysis_server/tool/lsp_spec/README.md)

## Development

To develop this extension, see the [Developing Extensions](https://zed.dev/docs/extensions/developing-extensions) section of the Zed docs.
