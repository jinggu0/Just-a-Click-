from pathlib import Path
import sys
import unittest

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / 'scripts'))
import benchmark_llm as B  # noqa: E402


class ResponseFormatTests(unittest.TestCase):
    def test_auto_keeps_existing_contract_behavior(self):
        schema = {'type': 'object'}
        self.assertEqual(B.response_format_for('auto', True, schema),
                         {'type': 'json_object', 'schema': schema})
        self.assertEqual(B.response_format_for('auto', False, schema), {'type': 'json_object'})

    def test_explicit_modes(self):
        schema = {'type': 'object'}
        self.assertEqual(B.response_format_for('json', True, schema), {'type': 'json_object'})
        self.assertIsNone(B.response_format_for('none', True, schema))

    def test_unknown_mode_rejected(self):
        with self.assertRaises(ValueError):
            B.response_format_for('grammar', True, {})


if __name__ == '__main__':
    unittest.main()
