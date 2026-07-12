# Architecture Decision Records

[Home](../../) > [Docs](../) > Architecture Decision Records

This directory contains the Architecture Decision Records (ADRs) for the succinctly project.

An ADR is a short document that captures a single significant architectural or design decision
along with its context and consequences. ADRs record *why* the system is shaped the way it is —
and, just as importantly, which approaches were considered and **rejected** and why. They
complement the [knowledge map](../index.md), which explains *how the system works today*.

For more background on the practice, see
[Documenting Architecture Decisions](https://cognitect.com/blog/2011/11/15/documenting-architecture-decisions)
by Michael Nygard.

## Status Legend

| Emoji | Status     | Meaning                               |
|-------|------------|---------------------------------------|
| 🟡    | Proposed   | Under discussion, not yet agreed upon |
| ✅    | Accepted   | Agreed and in effect                  |
| ❌    | Deprecated | No longer applies                     |
| 🔄    | Superseded | Replaced by a newer ADR               |

## Inventory

| ADR                     | Status      | Date       | Title                                                  |
|-------------------------|-------------|------------|--------------------------------------------------------|
| [ADR-0000](adr-0000.md) | ✅ Accepted | 2026-07-12 | Use Architecture Decision Records                      |
| [ADR-0001](adr-0001.md) | ✅ Accepted | 2026-07-12 | Semi-Indexing over DOM Parsing                         |
| [ADR-0002](adr-0002.md) | ✅ Accepted | 2026-07-12 | Table-Driven PFSM as the Portable JSON Parser          |
| [ADR-0003](adr-0003.md) | ✅ Accepted | 2026-07-12 | Oracle Parser for YAML                                 |
| [ADR-0004](adr-0004.md) | ✅ Accepted | 2026-07-12 | Reject Software Prefetching for Large YAML Files (P2.6) |
| [ADR-0005](adr-0005.md) | ✅ Accepted | 2026-07-12 | Reject SIMD Threshold Tuning (P2.8)                    |
| [ADR-0006](adr-0006.md) | ✅ Accepted | 2026-07-12 | Reject Branchless Character Classification (P3)        |
| [ADR-0007](adr-0007.md) | ✅ Accepted | 2026-07-12 | Reject the Flow-Collection SIMD Fast Path (P5)         |
| [ADR-0008](adr-0008.md) | ✅ Accepted | 2026-07-12 | Reject BMI2 Quote-Indexing for YAML (P6)               |
| [ADR-0009](adr-0009.md) | ✅ Accepted | 2026-07-12 | Reject a Parse-Time Newline Index (P7)                 |
| [ADR-0010](adr-0010.md) | ✅ Accepted | 2026-07-12 | Reject AVX-512 SIMD Variants (P8)                      |

The inventory is maintained by the [`update-adr-inventory`](../../.claude/skills/update-adr-inventory/SKILL.md)
skill, which scans `adr-*.md` for the title and status and derives the date from git history.
Run it whenever an ADR is added or its status changes.
