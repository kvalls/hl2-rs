import unittest

from backlog import PATH, check

GOOD = """# Optimization backlog

## Measuring an item

Pick the scene and metric.

## Items

| ID | Area | Status | Finding | Measure | Next | Clarifications |
|---|---|---|---|---|---|---|
| O1 | GPU | open | GPU busy | Static spawn: GPU per pass | Split passes | |
| O2 | Render | rejected | One camera only | - | Nothing to skip | |
"""


def with_row(row):
    return GOOD + row + "\n"


class Backlog(unittest.TestCase):
    def test_a_sound_table_passes(self):
        self.assertEqual(check(GOOD), [])

    def test_the_repository_backlog_passes(self):
        self.assertEqual(check(PATH.read_text(encoding="utf-8")), [])

    def test_every_item_must_say_how_it_is_measured(self):
        problems = check(with_row("| O3 | UI | open | Slow console |  | Cache rows | |"))
        self.assertTrue(any("empty Measure" in p for p in problems), problems)
        problems = check(with_row("| O3 | UI | open | Slow console | - | Cache rows | |"))
        self.assertTrue(any("only a rejected item" in p for p in problems), problems)
        problems = check(with_row("| O3 | UI | open | Slow console | TBD | Cache rows | |"))
        self.assertTrue(any("placeholder" in p for p in problems), problems)
        problems = check(with_row("| O3 | UI | done | Slow console | todo | Cache rows | |"))
        self.assertTrue(any("placeholder" in p for p in problems), problems)

    def test_rows_must_keep_the_columns(self):
        problems = check(with_row("| O3 | UI | open | Slow | split | Hud stage | Cache | |"))
        self.assertTrue(any("8 cells" in p for p in problems), problems)
        problems = check(with_row("| O3 | UI | open | Slow | Cache | |"))
        self.assertTrue(any("6 cells" in p for p in problems), problems)
        old = GOOD.replace("| Finding | Measure | Next |", "| Finding | Next |")
        self.assertTrue(any("columns" in p for p in check(old)), check(old))

    def test_clarifications_may_be_empty_or_filled(self):
        self.assertEqual(check(with_row("| O3 | UI | open | Slow | Hud stage | Cache | Keep the console font? |")), [])

    def test_ids_and_statuses_are_checked(self):
        problems = check(with_row("| O1 | UI | open | Again | Hud stage | Cache | |"))
        self.assertTrue(any("duplicate id" in p for p in problems), problems)
        problems = check(with_row("| X9 | UI | open | Bad id | Hud stage | Cache | |"))
        self.assertTrue(any("look like O12" in p for p in problems), problems)
        problems = check(with_row("| O3 | UI | started | Bad status | Hud stage | Cache | |"))
        self.assertTrue(any("status 'started'" in p for p in problems), problems)

    def test_the_measuring_rules_must_stay(self):
        problems = check(GOOD.replace("## Measuring an item", "## Notes"))
        self.assertTrue(any("Measuring an item" in p for p in problems), problems)


if __name__ == "__main__":
    unittest.main()
