from pathlib import Path
import copy
import json
import sys
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / 'scripts'))
import lecture_eval as L  # noqa: E402

TRANSCRIPT = '\n'.join([
    '[s1 00:00] 오늘은 파일 입출력을 다룹니다.',
    '[s2 00:30] 중간고사는 10월 21일이고 범위는 3장까지입니다.',
    '[s3 01:00] open 함수로 파일을 엽니다. lc 함수로 오프셋을 옮깁니다.',
    '[s4 01:30] 과제는 다음 주쯤 제출하세요.',
])


def reference():
    return {
        'schema_version': 'lecture-reference-v1',
        'lecture_id': 'sysprog-2026-09-28',
        'course': '시스템 프로그래밍',
        'date': '2026-09-28',
        'permission': {'granted_by': '강의자', 'scope': '개인 평가용', 'confirmed_on': '2026-09-28'},
        'review': {'status': 'reviewed', 'reviewer': '수강생', 'notes': ''},
        'transcript_run': 'replay',
        'facts': [
            {'id': 'f1', 'text': 'open으로 파일을 연다', 'segments': ['s3'], 'importance': 'key'},
            {'id': 'f2', 'text': '오프셋은 lseek로 옮긴다', 'segments': ['s3'], 'importance': 'key'},
            {'id': 'f3', 'text': '오늘 주제는 파일 입출력', 'segments': ['s1'], 'importance': 'normal'},
        ],
        'notices': [
            {'kind': 'exam', 'content': '중간고사', 'date_text': '10월 21일', 'scope_text': '3장까지',
             'status': 'scheduled', 'segments': ['s2']},
            {'kind': 'assignment', 'content': '과제 제출', 'date_text': None, 'scope_text': None,
             'status': 'scheduled', 'segments': ['s4']},
        ],
        'code': [{'code': 'open()', 'segments': ['s3']}, {'code': 'lseek', 'segments': ['s3']}],
        'terms': [{'heard': 'lc 함수', 'correct': 'lseek', 'segments': ['s3']},
                  {'heard': '오픈', 'correct': 'open', 'segments': ['s3']}],
    }


def note(concepts, notices, code):
    return {
        'value': {
            'topic': {'content': '파일', 'source_refs': ['s1']},
            'concepts': [{'name': name, 'explanation': '설명', 'source_refs': refs} for name, refs in concepts],
            'examples': [], 'terms': [], 'review': [],
            'notices': notices,
            'code': [{'code': text, 'language': 'c', 'explanation': '', 'from_transcript': True,
                      'source_refs': ['s3']} for text in code],
        },
        'repairs': [],
    }


def notice(kind, refs, status='scheduled', date=None):
    return {'kind': kind, 'content': '공지', 'date_text': date, 'scope_text': None, 'status': status,
            'source_refs': refs}


def run(five=None, precise=None, mode='recording', after=250.0):
    return {
        'mode': mode,
        'after_recording_seconds': after,
        'five_minute_note': five,
        'precise_note': precise,
    }


class ReferenceTests(unittest.TestCase):
    def test_a_complete_reference_has_no_problems(self):
        self.assertEqual(L.reference_problems(reference()), [])

    def test_missing_permission_bad_enums_and_duplicate_ids_are_reported(self):
        value = reference()
        value['permission']['granted_by'] = ''
        value['notices'][0]['kind'] = 'quiz'
        value['facts'][1]['id'] = 'f1'
        value['facts'][2]['importance'] = 'high'
        value['review']['status'] = 'done'
        problems = L.reference_problems(value)
        self.assertEqual(len(problems), 5, problems)

    def test_the_schema_file_names_the_same_fields(self):
        schema = json.loads((ROOT / 'schemas' / 'lecture-reference-v1.json').read_text('utf-8'))
        self.assertEqual(sorted(schema['required']), sorted(reference().keys()))
        self.assertEqual(sorted(schema['$defs']['notice']['required']), sorted(reference()['notices'][0].keys()))


