#!/usr/bin/env python3
"""Summarize T15 raw JSONL. Small samples are diagnostic, not a p95 gate."""
import json
import statistics
import sys
rows = [json.loads(line) for line in open(sys.argv[1]) if line.strip()]
metrics = [("cold_ready_ms",), ("warm_reconnect_ms",), ("rebind_ms",),
           ("list", "ms"), ("open", "ms"), ("open", "frame_rx"),
           ("open", "quic_rx"), ("prompt", "ack_ms"),
           ("prompt", "first_token_ms"), ("prompt", "quic_rx")]
print('| Path | Metric | 1.0.6 median | Candidate median | Delta | n A/B |')
print('|---|---|---:|---:|---:|---|')
for mode in ('direct', 'relay', 'impaired'):
    if mode == 'impaired':
        print('| impaired | all metrics | unmeasured | unmeasured | proxy bypass invalidates path | 0/0 |')
        continue
    for keys in metrics:
        values = []
        for arm in ('baseline', 'candidate'):
            group = []
            for row in rows:
                if row['mode'] != mode or not row['label'].startswith(arm):
                    continue
                value = row
                for key in keys:
                    value = value[key]
                group.append(value)
            values.append(group)
        a, b = values
        ma = statistics.median(a) if a else None
        mb = statistics.median(b) if b else None
        fmt = lambda x: 'unmeasured' if x is None else f'{x:.2f}'
        print(f'| {mode} | {".".join(keys)} | {fmt(ma)} | {fmt(mb)} | {fmt(mb-ma if ma is not None and mb is not None else None)} | {len(a)}/{len(b)} |')
