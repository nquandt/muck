#!/usr/bin/env python3
"""Turns a bench/run.sh JSON-lines results file into a markdown summary table.

Usage: summarize.py RESULTS.jsonl
"""
import json
import sys
from collections import defaultdict


def main() -> None:
    if len(sys.argv) != 2:
        print("usage: summarize.py RESULTS.jsonl", file=sys.stderr)
        sys.exit(1)

    rows = []
    with open(sys.argv[1]) as f:
        for line in f:
            line = line.strip()
            if line:
                rows.append(json.loads(line))

    if not rows:
        print("_No results — every tool runner may have been skipped (not installed?)._")
        return

    by_corpus = defaultdict(list)
    for row in rows:
        by_corpus[row["corpus"]].append(row)

    cold_by_tool = defaultdict(dict)  # corpus -> tool -> cold_ms
    for row in rows:
        cold_by_tool[row["corpus"]][row["tool"]] = row["cold_ms"]

    print("# muck benchmark results\n")

    for corpus, corpus_rows in by_corpus.items():
        print(f"## {corpus}\n")

        print("**Cold start** (index build / container start, or first-scan for index-less tools):\n")
        print("| Tool | Cold (ms) |")
        print("|---|---|")
        for tool, cold_ms in sorted(cold_by_tool[corpus].items(), key=lambda kv: kv[1]):
            print(f"| {tool} | {cold_ms} |")
        print()

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
