#!/usr/bin/env python3
"""Turns a bench/run.sh JSON-lines results file into a markdown summary table.

Usage: summarize.py RESULTS.jsonl
"""
import json
import sys
from collections import defaultdict


def main() -> None:
    # Windows' default console/file encoding for redirected stdout isn't UTF-8, which mangles
    # the em-dash used below — force it explicitly rather than relying on the platform default.
    sys.stdout.reconfigure(encoding="utf-8")

    if len(sys.argv) != 2:
        print("usage: summarize.py RESULTS.jsonl", file=sys.stderr)
        sys.exit(1)

    all_rows = []
    with open(sys.argv[1]) as f:
        for line in f:
            line = line.strip()
            if line:
                all_rows.append(json.loads(line))

    if not all_rows:
        print("_No results — every tool runner may have been skipped (not installed?)._")
        return

    resource_rows = [r for r in all_rows if r.get("resource")]
    rows = [r for r in all_rows if not r.get("resource")]

    by_corpus = defaultdict(list)
    for row in rows:
        by_corpus[row["corpus"]].append(row)

    resource_by_corpus = defaultdict(list)
    for row in resource_rows:
        resource_by_corpus[row["corpus"]].append(row)

    cold_by_tool = defaultdict(dict)  # corpus -> tool -> cold_ms
    for row in rows:
        cold_by_tool[row["corpus"]][row["tool"]] = row["cold_ms"]

    print("# muck benchmark results\n")

    all_corpora = sorted(set(by_corpus) | set(resource_by_corpus))
    for corpus in all_corpora:
        corpus_rows = by_corpus.get(corpus, [])
        print(f"## {corpus}\n")

        if corpus_rows:
            print("**Cold start** (index build / container start, or first-scan for index-less tools):\n")
            print("| Tool | Cold (ms) |")
            print("|---|---|")
            for tool, cold_ms in sorted(cold_by_tool[corpus].items(), key=lambda kv: kv[1]):
                print(f"| {tool} | {cold_ms} |")
            print()

        if corpus in resource_by_corpus:
            print("**Warm-state memory & disk** (measured once after indexing, before any queries run):\n")
            print("| Tool | Memory (MB) | Disk (MB) | Note |")
            print("|---|---|---|---|")
            for row in sorted(resource_by_corpus[corpus], key=lambda r: r["tool"]):
                mem = row["mem_mb"] if row["mem_mb"] is not None else "—"
                note = row.get("note") or ""
                print(f"| {row['tool']} | {mem} | {row['disk_mb']} | {note} |")
            print()

        if corpus_rows:
            print("**Hot-path query latency, median of repeated runs (ms):**\n")
            queries = sorted({row["query"] for row in corpus_rows})
            tools = sorted({row["tool"] for row in corpus_rows})
            header = "| Query | " + " | ".join(tools) + " |"
            sep = "|---|" + "|".join(["---"] * len(tools)) + "|"
            print(header)
            print(sep)
            lookup = {(row["tool"], row["query"]): row["hot_ms"] for row in corpus_rows}
            for query in queries:
                cells = [str(lookup.get((tool, query), "—")) for tool in tools]
                print(f"| {query} | " + " | ".join(cells) + " |")
            print()


if __name__ == "__main__":
    main()