class ScoreTests(unittest.TestCase):
    def good_run(self):
        notices = [notice('exam', ['s2'], date='10월 21일'), notice('assignment', ['s4'])]
        five = note([('open 함수', ['s3'])], notices, ['open()', 'lseek'])
        precise = note([('open 함수', ['s3']), ('주제', ['s1'])], notices, ['open()', 'lseek'])
        return run(five, precise)

    def test_a_matching_run_passes(self):
        scores = L.score(reference(), self.good_run(), TRANSCRIPT, TRANSCRIPT)
        self.assertEqual(scores['notices']['recall'], 1.0)
        self.assertEqual(scores['notices']['invented'], [])
        self.assertEqual(scores['code']['recall'], 1.0)
        self.assertEqual(scores['code']['precision'], 1.0)
        self.assertEqual(scores['facts']['five_minute_note']['key_recall'], 1.0)
        self.assertEqual(scores['facts']['precise_note']['normal_recall'], 1.0)
        verdict = L.verdict(scores, power={'ac_power': True})
        self.assertTrue(verdict['passed'], verdict)

    def test_invented_missed_and_wrong_notices_fail(self):
        value = self.good_run()
        notices = [notice('exam', ['s2'], date='10월 22일'), notice('exam', ['s1']),
                   notice('assignment', ['s4'], status='cancelled')]
        value['five_minute_note']['value']['notices'] = notices
        scores = L.score(reference(), value, TRANSCRIPT, TRANSCRIPT)
        self.assertEqual(scores['notices']['recall'], 1.0)
        self.assertEqual(len(scores['notices']['invented']), 1)
        self.assertEqual(scores['notices']['date_errors'], 1)
        self.assertEqual(scores['notices']['status_errors'], 1)
        value['five_minute_note']['value']['notices'] = [notice('assignment', ['s2'])]
        scores = L.score(reference(), value, TRANSCRIPT, TRANSCRIPT)
        self.assertEqual(scores['notices']['recall'], 0.0)
        self.assertFalse(L.verdict(scores, power={'ac_power': True})['passed'])

    def test_code_ignores_spacing_and_case(self):
        value = self.good_run()
        value['precise_note']['value']['code'] = []
        value['five_minute_note']['value']['code'][0]['code'] = ' OPEN() '
        value['five_minute_note']['value']['code'].append(dict(value['five_minute_note']['value']['code'][0], code='fd'))
        scores = L.score(reference(), value, TRANSCRIPT, TRANSCRIPT)
        self.assertEqual(scores['code']['recall'], 1.0)
        self.assertAlmostEqual(scores['code']['precision'], 2 / 3)

    def test_terms_count_correct_forms_in_transcript_and_notes(self):
        value = self.good_run()
        value['precise_note']['value']['terms'] = [{'term_ko': '오프셋 이동', 'definition': 'lseek로 옮긴다',
                                                    'term_en': None, 'source_refs': ['s3']}]
        scores = L.score(reference(), value, TRANSCRIPT, TRANSCRIPT)
        self.assertEqual(scores['terms']['correct_in_transcript'], 0.5)
        self.assertEqual(scores['terms']['correct_in_notes'], 1.0)

    def test_key_fact_recall_counts_shared_segments_per_note(self):
        value = self.good_run()
        value['five_minute_note']['value']['concepts'] = [{'name': '주제', 'explanation': '설명', 'source_refs': ['s1']}]
        scores = L.score(reference(), value, TRANSCRIPT, TRANSCRIPT)
        self.assertEqual(scores['facts']['five_minute_note']['key_recall'], 0.0)
        self.assertEqual(scores['facts']['five_minute_note']['key_missed'], ['f1', 'f2'])
        self.assertEqual(scores['facts']['precise_note']['key_recall'], 1.0)
        self.assertFalse(L.verdict(scores, power={'ac_power': True})['passed'])

    def test_a_different_transcript_is_not_scored(self):
        with self.assertRaises(ValueError):
            L.score(reference(), self.good_run(), TRANSCRIPT, TRANSCRIPT + '\n[s5 02:00] 더')

    def test_time_is_judged_only_for_recordings_on_ac(self):
        scores = L.score(reference(), self.good_run(), TRANSCRIPT, TRANSCRIPT)
        self.assertEqual(L.verdict(scores, power={'ac_power': True})['checks']['after_recording'], True)
        self.assertIsNone(L.verdict(scores, power={'ac_power': False})['checks']['after_recording'])
        self.assertIsNone(L.verdict(scores, power=None)['checks']['after_recording'])
        slow = self.good_run()
        slow['after_recording_seconds'] = 301.0
        scores = L.score(reference(), slow, TRANSCRIPT, TRANSCRIPT)
        self.assertFalse(L.verdict(scores, power={'ac_power': True})['checks']['after_recording'])
        replay = self.good_run()
        replay['mode'] = 'replay'
        scores = L.score(reference(), replay, TRANSCRIPT, TRANSCRIPT)
        self.assertIsNone(L.verdict(scores, power={'ac_power': True})['checks']['after_recording'])

    def test_a_lecture_folder_is_scored_from_its_files(self):
        with tempfile.TemporaryDirectory() as directory:
            lecture = Path(directory)
            (lecture / 'runs' / 'replay').mkdir(parents=True)
            (lecture / 'reference.json').write_text(json.dumps(reference(), ensure_ascii=False), 'utf-8')
            (lecture / 'runs' / 'replay' / 'transcript.txt').write_text(TRANSCRIPT, 'utf-8')
            (lecture / 'runs' / 'replay' / 'lecture-pipeline.json').write_text(
                json.dumps(self.good_run(), ensure_ascii=False), 'utf-8')
            result = L.score_lecture(lecture)
            self.assertEqual(result['lecture_id'], 'sysprog-2026-09-28')
            self.assertEqual([entry['run'] for entry in result['runs']], ['replay'])
            self.assertTrue(result['runs'][0]['verdict']['checks']['notices'])
            dumped = json.dumps(L.summary([result]), ensure_ascii=False)
            for content in ['open으로 파일을 연다', '중간고사', '파일 입출력', '시스템 프로그래밍']:
                self.assertNotIn(content, dumped)


if __name__ == '__main__':
    unittest.main()
