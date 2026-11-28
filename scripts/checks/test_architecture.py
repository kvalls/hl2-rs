import tempfile
import unittest
from pathlib import Path

from architecture import CONCRETE_PLUGIN, text_bytes


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


class TextBytes(unittest.TestCase):
    def test_line_ends_do_not_change_the_size(self):
        with tempfile.TemporaryDirectory() as folder:
            lf, crlf = Path(folder, "lf.md"), Path(folder, "crlf.md")
            lf.write_bytes(b"- one rule\n- another\n")
            crlf.write_bytes(b"- one rule\r\n- another\r\n")
            self.assertEqual(text_bytes(lf), 21)
            self.assertEqual(text_bytes(crlf), 21)
