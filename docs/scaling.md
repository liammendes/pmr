# Scaling and command history

Use `pmr start` for supervised services and `pmr run` for a large stream of one-shot commands. Both share the running-process limit. Completed `run` commands leave the active scheduler and remain searchable on disk; they do not accumulate in `ls` or `monit`.

## Run and find commands

```sh
pmr run /bin/echo --name greeting -- hello
pmr run /bin/sh --name build --wait -- -c 'cargo build'
pmr history
pmr history list --limit 50 --sort asc
pmr history list --search 'build AND cargo' --status succeeded
pmr history list --name build --namespace default --cursor 12345
pmr history show <uuid>
pmr history logs <uuid> --lines 20
pmr history cancel <uuid>
pmr logs --live --lines 10
```

Submission prints JSON containing a durable UUID and sequence number. It confirms acceptance, not success. `--wait` waits for completion, prints retained output, and returns the command's exit code (or 1 for cancellation or an incomplete capture). Supply `--id <uuid>` when retrying a submission whose response was lost. While its history record is retained, reusing that ID returns the original record, without executing it again. Commands inherit the submitting environment and working directory; use `--cwd` or `--interpreter` to override them. Input is closed, and commands run once without automatic restart. Background descendants are terminated when their command ends.

History returns JSON with `records` and `next_cursor`. Pass that cursor to the next request with the same filters and sort direction. Pages have a maximum of 1,000 records and use sequence comparisons instead of growing offsets. `--search` accepts SQLite FTS5 syntax over command text, name and namespace. It does not search output. Exact name, namespace, status, source and sequence lookups have database indexes. Broad full-text queries and combined filters still cost more as the number of matches grows.

The live viewer picks the newest matching service or queued command and retains its last result. Queued-command output refreshes every two seconds, uses the same bounded screen buffer as service logs, and supports exact name, namespace or UUID selectors plus `--out`/`--err`. Regex selectors apply to managed services. Normal `logs`, `ls`, `jlist` and `monit` remain service views; use `history` for the command archive.

## Capacity and backpressure

```sh
pmr limits
pmr limits --running 64 --queued 10000
```

The defaults permit 64 simultaneously running PMR child processes and 10,000 pending one-shot commands. Excess `run` work waits in submission order on disk; submission fails clearly when the queue is full. `start`, automatic restarts and reloads require an available slot and return or log an error when capacity is exhausted. A cluster reload needs a spare slot while its old worker continues serving traffic. Lowering the running limit lets existing children finish; it does not kill them.

These limits count PMR's immediate child processes, not threads or grandchildren. CPU, memory, file descriptors, OS process limits and disk throughput still set the machine's practical capacity. There is no claim that millions of processes can run simultaneously. Increase the limits deliberately for your workload. RPC concurrency is capped at 256 connections and requests at 2 MiB; clients should use backpressure and retry with the same command UUID after an uncertain submission.

## Storage, recovery and retention

The daemon creates `history.sqlite3` in `PMR_HOME`, with SQLite WAL journalling, full synchronous commits, an 8 MiB page cache, foreign keys and FTS5 indexes. It stores metadata and bounded output chunks in the database rather than creating two files per command. Only active commands have scheduler tasks in memory; pending command specifications stay on disk. The database is created with owner-only permissions. Pending specifications include the environment needed to execute the command; treat the database and its backups as sensitive. Logical deletion does not securely erase prior disk pages.

A daemon restart marks commands previously recorded as running as `interrupted`. They are never automatically replayed because they may already have performed a side effect. Commands still queued resume. Graceful shutdown terminates running command process groups and preserves the queue. After an ungraceful daemon or machine failure, reconcile external side effects and any surviving processes before explicitly resubmitting interrupted work. PMR does not promise exactly-once effects outside its database.

Each `pmr start` execution also records history metadata from this version onwards. Its output stays in the existing service log files. Existing files, saved services and old logs are left untouched: there is no speculative import of past executions from log text. Use normal `logs` for service output, including historical service files you retained yourself.

History and output age expiry are **disabled by default**. Command output has a separate default rolling tail of **1 MiB per command**, shared by stdout and stderr. Older chunks are discarded as new output arrives; metadata reports retained and dropped bytes. This bounds each command's output, not total archive size. Retained history will continue using disk until you opt into expiry or remove the home yourself.

```sh
pmr retention
pmr retention --output-bytes 8388608
# Optional: retain metadata for a year, output for a month, at most a million completed records.
pmr retention --history-days 365 --output-days 30 --max-records 1000000
pmr history prune --batch 100
# Disable age and count expiry again.
pmr retention --history-days 0 --output-days 0 --max-records 0
```

Settings persist in the database. The byte cap applies to commands launched after the change. Age policies use completion time and never prune queued or running records. A timer prunes at most 100 records per policy each minute; explicit pruning accepts batches up to 1,000. Enabling expiry makes old completed data eligible for irreversible removal. SQLite reuses freed pages; pruning does not promise the database file immediately shrinks. Keep a consistent SQLite backup, including any live WAL, before changing retention.

Output capture reads 4 KiB chunks and waits for persistence, applying pipe backpressure rather than growing an unbounded buffer. History output requests read at most about 1 MiB and report when the response is only a tail. The live screen retains 64 KiB and displays at most 4 KiB per line. Service log files keep their existing lifecycle and require separate rotation/retention. Full service inventory commands and custom metric sorts still scale with the number of registered services.

## Reproduce the checks

```sh
cargo test --locked -- --test-threads=1
cargo test --locked --lib million_record_history -- --ignored --nocapture
python3 benchmarks/history-smoke.py
```

The ignored benchmark populates one million persisted metadata records with all indexes enabled, checks index use, and times 100 latest-page and 100 exact-name queries. It is a storage/query benchmark, not one million executed operating-system commands. Integration tests exercise real concurrent command submissions, shared service capacity, full queues, cancellation, exit codes and restart recovery in isolated homes. The live-view tests also cover a million synthetic events and a 1 GiB sparse log. Capacity beyond those measured workloads remains unverified.

On the Apple M1 Max development machine, the one-million-record fixture took 126.86 seconds to populate. Its 200 bounded queries took 34.10 ms in total. A separate check completed 128 real commands with 64 running at once and an empty active command registry afterwards. These are single-machine checks, not maximum-capacity claims. [Raw results](../benchmarks/history-results-2026-10-16.json) record the workload and limitations.
