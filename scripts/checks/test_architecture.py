import unittest

from architecture import CONCRETE_PLUGIN


class ConcretePlugin(unittest.TestCase):
    def test_core_may_not_name_a_plugin(self):
        for text in ["hl2-debug = { workspace = true }", "use hl2_advanced_options::ui;",
                     '#[cfg(feature = "debug-tools")]', '#[cfg(feature = "advanced-options")]']:
            self.assertTrue(CONCRETE_PLUGIN.search(text), text)

    def test_unrelated_names_pass(self):
        for text in ["hl2_debug_overlay_helper", "pub mod extensions;", 'feature = "pure"']:
            self.assertFalse(CONCRETE_PLUGIN.search(text), text)


if __name__ == "__main__":
    unittest.main()
