# potato (TechEmpower Framework Benchmarks)

## Description

[potato](https://github.com/fawdlstty/potato) is a high-performance, concise HTTP framework with HTTP/1.1, HTTP/2, and HTTP/3 support.

This directory is a drop-in TechEmpower FrameworkBenchmarks framework definition: copy it to `frameworks/Rust/potato/` inside a FrameworkBenchmarks checkout and run with the standard TFB toolchain (`tfb --test potato`, `tfb --verify potato`).

Implemented test types: JSON serialization (`/json`) and Plaintext (`/plaintext`). Database-backed test types (db / queries / fortunes / updates / cached-queries) are not implemented — potato declares `"database": "None"`.

> **Note (2026-11):** TechEmpower sunset the FrameworkBenchmarks project on 2026-03-24
> ([announcement](https://github.com/TechEmpower/FrameworkBenchmarks/issues/10932)).
> The upstream repository is archived and no further rounds will be published, so the
> official leaderboard cannot accept new entries anymore. These files keep potato
> TFB-compatible for self-hosted benchmark runs (`tfb --verify` / `tfb --test` still work
> against a FrameworkBenchmarks checkout) and for any community continuation.

## Local run (without the TFB toolchain)

```sh
cargo run --release          # listens on 0.0.0.0:8080 (optional port argument)
curl -v http://127.0.0.1:8080/plaintext
curl -v http://127.0.0.1:8080/json
```

Load can be generated with the repo's harness:

```sh
cd benchmark
cargo run --release -- bench 8080 128 5000 /plaintext "Hello, World!"
cargo run --release -- bench 8080 128 5000 /json '{"message":"Hello, World!"}'
```

Git Bash note: prefix the command with `MSYS_NO_PATHCONV=1`, otherwise MSYS rewrites
`/plaintext`-style path arguments into Windows paths.

## Test URLs

### JSON Encoding

```
http://localhost:8080/json
```

### Plaintext

```
http://localhost:8080/plaintext
```
