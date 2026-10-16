# Scaling and retention

PMR can supervise successive commands, but it is not currently a database for
millions or billions of retained command results. The number of commands run over
time and the number of processes or records kept at once are different limits.

## What the live viewer bounds

`pmr logs --live` keeps one selected process and its latest output. It retains at
most 64 KiB of output text, caps individual displayed lines at 4 KiB, and keeps at
most the requested number of lines. Its incoming queue has 128 event slots and
the screen refreshes at most ten times per second. The queue limit counts events,
not bytes; unusually large individual event payloads can still use more memory.

Reading existing output seeks into the last 64 KiB of each selected log file.
It does not load the whole log. Output from older processes does not replace the
selected process's result. Deleting the selected process keeps its last result
visible until a newer process takes over.

Updated daemons answer `getLatestLogProcess` with one small record, without
refreshing every process's metrics or serializing its environment. For the default
`all` selector, this reads the last entry in the process ID index. Name, namespace
and regex selectors may scan backwards until a match is found. The viewer also
checks this lookup every two seconds. A bus-overflow notification reloads bounded
file tails to recover missed output; normal metadata refreshes preserve event order.

An already-running older daemon is supported: the viewer reads its inventory once
at startup, then follows events. That compatibility path does not have the new
indexed lookup or periodic recovery. Installing a binary does not automatically
restart an existing daemon and its applications.

## Other bounded reads

`ls` with the default ID ordering, `--sort asc`, or `--sort desc` selects the
requested IDs in the daemon before refreshing their metrics or copying their
environments. The response contains the selected rows and a total count. Older
daemons and legacy field sorts such as `--sort name:asc` use the full-inventory
compatibility path. `--limit 0`, `jlist`, and `prettylist` explicitly return all
records.

Ordinary log tailing locates the requested final lines using 8 KiB backwards
reads, then streams them forward one line at a time. It no longer loads an
entire log file. An individual unusually long line can still require a large
buffer in this text-output mode.

`monit` loads bounded tails only for its selected process, keeps one log cache,
and processes at most 128 queued events per frame. Its process inventory is
still a complete list.

## Verification

Automated checks feed one million synthetic sequential process events through
the viewer state and verify that only the latest result remains. Another check
uses a 1 GiB sparse log file to verify bounded tail reading. These are viewer
checks, not one million launched commands, a throughput benchmark, or evidence
that PMR can retain one million processes.

An isolated macOS smoke test also kept 32 real processes online while 64 short
commands completed, then verified all 96 records and the 25-row default list.
This is a regression check, not a maximum-capacity benchmark.

## What still limits command history

- The daemon retains process records, including environments, in an in-memory
  map. Stopped processes remain there until deleted.
- Full inventory, several lifecycle paths, custom field sorts and `monit` still
  scan or copy that map. Selecting log files by modification time also scans the
  current inventory and checks the candidate files.
- Process IDs are 32-bit and can be reused after the registry is emptied. They
  are not durable, globally unique history IDs.
- Deleting a process does not delete its log files. Log storage needs a separate
  retention policy.

## Concurrent process capacity

The target includes many simultaneously running commands as well as a large
history. The safe concurrency depends on the commands themselves, available
CPU and RAM, file descriptors, process limits, and disk throughput. A production
scheduler needs explicit admission limits, a bounded pending queue, and reserved
capacity for the supervisor to remain responsive when child processes are busy.
PMR does not yet enforce that resource budget or promise a fixed concurrency.

## Work needed for a large command archive

Keep the active supervisor registry small and move completed-command metadata
to an indexed, paginated store. Give historical runs durable IDs distinct from
the supervisor's process IDs. Store output separately with byte and age limits;
do not keep one indefinitely growing file per historical command. Archive only
after a run is complete and its output is durable.

The retention policy must specify what users need to search, how long results
must remain, storage limits, and whether expired output may be removed. Benchmark
actual command submission, completion, recovery and history queries at the
chosen retention size before claiming a scale target. The live-view changes do
not enable automatic pruning or delete existing history.
