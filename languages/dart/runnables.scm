; Entry point of a Flutter app or Dart program
((program
  (function_signature
    name: (identifier) @run))
  (#eq? @run "main")
  (#set! tag flutter-main))

; Entry point of a test file
((program
  (import_or_export
    (library_import
      (import_specification
        (configurable_uri
          (uri
            (string_literal) @_import)))))
  (function_signature
    name: (identifier) @run))
  (#match? @_import "^['\"]package:(flutter_test|test|integration_test)/")
  (#eq? @run "main")
  (#set! tag flutter-test-main))

; A test or a group of tests, at any depth
((expression_statement
  (identifier) @run
  .
  (selector
    (argument_part
      (arguments
        .
        (argument
          (string_literal) @DART_TEST_NAME)))))
  (#any-of? @run "test" "testWidgets" "group")
  (#set! tag flutter-test))
