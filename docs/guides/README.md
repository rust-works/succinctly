# Guides

[Home](../../) > [Docs](../) > Guides

Practical how-to documentation for using and developing succinctly.

## User Guides

| Guide                                                     | Description                                                                   |
|-----------------------------------------------------------|-------------------------------------------------------------------------------|
| [API Guide](api.md)                                       | Using the library in your Rust projects                                       |
| [CLI Guide](cli.md)                                       | Command-line tool (jq/yq replacement)                                         |
| [Choosing between jq and `succinctly jq`](choosing-jq.md) | Where each tool is the better choice, with the report rows behind every claim |

## Developer Guides

| Guide                                                             | Description                                                                                                  |
|-------------------------------------------------------------------|--------------------------------------------------------------------------------------------------------------|
| [Developer Guide](developer.md)                                   | Contributing to the codebase                                                                                 |
| [Release Guide](release.md)                                       | Release process (for maintainers)                                                                            |
| [Reproducing the jq comparison](reproducing-the-jq-comparison.md) | Valid `dev compare jq` runs on any machine or a scripted EC2 instance, and merging the platforms             |
| [Differential bug hunting](bug-hunting.md)                        | Running a hunt for unknown jq/yq divergences: strategies, coverage map, harness rules, finding to merged fix |
| [The jq regex corpus](jq-regex-corpus.md)                         | The differential corpus that gates jq-mode regex changes (ADR-0031): running it, reading it, the baseline    |

## Quick Links

- **New to succinctly?** Start with [Getting Started](../getting-started/)
- **Looking for benchmarks?** See [Benchmarks](../benchmarks/)
- **Understanding internals?** See [Architecture](../architecture/)
