# Plan: jq Remaining Work

Remaining work to achieve full jq CLI and module system compatibility.

See [jq Language Reference](../reference/jq-language.md) for implemented features.

## CLI Enhancements (Low Priority)

These are CLI-level features, not expression language:

- [ ] `--argjson name value` - Pass JSON value as variable
- [ ] `--slurpfile name file` - Slurp file contents into variable
- [ ] `--rawfile name file` - Read file as raw string into variable
- [ ] `--jsonargs` - Treat remaining args as JSON values

## Module System Gaps

- [ ] `module {...}` metadata is parsed but not used at runtime
- [ ] `$__PROGDIR__` - module directory path variable
- [ ] Module-relative paths in import/include
