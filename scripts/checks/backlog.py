#!/usr/bin/env python3
"""Structure of docs/optimization-backlog.md (run by architecture.py, so by validate.sh, CI and
the pre-commit hook; standard library only).

Every item must say how its gain will be measured: the items table has the columns in HEADER,
each row exactly those seven cells (Clarifications may be empty), a unique `O<number>` id, a known status, and non-empty Finding,
Measure and Next cells. Only a rejected item may leave its Measure as "-". The page keeps the
"Measuring an item" section the Measure cells draw on.
"""
import re
import sys
from pathlib import Path

PATH = Path(__file__).resolve().parents[2] / "docs" / "optimization-backlog.md"
HEADER = ["ID", "Area", "Status", "Finding", "Measure", "Next", "Clarifications"]
STATUSES = {"open", "measuring", "fixing", "partly done", "done", "rejected"}
RULES = "## Measuring an item"
ITEM = re.compile(r"O\d+")
PLACEHOLDER = re.compile(r"\b(todo|tbd)\b", re.I)


def cells(line):
    """A table row's cells, without the outer pipes."""
    return [cell.strip() for cell in line.strip().strip("|").split("|")]


def check(text):
    """Problems with the backlog's structure; empty when it is sound."""
    problems = []
    if RULES not in text:
        problems.append(f'missing the "{RULES[3:]}" section')
    lines = text.splitlines()
    header = next((i for i, line in enumerate(lines) if line.startswith("| ID |")), None)
    if header is None:
        return problems + ["no items table (a row starting with `| ID |`)"]
    if cells(lines[header]) != HEADER:
        problems.append(f"items table columns {cells(lines[header])}, expected {HEADER}")
    seen = set()
    for number, line in enumerate(lines[header + 2:], header + 3):
        if not line.startswith("|"):
            break
        row = cells(line)
        item = row[0] if row else "?"
        where = f"line {number} ({item})"
        if len(row) != len(HEADER):
            problems.append(f"{where}: {len(row)} cells, expected {len(HEADER)} (a stray `|`?)")
            continue
        values = dict(zip(HEADER, row))
        if not ITEM.fullmatch(item):
            problems.append(f"{where}: id must look like O12")
        elif item in seen:
            problems.append(f"{where}: duplicate id")
        seen.add(item)
        if values["Status"] not in STATUSES:
            problems.append(f"{where}: status {values['Status']!r}, expected one of {sorted(STATUSES)}")
        for column in ("Finding", "Measure", "Next"):
            if not values[column]:
                problems.append(f"{where}: empty {column}")
        if values["Measure"] == "-" and values["Status"] != "rejected":
            problems.append(f"{where}: only a rejected item may skip its Measure; name the scene "
                            "and metric its gain shows in")
        elif values["Status"] != "rejected" and PLACEHOLDER.search(values["Measure"]):
            problems.append(f"{where}: Measure cannot be a placeholder ({values['Measure']!r})")
    if not seen:
        problems.append("the items table has no items")
    return problems


def main():
    problems = check(PATH.read_text(encoding="utf-8"))
    for problem in problems:
        print(f"FAIL docs/optimization-backlog.md: {problem}")
    return 1 if problems else 0


if __name__ == "__main__":
    sys.exit(main())
