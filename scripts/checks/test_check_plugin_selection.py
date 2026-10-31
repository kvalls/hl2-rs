import unittest

from check_plugin_selection import unexpected_packages


class UnexpectedPackages(unittest.TestCase):
    TREE = "hl2-app v0.1.0\nhl2-debug v0.1.0\nhl2-bevy v0.1.0\n"

    def test_an_unselected_plugin_is_reported(self):
        self.assertEqual(unexpected_packages(self.TREE, []), ["hl2-debug"])

    def test_a_selected_plugin_is_allowed(self):
        self.assertEqual(unexpected_packages(self.TREE, ["debug-tools"]), [])


if __name__ == "__main__":
    unittest.main()
