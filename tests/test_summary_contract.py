import copy
import importlib.util
import json
from pathlib import Path
import unittest

ROOT = Path(__file__).resolve().parents[1]
SPEC = importlib.util.spec_from_file_location('summary_contract', ROOT / 'scripts/summary_contract.py')
C = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(C)


class ContractTests(unittest.TestCase):
    def setUp(self):
        self.segments = {'s1': '지민이 재시도를 맡습니다. 마감일은 정하지 않았습니다.'}
        self.summary = {'schema_version': 'meeting-summary-v1', 'title': '회의', 'decisions': [],
            'actions': [{'content': '재시도 구현', 'owner': '지민', 'deadline': None, 'source_refs': ['s1']}],
            'open_questions': []}

    def test_explicit_null_accepted(self):
        self.assertEqual(C.validate_summary(json.dumps(self.summary), self.segments), self.summary)

    def test_person_named_mijeong_is_not_an_unknown_placeholder(self):
        self.summary['actions'][0]['owner'] = '미정'
        segments = {'s1': '미정이 재시도를 맡습니다. 기한은 정하지 않았습니다.'}
        self.assertEqual(C.validate_summary(json.dumps(self.summary), segments), self.summary)

    def test_unknown_deadline_requires_null_even_if_marker_is_in_source(self):
        self.summary['actions'][0]['deadline'] = '미정'
        segments = {'s1': '지민이 맡습니다. 기한은 미정입니다.'}
        with self.assertRaises(ValueError):
            C.validate_summary(json.dumps(self.summary), segments)

    def test_bad_outputs_rejected(self):
        mutations = [
            lambda s: s['actions'][0].pop('deadline'),
            lambda s: s['actions'][0].update(deadline='내일'),
            lambda s: s['actions'][0].update(owner='민수'),
            lambda s: s['actions'][0].update(source_refs=['s99']),
            lambda s: s['actions'][0].update(source_refs=[]),
            lambda s: s['actions'][0].update(source_refs=['s1', 's1']),
            lambda s: s['actions'][0].update(content='  '),
            lambda s: s.update(extra='unexpected'),
            lambda s: s.update(schema_version='v99')]
        for change in mutations:
            with self.subTest(change=mutations.index(change)):
                value = copy.deepcopy(self.summary)
                change(value)
                with self.assertRaises(ValueError):
                    C.validate_summary(json.dumps(value), self.segments)

    def test_duplicate_json_key_rejected(self):
        with self.assertRaisesRegex(ValueError, 'Duplicate JSON'):
            C.validate_summary('{"title":"a","title":"b"}', self.segments)

    def test_truncated_generation_rejected_even_when_json_is_complete(self):
        with self.assertRaisesRegex(ValueError, 'Incomplete'):
            C.validate_summary(json.dumps(self.summary), self.segments, 'length')

    def test_duplicate_input_id_rejected(self):
        with self.assertRaises(ValueError):
            C.parse_segments('[s1 00:00] 첫 문장\n[s1 00:01] 다른 문장')

    def test_previous_cpu_regression_is_rejected(self):
        old = json.loads((ROOT / 'evaluation/results/2026-09-17-cpu-smoke.json').read_text('utf-8'))
        fixture = json.loads((ROOT / 'evaluation/fixtures/meeting-smoke.json').read_text('utf-8'))
        for output in old['reviewed_outputs']:
            value = json.loads(output['content'])
            value['schema_version'] = 'meeting-summary-v1'
            with self.assertRaisesRegex(ValueError, 'missing required'):
                C.validate_summary(json.dumps(value), C.parse_segments(fixture['transcript']))


if __name__ == '__main__':
    unittest.main()
